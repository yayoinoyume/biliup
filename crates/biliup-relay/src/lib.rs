pub mod app_state;
pub mod compat;
// 阶段替换：C 组插件仍按源仓库的 crate::cluster / crate::webui 路径引用，
// 这里先指向 compat 薄适配层，Phase 2/5 真实模块迁入后改为直接声明。
pub(crate) use compat::{cluster, webui};
pub mod config;
pub mod credential_bridge;
pub mod events;
pub(crate) mod logging;
pub mod plugins;
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

struct Inner {
    state: String,
    data_dir: PathBuf,
    ready: bool,
    storage: Option<Arc<Store>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            state: String::from(STATE_DISABLED),
            data_dir: PathBuf::new(),
            ready: false,
            storage: None,
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

    pub async fn shutdown(&self) {
        let held = {
            let mut inner = lock(&self.inner);
            if inner.storage.is_none() && inner.state == STATE_STOPPED {
                return;
            }
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
