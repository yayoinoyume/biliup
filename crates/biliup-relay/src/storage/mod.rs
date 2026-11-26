//! Installation-local data. Secrets are encrypted before SQLite sees them.
mod crypto;
mod documents;
pub mod paths;

pub(crate) use documents::reject_legacy_channels;

use crypto::Key;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_BACKUP_BYTES: usize = 128 * 1024 * 1024;
const SUPPORTED_SCHEMA_VERSION: u32 = 3;
pub const DB_FILE: &str = "relay.db";
const LEGACY_DB_FILE: &str = "bilistream.db";
// Every first protocol write must exclude binaries which do not enforce its hold.
const MANAGED_RECORDS: &[&str] = &[
    "cluster-identity",
    "cluster-membership",
    "cluster-local",
    "cluster-operations",
];
/// The document ids this installation can hold.
///
/// They keep their file-era spelling on purpose: the id is bound into the
/// encryption context and appears in backup exports, so renaming one is a data
/// migration, not a rename. No file by these names exists — the payload lives
/// in the `documents` table.
pub const NAMES: &[&str] = &[
    "config.json",
    "cookies.json",
    "channels.json",
    "areas.json",
    "cookies.txt",
    "youtube-accounts.json",
    "invalid_words.txt",
    "youtube_quota.json",
    "youtube_golive_hours.json",
    "warning_exempt_cooldown.json",
];

/// Document ids an older build may still hold or put in a backup, which this
/// build never reads. A restore accepts and drops them; a leftover stored copy
/// is left untouched and never exported (only [`NAMES`] are).
pub(crate) const RETIRED_NAMES: &[&str] = &["niconico_cookies.txt"];

#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub value: Value,
    pub revision: u64,
    pub updated_at: u64,
}

type Cache = Arc<RwLock<HashMap<String, Document>>>;
type Job = Box<dyn FnOnce(&mut Connection, &Key, &str, &Cache) + Send>;

pub struct Store {
    data_dir: PathBuf,
    runtime_dir: PathBuf,
    cache: Cache,
    sender: Mutex<Option<mpsc::SyncSender<Job>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    _instance: InstanceLock,
}

struct InstanceLock(File);
impl Drop for InstanceLock {
    fn drop(&mut self) {
        // A concurrent fork can temporarily inherit the descriptor before exec.
        // Explicitly release the lock on initialization failure and shutdown.
        let _ = self.0.unlock();
    }
}

fn import_legacy_database(legacy: &Path, target: &Path) -> io::Result<()> {
    fs::rename(legacy, target)?;
    for suffix in ["-wal", "-shm"] {
        let mut from = legacy.as_os_str().to_owned();
        from.push(suffix);
        let from = PathBuf::from(from);
        if from.try_exists()? {
            let mut to = target.as_os_str().to_owned();
            to.push(suffix);
            fs::rename(&from, PathBuf::from(to))?;
        }
    }
    Ok(())
}

