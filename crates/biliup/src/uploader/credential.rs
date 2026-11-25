use crate::client::StatefulClient;
use futures::Future;
use reqwest::header;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use crate::error::{Kind, Result};
use crate::uploader::bilibili::{BiliBili, ResponseData};
use base64::{Engine as _, engine::general_purpose};
use biliup_accounts::{CIPHERTEXT_PREFIX, Cipher, Zeroizing};
use cookie::Cookie;
use md5::{Digest, Md5};
use reqwest::header::{COOKIE, ORIGIN, REFERER, USER_AGENT};

use rsa::{Pkcs1v15Encrypt, RsaPublicKey, pkcs8::DecodePublicKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};
use url::Url;

// const APP_KEY: &str = "ae57252b0c09105d";
// const APPSEC: &str = "c75875c596a69eb55bd119e74b07cfe3";
// const APP_KEY: &str = "783bbb7264451d82";
// const APPSEC: &str = "2653583c8873dea268ab9386918b1d65";
// const APP_KEY: &str = "4409e2ce8ffd12b8";
// const APPSEC: &str = "59b43e04ad6965f34319062b478f83dd";
// const APP_KEY: &str = "37207f2beaebf8d7";
// const APPSEC: &str = "e988e794d4d4b6dd43bc0e89d6e90c43";
// const APP_KEY: &str = "bca7e84c2d947ac6";
// const APPSEC: &str = "60698ba2f68e01ce44738920a0ffe768";
// const APP_KEY: &str = "bb3101000e232e27";
// const APPSEC: &str = "36efcfed79309338ced0380abd824ac1";
pub(crate) enum AppKeyStore {
    BiliTV,
    Android,
    BCutAndroid,
}

impl AppKeyStore {
    pub fn app_key(&self) -> &'static str {
        match self {
            AppKeyStore::BiliTV => "4409e2ce8ffd12b8",
            AppKeyStore::Android => "783bbb7264451d82",
            AppKeyStore::BCutAndroid => "5dce947fe22167f9",
        }
    }

    pub fn appsec(&self) -> &'static str {
        match self {
            AppKeyStore::BiliTV => "59b43e04ad6965f34319062b478f83dd",
            AppKeyStore::Android => "2653583c8873dea268ab9386918b1d65",
            AppKeyStore::BCutAndroid => "5491a31c6bc11fb764a9b1f8d4acf092",
        }
    }
}

pub fn bilibili_from_cookies(file: impl AsRef<Path>, proxy: Option<&str>) -> Result<BiliBili> {
    let login_info = read_login_info(file.as_ref())?;
    bilibili_from_info(login_info, proxy)
}

fn is_encrypted_path(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("enc")
}

fn is_encrypted_target(path: &Path) -> bool {
    if is_encrypted_path(path) {
        return true;
    }
    let mut head = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(CIPHERTEXT_PREFIX.len() as u64)
                .read_to_end(&mut head)
        })
        .is_ok_and(|_| biliup_accounts::is_ciphertext(&head))
}

fn configured_cipher() -> Result<Cipher> {
    Cipher::from_env()?.ok_or_else(|| {
        Kind::Custom("凭据密钥未配置：请设置 BILIUP_ACCOUNT_KEY_FILE 指向密钥文件".into())
    })
}

fn read_login_info(path: &Path) -> Result<LoginInfo> {
    let bytes = Zeroizing::new(std::fs::read(path)?);
    let plaintext = if biliup_accounts::is_ciphertext(&bytes) {
        configured_cipher()?
            .open(&bytes)
            .map_err(|error| Kind::Custom(error.to_string()))?
    } else {
        bytes
    };
    Ok(serde_json::from_slice(&plaintext)?)
}

static CREDENTIAL_FILE_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>> =
    OnceLock::new();

fn replace_credential_file(temporary_path: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(not(windows))]
    {
        std::fs::rename(temporary_path, destination)
    }
    #[cfg(windows)]
    {
        if !destination.exists() {
            return std::fs::rename(temporary_path, destination);
        }
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        let backup = parent.join(format!(
            ".biliup-replace-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::rename(destination, &backup)?;
        if let Err(error) = std::fs::rename(temporary_path, destination) {
            let _ = std::fs::rename(&backup, destination);
            return Err(error);
        }
        std::fs::remove_file(backup)
    }
}

/// Resolve a credential path to a stable, absolute lock key.
///
/// Existing paths (including symlinks) are canonicalized. For a file that has
/// not been created yet, the nearest existing parent is canonicalized and the
/// remaining normal components are appended.
pub fn normalize_credential_path(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let path = path.as_ref();
    if path.exists() {
        return path.canonicalize();
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut existing = absolute.as_path();
    let mut suffix = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "credential path has no existing parent",
            ));
        };
        suffix.push(name.to_os_string());
        existing = existing.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "credential path has no existing parent",
            )
        })?;
    }

    let mut normalized = existing.canonicalize()?;
    for component in suffix.into_iter().rev() {
        let component_path = Path::new(&component);
        if !matches!(
            component_path.components().next(),
            Some(Component::Normal(_))
        ) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "credential path contains an invalid component",
            ));
        }
        normalized.push(component);
    }
    Ok(normalized)
}

