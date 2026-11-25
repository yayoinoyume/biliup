use biliup_accounts::AccountStore;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn backup_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn migrates_plaintext_accounts_and_stays_idempotent() {
    let dir = TempDir::new().unwrap();
    let data = dir.path().join("data");
    let backup = dir.path().join("backup");
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&backup).unwrap();

    fs::write(
        data.join("1001.json"),
        json!({"token_info": {"mid": 1001, "access_token": "a"}, "sso": []}).to_string(),
    )
    .unwrap();
    fs::write(
        data.join("1002.json"),
        json!({"token_info": {"mid": 1002, "access_token": "b"}}).to_string(),
    )
    .unwrap();
    fs::write(
        data.join("nomid.json"),
        json!({"hello": "world"}).to_string(),
    )
    .unwrap();
    fs::write(data.join("broken.json"), "{not json").unwrap();

    let store = AccountStore::open(&data, None).unwrap();
    let migrated = store.migrate_plaintext(&backup).unwrap();
    let mut mids: Vec<u64> = migrated.iter().map(|account| account.mid).collect();
    mids.sort();
    assert_eq!(mids, vec![1001, 1002]);

    for account in &migrated {
        assert_eq!(account.dest, store.path_for(account.mid));
        assert!(!account.source.try_exists().unwrap());
        assert!(account.dest.try_exists().unwrap());
        let loaded = store.load(account.mid).unwrap();
        assert_eq!(
            loaded.pointer("/token_info/mid").and_then(|v| v.as_u64()),
            Some(account.mid)
        );
    }
    assert_eq!(fs::read(store.path_for(1001)).unwrap()[..7], *b"BUPACC1");

    let dirs = backup_dirs(&backup);
    assert_eq!(dirs.len(), 1);
    let folder = &dirs[0];
    assert!(
        folder
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("accounts-migration-")
    );
    assert!(folder.join("1001.json").try_exists().unwrap());
    assert!(folder.join("1002.json").try_exists().unwrap());
    assert!(data.join("nomid.json").try_exists().unwrap());
    assert!(data.join("broken.json").try_exists().unwrap());

    let again = store.migrate_plaintext(&backup).unwrap();
    assert!(again.is_empty());
    assert_eq!(backup_dirs(&backup).len(), 1);
}

#[test]
fn existing_ciphertext_wins_over_plaintext() {
    let dir = TempDir::new().unwrap();
    let data = dir.path().join("data");
    let backup = dir.path().join("backup");
    fs::create_dir_all(&data).unwrap();

    let store = AccountStore::open(&data, None).unwrap();
    let encrypted = json!({"token_info": {"mid": 3033, "access_token": "encrypted"}});
    store.save(3033, &encrypted).unwrap();
    fs::write(
        data.join("3033.json"),
        json!({"token_info": {"mid": 3033, "access_token": "stale"}}).to_string(),
    )
    .unwrap();

    let migrated = store.migrate_plaintext(&backup).unwrap();
    assert!(migrated.is_empty());
    assert!(!data.join("3033.json").try_exists().unwrap());
    assert_eq!(store.load(3033).unwrap(), encrypted);

    let dirs = backup_dirs(&backup);
    assert_eq!(dirs.len(), 1);
    assert!(dirs[0].join("3033.json").try_exists().unwrap());
}

#[test]
fn skips_oversized_odd_and_nested_entries() {
    let dir = TempDir::new().unwrap();
    let data = dir.path().join("data");
    let backup = dir.path().join("backup");
    fs::create_dir_all(&data).unwrap();

    let store = AccountStore::open(&data, None).unwrap();
    fs::write(
        data.join("7001.json"),
        json!({"token_info": {"mid": 7001}}).to_string(),
    )
    .unwrap();
    fs::write(data.join("huge.json"), vec![b'0'; 8 * 1024 * 1024 + 2]).unwrap();
    fs::write(
        data.join("stringmid.json"),
        json!({"token_info": {"mid": "7002"}}).to_string(),
    )
    .unwrap();
    fs::write(
        data.join("accounts").join("7003.json"),
        json!({"token_info": {"mid": 7003}}).to_string(),
    )
    .unwrap();

    let migrated = store.migrate_plaintext(&backup).unwrap();
    let mids: Vec<u64> = migrated.iter().map(|account| account.mid).collect();
    assert_eq!(mids, vec![7001]);
    assert!(!store.path_for(7002).try_exists().unwrap());
    assert!(!store.path_for(7003).try_exists().unwrap());
    assert!(data.join("huge.json").try_exists().unwrap());
    assert!(data.join("stringmid.json").try_exists().unwrap());
    assert!(data.join("accounts").join("7003.json").try_exists().unwrap());
}