fn sql_error(error: rusqlite::Error) -> io::Error {
    // SQL errors never include bound parameters/secret payloads.
    io::Error::other(format!("本地数据库操作失败：{error}"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sensitive(name: &str) -> bool {
    !matches!(
        name,
        "channels.json"
            | "areas.json"
            | "invalid_words.txt"
            | "youtube_quota.json"
            | "youtube_golive_hours.json"
    )
}

fn context(id: &str, name: &str, revision: u64) -> Vec<u8> {
    format!("bilistream/v1/{id}/{name}/{revision}").into_bytes()
}

fn read_row(
    connection: &Connection,
    key: &Key,
    id: &str,
    name: &str,
) -> io::Result<Option<Document>> {
    let row: Option<(Vec<u8>, bool, i64, i64)> = connection
        .query_row(
            "SELECT payload, encrypted, revision, updated_at FROM documents WHERE name=?1",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(sql_error)?;
    let Some((bytes, encrypted, revision, updated_at)) = row else {
        return Ok(None);
    };
    let revision = u64::try_from(revision).map_err(|_| invalid("数据库记录版本无效"))?;
    let updated_at = u64::try_from(updated_at).map_err(|_| invalid("数据库记录时间无效"))?;
    if encrypted != sensitive(name) {
        return Err(invalid("数据库加密标记不匹配"));
    }
    let bytes = if encrypted {
        crypto::open(key, &context(id, name, revision), &bytes)?
    } else {
        bytes
    };
    let value = serde_json::from_slice(&bytes).map_err(|_| invalid("数据库记录格式无效"))?;
    Ok(Some(Document {
        value,
        revision,
        updated_at,
    }))
}

fn load_rows(
    connection: &Connection,
    key: &Key,
    id: &str,
) -> io::Result<HashMap<String, Document>> {
    let mut stmt = connection
        .prepare("SELECT name FROM documents")
        .map_err(sql_error)?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    let mut rows = HashMap::new();
    for name in names {
        if let Some(doc) = read_row(connection, key, id, &name)? {
            rows.insert(name, doc);
        }
    }
    Ok(rows)
}

pub struct Transaction<'a> {
    sql: rusqlite::Transaction<'a>,
    key: &'a Key,
    id: &'a str,
    changed: HashMap<String, Document>,
}

impl Transaction<'_> {
    /// Upgrade in the same FULL-sync transaction as the first identity or hold.
    /// SQLite rolls the PRAGMA back together with the documents on failure.
    pub fn require_managed_schema(&mut self) -> io::Result<()> {
        self.require_schema(2)
    }

    /// A signed bounded-stop promise must survive restart without an older
    /// binary reusing the identity while ignoring the execution watchdog.
    pub fn require_fencing_schema(&mut self) -> io::Result<()> {
        self.require_schema(3)
    }

    fn require_schema(&mut self, minimum: u32) -> io::Result<()> {
        let current: u32 = self
            .sql
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(sql_error)?;
        if current < minimum {
            self.sql
                .pragma_update(None, "user_version", minimum)
                .map_err(sql_error)?;
        }
        Ok(())
    }

    pub fn read(&self, name: &str) -> io::Result<Option<Document>> {
        read_row(&self.sql, self.key, self.id, name)
    }

    pub fn write(&mut self, name: &str, value: Value) -> io::Result<u64> {
        self.write_at(name, value, now())
    }

    pub fn write_at(&mut self, name: &str, value: Value, updated_at: u64) -> io::Result<u64> {
        if name.len() > 128 || name.contains(['/', '\\', '\0']) {
            return Err(invalid("数据记录名称无效"));
        }
        let bytes = serde_json::to_vec(&value).map_err(|_| invalid("数据记录格式无效"))?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid("数据记录过大"));
        }
        if MANAGED_RECORDS.contains(&name) {
            self.require_managed_schema()?;
        }
        let revision: i64 = self.sql.query_row("UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE name='revision' RETURNING CAST(value AS INTEGER)", [], |r| r.get(0)).map_err(sql_error)?;
        let stored_revision = u64::try_from(revision).map_err(|_| invalid("数据库记录版本无效"))?;
        let stored_updated_at =
            i64::try_from(updated_at).map_err(|_| invalid("数据库记录时间无效"))?;
        let encrypted = sensitive(name);
        let payload = if encrypted {
            crypto::seal(self.key, &context(self.id, name, stored_revision), &bytes)?
        } else {
            bytes
        };
        self.sql.execute("INSERT INTO documents(name,payload,encrypted,revision,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(name) DO UPDATE SET payload=excluded.payload, encrypted=excluded.encrypted, revision=excluded.revision, updated_at=excluded.updated_at", params![name, payload, encrypted, revision, stored_updated_at]).map_err(sql_error)?;
        self.changed.insert(
            name.to_owned(),
            Document {
                value,
                revision: stored_revision,
                updated_at,
            },
        );
        Ok(stored_revision)
    }

    pub fn compare_exchange(&mut self, name: &str, expected: u64, value: Value) -> io::Result<u64> {
        let current = self.read(name)?.map(|d| d.revision).unwrap_or(0);
        if current != expected {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "内容已更新，请刷新后重试",
            ));
        }
        self.write(name, value)
    }
}

