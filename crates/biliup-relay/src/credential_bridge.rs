//! B 站账号桥接：在转播加密存储与 biliup 明文 `cookies.json` 之间读取同一份账号凭据。
//!
//! 只读解析，绝不打印 cookie/token 明文；错误消息只点名缺了哪个字段名。

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use biliup_accounts::AccountProvider;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Credentials;
use crate::storage::{self, Store};

/// cookies 文档 id，与 `storage::NAMES` 中的一项一致。
const COOKIES_DOCUMENT: &str = "cookies.json";

/// 宿主在启动时注入的账号来源；`None` 时转播只用自己的 `cookies.json`。
static ACCOUNT_PROVIDER: RwLock<Option<Arc<dyn AccountProvider>>> = RwLock::new(None);

/// 换掉（或清空）本进程的账号来源：转播 `start` 时注入，重启后可重置。
pub fn set_account_provider(provider: Option<Arc<dyn AccountProvider>>) {
    *ACCOUNT_PROVIDER
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = provider;
}

/// 当前账号来源。
pub fn account_provider() -> Option<Arc<dyn AccountProvider>> {
    ACCOUNT_PROVIDER
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// 按登记行号取账号凭据文档；没注入来源或账号不存在各警告一次（只有 id）。
pub fn account_document(id: i64) -> Option<Value> {
    let Some(provider) = account_provider() else {
        tracing::warn!(id, "宿主没有注入账号来源，选中的账号用不了，回退到 cookies.json");
        return None;
    };
    let document = provider.resolve(id);
    if document.is_none() {
        tracing::warn!(id, "选中的账号不在账号来源里，回退到 cookies.json");
    }
    document
}

/// 账号文档里的 mid，用来确认行号仍然指向同一个账号。
pub fn account_mid(id: i64) -> Option<u64> {
    credentials_from_document(&account_document(id)?).ok()?.mid
}

/// 改动进程级账号来源的测试共用的锁：`RelayService::start` 也会写这个槽位。
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::Arc;

    pub(crate) struct MockProvider(pub(crate) std::collections::HashMap<i64, Value>);

    impl AccountProvider for MockProvider {
        fn list(&self) -> Vec<biliup_accounts::AccountMeta> {
            self.0
                .keys()
                .map(|id| biliup_accounts::AccountMeta {
                    id: *id,
                    mid: 0,
                    name: None,
                })
                .collect()
        }

        fn resolve(&self, id: i64) -> Option<Value> {
            self.0.get(&id).cloned()
        }
    }

    /// 装入一组合成账号文档；用完由调用方 `set_account_provider(None)` 收尾。
    pub(crate) fn install(documents: impl IntoIterator<Item = (i64, Value)>) {
        set_account_provider(Some(Arc::new(MockProvider(
            documents.into_iter().collect(),
        ))));
    }

    pub(crate) fn account_document(mid: u64, sessdata: &str) -> Value {
        serde_json::json!({
            "cookie_info": {"cookies": [
                {"name": "SESSDATA", "value": sessdata},
                {"name": "bili_jct", "value": "synthetic-account-csrf"},
                {"name": "DedeUserID", "value": mid.to_string()},
                {"name": "DedeUserID__ckMd5", "value": "synthetic-account-ckmd5"},
            ]},
            "token_info": {"access_token": "synthetic-account-token", "mid": mid},
        })
    }
}

/// 转播引擎读取 B 站账号的两条来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayBiliAccountSource {
    /// 转播模块 `relay.db` 内加密存储的 `cookies.json`。
    StandaloneEncrypted,
    /// 复用 biliup 现有的明文 `cookies.json` 文件。
    BiliupCookieFile { path: PathBuf },
}

/// 从两种来源解析出的同一份 B 站凭据。
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RelayBiliCredentials {
    pub sessdata: String,
    pub bili_jct: String,
    pub dede_user_id: String,
    pub dede_user_id_ck_md5: String,
    #[serde(default)]
    pub buvid3: String,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub mid: Option<u64>,
}

