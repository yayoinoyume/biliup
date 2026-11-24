pub mod app_state;
pub mod compat;
// 阶段替换：C/D 组插件仍按源仓库的 crate::cluster / crate::history / crate::webui 路径引用，
// 这里先指向 compat 薄适配层，Phase 2/5 真实模块迁入后改为直接声明。
pub(crate) use compat::{cluster, history, notify, webui};
pub mod config;
pub mod credential_bridge;
pub mod events;
pub(crate) mod logging;
pub mod plugins;
pub mod runtime;
pub mod storage;
pub mod ui_events;
#[cfg(test)]
pub(crate) mod test_support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

pub use config::{load_config, Config};
pub use credential_bridge::{
    load_credentials, RelayBiliAccountSource, RelayBiliCredentials, RelayBiliCredentialsError,
};
pub use storage::Store;
pub use compat::webui::state::{
    get_status_cache, refresh_status_cache_config_from, request_status_refresh, status_snapshot,
    update_status_cache, update_status_cache_with, wait_status_refresh_notified, BiliStatus,
    NicoStatus, PriorityChannelStatus, StatusData, TwStatus, YtStatus,
};

pub const DB_FILE: &str = storage::DB_FILE;
pub const SQLITE_DRIVER: &str = "rusqlite/bundled";
const STATE_DISABLED: &str = "disabled";
const STATE_READY: &str = "ready";
const STATE_STOPPED: &str = "stopped";
/// 停机时等待监控线程收尾的上限；超时后由 shutdown 兜底重跑清理。
const MONITOR_STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// 监控循环停机结果：区分「没有在跑」「确认停干净」「等待超时、清理未确认」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorStopOutcome {
    NotRunning,
    Stopped,
    TimedOut,
}

/// 启动监控循环的结果：上一轮收尾未结束时拒绝启动，避免旧线程误杀新推流。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorSpawnOutcome {
    Started,
    AlreadyRunning,
    CleanupPending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayInitConfig {
    pub data_dir: PathBuf,
    pub key_file: Option<PathBuf>,
    pub ffmpeg_path: Option<PathBuf>,
}

