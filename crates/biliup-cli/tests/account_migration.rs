//! 启动时的账号迁移：`data/<mid>.json` 明文 → `data/accounts/<mid>.json.enc`，
//! 每个账号「写密文 → 一个事务改两张表 → 提交后才挪明文进备份」。
//! 全程只用临时目录与合成凭据，不联网。

use biliup_cli::server::infrastructure::connection_pool::{ConnectionManager, ConnectionPool};
use biliup_cli::server::infrastructure::repositories::migrate_plaintext_accounts;
use std::fs;
use std::path::{Path, PathBuf};

const MID: u64 = 1234567890;
const OTHER_MID: u64 = 987654321;

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
    key: PathBuf,
    pool: ConnectionPool,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let key = root.path().join("accounts.key");
        // 先把 data/accounts 与密钥建出来：迁移测试不该依赖环境变量
        biliup_accounts::AccountStore::open(&data, Some(&key)).unwrap();
        let pool = ConnectionManager::new_pool(data.join("data.sqlite3").to_str().unwrap())
            .await
            .unwrap();
        Self {
            root,
            data,
            key,
            pool,
        }
    }

    fn plaintext(&self, mid: u64) -> PathBuf {
        self.data.join(format!("{mid}.json"))
    }

    fn encrypted(&self, mid: u64) -> PathBuf {
        self.data.join("accounts").join(format!("{mid}.json.enc"))
    }

    fn backup_root(&self) -> PathBuf {
        self.data.join(".backups")
    }

    fn absolute(&self, path: &Path) -> String {
        path.canonicalize().unwrap().to_string_lossy().into_owned()
    }

    /// 相对工作目录的登记写法，配合 `server_root` 一起用
    fn relative(&self, mid: u64) -> String {
        format!("data/{mid}.json")
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
            .bind(format!("template-{index}-{value}"))
            .bind(value)
            .execute(&self.pool)
            .await
            .unwrap();
        }
    }

    async fn migrate(&self) -> biliup_cli::server::infrastructure::repositories::AccountMigration {
        migrate_plaintext_accounts(
            &self.pool,
            &self.data,
            &self.backup_root(),
            &self.key,
            Some(self.root.path()),
        )
        .await
        .unwrap()
    }

    async fn configuration_values(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT value FROM configuration WHERE key = 'bilibili-cookies' ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    async fn user_cookies(&self) -> Vec<Option<String>> {
        sqlx::query_scalar("SELECT user_cookie FROM uploadstreamers ORDER BY id")
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }

    /// 库里的每个登记都指向真实存在的文件（不能有被挪走后的失效路径）
    async fn every_registration_exists(&self) -> bool {
        let mut values = self.configuration_values().await;
        values.extend(self.user_cookies().await.into_iter().flatten());
        values.iter().all(|value| {
            let path = Path::new(value);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.root.path().join(path)
            };
            path.is_file()
        })
    }

    fn backup_dirs(&self) -> Vec<PathBuf> {
        fs::read_dir(self.backup_root())
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }
}