impl std::fmt::Debug for RelayBiliCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayBiliCredentials")
            .field("sessdata", &crate::config::secret(&self.sessdata))
            .field("bili_jct", &crate::config::secret(&self.bili_jct))
            .field("dede_user_id", &crate::config::secret(&self.dede_user_id))
            .field(
                "dede_user_id_ck_md5",
                &crate::config::secret(&self.dede_user_id_ck_md5),
            )
            .field("buvid3", &crate::config::secret(&self.buvid3))
            .field("access_token", &crate::config::secret_opt(&self.access_token))
            .field("refresh_token", &crate::config::secret_opt(&self.refresh_token))
            .field("mid", &self.mid)
            .finish()
    }
}

impl RelayBiliCredentials {
    /// 转成 `config.rs` 需要的凭据结构（`buvid3` 缺失时为空）。
    pub fn into_config_credentials(self) -> Credentials {
        Credentials {
            sessdata: self.sessdata,
            bili_jct: self.bili_jct,
            dede_user_id: self.dede_user_id,
            dede_user_id_ckmd5: self.dede_user_id_ck_md5,
            buvid3: self.buvid3,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RelayBiliCredentialsError {
    #[error("B 站凭据文件不可读")]
    File(#[source] std::io::Error),
    #[error("B 站凭据 JSON 格式无效")]
    Format,
    #[error("B 站凭据缺少必需字段：{0}")]
    Missing(&'static str),
    #[error("转播存储不可用")]
    Storage(#[source] std::io::Error),
    #[error("转播存储中没有 cookies.json")]
    NotConfigured,
}

/// 从 `cookie_info.cookies` / `token_info` 形状的 JSON 解析凭据。
pub fn credentials_from_document(
    value: &Value,
) -> Result<RelayBiliCredentials, RelayBiliCredentialsError> {
    let cookies = value
        .pointer("/cookie_info/cookies")
        .and_then(Value::as_array)
        .ok_or(RelayBiliCredentialsError::Format)?;
    let token = value.get("token_info").filter(|token| token.is_object());
    Ok(RelayBiliCredentials {
        sessdata: cookie(cookies, "SESSDATA")?.to_owned(),
        bili_jct: cookie(cookies, "bili_jct")?.to_owned(),
        dede_user_id: cookie(cookies, "DedeUserID")?.to_owned(),
        dede_user_id_ck_md5: cookie(cookies, "DedeUserID__ckMd5")?.to_owned(),
        buvid3: optional_cookie(cookies, "buvid3").unwrap_or_default().to_owned(),
        access_token: token_string(token, "access_token"),
        refresh_token: token_string(token, "refresh_token"),
        mid: token
            .and_then(|token| token.get("mid"))
            .and_then(Value::as_u64),
    })
}

/// 解析 biliup 现有 `cookies.json` 的字节内容。
pub fn parse_biliup_cookie_file(
    bytes: &[u8],
) -> Result<RelayBiliCredentials, RelayBiliCredentialsError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| RelayBiliCredentialsError::Format)?;
    credentials_from_document(&value)
}

/// 解析 biliup 现有 `cookies.json` 文件。
pub fn read_biliup_cookie_file(
    path: &Path,
) -> Result<RelayBiliCredentials, RelayBiliCredentialsError> {
    let bytes = std::fs::read(path).map_err(RelayBiliCredentialsError::File)?;
    parse_biliup_cookie_file(&bytes)
}

/// 读取转播加密存储里的 `cookies.json`。
pub fn read_standalone_credentials(
    store: &Store,
) -> Result<RelayBiliCredentials, RelayBiliCredentialsError> {
    let document = store
        .read(COOKIES_DOCUMENT)
        .map_err(RelayBiliCredentialsError::Storage)?
        .ok_or(RelayBiliCredentialsError::NotConfigured)?;
    credentials_from_document(&document.value)
}

/// 按来源读取凭据；`StandaloneEncrypted` 走进程级文档库。
pub fn load_credentials(
    source: &RelayBiliAccountSource,
) -> Result<RelayBiliCredentials, RelayBiliCredentialsError> {
    match source {
        RelayBiliAccountSource::StandaloneEncrypted => {
            let store = storage::global().map_err(RelayBiliCredentialsError::Storage)?;
            read_standalone_credentials(&store)
        }
        RelayBiliAccountSource::BiliupCookieFile { path } => read_biliup_cookie_file(path),
    }
}

fn cookie<'a>(
    cookies: &'a [Value],
    name: &'static str,
) -> Result<&'a str, RelayBiliCredentialsError> {
    optional_cookie(cookies, name).ok_or(RelayBiliCredentialsError::Missing(name))
}

fn optional_cookie<'a>(cookies: &'a [Value], name: &str) -> Option<&'a str> {
    cookies
        .iter()
        .find(|cookie| cookie.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|cookie| cookie.get("value").and_then(Value::as_str))
        .filter(|value| !value.is_empty())
}