impl Default for RelayInitConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data/relay"),
            key_file: None,
            ffmpeg_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RelayHealthSnapshot {
    pub enabled: bool,
    pub state: String,
    pub data_dir: String,
    pub sqlite_driver: &'static str,
    pub storage_ready: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("转播数据目录不可用: {0}")]
    DataDir(#[source] std::io::Error),
    #[error("转播存储初始化失败: {0}")]
    Storage(#[source] std::io::Error),
}

/// 一个托管在单线程运行时线程上的监控循环。
///
/// `stop` 通知退出，`task` 等它结束，`cleaned` 由线程在退出收尾
/// （停弹幕 + 停推流）完成后置位，用来判断引擎清理是否已经真的做完。
struct Monitor {
    stop: tokio::sync::watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
    cleaned: Arc<AtomicBool>,
    cleanup_pending: Arc<AtomicBool>,
    epoch: u64,
}

impl Monitor {
    /// 监控线程是否已经跑完（含退出清理）。
    fn is_finished(&self) -> bool {
        self.cleaned.load(Ordering::SeqCst)
            || self.task.as_ref().is_none_or(|task| task.is_finished())
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // 直接 drop RelayService 时也要让监控线程立刻跳出 select 并自行收尾。
        let _ = self.stop.send(true);
    }
}

struct Inner {
    state: String,
    data_dir: PathBuf,
    ready: bool,
    storage: Option<Arc<Store>>,
    monitor: Option<Monitor>,
    /// 本服务是否还持有引擎（弹幕/推流）的停机责任；确认停机完成后才复位。
    engine_owned: bool,
    /// 监控循环代次：每次成功启动加一，旧 stop 只能复位自己那一代。
    epoch: u64,
    engine_owned_epoch: Option<u64>,
    /// 当前这一代循环的收尾标志；停机超时后置位，线程收尾完成后自行复位。
    cleanup_pending: Arc<AtomicBool>,
}

impl Inner {
    fn new() -> Self {
        Self {
            state: String::from(STATE_DISABLED),
            data_dir: PathBuf::new(),
            ready: false,
            storage: None,
            monitor: None,
            engine_owned: false,
            epoch: 0,
            engine_owned_epoch: None,
            cleanup_pending: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct RelayService {
    inner: Mutex<Inner>,
}

impl Default for RelayService {
    fn default() -> Self {
        Self::new()
    }
}

impl RelayService {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::new()),
        }
    }

    pub async fn start(&self, cfg: RelayInitConfig) -> Result<(), RelayError> {
        if lock(&self.inner).ready {
            return Ok(());
        }
        std::fs::create_dir_all(&cfg.data_dir).map_err(RelayError::DataDir)?;
        let store = storage::init(&cfg.data_dir, cfg.key_file.as_deref()).map_err(RelayError::Storage)?;
        let mut inner = lock(&self.inner);
        inner.state = String::from(STATE_READY);
        inner.data_dir = cfg.data_dir;
        inner.ready = true;
        inner.storage = Some(store);
        Ok(())
    }

    pub fn health(&self) -> RelayHealthSnapshot {
        let inner = lock(&self.inner);
        RelayHealthSnapshot {
            enabled: inner.ready,
            state: inner.state.clone(),
            data_dir: inner.data_dir.to_string_lossy().into_owned(),
            sqlite_driver: SQLITE_DRIVER,
            storage_ready: inner.storage.is_some(),
        }
    }

    /// 显式启动监控循环：默认 start 不发起任何外部网络轮询，由 Phase 2 启动控制 API 调用。
    ///
    /// 监控循环内部带着非 Send 的错误值，因此独占一条单线程运行时线程托管，
    /// 与源仓库 launch.rs 的处理方式一致；停止通过 watch 通道通知。
    pub fn spawn_monitor_loop(&self, ffmpeg_log_level: &'static str) -> MonitorSpawnOutcome {
        self.spawn_monitor_loop_with(move || async move {
            if let Err(error) = runtime::run_bilistream(ffmpeg_log_level).await {
                tracing::warn!("转播监控循环退出：{error}");
            }
        })
    }

    /// 用自定义任务启动监控循环；供自检与集成测试注入零外网任务，
    /// 与 spawn_monitor_loop 共用同一套线程、停止通道与退出清理路径。
    #[doc(hidden)]
    pub fn spawn_monitor_loop_with<F, Fut>(&self, monitor: F) -> MonitorSpawnOutcome
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()>,
    {
        let mut inner = lock(&self.inner);
        if inner.cleanup_pending.load(Ordering::SeqCst) {
            tracing::warn!("上一轮监控循环仍在收尾清理，暂不启动新的监控循环");
            return MonitorSpawnOutcome::CleanupPending;
        }
        if let Some(existing) = inner.monitor.as_ref() {
            if !existing.is_finished() {
                return MonitorSpawnOutcome::AlreadyRunning;
            }
            // 旧循环已自行结束：先放掉句柄，允许重新启动。
            inner.monitor = None;
        }
        inner.epoch += 1;
        let epoch = inner.epoch;
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let cleaned = Arc::new(AtomicBool::new(false));
        let thread_cleaned = Arc::clone(&cleaned);
        let cleanup_pending = Arc::new(AtomicBool::new(false));
        let thread_pending = Arc::clone(&cleanup_pending);
        let task = tokio::task::spawn_blocking(move || {
            let local = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(local) => local,
                Err(error) => {
                    tracing::error!("监控循环运行时创建失败：{error}");
                    // 引擎从未启动，直接标记清理完成，避免宿主重复停机。
                    thread_pending.store(false, Ordering::SeqCst);
                    thread_cleaned.store(true, Ordering::SeqCst);
                    return;
                }
            };
            local.block_on(async move {
                tokio::select! {
                    _ = stopped.changed() => {}
                    _ = monitor() => {}
                }
                // 无论是收到停止信号、通道被丢弃，还是任务自己结束，
                // 都在同一线程内把引擎停干净再退出。
                plugins::stop_danmaku().await;
                runtime::graceful_shutdown().await;
            });
            // 先放掉收尾标志，再宣告清理完成：新一轮循环只看前者。
            thread_pending.store(false, Ordering::SeqCst);
            thread_cleaned.store(true, Ordering::SeqCst);
        });
        inner.monitor = Some(Monitor {
            stop,
            task: Some(task),
            cleaned,
            cleanup_pending: Arc::clone(&cleanup_pending),
            epoch,
        });
        inner.engine_owned = true;
        inner.engine_owned_epoch = Some(epoch);
        inner.cleanup_pending = cleanup_pending;
        MonitorSpawnOutcome::Started
    }

    /// 通知并等待监控循环停止，使用默认停机上限。
    pub async fn stop_monitor_loop(&self) -> MonitorStopOutcome {
        self.stop_monitor_loop_with_timeout(MONITOR_STOP_TIMEOUT)
            .await
    }

    /// 通知并等待监控循环停止；超时返回 `TimedOut` 并保留收尾标志，
    /// 期间新的监控循环会被拒绝启动，直到旧线程自己完成清理。
    pub async fn stop_monitor_loop_with_timeout(&self, timeout: Duration) -> MonitorStopOutcome {
        let monitor = lock(&self.inner).monitor.take();
        let Some(mut monitor) = monitor else {
            return MonitorStopOutcome::NotRunning;
        };
        let epoch = monitor.epoch;
        let _ = monitor.stop.send(true);
        let cleaned = Arc::clone(&monitor.cleaned);
        let cleanup_pending = Arc::clone(&monitor.cleanup_pending);
        if let Some(task) = monitor.task.take() {
            match tokio::time::timeout(timeout, task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::error!("监控循环线程异常结束：{error}"),
                Err(_) => tracing::error!("监控循环在 {timeout:?} 内未结束，转由 shutdown 兜底清理"),
            }
        }
        if !cleaned.load(Ordering::SeqCst) {
            // 旧线程还在收尾：只标记它自己那一代，不碰可能已经启动的新循环。
            cleanup_pending.store(true, Ordering::SeqCst);
            return MonitorStopOutcome::TimedOut;
        }
        let mut inner = lock(&self.inner);
        release_engine_ownership(&mut inner, epoch);
        MonitorStopOutcome::Stopped
    }

    /// 优雅停机：先停监控循环，再停该循环启动的弹幕与推流，最后释放存储句柄，
    /// 保证同进程 start -> shutdown -> start 可干净重启且不泄漏句柄。
    ///
    /// 弹幕与推流是进程级单例，只在本服务还持有引擎停机责任时才动它们，
    /// 避免同进程里误停不是自己发起的引擎。
    pub async fn shutdown(&self) {
        self.stop_monitor_loop().await;
        if lock(&self.inner).engine_owned {
            // 监控线程没能自己收尾（超时/异常）时，这里按同一顺序补跑。
            plugins::stop_danmaku().await;
            runtime::graceful_shutdown().await;
            let mut inner = lock(&self.inner);
            inner.engine_owned = false;
            inner.engine_owned_epoch = None;
        }
        let held = {
            let mut inner = lock(&self.inner);
            let held = inner.storage.take().is_some();
            inner.ready = false;
            inner.state = String::from(STATE_STOPPED);
            held
        };
        if held {
            storage::shutdown();
        }
    }
}