#[tokio::test]
async fn migrates_plaintext_accounts_and_repoints_both_tables() {
    let fixture = Fixture::new().await;
    fs::write(fixture.plaintext(MID), credential_json(MID)).unwrap();
    // 同一个文件登记成绝对与相对两种写法：迁移后只留一行，模板全部指向同一个目标
    let absolute = fixture.absolute(&fixture.plaintext(MID));
    fixture.register(&absolute, 2).await;
    fixture.register(&fixture.relative(MID), 1).await;

    let report = fixture.migrate().await;

    assert_eq!(report.migrated, 1);
    assert_eq!(report.failed, 0);
    assert!(report.repointed >= 3, "{report:?}");
    let encrypted = fixture.encrypted(MID);
    assert!(encrypted.is_file());
    assert!(fs::read(&encrypted).unwrap().starts_with(b"BUPACC1"));
    assert!(!fixture.plaintext(MID).exists());

    let backups = fixture.backup_dirs();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert!(
        backups[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("accounts-migration-")
    );
    assert!(backups[0].join(format!("{MID}.json")).is_file());

    let dest = fixture.absolute(&encrypted);
    assert_eq!(fixture.configuration_values().await, vec![dest.clone()]);
    assert_eq!(fixture.user_cookies().await, vec![Some(dest.clone()); 3]);
    assert!(fixture.every_registration_exists().await);

    // 再跑一次：没有明文可迁，不该再改库、也不该再建备份目录
    let again = fixture.migrate().await;
    assert_eq!(again.migrated, 0);
    assert_eq!(again.repointed, 0);
    assert_eq!(again.failed, 0);
    assert!(encrypted.is_file());
    assert_eq!(fixture.configuration_values().await, vec![dest]);
    assert_eq!(fixture.backup_dirs().len(), 1);
}

#[tokio::test]
async fn a_failed_account_keeps_its_plaintext_and_the_database_consistent() {
    let fixture = Fixture::new().await;
    fs::write(fixture.plaintext(MID), credential_json(MID)).unwrap();
    fs::write(fixture.plaintext(OTHER_MID), credential_json(OTHER_MID)).unwrap();
    let first = fixture.absolute(&fixture.plaintext(MID));
    let second = fixture.absolute(&fixture.plaintext(OTHER_MID));
    fixture.register(&first, 1).await;
    fixture.register(&second, 1).await;
    // 第二个账号的密文目标被一个目录占住：写密文必然失败
    fs::create_dir_all(fixture.encrypted(OTHER_MID)).unwrap();

    let report = fixture.migrate().await;

    assert_eq!(report.migrated, 1, "{report:?}");
    assert_eq!(report.failed, 1, "{report:?}");
    // 第一个账号：密文 + 两张表都指向它
    let dest = fixture.absolute(&fixture.encrypted(MID));
    assert!(
        fs::read(fixture.encrypted(MID))
            .unwrap()
            .starts_with(b"BUPACC1")
    );
    assert_eq!(
        fixture.configuration_values().await,
        vec![dest.clone(), second.clone()]
    );
    assert_eq!(
        fixture.user_cookies().await,
        vec![Some(dest.clone()), Some(second.clone())]
    );
    assert!(fixture.every_registration_exists().await);
    // 第二个账号：明文还在、没被挪走，也没留下半截密文
    assert!(fixture.plaintext(OTHER_MID).is_file());
    assert!(
        fixture
            .backup_dirs()
            .iter()
            .all(|dir| !dir.join(format!("{OTHER_MID}.json")).exists())
    );
    assert_eq!(fixture.backup_dirs().len(), 1);

    // 修好阻塞后重启：第二个账号接着迁，第一个不重复
    fs::remove_dir_all(fixture.encrypted(OTHER_MID)).unwrap();
    let retry = fixture.migrate().await;
    assert_eq!(retry.migrated, 1, "{retry:?}");
    assert_eq!(retry.repointed, 2, "{retry:?}");
    assert_eq!(retry.failed, 0, "{retry:?}");
    assert!(!fixture.plaintext(OTHER_MID).exists());
    assert_eq!(
        fixture.configuration_values().await,
        vec![
            dest.clone(),
            fixture.absolute(&fixture.encrypted(OTHER_MID))
        ]
    );
    assert!(fixture.every_registration_exists().await);
}

#[tokio::test]
async fn leaves_json_that_is_not_an_account_alone() {
    let fixture = Fixture::new().await;
    let notes = fixture.data.join("notes.json");
    let broken = fixture.data.join("broken.json");
    fs::write(&notes, r#"{"hello": "world"}"#).unwrap();
    fs::write(&broken, "not json").unwrap();

    let report = fixture.migrate().await;

    assert_eq!(report.migrated, 0);
    assert_eq!(report.repointed, 0);
    assert_eq!(report.failed, 0);
    assert!(notes.is_file());
    assert!(broken.is_file());
    assert!(fixture.backup_dirs().is_empty());
    assert!(fixture.configuration_values().await.is_empty());
}