impl Store {
    /// Open (creating when needed) the document store.
    ///
    /// There is no import from a pre-SQL data directory any more: a legacy
    /// directory beside the executable is simply ignored. An installation that
    /// still holds JSON files must be upgraded through a build that imports
    /// them, once.
    pub fn open(data_dir: PathBuf, key_file: PathBuf) -> io::Result<Arc<Self>> {
        validate_key_location(&data_dir, &key_file)?;
        paths::private_dir(&data_dir)?;
        let lock_path = data_dir.join("instance.lock");
        let instance = match paths::create_private(&lock_path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)?,
            Err(e) => return Err(e),
        };
        instance
            .try_lock()
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "此数据目录已有程序在运行"))?;
        let instance = InstanceLock(instance);
        let db_path = data_dir.join(DB_FILE);
        // 旧版 bilistream 库原地改名导入，加密上下文与备份格式保持原样。
        let legacy_path = data_dir.join(LEGACY_DB_FILE);
        if !db_path.try_exists()? && legacy_path.try_exists()? {
            import_legacy_database(&legacy_path, &db_path)?;
        }
        let existing = db_path.try_exists()?;
        let key = paths::load_key(&key_file, existing)?;
        if !existing {
            drop(paths::create_private(&db_path)?);
        }
        let mut connection = Connection::open(&db_path).map_err(sql_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(sql_error)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;").map_err(sql_error)?;
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(sql_error)?;
        if version > SUPPORTED_SCHEMA_VERSION {
            return Err(invalid("数据库由更新版本创建，请升级程序"));
        }
        connection.execute_batch("CREATE TABLE IF NOT EXISTS meta(name TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS documents(name TEXT PRIMARY KEY,payload BLOB NOT NULL,encrypted INTEGER NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL); INSERT OR IGNORE INTO meta VALUES('revision','0');").map_err(sql_error)?;
        if version == 0 {
            connection
                .execute_batch("PRAGMA user_version=1")
                .map_err(sql_error)?;
        }
        let stored_id: Option<String> = connection
            .query_row("SELECT value FROM meta WHERE name='id'", [], |r| r.get(0))
            .optional()
            .map_err(sql_error)?;
        let id = match stored_id {
            Some(id) => id,
            None => paths::hex(&crypto::random::<16>()?),
        };
        connection
            .execute("INSERT OR IGNORE INTO meta VALUES('id',?1)", [&id])
            .map_err(sql_error)?;
        let check: Option<String> = connection
            .query_row("SELECT value FROM meta WHERE name='key_check'", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(sql_error)?;
        use base64::Engine;
        if let Some(check) = check {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(check)
                .map_err(|_| invalid("密钥校验记录无效"))?;
            if crypto::open(&key, id.as_bytes(), &bytes)? != b"bilistream-storage-key" {
                return Err(invalid("数据库密钥不匹配"));
            }
        } else {
            let check = crypto::seal(&key, id.as_bytes(), b"bilistream-storage-key")?;
            connection
                .execute(
                    "INSERT INTO meta VALUES('key_check',?1)",
                    [base64::engine::general_purpose::STANDARD.encode(check)],
                )
                .map_err(sql_error)?;
        }
        if !existing {
            // First-run default, not a migration: the player filter reads this
            // document and the settings page reports "过滤词不可用" when absent.
            connection
                .execute(
                    "INSERT OR IGNORE INTO documents(name,payload,encrypted,revision,updated_at) VALUES('invalid_words.txt',?1,0,0,?2)",
                    params![
                        serde_json::to_vec(&Value::String(String::new()))
                            .map_err(|_| invalid("默认文档无效"))?,
                        now()
                    ],
                )
                .map_err(sql_error)?;
        }
        let cache = Arc::new(RwLock::new(load_rows(&connection, &key, &id)?));
        let runtime_dir = data_dir.join("runtime");
        paths::private_dir(&runtime_dir)?;
        crate::plugins::youtube_cookies::cleanup_stale(&runtime_dir);
        let (sender, receiver) = mpsc::sync_channel::<Job>(32);
        let worker_cache = Arc::clone(&cache);
        let worker = std::thread::Builder::new()
            .name("bilistream-storage".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    job(&mut connection, &key, &id, &worker_cache);
                }
                let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
            })?;
        Ok(Arc::new(Self {
            data_dir,
            runtime_dir,
            cache,
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(Some(worker)),
            _instance: instance,
        }))
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub fn read(&self, name: &str) -> io::Result<Option<Document>> {
        let cache = self
            .cache
            .read()
            .map_err(|_| io::Error::other("数据库缓存不可用"))?;
        Ok(cache.get(name).cloned())
    }

    pub fn revision(&self, name: &str) -> io::Result<Option<u64>> {
        Ok(self
            .cache
            .read()
            .map_err(|_| io::Error::other("数据库缓存不可用"))?
            .get(name)
            .map(|doc| doc.revision))
    }

    pub fn read_many(&self, names: &[&str]) -> io::Result<HashMap<String, Document>> {
        let cache = self
            .cache
            .read()
            .map_err(|_| io::Error::other("数据库缓存不可用"))?;
        Ok(names
            .iter()
            .filter_map(|name| cache.get(*name).map(|doc| (name.to_string(), doc.clone())))
            .collect())
    }

    pub fn snapshot(&self) -> io::Result<HashMap<String, Document>> {
        Ok(self
            .cache
            .read()
            .map_err(|_| io::Error::other("数据库缓存不可用"))?
            .clone())
    }

    pub fn transaction<T, F>(&self, edit: F) -> io::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Transaction<'_>) -> io::Result<T> + Send + 'static,
    {
        let (reply, answer) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |connection, key, id, cache| {
            let result = (|| {
                let sql = connection
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(sql_error)?;
                let mut tx = Transaction {
                    sql,
                    key,
                    id,
                    changed: HashMap::new(),
                };
                let value = edit(&mut tx)?;
                let changed = std::mem::take(&mut tx.changed);
                // Acquire publication lock before commit; never leave a committed
                // database with a stale cache because of a poisoned lock.
                let mut published = cache
                    .write()
                    .map_err(|_| io::Error::other("数据库缓存不可用"))?;
                tx.sql.commit().map_err(sql_error)?;
                published.extend(changed);
                Ok(value)
            })();
            let _ = reply.send(result);
        });
        self.sender
            .lock()
            .map_err(|_| io::Error::other("数据库工作线程不可用"))?
            .as_ref()
            .ok_or_else(|| io::Error::other("数据库已关闭"))?
            .send(job)
            .map_err(|_| io::Error::other("数据库工作线程已停止"))?;
        answer
            .recv()
            .map_err(|_| io::Error::other("数据库事务未完成"))?
    }

    pub fn write(&self, name: &str, value: Value) -> io::Result<u64> {
        let name = name.to_owned();
        self.transaction(move |tx| tx.write(&name, value))
    }

    pub fn compare_exchange(&self, name: &str, expected: u64, value: Value) -> io::Result<u64> {
        let name = name.to_owned();
        self.transaction(move |tx| tx.compare_exchange(&name, expected, value))
    }

    pub fn export_backup(&self, password: &str) -> io::Result<Vec<u8>> {
        let snapshot = self.portable_snapshot()?;
        let bytes = serde_json::to_vec(&snapshot).map_err(|_| invalid("无法生成备份"))?;
        crypto::export(password, &bytes)
    }

    pub fn restore_backup(&self, password: &str, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_BACKUP_BYTES {
            return Err(invalid("备份文件过大"));
        }
        let plain = crypto::restore(password, bytes)?;
        let mut docs: HashMap<String, Document> =
            serde_json::from_slice(&plain).map_err(|_| invalid("备份内容无效"))?;
        docs.retain(|name, _| !RETIRED_NAMES.contains(&name.as_str()));
        for (name, doc) in &docs {
            documents::validate(name, &doc.value)?;
        }
        self.transaction(move |tx| {
            let protected = MANAGED_RECORDS.iter().try_fold(false, |found, name| {
                Ok::<_, io::Error>(tx.read(name)?.is_some() || found)
            })?;
            if protected {
                let local = tx.read("cluster-local")?;
                let lifecycle = local.as_ref().and_then(|d| d.value["lifecycle"].as_str());
                // Unknown or incomplete state fails closed. Pairing/managed may
                // have a prepared reservation even before config exists.
                if !matches!(lifecycle, Some("join_ready" | "left" | "incompatible_held"))
                    || tx.read("cluster-operations")?.is_some()
                    || tx.read("cluster-membership")?.is_some()
                {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "集群正在运行或等待恢复，不能覆盖恢复备份",
                    ));
                }
            }
            if let Some(config) = docs.get_mut("config.json") {
                // An old backup carries no membership records. Sanitize before
                // publishing it, even when this running destination initialized
                // while empty: neither downgrade nor export may revive its peers.
                if protected || config.value["cluster"]["enabled"] == true {
                    sanitize_unjoined_config(&mut config.value)?;
                }
            }
            // Restore only into an unused installation. Live replacement would
            // race active refreshes/streams and is deliberately not exposed.
            if tx.read("config.json")?.is_some() || tx.read("cookies.json")?.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "请在空的数据目录中恢复备份",
                ));
            }
            for (name, doc) in docs {
                tx.write_at(&name, doc.value, doc.updated_at)?;
            }
            // Backups never carry notify records (not in NAMES), but the
            // target may hold an older plain `notify-config`: seal its openid
            Ok(())
        })
    }

    fn portable_snapshot(&self) -> io::Result<HashMap<String, Document>> {
        let mut snapshot = self.snapshot()?;
        let protected = MANAGED_RECORDS
            .iter()
            .any(|name| snapshot.contains_key(*name));
        if let Some(config) = snapshot.get_mut("config.json") {
            if protected || config.value["cluster"]["enabled"] == true {
                sanitize_unjoined_config(&mut config.value)?;
            }
        }
        // Identity, password, sessions, reservations and recovery journals stay local.
        snapshot.retain(|name, _| NAMES.contains(&name.as_str()));
        Ok(snapshot)
    }
}

