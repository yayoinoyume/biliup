//! 启动时的账号迁移：`data/<mid>.json` 明文 → `data/accounts/<mid>.json.enc`，
//! 同一个事务里把 `configuration.value` 与 `uploadstreamers.user_cookie` 改成新路径。
//! 全程只用临时目录与合成凭据，不联网。

use biliup_cli::server::infrastructure::connection_pool::{ConnectionManager, ConnectionPool};
use biliup_cli::server::infrastructure::repositories::migrate_plaintext_accounts;
use std::fs;
use std::path::{Path, PathBuf};

const MID: u64 = 1234567890;

fn credential_json(mid: u64) -> String {
    format!(
        r#"{{
            "cookie_info": {{ "cookies": [] }},
            "sso": [],
            "token_info": {{
                "access_token": "synthetic-access-token",
                "expires_in": 3600,
                "mid": {mid},
                "refresh_token": "synthetic-refresh-token"
            }},
            "platform": "Android"
        }}"#
    )
}

struct Fixture {
    root: tempfile::TempDir,
    data: PathBuf,
    pool: ConnectionPool,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let pool = ConnectionManager::new_pool(data.join("data.sqlite3").to_str().unwrap())
            .await
            .unwrap();
        Self { root, data, pool }
    }

    fn plaintext(&self) -> PathBuf {
        self.data.join(format!("{MID}.json"))
    }

    fn encrypted(&self) -> PathBuf {
        self.data.join("accounts").join(format!("{MID}.json.enc"))
    }

    fn backup_root(&self) -> PathBuf {
        self.data.join(".backups")
    }

    async fn register(&self, value: &str, templates: usize) {
        sqlx::query("INSERT INTO configuration (key, value) VALUES ('bilibili-cookies', ?1)")
            .bind(value)
            .execute(&self.pool)
            .await
            .unwrap();
        for index in 0..templates {
            sqlx::query(
                "INSERT INTO uploadstreamers (template_name, tags, user_cookie) VALUES (?1, '[]', ?2)",
            )
            .bind(format!("template-{index}"))
            .bind(value)
            .execute(&self.pool)
            .await
            .unwrap();
        }
    }

    async fn configuration_values(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT value FROM configuration WHERE key = 'bilibili-cookies'")
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }

    async fn user_cookies(&self) -> Vec<Option<String>> {
        sqlx::query_scalar("SELECT user_cookie FROM uploadstreamers")
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }
}

fn canonical(path: &Path) -> String {
    path.canonicalize().unwrap().to_string_lossy().into_owned()
}

#[tokio::test]
async fn migrates_plaintext_accounts_and_repoints_both_tables() {
    let fixture = Fixture::new().await;
    let plaintext = fixture.plaintext();
    fs::write(&plaintext, credential_json(MID)).unwrap();
    // 登记行与两条模板都用同一个绝对路径，迁移后都该指向密文
    let registered = canonical(&plaintext);
    fixture.register(&registered, 2).await;

    let report = migrate_plaintext_accounts(&fixture.pool, &fixture.data, &fixture.backup_root())
        .await
        .unwrap();

    assert_eq!(report.migrated, 1);
    let encrypted = fixture.encrypted();
    assert!(encrypted.is_file());
    assert!(fs::read(&encrypted).unwrap().starts_with(b"BUPACC1"));
    assert!(!plaintext.exists());

    let backups: Vec<_> = fs::read_dir(fixture.backup_root())
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert!(
        backups[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("accounts-migration-")
    );
    assert!(backups[0].join(format!("{MID}.json")).is_file());

    let dest = canonical(&encrypted);
    assert_eq!(fixture.configuration_values().await, vec![dest.clone()]);
    assert_eq!(fixture.user_cookies().await, vec![Some(dest.clone()); 2]);

    // 再跑一次：没有明文可迁，也不该再改库
    let again = migrate_plaintext_accounts(&fixture.pool, &fixture.data, &fixture.backup_root())
        .await
        .unwrap();
    assert_eq!(again.migrated, 0);
    assert_eq!(again.repointed, 0);
    assert!(encrypted.is_file());
    assert_eq!(fixture.configuration_values().await, vec![dest]);
    assert_eq!(
        fs::read_dir(fixture.backup_root()).unwrap().count(),
        1,
        "第二次运行不该再建备份目录"
    );
    assert!(fixture.root.path().is_dir());
}

#[tokio::test]
async fn leaves_json_that_is_not_an_account_alone() {
    let fixture = Fixture::new().await;
    let notes = fixture.data.join("notes.json");
    let broken = fixture.data.join("broken.json");
    fs::write(&notes, r#"{"hello": "world"}"#).unwrap();
    fs::write(&broken, "not json").unwrap();

    let report = migrate_plaintext_accounts(&fixture.pool, &fixture.data, &fixture.backup_root())
        .await
        .unwrap();

    assert_eq!(report.migrated, 0);
    assert_eq!(report.repointed, 0);
    assert!(notes.is_file());
    assert!(broken.is_file());
    assert!(!fixture.backup_root().exists());
    assert!(fixture.configuration_values().await.is_empty());
}
