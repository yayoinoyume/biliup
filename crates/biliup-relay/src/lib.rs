use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;

pub const DB_FILE: &str = "relay.sqlite3";
pub const SQLITE_DRIVER: &str = "rusqlite/bundled";
const STATE_DISABLED: &str = "disabled";
const STATE_READY: &str = "ready";
const STATE_STOPPED: &str = "stopped";
const CREATE_PROBE: &str =
    "CREATE TABLE IF NOT EXISTS relay_probe (id INTEGER PRIMARY KEY CHECK (id = 1), ticket TEXT NOT NULL)";
const WRITE_PROBE: &str =
    "INSERT INTO relay_probe (id, ticket) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET ticket = excluded.ticket";
const READ_PROBE: &str = "SELECT ticket FROM relay_probe WHERE id = 1";

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
    Storage(#[from] rusqlite::Error),
    #[error("转播存储探活读回不一致")]
    ProbeMismatch,
}

#[derive(Debug)]
struct Inner {
    state: String,
    data_dir: PathBuf,
    ready: bool,
    storage: Option<rusqlite::Connection>,
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
        let connection = rusqlite::Connection::open(cfg.data_dir.join(DB_FILE))?;
        connection.execute_batch(CREATE_PROBE)?;
        let ticket = probe_ticket();
        connection.execute(WRITE_PROBE, [&ticket])?;
        let read: String = connection.query_row(READ_PROBE, [], |row| row.get(0))?;
        if read != ticket {
            return Err(RelayError::ProbeMismatch);
        }
        let mut inner = lock(&self.inner);
        inner.state = String::from(STATE_READY);
        inner.data_dir = cfg.data_dir;
        inner.ready = true;
        inner.storage = Some(connection);
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
        let mut inner = lock(&self.inner);
        if inner.storage.is_none() && inner.state == STATE_STOPPED {
            return;
        }
        inner.storage = None;
        inner.ready = false;
        inner.state = String::from(STATE_STOPPED);
    }
}

pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn lock(inner: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn probe_ticket() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(elapsed) => format!("{}-{}", elapsed.as_secs(), elapsed.subsec_nanos()),
        Err(_) => String::from("0-0"),
    }
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
    async fn start_probes_sqlite_and_shutdown_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let relay = RelayService::new();
        let cfg = RelayInitConfig {
            data_dir: dir.path().join("relay"),
            key_file: None,
            ffmpeg_path: None,
        };

        relay.start(cfg.clone()).await.expect("启动应成功");
        let health = relay.health();
        assert_eq!(health.state, STATE_READY);
        assert!(health.enabled);
        assert!(health.storage_ready);
        assert_eq!(health.data_dir, cfg.data_dir.display().to_string());
        assert!(cfg.data_dir.join(DB_FILE).exists());

        let probe = rusqlite::Connection::open(cfg.data_dir.join(DB_FILE)).unwrap();
        let rows: i64 = probe
            .query_row("SELECT COUNT(*) FROM relay_probe", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        drop(probe);

        relay.start(cfg).await.expect("重复启动应幂等");
        assert_eq!(relay.health().state, STATE_READY);

        relay.shutdown().await;
        relay.shutdown().await;
        let health = relay.health();
        assert_eq!(health.state, STATE_STOPPED);
        assert!(!health.enabled);
        assert!(!health.storage_ready);
    }
}