/// Sanitize a copy only: never change the source installation's streaming choices.
fn sanitize_unjoined_config(config: &mut Value) -> io::Result<()> {
    let object = config
        .as_object_mut()
        .ok_or_else(|| invalid("配置格式无效"))?;
    for platform in ["youtube", "twitch", "niconico"] {
        let settings = object
            .entry(platform)
            .or_insert_with(|| serde_json::json!({}));
        settings
            .as_object_mut()
            .ok_or_else(|| invalid("配置格式无效"))?
            .insert("enable_monitor".into(), Value::Bool(false));
    }
    let priority = object
        .entry("priority_channel")
        .or_insert_with(|| serde_json::json!({}));
    let priority = priority
        .as_object_mut()
        .ok_or_else(|| invalid("配置格式无效"))?;
    priority.insert("enabled".into(), Value::Bool(false));
    priority.insert("auto_restart".into(), Value::Bool(false));
    let cluster = object
        .entry("cluster")
        .or_insert_with(|| serde_json::json!({}));
    let cluster = cluster
        .as_object_mut()
        .ok_or_else(|| invalid("配置格式无效"))?;
    cluster.insert("enabled".into(), Value::Bool(false));
    cluster.insert("peers".into(), serde_json::json!([]));
    for field in ["node_id", "node_name", "public_api_url"] {
        cluster.insert(field.into(), Value::String(String::new()));
    }
    let public = cluster
        .entry("public_status")
        .or_insert_with(|| serde_json::json!({}));
    public
        .as_object_mut()
        .ok_or_else(|| invalid("配置格式无效"))?
        .insert("node_id".into(), Value::String(String::new()));
    Ok(())
}

