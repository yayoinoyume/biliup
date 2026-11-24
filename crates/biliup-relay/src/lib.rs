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
            // 先宣告清理完成，再撤销占位：停机侧在占位后总会复查 cleaned，
            // 这个顺序保证任何交错都收敛到「占位已撤销」，不会永久拒启。
            thread_cleaned.store(true, Ordering::SeqCst);
            thread_pending.store(false, Ordering::SeqCst);
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
        self.stop_monitor_loop_inner(MONITOR_STOP_TIMEOUT).await.0
    }

    /// 通知并等待监控循环停止；超时返回 `TimedOut` 并保留收尾标志，
    /// 期间新的监控循环会被拒绝启动，直到旧线程自己完成清理。
    pub async fn stop_monitor_loop_with_timeout(&self, timeout: Duration) -> MonitorStopOutcome {
        self.stop_monitor_loop_inner(timeout).await.0
    }

    /// 停机主体：占位与 `take()` 同锁完成，等待期间不会漏放新代。
    /// 返回结果与本次停下的代次，供 shutdown 按代兜底。
    async fn stop_monitor_loop_inner(&self, timeout: Duration) -> (MonitorStopOutcome, Option<u64>) {
        let monitor = {
            let mut inner = lock(&self.inner);
            let monitor = inner.monitor.take();
            if monitor.is_some() {
                // 占位必须和 take 在同一临界区：否则等待窗口内新 spawn 会放行，
                // 旧线程收尾时的全局停机会误杀新代推流。
                inner.cleanup_pending.store(true, Ordering::SeqCst);
            }
            monitor
        };
        let Some(mut monitor) = monitor else {
            return (MonitorStopOutcome::NotRunning, None);
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
            // 线程仍在收尾：保留占位，由它的终态复位；所有权留到 shutdown 兜底。
            return (MonitorStopOutcome::TimedOut, Some(epoch));
        }
        // 线程已收尾（或在本次等待里刚收尾）：撤销占位并释放本代所有权。
        cleanup_pending.store(false, Ordering::SeqCst);
        let mut inner = lock(&self.inner);
        release_engine_ownership(&mut inner, epoch);
        (MonitorStopOutcome::Stopped, Some(epoch))
    }

    /// 优雅停机：先停监控循环，再停该循环启动的弹幕与推流，最后释放存储句柄，
    /// 保证同进程 start -> shutdown -> start 可干净重启且不泄漏句柄。
    ///
    /// 弹幕与推流是进程级单例，只在本服务还持有引擎停机责任时才动它们，
    /// 避免同进程里误停不是自己发起的引擎。
    pub async fn shutdown(&self) {
        let (outcome, stopped_epoch) = self.stop_monitor_loop_inner(MONITOR_STOP_TIMEOUT).await;
        let cleanup_epoch = match outcome {
            MonitorStopOutcome::TimedOut => stopped_epoch,
            // 上一轮已经 take 走线程并超时：所有权仍挂在那一代上。
            MonitorStopOutcome::NotRunning => lock(&self.inner).engine_owned_epoch,
            MonitorStopOutcome::Stopped => None,
        };
        if let Some(epoch) = cleanup_epoch {
            let owned = {
                let inner = lock(&self.inner);
                inner.engine_owned && inner.engine_owned_epoch == Some(epoch)
            };
            if owned {
                // 监控线程没能自己收尾（超时/异常）时，这里按同一顺序补跑。
                plugins::stop_danmaku().await;
                runtime::graceful_shutdown().await;
                let mut inner = lock(&self.inner);
                if inner.engine_owned_epoch == Some(epoch) {
                    inner.engine_owned = false;
                    inner.engine_owned_epoch = None;
                }
            }
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
        // 边界：没有待收尾的清理、且监控线程已跑完（或从未启动）时才动全局
        // 存储。stop 超时后线程已自行收尾的情况也走这里释放。
        let cleanup_done = !inner.cleanup_pending.load(Ordering::SeqCst)
            && inner
                .monitor
                .as_ref()
                .is_none_or(|monitor| monitor.is_finished());
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
            if outcome != MonitorSpawnOutcome::CleanupPending {
                return outcome;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "等待旧线程收尾超时：{:?}",
                started.elapsed()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn wait_until_monitor_cleaned(relay: &RelayService) {
        let started = std::time::Instant::now();
        loop {
            {
                let inner = lock(&relay.inner);
                let done = !inner.cleanup_pending.load(Ordering::SeqCst)
                    && inner
                        .monitor
                        .as_ref()
                        .is_some_and(|monitor| monitor.cleaned.load(Ordering::SeqCst));
                if done {
                    return;
                }
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "等待监控收尾超时：{:?}",
                started.elapsed()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn gated_monitor() -> (
        tokio::sync::oneshot::Receiver<()>,
        std::sync::mpsc::Sender<()>,
        impl FnOnce() -> std::future::Ready<()> + Send + 'static,
    ) {
        let (started, monitor_started) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let task = move || {
            let _ = started.send(());
            let _ = released.recv();
            std::future::ready(())
        };
        (monitor_started, release, task)
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
        // 引擎是进程级单例：与 ffmpeg 的生命周期测试互斥，避免互相清空推流。
        let _engine = crate::plugins::ffmpeg::FFMPEG_TEST_GUARD
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
            relay.spawn_monitor_loop_with(|| async {}),
            MonitorSpawnOutcome::AlreadyRunning
        );
        assert_eq!(
            relay
                .stop_monitor_loop_with_timeout(Duration::from_secs(5))
                .await,
            MonitorStopOutcome::Stopped
        );

        let (monitor_started, release, task) = gated_monitor();
        assert_eq!(
            relay.spawn_monitor_loop_with(task),
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
        release.send(()).expect("放行旧线程");
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
    async fn a_running_stop_rejects_a_new_monitor_until_the_thread_finishes() {
        let _serial = ENGINE_SERIAL.lock().await;
        let _store = storage::TEST_STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 引擎是进程级单例：与 ffmpeg 的生命周期测试互斥，避免互相清空推流。
        let _engine = crate::plugins::ffmpeg::FFMPEG_TEST_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_dir, relay) = boot_relay_for_monitor().await;

        let (monitor_started, release, task) = gated_monitor();
        assert_eq!(
            relay.spawn_monitor_loop_with(task),
            MonitorSpawnOutcome::Started
        );
        monitor_started.await.expect("监控任务必须进入监控分支");

        let stop = async { relay.stop_monitor_loop_with_timeout(Duration::from_secs(10)).await };
        let probe = async {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let outcome = loop {
                match relay.spawn_monitor_loop_with(|| async {}) {
                    MonitorSpawnOutcome::CleanupPending => {
                        break MonitorSpawnOutcome::CleanupPending
                    }
                    other => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "等待窗口内不得放行新代：{other:?}"
                        );
                        tokio::task::yield_now().await;
                    }
                }
            };
            release.send(()).expect("放行旧线程");
            outcome
        };
        let (stopped, rejected) = tokio::join!(stop, probe);

        assert_eq!(rejected, MonitorSpawnOutcome::CleanupPending);
        assert_eq!(stopped, MonitorStopOutcome::Stopped);

        // 代际隔离：旧代（epoch=1）的释放不得清掉新代（epoch=2）的所有权。
        assert_eq!(
            relay.spawn_monitor_loop_with(|| async {}),
            MonitorSpawnOutcome::Started
        );
        {
            let mut inner = lock(&relay.inner);
            assert!(inner.engine_owned);
            assert_eq!(inner.engine_owned_epoch, Some(2));
            release_engine_ownership(&mut inner, 1);
            assert!(inner.engine_owned, "旧代 release 不得清除新代所有权");
            assert_eq!(inner.engine_owned_epoch, Some(2));
        }

        relay
            .stop_monitor_loop_with_timeout(Duration::from_secs(5))
            .await;
        relay.shutdown().await;
    }

    #[tokio::test]
    async fn drop_releases_storage_after_the_monitor_finishes() {
        let _serial = ENGINE_SERIAL.lock().await;
        let _store = storage::TEST_STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 引擎是进程级单例：与 ffmpeg 的生命周期测试互斥，避免互相清空推流。
        let _engine = crate::plugins::ffmpeg::FFMPEG_TEST_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (dir, relay) = boot_relay_for_monitor().await;

        assert_eq!(
            relay.spawn_monitor_loop_with(|| async {}),
            MonitorSpawnOutcome::Started
        );
        wait_until_monitor_cleaned(&relay).await;

        let probe = storage::global().expect("存储应已打开");
        let weak = Arc::downgrade(&probe);
        drop(probe);

        drop(relay);

        assert!(
            weak.upgrade().is_none(),
            "监控已收尾时 drop 必须立刻释放全局存储句柄"
        );
        let reopened = RelayService::new();
        reopened
            .start(RelayInitConfig {
                data_dir: dir.path().join("relay"),
                key_file: Some(dir.path().join("keys/master.key")),
                ffmpeg_path: None,
            })
            .await
            .expect("drop 之后应能立刻重开存储");
        reopened.shutdown().await;
    }
}
