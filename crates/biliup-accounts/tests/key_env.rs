use biliup_accounts::{AccountStore, KEY_FILE_ENV};
use serde_json::json;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

#[test]
fn key_path_env_override_and_key_validation() {
    let dir = TempDir::new().unwrap();
    let data = dir.path().join("data");
    let custom = dir.path().join("keys").join("custom.key");
    let value = json!({"token_info": {"mid": 5}});

    unsafe { std::env::set_var(KEY_FILE_ENV, &custom) };
    let store = AccountStore::open(&data, None).unwrap();
    store.save(5, &value).unwrap();
    assert!(custom.try_exists().unwrap());
    assert!(!data.join("accounts.key").try_exists().unwrap());
    assert_eq!(
        fs::metadata(&custom).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(store.load(5).unwrap(), value);
    unsafe { std::env::remove_var(KEY_FILE_ENV) };

    fs::set_permissions(&custom, fs::Permissions::from_mode(0o644)).unwrap();
    let error = AccountStore::open(&data, Some(&custom)).err().unwrap();
    assert_eq!(error.kind(), ErrorKind::PermissionDenied);

    let short = dir.path().join("short.key");
    fs::write(&short, b"abc\n").unwrap();
    fs::set_permissions(&short, fs::Permissions::from_mode(0o600)).unwrap();
    let error = AccountStore::open(&data, Some(&short)).err().unwrap();
    assert_eq!(error.kind(), ErrorKind::InvalidData);

    let error = AccountStore::open(&data, None).err().unwrap();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.to_string().contains("密钥文件缺失"));
    assert!(!data.join("accounts.key").try_exists().unwrap());
}