impl Drop for Store {
    fn drop(&mut self) {
        self.sender
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(worker) = self
            .worker
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}

static GLOBAL: RwLock<Option<Arc<Store>>> = RwLock::new(None);
static REGISTERED: RwLock<Option<(PathBuf, PathBuf)>> = RwLock::new(None);
static INITIALIZING: Mutex<()> = Mutex::new(());

/// 进程级全局状态的唯一测试串行锁。
///
/// 存储、RelayService、history 统计引擎与事件总线都是进程级单例：任何会启动或
/// 停机它们的测试都必须先拿这把锁，否则一个用例的 shutdown 会杀掉另一个用例正在
/// 驱动的写入线程，或把它的全局存储换掉。统一入口是 lock_test_global()，它还会把
/// 同一线程重复获取（std::sync::Mutex 不可重入，必然死锁）变成一条明确的 panic。
#[cfg(test)]
pub(crate) static TEST_STORE_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    static TEST_GLOBAL_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// lock_test_global() 的守卫：释放时清掉线程内的“已持有”标记。
#[cfg(test)]
pub(crate) struct TestGlobalGuard(Option<std::sync::MutexGuard<'static, ()>>);

#[cfg(test)]
impl Drop for TestGlobalGuard {
    fn drop(&mut self) {
        self.0 = None;
        TEST_GLOBAL_HELD.with(|held| held.set(false));
    }
}

/// 获取进程级全局状态的测试串行锁。
///
/// 同一线程重复获取会 panic 而不是静默死锁。
#[cfg(test)]
pub(crate) fn lock_test_global() -> TestGlobalGuard {
    TEST_GLOBAL_HELD.with(|held| {
        assert!(
            !held.get(),
            "测试全局锁被同一线程重复获取：cargo test 会在此静默死锁"
        );
        held.set(true);
    });
    let guard = TEST_STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    TestGlobalGuard(Some(guard))
}

/// 解析真实路径：先 canonicalize 最近存在的祖先，再折叠尾部尚不存在的组件。
/// 父目录尚未创建时也不能跳过软链与 `..` 校验。
fn normalize_for_check(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let existing = absolute
        .ancestors()
        .find(|candidate| candidate.symlink_metadata().is_ok())
        .ok_or_else(|| io::Error::other("密钥路径无法解析"))?;
    let mut normalized = existing.canonicalize()?;
    let tail = absolute.strip_prefix(existing).unwrap_or(Path::new(""));
    for component in tail.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(name) => normalized.push(name),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {}
        }
    }
    Ok(normalized)
}