/// Serialize credential reads, refreshes, writes, registration and deletion
/// inside the current process using the normalized physical path.
pub async fn lock_credential_file(path: impl AsRef<Path>) -> std::io::Result<OwnedMutexGuard<()>> {
    let key = normalize_credential_path(path)?;
    let lock = {
        let locks = CREDENTIAL_FILE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut locks = locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks
            .entry(key)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    };
    Ok(lock.lock_owned().await)
}

fn write_login_info_unlocked(path: &Path, login_info: &LoginInfo) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(login_info)?;
    bytes.push(b'\n');
    let bytes = if is_encrypted_target(path) {
        configured_cipher()?
            .seal(&bytes)
            .map_err(|error| Kind::Custom(error.to_string()))?
    } else {
        bytes
    };
    write_bytes_unlocked(path, &bytes)
}

fn write_bytes_unlocked(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("credentials");
    let mut temporary_path;
    let mut temporary_file;
    loop {
        temporary_path = parent.join(format!(
            ".{file_name}.biliup-write-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temporary_path) {
            Ok(file) => {
                temporary_file = file;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    let write_result = (|| -> Result<()> {
        temporary_file.write_all(bytes)?;
        temporary_file.sync_all()?;
        replace_credential_file(&temporary_path, path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    write_result
}

/// Atomically write a credential file with owner-only permissions on Unix.
pub async fn save_login_info(path: impl AsRef<Path>, login_info: &LoginInfo) -> Result<()> {
    let path = normalize_credential_path(path)?;
    let _guard = lock_credential_file(&path).await?;
    write_login_info_unlocked(&path, login_info)
}

/// Refresh one credential file while holding the same lock used by readers and
/// atomic writers.
pub async fn renew_login_info_file(path: impl AsRef<Path>, proxy: Option<&str>) -> Result<()> {
    let path = normalize_credential_path(path)?;
    let _guard = lock_credential_file(&path).await?;
    let login_info = read_login_info(&path)?;
    let new_info = Credential::new(proxy).renew_tokens(login_info).await?;
    write_login_info_unlocked(&path, &new_info)
}

pub fn bilibili_from_info(login_info: LoginInfo, proxy: Option<&str>) -> Result<BiliBili> {
    let client = Credential::new(proxy);
    client.set_cookie(&login_info.cookie_info)?;
    debug!("通过cookie登录");
    Ok(BiliBili {
        client: client.0.client,
        login_info,
    })
}

pub async fn login_by_cookies(file: impl AsRef<Path>, proxy: Option<&str>) -> Result<BiliBili> {
    let path = normalize_credential_path(file)?;
    let _guard = lock_credential_file(&path).await?;
    let login_info = read_login_info(&path)?;

    let client: Credential = Credential::new(proxy);
    let need_refresh = client.validate_tokens(&login_info).await?;

    if need_refresh {
        let new_info = client.renew_tokens(login_info).await?;
        write_login_info_unlocked(&path, &new_info)?;
        bilibili_from_info(new_info, proxy)
    } else {
        debug!("无需更新cookie");
        bilibili_from_info(login_info, proxy)
    }
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum ResponseValue {
    Login(LoginInfo),
    OAuth(OAuthInfo),
    Value(serde_json::Value),
}

#[derive(Deserialize, Serialize, Clone)]
pub struct LoginInfo {
    pub cookie_info: serde_json::Value,
    // message: String,
    pub sso: Vec<String>,
    // status: u8,
    pub token_info: TokenInfo,
    // url: String,
    pub platform: Option<String>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct TokenInfo {
    pub access_token: String,
    expires_in: u32,
    pub mid: u64,
    refresh_token: String,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct OAuthInfo {
    pub mid: u64,
    pub access_token: String,
    pub expires_in: u32,
    pub refresh: bool,
}

impl std::fmt::Debug for ResponseValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Login(info) => f.debug_tuple("Login").field(info).finish(),
            Self::OAuth(info) => f.debug_tuple("OAuth").field(info).finish(),
            Self::Value(_) => f.debug_tuple("Value").field(&"[redacted]").finish(),
        }
    }
}

impl std::fmt::Debug for LoginInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginInfo")
            .field("cookie_info", &"[redacted]")
            .field("sso", &"[redacted]")
            .field("token_info", &self.token_info)
            .field("platform", &self.platform)
            .finish()
    }
}

impl std::fmt::Debug for TokenInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenInfo")
            .field("access_token", &"[redacted]")
            .field("expires_in", &self.expires_in)
            .field("mid", &self.mid)
            .field("refresh_token", &"[redacted]")
            .finish()
    }
}

impl std::fmt::Debug for OAuthInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthInfo")
            .field("mid", &self.mid)
            .field("access_token", &"[redacted]")
            .field("expires_in", &self.expires_in)
            .field("refresh", &self.refresh)
            .finish()
    }
}

#[cfg(test)]
mod redaction_tests {
    use super::{
        LoginInfo, OAuthInfo, ResponseValue, TokenInfo, bilibili_from_info, save_login_info,
    };
    use serde_json::json;

    fn login_info(token: &str) -> LoginInfo {
        LoginInfo {
            cookie_info: json!({"cookies": [{"name": "SESSDATA", "value": token}]}),
            sso: vec!["example.com".into()],
            token_info: TokenInfo {
                access_token: token.into(),
                expires_in: 3600,
                mid: 42,
                refresh_token: token.into(),
            },
            platform: Some("Android".into()),
        }
    }

    #[test]
    fn credential_debug_output_redacts_all_secret_material() {
        let mut login = login_info("cookie-secret");
        login.token_info.access_token = "access-secret".into();
        login.token_info.refresh_token = "refresh-secret".into();
        let login_debug = format!("{login:?}");
        for secret in ["cookie-secret", "access-secret", "refresh-secret"] {
            assert!(!login_debug.contains(secret));
        }
        assert!(login_debug.contains("[redacted]"));
        assert!(login_debug.contains("42"));

        let oauth = OAuthInfo {
            mid: 42,
            access_token: "oauth-secret".into(),
            expires_in: 3600,
            refresh: true,
        };
        assert!(!format!("{oauth:?}").contains("oauth-secret"));
        assert!(
            !format!("{:?}", ResponseValue::Value(json!({"token": "raw-secret"})))
                .contains("raw-secret")
        );
    }

    #[test]
    fn malformed_cookie_info_returns_an_error_without_panicking_or_leaking_values() {
        for cookies in [
            json!(null),
            json!({}),
            json!({"cookies": {}}),
            json!({"cookies": [{"value": "cookie-secret"}]}),
            json!({"cookies": [{"name": "", "value": "cookie-secret"}]}),
            json!({"cookies": [{"name": "SESSDATA", "value": 42}]}),
        ] {
            let mut login = login_info("cookie-secret");
            login.cookie_info = cookies;
            let error = bilibili_from_info(login, None).unwrap_err();
            assert!(!error.to_string().contains("cookie-secret"));
        }
    }

    #[test]
    fn valid_cookie_info_loads_every_cookie() {
        use reqwest::cookie::CookieStore;

        let credential = super::Credential::new(None);
        credential
            .set_cookie(&json!({"cookies": [
                {"name": "SESSDATA", "value": "session"},
                {"name": "bili_jct", "value": "csrf"}
            ]}))
            .unwrap();
        let header = credential
            .0
            .cookie_store
            .cookies(&url::Url::parse("https://member.bilibili.com/").unwrap())
            .unwrap();
        let header = header.to_str().unwrap();
        assert!(header.contains("SESSDATA=session"));
        assert!(header.contains("bili_jct=csrf"));
    }

    #[test]
    fn malformed_cookie_refresh_preserves_the_previous_cookie_store() {
        use reqwest::cookie::CookieStore;

        let credential = super::Credential::new(None);
        credential
            .set_cookie(&json!({"cookies": [
                {"name": "SESSDATA", "value": "previous-session"}
            ]}))
            .unwrap();
        let error = credential
            .set_cookie(&json!({"cookies": [
                {"name": "SESSDATA", "value": "replacement-secret"},
                {"name": "bili_jct", "value": null}
            ]}))
            .unwrap_err();
        assert!(!error.to_string().contains("replacement-secret"));

        let origin = url::Url::parse("https://member.bilibili.com/").unwrap();
        let header = credential.0.cookie_store.cookies(&origin).unwrap();
        assert_eq!(header.to_str().unwrap(), "SESSDATA=previous-session");
    }

    #[tokio::test]
    async fn concurrent_credential_writes_remain_atomic_and_parseable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let mut tasks = Vec::new();
        for index in 0..12 {
            let path = path.clone();
            tasks.push(tokio::spawn(async move {
                save_login_info(path, &login_info(&format!("token-{index}")))
                    .await
                    .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        let saved: LoginInfo =
            serde_json::from_reader(std::fs::File::open(&path).unwrap()).unwrap();
        assert!(saved.token_info.access_token.starts_with("token-"));
        assert_eq!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(|entry| entry.ok())
                .count(),
            1
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn atomic_save_updates_a_symlink_target_without_replacing_the_link() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.json");
        let link = dir.path().join("cookies.json");
        save_login_info(&target, &login_info("before"))
            .await
            .unwrap();
        symlink(&target, &link).unwrap();

        save_login_info(&link, &login_info("after")).await.unwrap();

        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let saved: LoginInfo =
            serde_json::from_reader(std::fs::File::open(target).unwrap()).unwrap();
        assert_eq!(saved.token_info.access_token, "after");
    }
}

#[cfg(test)]
mod encrypted_credential_tests {
    use super::{
        LoginInfo, TokenInfo, read_login_info, save_login_info, write_login_info_unlocked,
    };
    use biliup_accounts::KEY_FILE_ENV;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use tempfile::TempDir;

    static KEY_ENV: Mutex<()> = Mutex::new(());

    const DAMAGED: &str = "密钥不匹配或加密数据已损坏";
    const MISSING_KEY: &str = "密钥未配置";

    fn login_info(token: &str) -> LoginInfo {
        LoginInfo {
            cookie_info: json!({"cookies": [{"name": "SESSDATA", "value": token}]}),
            sso: vec!["example.com".into()],
            token_info: TokenInfo {
                access_token: token.into(),
                expires_in: 3600,
                mid: 42,
                refresh_token: token.into(),
            },
            platform: Some("Android".into()),
        }
    }

    fn write_key(dir: &Path, fill: u8) -> PathBuf {
        let path = dir.join("accounts.key");
        std::fs::write(&path, format!("{}\n", format!("{fill:02x}").repeat(32))).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        path
    }

    fn set_key(path: &Path) {
        unsafe { std::env::set_var(KEY_FILE_ENV, path) }
    }

    fn clear_key() {
        unsafe { std::env::remove_var(KEY_FILE_ENV) }
    }

    #[cfg(unix)]
    fn assert_private(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(not(unix))]
    fn assert_private(_path: &Path) {}

    #[tokio::test]
    async fn plaintext_files_keep_the_legacy_format() {
        let _guard = KEY_ENV.lock().unwrap_or_else(|error| error.into_inner());
        clear_key();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cookies.json");

        save_login_info(&path, &login_info("plain-secret")).await.unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert!(raw.starts_with(b"{"));
        assert!(!biliup_accounts::is_ciphertext(&raw));
        assert_private(&path);

        let loaded = read_login_info(&path).unwrap();
        assert_eq!(loaded.token_info.access_token, "plain-secret");

        let legacy = dir.path().join("legacy.json");
        std::fs::write(
            &legacy,
            serde_json::to_vec_pretty(&login_info("legacy-secret")).unwrap(),
        )
        .unwrap();
        let loaded = read_login_info(&legacy).unwrap();
        assert_eq!(loaded.token_info.access_token, "legacy-secret");
        assert_eq!(loaded.token_info.mid, 42);
    }

    #[tokio::test]
    async fn encrypted_files_round_trip_through_the_key_env() {
        let _guard = KEY_ENV.lock().unwrap_or_else(|error| error.into_inner());
        let dir = TempDir::new().unwrap();
        set_key(&write_key(dir.path(), 0x11));
        let path = dir.path().join("cookies.json.enc");

        save_login_info(&path, &login_info("enc-secret")).await.unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(&raw[..7], b"BUPACC1");
        assert!(!String::from_utf8_lossy(&raw).contains("enc-secret"));
        assert_private(&path);

        let loaded = read_login_info(&path).unwrap();
        assert_eq!(loaded.token_info.access_token, "enc-secret");
        assert_eq!(loaded.token_info.mid, 42);

        write_login_info_unlocked(&path, &loaded).unwrap();
        let rewritten = std::fs::read(&path).unwrap();
        assert!(biliup_accounts::is_ciphertext(&rewritten));
        assert_eq!(
            read_login_info(&path).unwrap().token_info.access_token,
            "enc-secret"
        );
        clear_key();
    }

    #[tokio::test]
    async fn wrong_key_and_tampered_ciphertext_are_rejected() {
        let _guard = KEY_ENV.lock().unwrap_or_else(|error| error.into_inner());
        let dir = TempDir::new().unwrap();
        let well = dir.path().join("well");
        let other = dir.path().join("other");
        std::fs::create_dir_all(&well).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        set_key(&write_key(&well, 0x22));
        let path = well.join("cookies.json.enc");
        save_login_info(&path, &login_info("guarded-secret")).await.unwrap();

        set_key(&write_key(&other, 0x33));
        let error = read_login_info(&path).unwrap_err().to_string();
        assert!(error.contains(DAMAGED), "{error}");
        assert!(!error.contains("guarded-secret"));

        set_key(&well.join("accounts.key"));
        let mut raw = std::fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0x01;
        std::fs::write(&path, &raw).unwrap();
        let error = read_login_info(&path).unwrap_err().to_string();
        assert!(error.contains(DAMAGED), "{error}");
        assert!(!error.contains("guarded-secret"));
        clear_key();
    }

    #[tokio::test]
    async fn unknown_ciphertext_version_reports_damage() {
        let _guard = KEY_ENV.lock().unwrap_or_else(|error| error.into_inner());
        let dir = TempDir::new().unwrap();
        set_key(&write_key(dir.path(), 0x66));
        let path = dir.path().join("future.json");
        let mut future = b"BUPACC2".to_vec();
        future.extend_from_slice(&[0u8; 64]);
        std::fs::write(&path, &future).unwrap();
        assert!(biliup_accounts::is_ciphertext(&future));

        let error = read_login_info(&path).unwrap_err().to_string();
        assert!(error.contains(DAMAGED), "{error}");
        clear_key();
    }

    #[tokio::test]
    async fn renamed_ciphertext_file_stays_encrypted_on_write_back() {
        let _guard = KEY_ENV.lock().unwrap_or_else(|error| error.into_inner());
        let dir = TempDir::new().unwrap();
        set_key(&write_key(dir.path(), 0x55));
        let encrypted = dir.path().join("cookies.json.enc");
        save_login_info(&encrypted, &login_info("renamed-secret"))
            .await
            .unwrap();

        let renamed = dir.path().join("x.json");
        std::fs::rename(&encrypted, &renamed).unwrap();
        save_login_info(&renamed, &login_info("renamed-secret"))
            .await
            .unwrap();

        let raw = std::fs::read(&renamed).unwrap();
        assert_eq!(&raw[..7], b"BUPACC1");
        assert!(!String::from_utf8_lossy(&raw).contains("renamed-secret"));
        assert_eq!(
            read_login_info(&renamed).unwrap().token_info.access_token,
            "renamed-secret"
        );
        assert_private(&renamed);

        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        clear_key();
    }

    #[tokio::test]
    async fn encrypted_files_without_a_key_report_the_missing_setting() {
        let _guard = KEY_ENV.lock().unwrap_or_else(|error| error.into_inner());
        let dir = TempDir::new().unwrap();
        set_key(&write_key(dir.path(), 0x44));
        let path = dir.path().join("cookies.json.enc");
        save_login_info(&path, &login_info("unconfigured-secret"))
            .await
            .unwrap();

        clear_key();
        let error = read_login_info(&path).unwrap_err().to_string();
        assert!(error.contains(MISSING_KEY), "{error}");
        assert!(!error.contains("unconfigured-secret"));

        let fresh = dir.path().join("fresh.json.enc");
        let error = save_login_info(&fresh, &login_info("fresh-secret"))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(MISSING_KEY), "{error}");
        assert!(!fresh.exists());
    }
}

#[derive(Debug)]
pub struct Credential(StatefulClient);

impl Credential {
    pub fn new(proxy: Option<&str>) -> Self {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            "Referer",
            header::HeaderValue::from_static("https://www.bilibili.com/"),
        );
        Self(StatefulClient::new(headers, proxy))
    }

    pub async fn validate_tokens(&self, login_info: &LoginInfo) -> Result<bool> {
        let keypair = match login_info.platform.as_deref() {
            Some("BiliTV") => AppKeyStore::BiliTV,
            Some("Android") | None => AppKeyStore::Android,
            Some(_) => return Err("未知平台".into()),
        };

        let payload = {
            let mut payload = json!({
                "access_key": login_info.token_info.access_token,
                "actionKey": "appkey",
                "appkey": keypair.app_key(),
                "ts": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            });

            let urlencoded = serde_urlencoded::to_string(&payload)?;
            let sign = Self::sign(&urlencoded, keypair.appsec());
            payload["sign"] = Value::from(sign);
            payload
        };

        let response: ResponseData<ResponseValue> = self
            .0
            .client
            .get("https://passport.bilibili.com/x/passport-login/oauth2/info")
            .query(&payload)
            .send()
            .await?
            .json()
            .await?;
        // if response.code != 0 {
        //     return Err(CustomError::Custom(response.to_string()));
        // }

        let response_code = response.code;
        let refresh = match response {
            ResponseData {
                data: Some(ResponseValue::OAuth(OAuthInfo { refresh, .. })),
                ..
            } => refresh,
            _ => {
                return Err(Kind::Custom(format!(
                    "credential validation failed with Bilibili code {response_code}"
                )));
            }
        };

        debug!("验证cookie");
        Ok(refresh)
    }

    pub async fn renew_tokens(&self, login_info: LoginInfo) -> Result<LoginInfo> {
        let keypair = match login_info.platform.as_deref() {
            Some("BiliTV") => AppKeyStore::BiliTV,
            Some("Android") | None => AppKeyStore::Android,
            Some(_) => return Err("未知平台".into()),
        };
        let payload = {
            let mut payload = json!({
                "access_key": login_info.token_info.access_token,
                "actionKey": "appkey",
                "appkey": keypair.app_key(),
                "refresh_token": login_info.token_info.refresh_token,
                "ts": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            });

            let urlencoded = serde_urlencoded::to_string(&payload)?;
            let sign = Self::sign(&urlencoded, keypair.appsec());
            payload["sign"] = Value::from(sign);
            payload
        };
        let response: ResponseData<ResponseValue> = self
            .0
            .client
            .post("https://passport.bilibili.com/x/passport-login/oauth2/refresh_token")
            .form(&payload)
            .send()
            .await?
            .json()
            .await?;
        info!("更新cookie");
        let response_code = response.code;
        match response.data {
            Some(ResponseValue::Login(info)) if !info.cookie_info.is_null() => {
                self.set_cookie(&info.cookie_info)?;
                Ok(LoginInfo {
                    platform: login_info.platform,
                    ..info
                })
            }
            _ => Err(Kind::Custom(format!(
                "credential refresh failed with Bilibili code {response_code}"
            ))),
        }
    }

    pub async fn login_by_password(&self, username: &str, password: &str) -> Result<LoginInfo> {
        // The type of `payload` is `serde_json::Value`
        let mut rng = rand::thread_rng();
        let (key_hash, pub_key) = self.get_key().await?;
        let pub_key = RsaPublicKey::from_public_key_pem(&pub_key).unwrap();
        let enc_data = pub_key
            .encrypt(&mut rng, Pkcs1v15Encrypt, (key_hash + password).as_bytes())
            .expect("failed to encrypt");
        let encrypt_password = general_purpose::STANDARD_NO_PAD.encode(enc_data);
        let mut payload = json!({
            "actionKey": "appkey",
            "appkey": AppKeyStore::Android.app_key(),
            "build": 6270200,
            "captcha": "",
            "challenge": "",
            "channel": "bili",
            "device": "phone",
            "mobi_app": "android",
            "password": encrypt_password,
            "permission": "ALL",
            "platform": "android",
            "seccode": "",
            "subid": 1,
            "ts": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            "username": username,
            "validate": "",
        });
        let urlencoded = serde_urlencoded::to_string(&payload)?;
        let sign = Self::sign(&urlencoded, AppKeyStore::Android.appsec());
        payload["sign"] = Value::from(sign);
        let response: ResponseData<ResponseValue> = self
            .0
            .client
            .post("https://passport.bilibili.com/x/passport-login/oauth2/login")
            .form(&payload)
            .send()
            .await?
            .json()
            .await?;
        info!("通过密码登录");
        let response_code = response.code;
        match response.data {
            Some(ResponseValue::Login(info)) if !info.cookie_info.is_null() => {
                self.set_cookie(&info.cookie_info)?;
                Ok(LoginInfo {
                    platform: Some("Android".to_string()),
                    ..info
                })
            }
            _ => Err(Kind::Custom(format!(
                "password login failed with Bilibili code {response_code}"
            ))),
        }
    }

    pub async fn login_by_sms(
        &self,
        code: u32,
        mut payload: serde_json::Value,
    ) -> Result<LoginInfo> {
        payload["code"] = Value::from(code);
        let urlencoded = serde_urlencoded::to_string(&payload)?;
        let sign = Self::sign(&urlencoded, AppKeyStore::Android.appsec());
        payload["sign"] = Value::from(sign);
        let res: ResponseData<ResponseValue> = self
            .0
            .client
            .post("https://passport.bilibili.com/x/passport-login/login/sms")
            .form(&payload)
            .send()
            .await?
            .json()
            .await?;
        let response_code = res.code;
        match res.data {
            Some(ResponseValue::Login(info)) => {
                self.set_cookie(&info.cookie_info)?;
                Ok(LoginInfo {
                    platform: Some("Android".to_string()),
                    ..info
                })
            }
            _ => Err(Kind::Custom(format!(
                "SMS login failed with Bilibili code {response_code}"
            ))),
        }
    }

    pub async fn send_sms(
        &self,
        phone_number: u64,
        country_code: u32,
    ) -> Result<serde_json::Value> {
        self.send_sms_with_recaptcha(phone_number, country_code, None, None, None)
            .await
    }

    pub async fn send_sms_handle_recaptcha<F, Fut>(
        &self,
        phone_number: u64,
        country_code: u32,
        recaptcha_handler: F,
    ) -> Result<serde_json::Value>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<(String, String)>>,
    {
        let url_string = match self
            .send_sms_with_recaptcha(phone_number, country_code, None, None, None)
            .await
        {
            Ok(res) => return Ok(res),
            Err(Kind::NeedRecaptcha(url)) => url,
            Err(e) => return Err(e),
        };

        let recaptcha = {
            Url::parse(&url_string)
                .map_err(|_| Kind::from("url parse error"))?
                .query_pairs()
                .find(|(k, _)| k == "recaptcha_token")
                .map(|(_, v)| v.to_string())
                .ok_or(Kind::from("cannot find recaptcha_token"))
        }?;

        info!("需要滑动验证码");
        let (challenge, validate) = recaptcha_handler(url_string).await?;

        self.send_sms_with_recaptcha(
            phone_number,
            country_code,
            Some(challenge.as_str()),
            Some(validate.as_str()),
            Some(recaptcha.as_ref()),
        )
        .await
    }

    pub async fn send_sms_with_recaptcha(
        &self,
        phone_number: u64,
        country_code: u32,
        challenge: Option<&str>,
        validate: Option<&str>,
        recaptcha: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut payload = json!({
            "actionKey": "appkey",
            "appkey": AppKeyStore::Android.app_key(),
            "build": 6510400,
            "buvid": &self.0.buvid,
            "channel": "bili",
            "cid": country_code,
            "device": "phone",
            "mobi_app": "android",
            "platform": "android",
            // "platform": "pc",
            "tel": phone_number,
            "ts": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        });

        if let (Some(c), Some(v), Some(r)) = (challenge, validate, recaptcha) {
            payload["gee_challenge"] = Value::from(c);
            payload["gee_seccode"] = Value::from(format!("{v}|jordan"));
            payload["gee_validate"] = Value::from(v);
            payload["recaptcha_token"] = Value::from(r);
        }

        let urlencoded = serde_urlencoded::to_string(&payload)?;
        let sign = Self::sign(&urlencoded, AppKeyStore::Android.appsec());
        let urlencoded = format!("{}&sign={}", urlencoded, sign);
        // let mut form = payload.clone();
        // form["sign"] = Value::from(sign);
        let res: ResponseData<ResponseValue> = self
            .0
            .client
            .post("https://passport.bilibili.com/x/passport-login/sms/send")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(urlencoded)
            .send()
            .await?
            .json()
            .await?;
        // println!("{}", res);
        let response_code = res.code;
        match res.data {
            Some(ResponseValue::Value(mut data))
                if !data["captcha_key"]
                    .as_str()
                    .ok_or("send sms error")?
                    .is_empty() =>
            {
                payload["captcha_key"] = data["captcha_key"].take();
                Ok(payload)
            }
            Some(ResponseValue::Value(data))
                if !data["recaptcha_url"].as_str().unwrap_or("").is_empty() =>
            {
                let url = data["recaptcha_url"].as_str().unwrap().to_string();
                Err(Kind::NeedRecaptcha(url))
            }
            _ => Err(Kind::Custom(format!(
                "SMS request failed with Bilibili code {response_code}"
            ))),
        }
    }

    pub async fn login_by_qrcode(&self, value: Value) -> Result<LoginInfo> {
        let mut form = json!({
            "appkey": AppKeyStore::BiliTV.app_key(),
            "auth_code": value["data"]["auth_code"],
            "local_id": "0",
            "ts": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
        });
        let urlencoded = serde_urlencoded::to_string(&form)?;
        let sign = Self::sign(&urlencoded, AppKeyStore::BiliTV.appsec());
        form["sign"] = Value::from(sign);
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let raw = self
                .0
                .client
                .post("https://passport.bilibili.com/x/passport-tv-login/qrcode/poll")
                .form(&form)
                .send()
                .await?
                .error_for_status()?;
            let full = raw.bytes().await?;

            let res: ResponseData<ResponseValue> = serde_json::from_slice(&full).map_err(|_| {
                Kind::Custom(format!(
                    "error decoding credential API response body ({} bytes)",
                    full.len()
                ))
            })?;
            match res {
                ResponseData {
                    code: 0,
                    data: Some(ResponseValue::Login(info)),
                    ..
                } => {
                    self.set_cookie(&info.cookie_info)?;
                    break Ok(LoginInfo {
                        platform: Some("BiliTV".to_string()),
                        ..info
                    });
                }
                ResponseData { code: 86039, .. } => {
                    // 二维码尚未确认;
                    // form["ts"] = Value::from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
                }
                _ => {
                    break Err(Kind::Custom(format!("{res:#?}")));
                }
            }
        }
    }

    /// 获取 Web 端 buvid3 和 buvid4
    pub async fn get_web_buvid(&self) -> Result<(String, String)> {
        let res: ResponseData<Value> = self
            .0
            .client
            .get("https://api.bilibili.com/x/frontend/finger/spi")
            .send()
            .await?
            .json()
            .await?;
        match res.data {
            Some(value) => {
                let buvid3 = value["b_3"].as_str().ok_or("cannot find b_3")?.to_owned();
                let buvid4 = value["b_4"].as_str().ok_or("cannot find b_4")?.to_owned();
                Ok((buvid3, buvid4))
            }
            None => Err(Kind::Custom(format!("cannot find buvid: {:#?}", res))),
        }
    }

    pub async fn get_qrcode(&self) -> Result<Value> {
        let mut form = json!({
            "appkey": AppKeyStore::BiliTV.app_key(),
            "local_id": "0",
            "ts": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
        });
        let urlencoded = serde_urlencoded::to_string(&form)?;
        let sign = Self::sign(&urlencoded, AppKeyStore::BiliTV.appsec());
        form["sign"] = Value::from(sign);
        Ok(self
            .0
            .client
            .post("https://passport.bilibili.com/x/passport-tv-login/qrcode/auth_code")
            .form(&form)
            .send()
            .await?
            .json()
            .await?)
    }

    pub async fn get_key(&self) -> Result<(String, String)> {
        let payload = json!({
            "appkey": AppKeyStore::Android.app_key(),
            "sign": Credential::sign(&format!("appkey={}", AppKeyStore::Android.app_key()), AppKeyStore::Android.appsec()),
        });
        let response: Value = self
            .0
            .client
            .get("https://passport.bilibili.com/x/passport-login/web/key")
            .json(&payload)
            .send()
            .await?
            .json()
            .await?;
        let response = response
            .get("data")
            .ok_or_else(|| Kind::Custom(response.to_string()))?;
        let hash = response
            .get("hash")
            .and_then(Value::as_str)
            .ok_or_else(|| Kind::Custom(response.to_string()))?;
        let key = response
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| Kind::Custom(response.to_string()))?;
        Ok((hash.to_string(), key.to_string()))
    }

    pub async fn login_by_web_qrcode(
        &self,
        sess_data: &str,
        dede_user_id: &str,
    ) -> Result<LoginInfo> {
        info!("login_by_web_qrcode");
        let qrcode: Value = self.0.client
            .get("https://passport.bilibili.com/qrcode/getLoginUrl")
            .header(USER_AGENT, "Mozilla/5.0 (X11; Linux x86_64; rv:38.0) Gecko/20100101 Firefox/38.0 Iceweasel/38.2.1 BiliApp")
            .send()
            .await?
            .json()
            .await?;
        let oauth_key = qrcode["data"]["oauthKey"].as_str();
        let cookies = format!("SESSDATA={sess_data}; DedeUserID={dede_user_id}");
        self.0.client
            .post("https://passport.bilibili.com/qrcode/login/confirm")
            .header(USER_AGENT, "Mozilla/5.0 (X11; Linux x86_64; rv:38.0) Gecko/20100101 Firefox/38.0 Iceweasel/38.2.1 BiliApp")
            .header(COOKIE, cookies)
            .header(REFERER, "https://passport.bilibili.com/mobile/h5-confirm.html")
            .header(ORIGIN, "https://passport.bilibili.com")
            .form(&[("oauthKey", oauth_key)])
            .send()
            .await?.error_for_status()?;
        self.0.client
            .post("https://passport.bilibili.com/qrcode/getLoginInfo")
            .header(USER_AGENT, "Mozilla/5.0 (X11; Linux x86_64; rv:38.0) Gecko/20100101 Firefox/38.0 Iceweasel/38.2.1 BiliApp")
            .form(&[("oauthKey", oauth_key)])
            .send()
            .await?.error_for_status()?;
        let sess_data = self.get_cookie("SESSDATA")?;
        let bili_jct = self.get_cookie("bili_jct")?;
        self.login_by_web_cookies(&sess_data, &bili_jct).await
    }

    pub async fn login_by_web_cookies(&self, sess_data: &str, bili_jct: &str) -> Result<LoginInfo> {
        info!("获取二维码");
        let qrcode = self.get_qrcode().await?;
        let auth_code = qrcode["data"]["auth_code"]
            .as_str()
            .ok_or("Cannot get auth_code")?;
        self.web_confirm_qrcode(auth_code, sess_data, bili_jct)
            .await?;
        self.login_by_qrcode(qrcode).await
    }

    async fn web_confirm_qrcode(
        &self,
        auth_code: &str,
        sess_data: &str,
        bili_jct: &str,
    ) -> Result<()> {
        let form = json!({
            "auth_code": auth_code,
            "csrf": bili_jct,
            "scanning_type": 3,
        });
        let cookies = format!("SESSDATA={}; bili_jct={}", sess_data, bili_jct);
        info!("自动确认二维码");
        let response = self.0.client
            .post("https://passport.bilibili.com/x/passport-tv-login/h5/qrcode/confirm")
            .header("Cookie", cookies)
            // .header("native_api_from", "h5")
            .header(USER_AGENT, "Mozilla/5.0 (X11; Linux x86_64; rv:38.0) Gecko/20100101 Firefox/38.0 Iceweasel/38.2.1 BiliApp")
            .form(&form)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(Kind::Custom(format!(
                "Web QR confirmation failed with HTTP {}",
                response.status()
            )));
        }
        let res: ResponseData = response.json().await?;
        if res.code != 0 {
            return Err(Kind::Custom(format!(
                "Web QR confirmation failed with Bilibili code {}",
                res.code
            )));
        }
        Ok(())
    }

    pub fn sign(param: &str, app_sec: &str) -> String {
        let mut hasher = Md5::new();
        // process input message
        hasher.update(format!("{}{}", param, app_sec));
        // acquire hash digest in the form of GenericArray,
        // which in this case is equivalent to [u8; 16]
        format!("{:x}", hasher.finalize())
    }

    fn set_cookie(&self, cookie_info: &serde_json::Value) -> Result<()> {
        let cookies = cookie_info["cookies"]
            .as_array()
            .ok_or_else(|| Kind::Custom("credential cookies must be an array".into()))?;
        let mut store = self
            .0
            .cookie_store
            .lock()
            .map_err(|_| Kind::Custom("credential cookie store is unavailable".into()))?;
        let mut updated = store.clone();
        let origin = Url::parse("https://bilibili.com/").expect("static cookie origin");
        for (index, cookie) in cookies.iter().enumerate() {
            let name = cookie["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or_else(|| Kind::Custom(format!("credential cookie {index} has no name")))?;
            let value = cookie["value"].as_str().ok_or_else(|| {
                Kind::Custom(format!("credential cookie {index} has no string value"))
            })?;
            let cookie = Cookie::build((name, value)).domain("bilibili.com").into();

            updated
                .insert_raw(&cookie, &origin)
                .map_err(|_| Kind::Custom(format!("credential cookie {index} is invalid")))?;
        }
        *store = updated;
        Ok(())
    }

    fn get_cookie(&self, name: &str) -> Result<String> {
        let store = self.0.cookie_store.lock().unwrap();
        for item in store.iter_any() {
            if item.name() == name {
                return Ok(item.value().to_string());
            }
        }
        Err(Kind::Custom(format!("{name} not exist")))
    }
}

impl Default for Credential {
    fn default() -> Self {
        Self::new(None)
    }
}
