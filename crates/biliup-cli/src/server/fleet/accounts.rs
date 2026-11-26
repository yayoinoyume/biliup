//! 节点上本机登记的 B 站账号。
//!
//! 只读 `configuration` 表里 `bilibili-cookies` 登记的凭据文件，从里面取 `token_info.mid`，不联网。
//! 上报给控制面的只有 mid 与昵称；凭据文件路径只在本机用来把模板的 `account_mid` 换回 `user_cookie`。

use super::model::Account;
use crate::server::infrastructure::connection_pool::ConnectionPool;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tracing::{debug, warn};

static UNSUPPORTED_CIPHERTEXT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// 读一份登记凭据的明文文本。密文不做同步，但也不能静默丢账号：同一个文件只警告一次
/// （Fleet 心跳每 10 秒扫一遍），然后返回 `None`。
///
/// 先看字节再转字符串：真实密文是非 UTF-8 二进制，先 `read_to_string` 会在魔法判断之前就失败。
pub(crate) fn plaintext_credential(
    path: &str,
    bytes: Vec<u8>,
    unsupported: &'static str,
) -> Option<String> {
    if biliup_accounts::is_ciphertext(&bytes) {
        let known = UNSUPPORTED_CIPHERTEXT.get_or_init(|| Mutex::new(HashSet::new()));
        let mut known = known
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if known.insert(format!("{unsupported}:{path}")) {
            warn!(path = %path, "{unsupported}");
        } else {
            debug!(path = %path, "{unsupported}");
        }
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// 本机的一个账号与它的凭据文件
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAccount {
    pub mid: u64,
    pub uname: String,
    /// 登记时的规范路径，与 `uploadstreamers.user_cookie` 同一写法
    pub path: String,
}

impl LocalAccount {
    pub fn public(&self) -> Account {
        Account {
            mid: self.mid,
            uname: self.uname.clone(),
        }
    }
}

/// 从凭据文件内容里取 mid 与昵称。biliup-rs 登录写出的文件没有昵称，这时昵称为空。
pub fn parse_credential(text: &str) -> Option<(u64, String)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let mid = value.pointer("/token_info/mid").and_then(|mid| {
        mid.as_u64()
            .or_else(|| mid.as_str().and_then(|s| s.trim().parse().ok()))
    })?;
    if mid == 0 {
        return None;
    }
    let uname = ["/uname", "/token_info/uname", "/name"]
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
        .map(|name| name.trim().to_string())
        .unwrap_or_default();
    Some((mid, uname))
}

/// 本机登记的账号，按登记顺序；同一个 mid 登记了几份凭据文件时取最早登记的那份
pub async fn scan(pool: &ConnectionPool) -> Vec<LocalAccount> {
    let paths: Vec<String> = match sqlx::query_scalar(
        "SELECT value FROM configuration WHERE key = 'bilibili-cookies' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    {
        Ok(paths) => paths,
        Err(e) => {
            debug!(error = %e, "could not list bilibili credentials");
            return Vec::new();
        }
    };
    let mut accounts: Vec<LocalAccount> = Vec::new();
    for path in paths {
        let Ok(bytes) = tokio::fs::read(Path::new(&path)).await else {
            continue;
        };
        let Some(text) = plaintext_credential(
            &path,
            bytes,
            "B 站账号凭据是加密文件，Fleet 账号上报不支持密文，这回不报这个账号",
        ) else {
            continue;
        };
        let Some((mid, uname)) = parse_credential(&text) else {
            continue;
        };
        if accounts.iter().any(|account| account.mid == mid) {
            continue;
        }
        accounts.push(LocalAccount { mid, uname, path });
    }
    accounts
}

pub fn public(accounts: &[LocalAccount]) -> Vec<Account> {
    accounts.iter().map(LocalAccount::public).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    struct CapturedLog(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedLog {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct LogWriter(Arc<Mutex<Vec<u8>>>);

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogWriter {
        type Writer = CapturedLog;

        fn make_writer(&'a self) -> CapturedLog {
            CapturedLog(self.0.clone())
        }
    }

    /// 本轮测试线程上的日志收集器：返回共享缓冲与订阅者，用 `tracing::subscriber::set_default` 装上
    pub(crate) fn captured_log() -> (
        Arc<Mutex<Vec<u8>>>,
        impl tracing::Subscriber + Send + Sync + 'static,
    ) {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(LogWriter(buffer.clone()))
            .with_ansi(false)
            .finish();
        (buffer, subscriber)
    }

    pub(crate) fn take_log(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(
            buffer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        )
        .unwrap()
    }

    /// 真实密文：`BUPACC1` 头 + 12 字节 nonce + 随机二进制负载，绝不是 ASCII
    pub(crate) fn encrypted_credential(dir: &Path, mid: u64) -> (PathBuf, Vec<u8>) {
        let store =
            biliup_accounts::AccountStore::open(dir, Some(&dir.join("accounts.key"))).unwrap();
        let path = store
            .save(mid, &serde_json::json!({ "token_info": { "mid": mid } }))
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"BUPACC1"));
        assert!(
            std::str::from_utf8(&bytes).is_err(),
            "密文必须是二进制，不能用 ASCII 假密文"
        );
        (path, bytes)
    }

    #[test]
    fn mid_comes_from_token_info_and_the_rest_is_ignored() {
        let text = serde_json::json!({
            "cookie_info": { "cookies": [{ "name": "SESSDATA", "value": "secret" }] },
            "token_info": { "access_token": "secret", "mid": 42, "expires_in": 1, "refresh_token": "r" },
            "platform": "BiliTV",
        })
        .to_string();
        assert_eq!(parse_credential(&text), Some((42, String::new())));
        let named =
            serde_json::json!({ "token_info": { "mid": "7" }, "uname": " 甲 " }).to_string();
        assert_eq!(parse_credential(&named), Some((7, "甲".into())));
        assert_eq!(parse_credential("{}"), None);
        assert_eq!(parse_credential("not json"), None);
        assert_eq!(parse_credential(r#"{"token_info":{"mid":0}}"#), None);
    }

    #[tokio::test]
    async fn registered_credential_files_are_scanned_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("data.sqlite3");
        let pool = crate::server::infrastructure::connection_pool::ConnectionManager::new_pool(
            db.to_str().unwrap(),
        )
        .await
        .unwrap();
        let write = |name: &str, body: Value| {
            let path = dir.path().join(name);
            std::fs::write(&path, body.to_string()).unwrap();
            path.to_string_lossy().into_owned()
        };
        let first = write("a.json", serde_json::json!({ "token_info": { "mid": 42 } }));
        let duplicate = write("b.json", serde_json::json!({ "token_info": { "mid": 42 } }));
        let broken = write("c.json", serde_json::json!({ "cookie_info": {} }));
        let other = write("d.json", serde_json::json!({ "token_info": { "mid": 7 } }));
        let missing = dir.path().join("gone.json").to_string_lossy().into_owned();
        let (encrypted, _) = encrypted_credential(dir.path(), 99);
        let encrypted = encrypted.to_string_lossy().into_owned();
        for path in [&first, &duplicate, &broken, &missing, &other, &encrypted] {
            sqlx::query("INSERT INTO configuration (key, value) VALUES ('bilibili-cookies', ?)")
                .bind(path)
                .execute(&pool)
                .await
                .unwrap();
        }
        let accounts = scan(&pool).await;
        assert_eq!(
            accounts
                .iter()
                .map(|a| (a.mid, a.path.as_str()))
                .collect::<Vec<_>>(),
            [(42, first.as_str()), (7, other.as_str())]
        );
        assert_eq!(
            public(&accounts),
            [
                Account {
                    mid: 42,
                    uname: String::new()
                },
                Account {
                    mid: 7,
                    uname: String::new()
                }
            ]
        );
    }

    #[tokio::test]
    async fn binary_ciphertext_credentials_warn_once_and_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let (path, bytes) = encrypted_credential(dir.path(), 42);
        // 先 `read_to_string` 会在魔法判断之前就失败，所以这里真的不能是 UTF-8
        assert!(std::str::from_utf8(&bytes).is_err());
        let path = path.to_string_lossy().into_owned();
        let pool = crate::server::infrastructure::connection_pool::ConnectionManager::new_pool(
            dir.path().join("data.sqlite3").to_str().unwrap(),
        )
        .await
        .unwrap();
        sqlx::query("INSERT INTO configuration (key, value) VALUES ('bilibili-cookies', ?1)")
            .bind(&path)
            .execute(&pool)
            .await
            .unwrap();

        let (buffer, subscriber) = captured_log();
        {
            let _guard = tracing::subscriber::set_default(subscriber);
            assert!(scan(&pool).await.is_empty(), "密文账号不能当明文上报");
        }
        let log = take_log(&buffer);
        assert!(log.contains("不支持密文"), "{log}");
        assert!(log.contains(&path), "警告里要带路径：{log}");

        // 同一个文件只警告一次：第二次扫描只剩 debug，默认级别收不到
        let (buffer, subscriber) = captured_log();
        {
            let _guard = tracing::subscriber::set_default(subscriber);
            assert!(scan(&pool).await.is_empty());
        }
        assert!(!take_log(&buffer).contains("不支持密文"));
    }
}