/// 词法路径加真实路径双重校验，软链与 `..` 都不能把密钥带进数据目录。
fn validate_key_location(data_dir: &Path, key_file: &Path) -> io::Result<()> {
    if normalize_for_check(key_file)?.starts_with(normalize_for_check(data_dir)?) {
        return Err(invalid("密钥文件必须在数据库目录之外"));
    }
    Ok(())
}

/// 收口进程级文档库的路径并在同一步打开它。
///
/// 同一进程可安全地 start -> shutdown -> start：路径相同时复用已打开的句柄，
/// 路径不同时先释放旧句柄（关闭 worker、SQLite 连接与实例锁）再打开新路径。
pub fn init(data_dir: &Path, key_file: Option<&Path>) -> io::Result<Arc<Store>> {
    let data = std::path::absolute(data_dir)?;
    let key = match key_file {
        Some(path) => std::path::absolute(path)?,
        None => paths::default_key_file(&data)?,
    };
    validate_key_location(&data, &key)?;
    let _guard = INITIALIZING
        .lock()
        .map_err(|_| io::Error::other("数据库初始化不可用"))?;
    if let Some((registered, registered_key)) = REGISTERED
        .read()
        .map_err(|_| io::Error::other("数据库初始化不可用"))?
        .as_ref()
    {
        // 同一条物理路径的不同拼写（软链、..）不该被当成换库，也不该报实例锁冲突；
        // 解析失败时退回字面比较，避免旧的登记路径把本次 init 直接判死。
        let same_data = match (normalize_for_check(registered), normalize_for_check(&data)) {
            (Ok(a), Ok(b)) => a == b,
            _ => registered == &data,
        };
        let same_key = match (normalize_for_check(registered_key), normalize_for_check(&key)) {
            (Ok(a), Ok(b)) => a == b,
            _ => registered_key == &key,
        };
        if same_data && same_key {
            if let Some(store) = GLOBAL
                .read()
                .map_err(|_| io::Error::other("数据库初始化不可用"))?
                .as_ref()
            {
                return Ok(Arc::clone(store));
            }
        }
    }
    let store = Store::open(data.clone(), key.clone())?;
    *REGISTERED
        .write()
        .map_err(|_| io::Error::other("数据库初始化不可用"))? = Some((data, key));
    let previous = GLOBAL
        .write()
        .map_err(|_| io::Error::other("数据库初始化不可用"))?
        .replace(Arc::clone(&store));
    drop(previous);
    Ok(store)
}

