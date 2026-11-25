use biliup_accounts::AccountStore;
use serde_json::json;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

fn mode(path: &std::path::Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn rejected(store: &AccountStore, mid: u64, bytes: &[u8]) {
    fs::write(store.path_for(mid), bytes).unwrap();
    let error = store.load(mid).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.to_string().contains("密钥不匹配或加密数据已损坏"));
}

#[test]
fn round_trip_keeps_plaintext_out_of_file() {
    let dir = TempDir::new().unwrap();
    let store = AccountStore::open(dir.path(), None).unwrap();
    let value = json!({
        "cookie_info": {"cookies": [{"name": "SESSDATA", "value": "secret"}]},
        "token_info": {"mid": 4242, "access_token": "secret-access", "refresh_token": "secret-refresh"},
        "sso": ["https://passport.bilibili.com"],
        "platform": "BiliTV"
    });

    let path = store.save(4242, &value).unwrap();
    assert_eq!(path, store.path_for(4242));
    assert_eq!(path, dir.path().join("accounts").join("4242.json.enc"));

    let raw = fs::read(&path).unwrap();
    assert_eq!(&raw[..7], b"BUPACC1");
    assert_ne!(raw[0], b'{');
    let text = String::from_utf8_lossy(&raw);
    assert!(!text.contains("secret-access"));
    assert!(!text.contains("SESSDATA"));
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&dir.path().join("accounts")), 0o700);

    let loaded = store.load(4242).unwrap();
    assert_eq!(loaded, value);

    let key_path = dir.path().join("accounts.key");
    assert_eq!(mode(&key_path), 0o600);
    let key = fs::read_to_string(&key_path).unwrap();
    assert_eq!(key.len(), 65);
    assert!(key.ends_with('\n'));
    assert!(key.trim().chars().all(|c| c.is_ascii_hexdigit()));

    let reopened = AccountStore::open(dir.path(), None).unwrap();
    assert_eq!(fs::read_to_string(&key_path).unwrap(), key);
    assert_eq!(reopened.load(4242).unwrap(), value);
}

#[test]
fn wrong_key_is_rejected() {
    let dir = TempDir::new().unwrap();
    let store = AccountStore::open(dir.path(), None).unwrap();
    store.save(7, &json!({"token_info": {"mid": 7}})).unwrap();

    fs::remove_file(dir.path().join("accounts.key")).unwrap();
    let other = AccountStore::open(dir.path(), None).unwrap();
    let error = other.load(7).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.to_string().contains("密钥不匹配或加密数据已损坏"));
}

#[test]
fn damaged_files_are_rejected_without_panic() {
    let dir = TempDir::new().unwrap();
    let store = AccountStore::open(dir.path(), None).unwrap();
    let path = store.save(9, &json!({"token_info": {"mid": 9}})).unwrap();
    let good = fs::read(&path).unwrap();

    let mut tampered = good.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    rejected(&store, 9, &tampered);

    rejected(&store, 9, &good[..good.len() - 1]);

    rejected(&store, 9, &good[..7]);

    let mut foreign = good.clone();
    foreign[..7].copy_from_slice(b"BUPACC2");
    rejected(&store, 9, &foreign);

    rejected(&store, 9, &[]);

    let missing = store.load(1234).unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound);
}