fn token_string(token: Option<&Value>, name: &str) -> Option<String> {
    token
        .and_then(|token| token.get(name))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn biliup_document() -> Value {
        json!({
            "cookie_info": {"cookies": [
                {"name": "SESSDATA", "value": "synthetic-sessdata-7f3a"},
                {"name": "bili_jct", "value": "synthetic-csrf-91c2"},
                {"name": "DedeUserID", "value": "synthetic-uid-55"},
                {"name": "DedeUserID__ckMd5", "value": "synthetic-ckmd5-0b"},
                {"name": "buvid3", "value": "synthetic-buvid3-4d"},
            ]},
            "token_info": {
                "access_token": "synthetic-access-2c",
                "refresh_token": "synthetic-refresh-6e",
                "mid": 900719925474099u64,
                "expires_in": 3600,
            },
            "sso": ["https://passport.biligame.com"],
            "platform": "BiliTV",
        })
    }

    fn isolated_store(dir: &Path) -> std::sync::Arc<Store> {
        Store::open(dir.join("data"), dir.join("keys/master.key")).unwrap()
    }

    #[test]
    fn reads_cookies_and_token_info_from_a_biliup_file() {
        let credentials =
            parse_biliup_cookie_file(&serde_json::to_vec(&biliup_document()).unwrap()).unwrap();
        assert_eq!(credentials.sessdata, "synthetic-sessdata-7f3a");
        assert_eq!(credentials.bili_jct, "synthetic-csrf-91c2");
        assert_eq!(credentials.dede_user_id, "synthetic-uid-55");
        assert_eq!(credentials.dede_user_id_ck_md5, "synthetic-ckmd5-0b");
        assert_eq!(credentials.buvid3, "synthetic-buvid3-4d");
        assert_eq!(credentials.access_token.as_deref(), Some("synthetic-access-2c"));
        assert_eq!(
            credentials.refresh_token.as_deref(),
            Some("synthetic-refresh-6e")
        );
        assert_eq!(credentials.mid, Some(900719925474099));
    }

    #[test]
    fn a_missing_cookie_is_named_without_its_value() {
        let mut document = biliup_document();
        document["cookie_info"]["cookies"] = json!([
            {"name": "SESSDATA", "value": "synthetic-sessdata-7f3a"},
            {"name": "bili_jct", "value": "synthetic-csrf-91c2"},
            {"name": "DedeUserID", "value": "synthetic-uid-55"},
        ]);
        let error = credentials_from_document(&document).unwrap_err();
        assert_eq!(error.to_string(), "B 站凭据缺少必需字段：DedeUserID__ckMd5");
        let text = format!("{error} {error:?}");
        for leaked in ["synthetic-sessdata-7f3a", "synthetic-csrf-91c2", "synthetic-uid-55"] {
            assert!(!text.contains(leaked), "{leaked}");
        }
    }

    #[test]
    fn empty_values_and_broken_json_are_refused_without_echoing_content() {
        let mut document = biliup_document();
        document["cookie_info"]["cookies"][0]["value"] = json!("");
        let error = credentials_from_document(&document).unwrap_err();
        assert!(matches!(error, RelayBiliCredentialsError::Missing("SESSDATA")));

        let error = parse_biliup_cookie_file(b"synthetic-not-json").unwrap_err();
        assert_eq!(error.to_string(), "B 站凭据 JSON 格式无效");
        assert!(!format!("{error:?}").contains("synthetic-not-json"));
        assert!(matches!(
            credentials_from_document(&json!({"cookie_info": {"cookies": "synthetic"}})),
            Err(RelayBiliCredentialsError::Format)
        ));
    }

    #[test]
    fn a_relay_document_without_a_token_block_still_reads_cookies() {
        let credentials = credentials_from_document(&json!({
            "cookie_info": {"cookies": [
                {"name": "SESSDATA", "value": "synthetic-sessdata-7f3a"},
                {"name": "bili_jct", "value": "synthetic-csrf-91c2"},
                {"name": "DedeUserID", "value": "synthetic-uid-55"},
                {"name": "DedeUserID__ckMd5", "value": "synthetic-ckmd5-0b"},
            ]},
            "token_info": {},
        }))
        .unwrap();
        assert!(credentials.buvid3.is_empty());
        assert_eq!(credentials.access_token, None);
        assert_eq!(credentials.refresh_token, None);
        assert_eq!(credentials.mid, None);
    }

    #[test]
    fn standalone_and_file_sources_yield_the_same_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let store = isolated_store(dir.path());
        let document = biliup_document();
        store.write(COOKIES_DOCUMENT, document.clone()).unwrap();
        let path = dir.path().join("cookies.json");
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();

        let standalone = read_standalone_credentials(&store).unwrap();
        let file = read_biliup_cookie_file(&path).unwrap();
        assert_eq!(standalone, file);
        assert_eq!(
            load_credentials(&RelayBiliAccountSource::BiliupCookieFile { path }).unwrap(),
            standalone
        );
    }

    #[test]
    fn standalone_encrypted_source_reads_through_the_process_store() {
        let _guard = crate::storage::lock_test_global();
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("keys/master.key");
        let store = crate::storage::init(&dir.path().join("data"), Some(&key)).unwrap();
        store.write(COOKIES_DOCUMENT, biliup_document()).unwrap();
        let credentials = load_credentials(&RelayBiliAccountSource::StandaloneEncrypted).unwrap();
        assert_eq!(credentials.sessdata, "synthetic-sessdata-7f3a");
        assert_eq!(credentials.dede_user_id_ck_md5, "synthetic-ckmd5-0b");
        drop(store);
        crate::storage::shutdown();
    }

    #[test]
    fn a_store_without_cookies_reports_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        let store = isolated_store(dir.path());
        assert!(matches!(
            read_standalone_credentials(&store),
            Err(RelayBiliCredentialsError::NotConfigured)
        ));
    }

    #[test]
    fn an_unreadable_file_reports_a_redacted_error() {
        let error = read_biliup_cookie_file(Path::new("synthetic-missing-cookies.json")).unwrap_err();
        assert_eq!(error.to_string(), "B 站凭据文件不可读");
        assert!(!format!("{error:?}").contains("synthetic-missing-cookies.json"));
    }

    #[test]
    fn the_source_enum_round_trips_through_json() {
        for source in [
            RelayBiliAccountSource::StandaloneEncrypted,
            RelayBiliAccountSource::BiliupCookieFile {
                path: PathBuf::from("data/cookies.json"),
            },
        ] {
            let json = serde_json::to_value(&source).unwrap();
            assert_eq!(
                serde_json::from_value::<RelayBiliAccountSource>(json.clone()).unwrap(),
                source
            );
            assert_eq!(json["kind"], json!(serde_json::to_value(match source {
                RelayBiliAccountSource::StandaloneEncrypted => "standalone_encrypted",
                RelayBiliAccountSource::BiliupCookieFile { .. } => "biliup_cookie_file",
            })
            .unwrap()));
        }
    }

    #[test]
    fn credentials_convert_to_the_config_shape_and_never_debug_their_values() {
        let credentials = credentials_from_document(&biliup_document()).unwrap();
        for text in [format!("{credentials:?}"), format!("{credentials:#?}")] {
            for leaked in [
                "synthetic-sessdata-7f3a",
                "synthetic-csrf-91c2",
                "synthetic-ckmd5-0b",
                "synthetic-buvid3-4d",
                "synthetic-access-2c",
                "synthetic-refresh-6e",
            ] {
                assert!(!text.contains(leaked), "{leaked} leaked into Debug output");
            }
            assert!(text.contains("[redacted]"), "{text}");
        }
        let config = credentials.into_config_credentials();
        assert_eq!(config.sessdata, "synthetic-sessdata-7f3a");
        assert_eq!(config.dede_user_id_ckmd5, "synthetic-ckmd5-0b");
        assert_eq!(config.buvid3, "synthetic-buvid3-4d");
    }
}