impl Drop for RelayService {
    fn drop(&mut self) {
        // Monitor::drop 会发出停止信号，监控线程自行完成停弹幕与停推流。
        let inner = self
            .inner
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 边界：只有监控线程真的跑完（cleaned 已置位）且没有待收尾的清理，
        // 才敢动全局存储；线程仍在运行或仍在收尾时保留句柄，交给 shutdown。
        let cleanup_done = inner.monitor.as_ref().is_some_and(|monitor| {
            monitor.is_finished() && monitor.cleaned.load(Ordering::SeqCst)
        }) && !inner.cleanup_pending.load(Ordering::SeqCst);
        if inner.engine_owned && !cleanup_done {
            return;
        }
        if inner.storage.take().is_some() {
            storage::shutdown();
        }
    }
}

/// 本地推流网络状态的只读观察口：是否真有本地推流进程在跑。
///
/// 数据来自阶段适配层里真实读取 ffmpeg 指标的实现，只暴露宿主自检与
/// 集成测试需要的那一位；Phase 2 接上真实 WebUI 后由 REST 取代。
#[doc(hidden)]
pub async fn local_publisher_running() -> bool {
    compat::webui::api::current_network_status()
        .await
        .ffmpeg_running
}

pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// 只复位发起本次停机的那一代循环的所有权；旧 stop 不能抹掉新 spawn。
fn release_engine_ownership(inner: &mut Inner, epoch: u64) {
    if inner.engine_owned_epoch == Some(epoch) {
        inner.engine_owned = false;
        inner.engine_owned_epoch = None;
    }
}