pub fn global() -> io::Result<Arc<Store>> {
    if let Some(store) = GLOBAL
        .read()
        .map_err(|_| io::Error::other("数据库初始化不可用"))?
        .as_ref()
    {
        return Ok(Arc::clone(store));
    }
    let _guard = INITIALIZING
        .lock()
        .map_err(|_| io::Error::other("数据库初始化不可用"))?;
    if let Some(store) = GLOBAL
        .read()
        .map_err(|_| io::Error::other("数据库初始化不可用"))?
        .as_ref()
    {
        return Ok(Arc::clone(store));
    }
    let (data, key) = match REGISTERED
        .read()
        .map_err(|_| io::Error::other("数据库初始化不可用"))?
        .as_ref()
    {
        Some((data, key)) => (data.clone(), key.clone()),
        None => fallback_paths()?,
    };
    let store = Store::open(data, key)?;
    *GLOBAL
        .write()
        .map_err(|_| io::Error::other("数据库初始化不可用"))? = Some(Arc::clone(&store));
    Ok(store)
}

/// 释放进程级文档库：关闭 worker、SQLite 连接与实例锁，并清空已登记路径。
pub fn shutdown() {
    let _guard = INITIALIZING.lock().unwrap_or_else(|e| e.into_inner());
    let store = GLOBAL.write().unwrap_or_else(|e| e.into_inner()).take();
    *REGISTERED.write().unwrap_or_else(|e| e.into_inner()) = None;
    drop(store);
}

#[cfg(test)]
fn fallback_paths() -> io::Result<(PathBuf, PathBuf)> {
    let root = std::env::temp_dir().join(format!("relay-unit-store-{}", std::process::id()));
    Ok((root.join("data"), root.join("keys/master.key")))
}

#[cfg(not(test))]
fn fallback_paths() -> io::Result<(PathBuf, PathBuf)> {
    Err(io::Error::other("转播存储尚未初始化"))
}

pub fn cache_file(name: &str) -> io::Result<PathBuf> {
    if !matches!(name, "cover.jpg" | "pic_for_crop.jpg") {
        return Err(invalid("缓存文件名称无效"));
    }
    let directory = global()?.data_dir().join("cache");
    paths::private_dir(&directory)?;
    Ok(directory.join(name))
}

/// Read one document by its record id (e.g. `"areas.json"`).
pub fn read_document<T: serde::de::DeserializeOwned>(name: &str) -> io::Result<T> {
    let doc = global()?
        .read(name)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("尚未配置 {name}")))?;
    serde_json::from_value(doc.value).map_err(|_| invalid(&format!("{name} 数据格式无效")))
}

pub fn read_text(name: &str) -> io::Result<String> {
    read_document(name)
}
pub fn contains(name: &str) -> io::Result<bool> {
    Ok(global()?.revision(name)?.is_some())
}
/// Write one document by its record id (e.g. `"areas.json"`), returning the new revision.
pub fn write_document<T: Serialize>(name: &str, value: &T) -> io::Result<u64> {
    global()?.write(
        name,
        serde_json::to_value(value).map_err(|_| invalid("数据格式无效"))?,
    )
}

#[cfg(test)]
pub(crate) mod tests;
