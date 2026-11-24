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
#[cfg(test)]
pub(crate) mod test_support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;

pub use config::{load_config, Config};
pub use credential_bridge::{
    load_credentials, RelayBiliAccountSource, RelayBiliCredentials, RelayBiliCredentialsError,
};
pub use storage::Store;
pub use compat::webui::state::{
    update_status_cache, BiliStatus, NicoStatus, PriorityChannelStatus, StatusData, TwStatus,
    YtStatus,
};

pub const DB_FILE: &str = storage::DB_FILE;
pub const SQLITE_DRIVER: &str = "rusqlite/bundled";
const STATE_DISABLED: &str = "disabled";
const STATE_READY: &str = "ready";
const STATE_STOPPED: &str = "stopped";

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

/// 一个托管在单线程运行时线程上的监控循环；`stop` 通知退出，`task` 等它结束。
struct Monitor {
    stop: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

struct Inner {
    state: String,
    data_dir: PathBuf,
    ready: bool,
    storage: Option<Arc<Store>>,
    monitor: Option<Monitor>,
}

impl Inner {
    fn new() -> Self {
        Self {
            state: String::from(STATE_DISABLED),
            data_dir: PathBuf::new(),
            ready: false,
            storage: None,
            monitor: None,
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
    pub fn spawn_monitor_loop(&self, ffmpeg_log_level: &'static str) {
        let mut inner = lock(&self.inner);
        if inner.monitor.is_some() {
            return;
        }
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let task = tokio::task::spawn_blocking(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::warn!("监控循环运行时创建失败：{error}");
                    return;
                }
            };
            runtime.block_on(async move {
                tokio::select! {
                    _ = stopped.changed() => {}
                    outcome = runtime::run_bilistream(ffmpeg_log_level) => {
                        if let Err(error) = outcome {
                            tracing::warn!("转播监控循环退出：{error}");
                        }
                    }
                }
            });
        });
        inner.monitor = Some(Monitor { stop, task });
    }

    /// 通知并等待监控循环停止；返回是否真的有一个监控循环被停下。
    pub async fn stop_monitor_loop(&self) -> bool {
        let monitor = lock(&self.inner).monitor.take();
        let Some(monitor) = monitor else {
            return false;
        };
        let _ = monitor.stop.send(true);
        let _ = monitor.task.await;
        true
    }

    /// 优雅停机：先停监控循环，再停该循环启动的弹幕与推流，最后释放存储句柄，
    /// 保证同进程 start -> shutdown -> start 可干净重启且不泄漏句柄。
    ///
    /// 弹幕与推流是进程级单例，只在本服务确实启动过监控循环时才停它们，
    /// 避免同进程里误停不是自己发起的引擎。
    pub async fn shutdown(&self) {
        let monitored = self.stop_monitor_loop().await;
        if monitored {
            plugins::stop_danmaku().await;
            runtime::graceful_shutdown().await;
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

pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
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
}