fn lock(inner: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypto_provider_can_be_installed_repeatedly() {
        install_crypto_provider();
        install_crypto_provider();
        install_crypto_provider();
        reqwest::Client::builder()
            .build()
            .expect("装好 provider 之后 rustls 客户端应能构建");
    }

    #[test]
    fn default_data_dir_is_the_relative_relay_directory() {
        let cfg = RelayInitConfig::default();
        assert_eq!(cfg.data_dir, PathBuf::from("data/relay"));
        assert!(cfg.key_file.is_none());
        assert!(cfg.ffmpeg_path.is_none());
    }

    #[test]
    fn a_fresh_service_reports_disabled_without_touching_disk() {
        let relay = RelayService::new();
        let health = relay.health();
        assert_eq!(health.state, STATE_DISABLED);
        assert!(!health.enabled);
        assert!(!health.storage_ready);
        assert_eq!(health.sqlite_driver, SQLITE_DRIVER);
        assert!(health.data_dir.is_empty());
    }

    #[tokio::test]
    async fn start_opens_the_encrypted_store_and_shutdown_is_idempotent() {
        let _guard = storage::TEST_STORE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let relay = RelayService::new();
        let key_file = dir.path().join("keys/master.key");
        let cfg = RelayInitConfig {
            data_dir: dir.path().join("relay"),
            key_file: Some(key_file.clone()),
            ffmpeg_path: None,
        };

        relay.start(cfg.clone()).await.expect("启动应成功");
        let health = relay.health();
        assert_eq!(health.state, STATE_READY);
        assert!(health.enabled);
        assert!(health.storage_ready);
        assert_eq!(health.data_dir, cfg.data_dir.display().to_string());

        let database = cfg.data_dir.join(DB_FILE);
        assert!(database.exists(), "加密库应落在 data 目录内");
        assert!(
            key_file.exists(),
            "显式 key_file 必须落在数据目录之外"
        );
        assert!(
            !cfg.data_dir
                .read_dir()
                .unwrap()
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().ends_with(".key")),
            "密钥文件不得写进数据目录"
        );

        let probe = rusqlite::Connection::open(&database).unwrap();
        let documents: i64 = probe
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(documents, 1, "新建库应写入空的过滤词文档");
        let encrypted: i64 = probe
            .query_row(
                "SELECT encrypted FROM documents WHERE name='invalid_words.txt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(encrypted, 0, "非敏感文档以明文存储");
        drop(probe);

        relay.start(cfg.clone()).await.expect("重复启动应幂等");
        assert_eq!(relay.health().state, STATE_READY);

        relay.shutdown().await;
        relay.shutdown().await;
        let health = relay.health();
        assert_eq!(health.state, STATE_STOPPED);
        assert!(!health.enabled);
        assert!(!health.storage_ready);

        relay.start(cfg.clone()).await.expect("关闭后应能在同进程重启");
        assert_eq!(relay.health().state, STATE_READY);
        let reopened = rusqlite::Connection::open(&database).unwrap();
        let documents: i64 = reopened
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(documents, 1, "重启后原有文档仍可读");
        drop(reopened);
        relay.shutdown().await;
    }

    static ENGINE_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn boot_relay_for_monitor() -> (tempfile::TempDir, RelayService) {
        let dir = tempfile::tempdir().unwrap();
        let relay = RelayService::new();
        relay
            .start(RelayInitConfig {
                data_dir: dir.path().join("relay"),
                key_file: Some(dir.path().join("keys/master.key")),
                ffmpeg_path: None,
            })
            .await
            .expect("启动应成功");
        (dir, relay)
    }

    async fn spawn_once_cleanup_finishes(relay: &RelayService) -> MonitorSpawnOutcome {
        let started = std::time::Instant::now();
        loop {
            let outcome = relay.spawn_monitor_loop_with(|| async {});
            if outcome != MonitorSpawnOutcome::CleanupPending
                || started.elapsed() > Duration::from_secs(5)
            {
                return outcome;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[test]
    fn releasing_engine_ownership_is_scoped_to_its_own_epoch() {
        let mut inner = Inner::new();
        inner.epoch = 7;
        inner.engine_owned = true;
        inner.engine_owned_epoch = Some(7);

        release_engine_ownership(&mut inner, 6);
        assert!(inner.engine_owned, "旧代次的 stop 不得复位所有权");
        assert_eq!(inner.engine_owned_epoch, Some(7));

        release_engine_ownership(&mut inner, 7);
        assert!(!inner.engine_owned);
        assert_eq!(inner.engine_owned_epoch, None);
    }

    #[tokio::test]
    async fn monitor_stop_reports_all_three_outcomes() {
        let _serial = ENGINE_SERIAL.lock().await;
        let _store = storage::TEST_STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_dir, relay) = boot_relay_for_monitor().await;

        assert_eq!(
            relay.stop_monitor_loop().await,
            MonitorStopOutcome::NotRunning
        );

        assert_eq!(
            relay.spawn_monitor_loop_with(|| async {}),
            MonitorSpawnOutcome::Started
        );
        assert_eq!(
            relay
                .stop_monitor_loop_with_timeout(Duration::from_secs(5))
                .await,
            MonitorStopOutcome::Stopped
        );

        let (started, monitor_started) = tokio::sync::oneshot::channel();
        assert_eq!(
            relay.spawn_monitor_loop_with(move || async move {
                let _ = started.send(());
                std::thread::sleep(Duration::from_millis(400));
            }),
            MonitorSpawnOutcome::Started
        );
        monitor_started.await.expect("监控任务必须进入监控分支");
        assert_eq!(
            relay
                .stop_monitor_loop_with_timeout(Duration::from_millis(50))
                .await,
            MonitorStopOutcome::TimedOut
        );
        // 旧线程还在收尾时不得启动新循环，否则旧清理会误杀新推流。
        assert_eq!(
            relay.spawn_monitor_loop_with(|| async {}),
            MonitorSpawnOutcome::CleanupPending
        );
        assert_eq!(
            spawn_once_cleanup_finishes(&relay).await,
            MonitorSpawnOutcome::Started
        );
        assert_eq!(
            relay
                .stop_monitor_loop_with_timeout(Duration::from_secs(5))
                .await,
            MonitorStopOutcome::Stopped
        );
        relay.shutdown().await;
    }

    #[tokio::test]
    async fn a_stale_stop_never_clears_the_new_monitor_epoch() {
        let _serial = ENGINE_SERIAL.lock().await;
        let _store = storage::TEST_STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_dir, relay) = boot_relay_for_monitor().await;

        let (started, monitor_started) = tokio::sync::oneshot::channel();
        assert_eq!(
            relay.spawn_monitor_loop_with(move || async move {
                let _ = started.send(());
                std::thread::sleep(Duration::from_millis(300));
            }),
            MonitorSpawnOutcome::Started
        );
        monitor_started.await.expect("监控任务必须进入监控分支");

        let stop = relay.stop_monitor_loop_with_timeout(Duration::from_secs(5));
        let interleave = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            relay.spawn_monitor_loop_with(|| async {
                tokio::time::sleep(Duration::from_millis(80)).await;
            })
        };
        let (outcome, spawned) = tokio::join!(stop, interleave);

        assert_eq!(spawned, MonitorSpawnOutcome::Started);
        assert_eq!(outcome, MonitorStopOutcome::Stopped);
        let inner = lock(&relay.inner);
        assert!(inner.engine_owned, "旧 stop 不得抹掉新 spawn 的所有权");
        assert_eq!(inner.engine_owned_epoch, Some(2));
        drop(inner);

        relay
            .stop_monitor_loop_with_timeout(Duration::from_secs(5))
            .await;
        relay.shutdown().await;
    }
}
