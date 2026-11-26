//! 转播选账号用的 SQLite 账号快照：只用临时库与合成凭据，不联网。

use biliup_accounts::{AccountProvider, AccountStore};
use biliup_cli::SqliteAccountProvider;
use biliup_cli::server::infrastructure::connection_pool::{ConnectionManager, ConnectionPool};
use serde_json::{Value, json};
use std::sync::Mutex;

static KEY_ENV_LOCK: Mutex<()> = Mutex::new(());

fn login_info(mid: u64, sessdata: &str) -> Value {
    json!({
        "cookie_info": {"cookies": [
            {"name": "SESSDATA", "value": sessdata},
            {"name": "bili_jct", "value": "synthetic-csrf"},
            {"name": "DedeUserID", "value": mid.to_string()},
            {"name": "DedeUserID__ckMd5", "value": "synthetic-ckmd5"},
            {"name": "buvid3", "value": "synthetic-buvid3"},
        ]},
        "sso": [],
        "token_info": {
            "access_token": "synthetic-access-token",
            "expires_in": 3600,
            "mid": mid,
            "refresh_token": "synthetic-refresh-token",
        },
        "platform": "Android",
    })
}

async fn register(pool: &ConnectionPool, path: &str) -> i64 {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO configuration (key, value) VALUES ('bilibili-cookies', ?1) RETURNING id",
    )
    .bind(path)
    .fetch_one(pool)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn registered_credentials_are_listed_and_resolved_by_row_id() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.json");
    std::fs::write(
        &good,
        login_info(1234, "synthetic-sessdata-7f3a").to_string(),
    )
    .unwrap();
    let broken = dir.path().join("broken.json");
    std::fs::write(&broken, "synthetic-not-a-credential").unwrap();
    let missing = dir.path().join("gone.json");

    let pool = ConnectionManager::new_pool(dir.path().join("data.sqlite3").to_str().unwrap())
        .await
        .unwrap();
    let good_id = register(&pool, good.to_str().unwrap()).await;
    let broken_id = register(&pool, broken.to_str().unwrap()).await;
    let missing_id = register(&pool, missing.to_str().unwrap()).await;

    let provider = SqliteAccountProvider::load(&pool).await;
    let listed = provider.list();
    assert_eq!(listed.len(), 1, "读不了的账号必须跳过，不能让整份快照变空");
    assert_eq!(listed[0].id, good_id);
    assert_eq!(listed[0].mid, 1234);

    let document = provider.resolve(good_id).expect("登记的账号应能解析出凭据");
    assert_eq!(
        document["cookie_info"]["cookies"][0]["value"],
        json!("synthetic-sessdata-7f3a")
    );
    assert_eq!(document["token_info"]["mid"], json!(1234));
    assert!(provider.resolve(broken_id).is_none());
    assert!(provider.resolve(missing_id).is_none());
    assert!(provider.resolve(good_id + 1000).is_none());
}

#[tokio::test]
async fn a_credential_that_does_not_decrypt_with_the_active_key_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let key = dir.path().join("accounts.key");
    let store = AccountStore::open(&data, Some(&key)).unwrap();
    let encrypted = store
        .save(4321, &login_info(4321, "synthetic-sessdata-9c1d"))
        .unwrap();
    assert!(std::fs::read(&encrypted).unwrap().starts_with(b"BUPACC1"));

    let other = dir.path().join("other");
    let stranger = AccountStore::open(&other, Some(&other.join("accounts.key"))).unwrap();
    let foreign = stranger
        .save(5678, &login_info(5678, "synthetic-sessdata-4e2b"))
        .unwrap();

    let pool = ConnectionManager::new_pool(dir.path().join("data.sqlite3").to_str().unwrap())
        .await
        .unwrap();
    let encrypted_id = register(&pool, encrypted.to_str().unwrap()).await;
    let foreign_id = register(&pool, foreign.to_str().unwrap()).await;

    unsafe { std::env::remove_var(biliup_accounts::KEY_FILE_ENV) };
    let provider = SqliteAccountProvider::load(&pool).await;
    assert!(
        provider.list().is_empty(),
        "密钥未配置时密文账号一律读不了，也不能编造凭据"
    );

    let _guard = KEY_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var(biliup_accounts::KEY_FILE_ENV, &key) };
    provider.reload(&pool).await;

    let listed = provider.list();
    assert_eq!(
        listed
            .iter()
            .map(|account| (account.id, account.mid))
            .collect::<Vec<_>>(),
        [(encrypted_id, 4321)]
    );
    assert_eq!(
        provider.resolve(encrypted_id).unwrap()["cookie_info"]["cookies"][0]["value"],
        json!("synthetic-sessdata-9c1d")
    );
    assert!(
        provider.resolve(foreign_id).is_none(),
        "别的密钥的账号要跳过"
    );
    unsafe { std::env::remove_var(biliup_accounts::KEY_FILE_ENV) };
}
