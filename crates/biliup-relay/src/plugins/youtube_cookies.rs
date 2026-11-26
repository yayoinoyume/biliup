#![allow(dead_code)]
//! A managed Cookie jar with serialized, private-file yt-dlp round trips.
use crate::storage::{self, paths, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

pub const MAX_COOKIE_BYTES: usize = 1024 * 1024;
const NAME: &str = "cookies.txt";
const ACCOUNTS: &str = "youtube-accounts.json";
const MAX_ACCOUNTS: usize = 16;
const MAX_LABEL_CHARS: usize = 80;
const MAX_BROWSER_CHARS: usize = 64;
const MAX_PROFILE_CHARS: usize = 64;

#[derive(Debug, Serialize)]
pub struct CookieSummary {
    pub count: usize,
}
#[derive(Serialize)]
pub struct CookieStatus {
    pub configured: bool,
    pub count: usize,
    pub revision: u64,
    pub updated_at: u64,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Cookie 文件格式无效，请导出 Netscape 格式",
    )
}

pub fn validate_netscape(input: &str) -> io::Result<CookieSummary> {
    if input.len() > MAX_COOKIE_BYTES || input.contains('\0') {
        return Err(invalid());
    }
    if !input.is_empty()
        && !input.lines().next().is_some_and(|line| {
            let line = line.trim_end_matches('\r');
            line == "# Netscape HTTP Cookie File" || line == "# HTTP Cookie File"
        })
    {
        return Err(invalid());
    }
    let mut count = 0;
    let mut keys = HashSet::new();
    for line in input.lines() {
        let line = line.trim_end_matches('\r');
        let line = if let Some(value) = line.strip_prefix("#HttpOnly_") {
            value
        } else {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            line
        };
        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() != 7
            || parts[0].is_empty()
            || parts[0]
                .chars()
                .any(|c| c.is_whitespace() || c == '/' || c == ':')
            || !matches!(parts[1], "TRUE" | "FALSE")
            || !parts[2].starts_with('/')
            || !matches!(parts[3], "TRUE" | "FALSE")
            || (!parts[4].is_empty() && parts[4].parse::<u64>().is_err())
            || parts[5..]
                .iter()
                .any(|part| part.chars().any(char::is_control))
        {
            return Err(invalid());
        }
        if !keys.insert((parts[0], parts[2], parts[5])) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cookie 文件包含重复记录",
            ));
        }
        count += 1;
    }
    Ok(CookieSummary { count })
}

fn status_from(store: &Store) -> io::Result<CookieStatus> {
    let Some(doc) = store.read(NAME)? else {
        return Ok(CookieStatus {
            configured: false,
            count: 0,
            revision: 0,
            updated_at: 0,
        });
    };
    let text = doc.value.as_str().ok_or_else(invalid)?;
    let count = validate_netscape(text)?.count;
    Ok(CookieStatus {
        configured: count != 0,
        count,
        revision: doc.revision,
        updated_at: doc.updated_at,
    })
}

pub fn status() -> io::Result<CookieStatus> {
    status_from(storage::global()?.as_ref())
}

pub async fn import(input: String, expected_revision: u64) -> io::Result<CookieStatus> {
    if validate_netscape(&input)?.count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "文件中没有 Cookie；清除登录请使用清除按钮",
        ));
    }
    let store = storage::global()?;
    tokio::task::spawn_blocking(move || {
        store.compare_exchange(NAME, expected_revision, Value::String(input))?;
        status_from(&store)
    })
    .await
    .map_err(io::Error::other)?
}

pub async fn clear(expected_revision: u64) -> io::Result<CookieStatus> {
    let store = storage::global()?;
    tokio::task::spawn_blocking(move || {
        store.compare_exchange(NAME, expected_revision, Value::String(String::new()))?;
        status_from(&store)
    })
    .await
    .map_err(io::Error::other)?
}

/// 一个 YouTube 账号：浏览器登录态或导出过的 cookies.txt，二者只存其一。
///
/// 文档整体加密入库（youtube-accounts.json 不在明文名单里），接口从不回传 Cookie。
#[derive(Clone, Serialize, Deserialize)]
struct Account {
    id: String,
    label: String,
    source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    browser: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cookies: Option<String>,
    count: usize,
    created_at: u64,
    updated_at: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct AccountsDoc {
    #[serde(default)]
    revision: u64,
    #[serde(default)]
    selected_id: Option<String>,
    #[serde(default)]
    accounts: Vec<Account>,
}

#[derive(Debug, Serialize)]
pub struct AccountView {
    pub id: String,
    pub label: String,
    pub source_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub cookie_count: usize,
    pub updated_at: u64,
}

#[derive(Debug, Serialize)]
pub struct AccountsStatus {
    pub revision: u64,
    pub selected_id: Option<String>,
    pub accounts: Vec<AccountView>,
}

pub enum AccountSource {
    Browser {
        browser: String,
        profile: Option<String>,
    },
    Cookies {
        content: String,
    },
}

fn accounts_error(message: &str, kind: io::ErrorKind) -> io::Error {
    io::Error::new(kind, message)
}

/// 单行文本：去掉首尾空白后仍有内容，且不含控制字符。
fn clean_text(value: &str, limit: usize, what: &str) -> io::Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > limit || trimmed.chars().any(char::is_control)
    {
        return Err(accounts_error(
            &format!("{what}无效或过长"),
            io::ErrorKind::InvalidInput,
        ));
    }
    Ok(trimmed.to_owned())
}

/// yt-dlp 的 --cookies-from-browser 参数片段：不允许冒号分段，也不许以 - 开头。
fn clean_arg(value: &str, limit: usize, what: &str) -> io::Result<String> {
    let value = clean_text(value, limit, what)?;
    if value.starts_with('-') || value.contains(':') {
        return Err(accounts_error(
            &format!("{what}无效或过长"),
            io::ErrorKind::InvalidInput,
        ));
    }
    Ok(value)
}

fn accounts_from(value: Option<&Value>) -> io::Result<AccountsDoc> {
    let Some(value) = value else {
        return Ok(AccountsDoc::default());
    };
    serde_json::from_value(value.clone()).map_err(|_| {
        accounts_error(
            "YouTube 账号数据已损坏，请重新导入",
            io::ErrorKind::InvalidData,
        )
    })
}

fn view_of(doc: &AccountsDoc) -> AccountsStatus {
    AccountsStatus {
        revision: doc.revision,
        selected_id: doc.selected_id.clone(),
        accounts: doc
            .accounts
            .iter()
            .map(|account| AccountView {
                id: account.id.clone(),
                label: account.label.clone(),
                source_type: account.source.clone(),
                browser: account.browser.clone(),
                profile: account.profile.clone(),
                cookie_count: account.count,
                updated_at: account.updated_at,
            })
            .collect(),
    }
}

fn status_of(store: &Store) -> io::Result<AccountsStatus> {
    let doc = store.read(ACCOUNTS)?;
    let doc = accounts_from(doc.as_ref().map(|doc| &doc.value))?;
    Ok(view_of(&doc))
}

pub fn accounts_status() -> io::Result<AccountsStatus> {
    status_of(storage::global()?.as_ref())
}

/// 读-改-写在一段 IMMEDIATE 事务里完成：revision 冲突时整次修改都不生效。
fn update(
    store: Arc<Store>,
    expected: u64,
    edit: impl FnOnce(&mut AccountsDoc) -> io::Result<()> + Send + 'static,
) -> io::Result<AccountsStatus> {
    store.transaction(move |tx| {
        let mut doc = accounts_from(tx.read(ACCOUNTS)?.as_ref().map(|doc| &doc.value))?;
        if doc.revision != expected {
            return Err(accounts_error(
                "账号列表已更新，请刷新后重试",
                io::ErrorKind::WouldBlock,
            ));
        }
        edit(&mut doc)?;
        doc.revision += 1;
        tx.write(
            ACCOUNTS,
            serde_json::to_value(&doc).map_err(|_| invalid())?,
        )?;
        Ok(view_of(&doc))
    })
}

fn prepare(label: &str, source: AccountSource) -> io::Result<Account> {
    let label = clean_text(label, MAX_LABEL_CHARS, "账号名称")?;
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).map_err(|_| io::Error::other("无法生成账号标识"))?;
    let id = nonce.iter().map(|n| format!("{n:02x}")).collect::<String>();
    let now = storage::now();
    let (source, browser, profile, cookies, count) = match source {
        AccountSource::Browser { browser, profile } => {
            let browser = clean_arg(&browser, MAX_BROWSER_CHARS, "浏览器名")?;
            let profile = profile
                .map(|value| clean_arg(&value, MAX_PROFILE_CHARS, "浏览器配置"))
                .transpose()?;
            ("browser", Some(browser), profile, None, 0)
        }
        AccountSource::Cookies { content } => {
            let summary = validate_netscape(&content)?;
            if summary.count == 0 {
                return Err(accounts_error(
                    "文件中没有 Cookie，请重新导出 cookies.txt",
                    io::ErrorKind::InvalidInput,
                ));
            }
            ("cookies", None, None, Some(content), summary.count)
        }
    };
    Ok(Account {
        id,
        label,
        source: source.to_owned(),
        browser,
        profile,
        cookies,
        count,
        created_at: now,
        updated_at: now,
    })
}

pub async fn add(
    label: String,
    expected_revision: u64,
    source: AccountSource,
) -> io::Result<AccountsStatus> {
    let account = prepare(&label, source)?;
    let store = storage::global()?;
    tokio::task::spawn_blocking(move || {
        update(store, expected_revision, move |doc| {
            if doc.accounts.len() >= MAX_ACCOUNTS {
                return Err(accounts_error(
                    "账号数量已达上限，请先删除不用的账号",
                    io::ErrorKind::InvalidInput,
                ));
            }
            doc.accounts.push(account);
            Ok(())
        })
    })
    .await
    .map_err(io::Error::other)?
}

pub async fn delete(id: String, expected_revision: u64) -> io::Result<AccountsStatus> {
    let store = storage::global()?;
    tokio::task::spawn_blocking(move || {
        update(store, expected_revision, move |doc| {
            if doc.accounts.iter().all(|account| account.id != id) {
                return Err(accounts_error(
                    "账号不存在，请刷新后重试",
                    io::ErrorKind::InvalidInput,
                ));
            }
            doc.accounts.retain(|account| account.id != id);
            if doc.selected_id.as_deref() == Some(id.as_str()) {
                doc.selected_id = None;
            }
            Ok(())
        })
    })
    .await
    .map_err(io::Error::other)?
}

pub async fn select(id: Option<String>, expected_revision: u64) -> io::Result<AccountsStatus> {
    let store = storage::global()?;
    tokio::task::spawn_blocking(move || {
        update(store, expected_revision, move |doc| {
            match id {
                Some(id) => {
                    if doc.accounts.iter().all(|account| account.id != id) {
                        return Err(accounts_error(
                            "账号不存在，请刷新后重试",
                            io::ErrorKind::InvalidInput,
                        ));
                    }
                    doc.selected_id = Some(id);
                }
                None => doc.selected_id = None,
            }
            Ok(())
        })
    })
    .await
    .map_err(io::Error::other)?
}

/// 写回账号里的 Cookie（yt-dlp 会刷新过期时间）：文档已变则本次结果作废。
fn writeback(store: &Store, id: String, revision: u64, text: String) -> io::Result<()> {
    store.transaction(move |tx| {
        let mut doc = accounts_from(tx.read(ACCOUNTS)?.as_ref().map(|doc| &doc.value))?;
        if doc.revision != revision {
            return Err(accounts_error(
                "账号 Cookie 已更新，请重试查询",
                io::ErrorKind::WouldBlock,
            ));
        }
        let now = storage::now();
        let count = validate_netscape(&text)?.count;
        let Some(account) = doc.accounts.iter_mut().find(|account| account.id == id) else {
            return Err(accounts_error(
                "账号已被删除，请重新选择",
                io::ErrorKind::WouldBlock,
            ));
        };
        account.cookies = Some(text);
        account.count = count;
        account.updated_at = now;
        doc.revision += 1;
        tx.write(
            ACCOUNTS,
            serde_json::to_value(&doc).map_err(|_| invalid())?,
        )?;
        Ok(())
    })
}

fn queue(store: &Store) -> Arc<AsyncMutex<()>> {
    static QUEUES: OnceLock<Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>> = OnceLock::new();
    let mut queues = QUEUES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    Arc::clone(
        queues
            .entry(store.data_dir().to_path_buf())
            .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
    )
}

struct TemporaryJar {
    directory: PathBuf,
    path: PathBuf,
}
impl TemporaryJar {
    fn create(store: &Store, text: &str) -> io::Result<Self> {
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| io::Error::other("无法创建私有 Cookie 文件"))?;
        let suffix: String = nonce.iter().map(|n| format!("{n:02x}")).collect();
        let directory = store
            .runtime_dir()
            .join(format!("yt-cookie-{}-{suffix}", std::process::id()));
        cleanup_stale(store.runtime_dir());
        paths::private_dir(&directory)?;
        let path = directory.join("cookies.txt");
        if let Err(error) = paths::write_private(&path, text.as_bytes()) {
            let _ = fs::remove_dir(&directory);
            return Err(error);
        }
        Ok(Self { directory, path })
    }
}
impl Drop for TemporaryJar {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(self.directory.join("child.pid"));
        let _ = fs::remove_dir(&self.directory);
    }
}

pub(crate) fn cleanup_stale(directory: &std::path::Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("yt-cookie-")
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        let path = entry.path();
        let Some(pid) = fs::read_to_string(path.join("child.pid"))
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        #[cfg(unix)]
        let alive = unsafe { libc::kill(pid as i32, 0) == 0 }
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
        // shortcut: Windows 暂不引入系统 API 依赖判断进程存活，退化为清理过期临时文件；后续 Windows 发布前按需补齐。
        #[cfg(windows)]
        let alive = false;
        if !alive {
            let _ = fs::remove_file(path.join("cookies.txt"));
            let _ = fs::remove_file(path.join("child.pid"));
            let _ = fs::remove_dir(path);
        }
    }
}

/// 运行期凭据：选中的账号优先，其后才回退旧的浏览器设置与 cookies.txt。
enum Credential {
    Browser(String),
    LegacyJar(String),
    AccountJar {
        id: String,
        revision: u64,
        text: String,
    },
}

fn resolve(store: &Store, legacy_browser: Option<String>) -> io::Result<Option<Credential>> {
    let doc = store.read(ACCOUNTS)?;
    let doc = accounts_from(doc.as_ref().map(|doc| &doc.value))?;
    if let Some(id) = doc.selected_id.clone() {
        match doc.accounts.iter().find(|account| account.id == id) {
            Some(account) if account.source == "browser" => {
                let browser = account.browser.clone().unwrap_or_default();
                if !browser.is_empty() {
                    let mut value = browser;
                    if let Some(profile) = account.profile.clone().filter(|p| !p.is_empty()) {
                        value.push(':');
                        value.push_str(&profile);
                    }
                    return Ok(Some(Credential::Browser(value)));
                }
                tracing::warn!("已选中的 YouTube 账号缺少浏览器名，回退旧 Cookie 配置");
            }
            Some(account)
                if account
                    .cookies
                    .as_deref()
                    .is_some_and(|text| validate_netscape(text).is_ok_and(|s| s.count > 0)) =>
            {
                return Ok(Some(Credential::AccountJar {
                    id: account.id.clone(),
                    revision: doc.revision,
                    text: account.cookies.clone().unwrap_or_default(),
                }));
            }
            _ => tracing::warn!("已选中的 YouTube 账号不可用，回退旧 Cookie 配置"),
        }
    }
    if let Some(browser) = legacy_browser.filter(|value| !value.trim().is_empty()) {
        return Ok(Some(Credential::Browser(browser)));
    }
    let Some(doc) = store.read(NAME)? else {
        return Ok(None);
    };
    let text = doc.value.as_str().ok_or_else(invalid)?;
    if validate_netscape(text)?.count == 0 {
        return Ok(None);
    }
    Ok(Some(Credential::LegacyJar(text.to_owned())))
}

pub async fn run(command: Command, browser: Option<String>, limit: Duration) -> io::Result<Output> {
    run_with_store(storage::global()?, command, browser, limit).await
}

async fn run_with_store(
    store: Arc<Store>,
    mut command: Command,
    browser: Option<String>,
    limit: Duration,
) -> io::Result<Output> {
    let deadline = tokio::time::Instant::now() + limit;
    let (mut sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = async {
            let queue = queue(&store);
            let _guard = tokio::select! {
                biased;
                _ = sender.closed() => return Err(io::Error::new(io::ErrorKind::Interrupted, "Cookie 查询已取消")),
                result = tokio::time::timeout_at(deadline, queue.lock()) => result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Cookie 查询等待超时"))?,
            };
            let mut jar = None;
            let mut expected = 0;
            let mut account: Option<(String, u64)> = None;
            // Ignore external yt-dlp config; it could override the managed jar.
            command.args(["--ignore-config", "--no-cookies", "--no-cookies-from-browser"]);
            match resolve(&store, browser)? {
                Some(Credential::Browser(value)) => {
                    command.arg("--cookies-from-browser").arg(value);
                }
                Some(Credential::LegacyJar(text)) => {
                    expected = store.revision(NAME)?.unwrap_or(0);
                    let temporary = TemporaryJar::create(&store, &text)?;
                    command.arg("--cookies").arg(&temporary.path);
                    jar = Some(temporary);
                }
                Some(Credential::AccountJar { id, revision, text }) => {
                    account = Some((id, revision));
                    let temporary = TemporaryJar::create(&store, &text)?;
                    command.arg("--cookies").arg(&temporary.path);
                    jar = Some(temporary);
                }
                None => {}
            }
            let mut output = super::subprocess::command_output_cancellable(command, deadline, "yt-dlp", sender.closed(), jar.as_ref().map(|jar| jar.directory.join("child.pid")).as_deref()).await?;
            if let Some(jar) = jar {
                if account.is_none() {
                    let current = store.read(NAME)?.map(|d| d.revision).unwrap_or(0);
                    if current != expected { return Err(io::Error::new(io::ErrorKind::WouldBlock, "Cookie 已更新，请重试查询")); }
                }
                if output.status.success() {
                    let metadata = fs::metadata(&jar.path)?;
                    if metadata.len() > MAX_COOKIE_BYTES as u64 { return Err(invalid()); }
                    let updated = fs::read_to_string(&jar.path)?;
                    if updated.is_empty() || !updated.ends_with('\n') { return Err(invalid()); }
                    validate_netscape(&updated)?;
                    let store = Arc::clone(&store);
                    match account {
                        Some((id, revision)) => {
                            tokio::task::spawn_blocking(move || writeback(&store, id, revision, updated)).await.map_err(io::Error::other)??;
                        }
                        None => {
                            tokio::task::spawn_blocking(move || store.compare_exchange(NAME, expected, Value::String(updated))).await.map_err(io::Error::other)??;
                        }
                    }
                }
                drop(jar);
            }
            output.stderr = safe_stderr(&output.stderr);
            Ok(output)
        }.await;
        let _ = sender.send(result);
    });
    receiver
        .await
        .map_err(|_| io::Error::other("Cookie 查询已停止"))?
}

fn safe_stderr(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    if !text.contains("ERROR") {
        return Vec::new();
    }
    // Preserve only the schedule information used by the monitor, never raw
    // cookie lines, authenticated URLs or upstream diagnostic payloads.
    if let Ok(pattern) = regex::Regex::new(r"(?i)(\d+)\s+(minutes?|hours?|days?)") {
        if let Some(m) = pattern.captures(&text) {
            return format!(
                "ERROR: [youtube] This live event will begin in {} {}",
                &m[1], &m[2]
            )
            .into_bytes();
        }
    }
    b"ERROR: [youtube] stream unavailable".to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    const JAR: &str = "# Netscape HTTP Cookie File\n#HttpOnly_.example.invalid\tTRUE\t/\tTRUE\t0\tsession\tsynthetic\n";
    #[test]
    fn validates_http_only_session_and_rejects_bad_rows_without_echo() {
        assert_eq!(validate_netscape(JAR).unwrap().count, 1);
        assert!(!validate_netscape("bad\tsecret-value")
            .unwrap_err()
            .to_string()
            .contains("secret-value"));
        assert!(validate_netscape(&format!("{JAR}{}", JAR.lines().last().unwrap())).is_err());
        assert_eq!(
            validate_netscape("# Netscape HTTP Cookie File\n")
                .unwrap()
                .count,
            0
        );
    }
    #[test]
    fn stderr_never_returns_secret_cookie_lines() {
        assert_eq!(
            safe_stderr(b"WARNING: skipping cookie file entry: secret-value\nERROR: broken"),
            b"ERROR: [youtube] stream unavailable"
        );
        assert_eq!(
            safe_stderr(b"ERROR: [youtube] scheduled in 12 minutes secret"),
            b"ERROR: [youtube] This live event will begin in 12 minutes"
        );
    }
}

#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    const INITIAL: &str =
        "# Netscape HTTP Cookie File\n.example.invalid\tTRUE\t/\tTRUE\t0\tvalue\t1\n";
    const SCRIPT: &str = r#"
import pathlib,sys,time
mode,marker,release=sys.argv[1:4]
path=pathlib.Path(sys.argv[sys.argv.index('--cookies')+1])
if mode in ('wait','sleep'):
    pathlib.Path(marker).write_text('started')
    if mode=='sleep': time.sleep(60)
    else:
        while not pathlib.Path(release).exists(): time.sleep(.01)
if mode=='truncate': path.write_text('')
elif mode=='bad': path.write_text('# Netscape HTTP Cookie File\ninvalid-secret-row\n')
elif mode=='delete': path.write_text('# Netscape HTTP Cookie File\n')
else:
    count=int(path.read_text().splitlines()[-1].split('\t')[-1])
    path.write_text('# Netscape HTTP Cookie File\n.example.invalid\tTRUE\t/\tTRUE\t0\tvalue\t'+str(count+1)+'\n')
if mode=='failure':
    print('ERROR: synthetic-secret-row',file=sys.stderr)
    sys.exit(1)
print('synthetic-result')
"#;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let mut nonce = [0u8; 12];
            getrandom::fill(&mut nonce).unwrap();
            let name: String = nonce.iter().map(|n| format!("{n:02x}")).collect();
            let root = std::env::temp_dir().join(format!("bilistream-cookie-test-{name}"));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn store(&self) -> Arc<Store> {
            let store = Store::open(self.0.join("data"), self.0.join("keys/master")).unwrap();
            store.write(NAME, json!(INITIAL)).unwrap();
            store
        }
        fn empty_store(&self) -> Arc<Store> {
            Store::open(self.0.join("accounts-data"), self.0.join("accounts-keys/master")).unwrap()
        }
        fn command(&self, mode: &str) -> Command {
            let mut command = Command::new("python3");
            command
                .arg("-c")
                .arg(SCRIPT)
                .arg(mode)
                .arg(self.0.join("started"))
                .arg(self.0.join("release"));
            command
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    async fn wait_for(path: &Path) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    fn clean(store: &Store) {
        assert_eq!(fs::read_dir(store.runtime_dir()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn concurrent_queries_observe_previous_cookie_writeback_and_cleanup() {
        let dir = Directory::new();
        let store = dir.store();
        let first = run_with_store(
            Arc::clone(&store),
            dir.command("increment"),
            None,
            Duration::from_secs(5),
        );
        let second = run_with_store(
            Arc::clone(&store),
            dir.command("increment"),
            None,
            Duration::from_secs(5),
        );
        let (a, b) = tokio::join!(first, second);
        assert!(a.unwrap().status.success());
        assert!(b.unwrap().status.success());
        assert!(store
            .read(NAME)
            .unwrap()
            .unwrap()
            .value
            .as_str()
            .unwrap()
            .ends_with("\t3\n"));
        clean(&store);
    }

    #[tokio::test]
    async fn rejected_partial_and_failed_output_preserve_original_jar() {
        let dir = Directory::new();
        let store = dir.store();
        for mode in ["truncate", "bad", "failure"] {
            let result = run_with_store(
                Arc::clone(&store),
                dir.command(mode),
                None,
                Duration::from_secs(5),
            )
            .await;
            if mode == "failure" {
                let output = result.unwrap();
                assert!(!output.status.success());
                assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-row"));
            } else {
                assert!(result.is_err());
            }
            assert_eq!(store.read(NAME).unwrap().unwrap().value, INITIAL);
            clean(&store);
        }
    }

    #[tokio::test]
    async fn successful_cookie_deletion_is_not_resurrected() {
        let dir = Directory::new();
        let store = dir.store();
        run_with_store(
            Arc::clone(&store),
            dir.command("delete"),
            None,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert!(!status_from(&store).unwrap().configured);
        clean(&store);
    }

    #[tokio::test]
    async fn clear_during_query_rejects_old_output_and_writeback() {
        let dir = Directory::new();
        let store = dir.store();
        let copy = Arc::clone(&store);
        let command = dir.command("wait");
        let query = tokio::spawn(async move {
            run_with_store(copy, command, None, Duration::from_secs(5)).await
        });
        wait_for(&dir.0.join("started")).await;
        let revision = store.revision(NAME).unwrap().unwrap();
        store.compare_exchange(NAME, revision, json!("")).unwrap();
        fs::write(dir.0.join("release"), b"ready").unwrap();
        assert_eq!(
            query.await.unwrap().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(!status_from(&store).unwrap().configured);
        clean(&store);
    }

    #[tokio::test]
    async fn cancelled_query_reaps_child_before_removing_cookie_file() {
        let dir = Directory::new();
        let store = dir.store();
        let copy = Arc::clone(&store);
        let command = dir.command("sleep");
        let query = tokio::spawn(async move {
            run_with_store(copy, command, None, Duration::from_secs(10)).await
        });
        wait_for(&dir.0.join("started")).await;
        let job = fs::read_dir(store.runtime_dir())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let pid: i32 = fs::read_to_string(job.join("child.pid"))
            .unwrap()
            .parse()
            .unwrap();
        query.abort();
        assert!(query.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(5), async {
            while job.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
        assert_eq!(store.read(NAME).unwrap().unwrap().value, INITIAL);
        clean(&store);
    }

    #[tokio::test]
    async fn timed_out_query_keeps_data_and_cleans_runtime_directory() {
        let dir = Directory::new();
        let store = dir.store();
        let result = run_with_store(
            Arc::clone(&store),
            dir.command("sleep"),
            None,
            Duration::from_millis(150),
        )
        .await;
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert_eq!(store.read(NAME).unwrap().unwrap().value, INITIAL);
        clean(&store);
    }

    #[test]
    fn account_validation_rejects_control_chars_and_empty_cookies() {
        assert!(prepare("", AccountSource::Cookies { content: INITIAL.to_owned() }).is_err());
        assert!(prepare(
            "ok",
            AccountSource::Browser {
                browser: "chrome\n--evil".to_owned(),
                profile: None,
            }
        )
        .is_err());
        assert!(prepare(
            "ok",
            AccountSource::Browser {
                browser: "chrome:Work".to_owned(),
                profile: None,
            }
        )
        .is_err());
        assert!(prepare(
            "ok",
            AccountSource::Browser {
                browser: "--cookies-from-browser".to_owned(),
                profile: None,
            }
        )
        .is_err());
        assert!(prepare(
            "ok",
            AccountSource::Cookies {
                content: "# Netscape HTTP Cookie File\n".to_owned(),
            }
        )
        .is_err());
        assert!(prepare(
            "ok",
            AccountSource::Cookies {
                content: "invalid-secret-row".to_owned(),
            }
        )
        .is_err());

        let account = prepare(
            " 手机 ",
            AccountSource::Browser {
                browser: " chrome ".to_owned(),
                profile: Some(" Default ".to_owned()),
            },
        )
        .unwrap();
        assert_eq!(account.label, "手机");
        assert_eq!(account.browser.as_deref(), Some("chrome"));
        assert_eq!(account.profile.as_deref(), Some("Default"));
        assert_eq!(account.count, 0);
        assert_eq!(account.id.len(), 16);
        assert!(account.id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn accounts_edit_atomically_and_reject_a_stale_revision() {
        let dir = Directory::new();
        let store = dir.empty_store();
        let added = update(Arc::clone(&store), 0, |doc| {
            doc.accounts.push(prepare(
                "手机",
                AccountSource::Cookies {
                    content: INITIAL.to_owned(),
                },
            )?);
            doc.selected_id = doc.accounts.first().map(|account| account.id.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(added.revision, 1);
        assert_eq!(added.accounts.len(), 1);
        assert_eq!(added.accounts[0].cookie_count, 1);
        assert_eq!(added.accounts[0].source_type, "cookies");
        assert_eq!(added.selected_id, Some(added.accounts[0].id.clone()));

        let stale = update(Arc::clone(&store), 0, |doc| {
            doc.accounts.clear();
            Ok(())
        })
        .unwrap_err();
        assert_eq!(stale.kind(), io::ErrorKind::WouldBlock);

        let cleared = update(Arc::clone(&store), 1, |doc| {
            doc.selected_id = None;
            Ok(())
        })
        .unwrap();
        assert_eq!(cleared.revision, 2);
        assert_eq!(cleared.selected_id, None);

        let id = added.accounts[0].id.clone();
        let deleted = update(Arc::clone(&store), 2, move |doc| {
            doc.accounts.retain(|account| account.id != id);
            Ok(())
        })
        .unwrap();
        assert!(deleted.accounts.is_empty());
    }

    #[test]
    fn selected_account_wins_over_legacy_browser_and_cookies_txt() {
        let dir = Directory::new();
        let store = dir.store();
        assert!(matches!(
            resolve(&store, Some("chrome".to_owned())).unwrap(),
            Some(Credential::Browser(_))
        ));
        assert!(matches!(
            resolve(&store, None).unwrap(),
            Some(Credential::LegacyJar(_))
        ));

        let account = prepare(
            "浏览器",
            AccountSource::Browser {
                browser: "firefox".to_owned(),
                profile: Some("work".to_owned()),
            },
        )
        .unwrap();
        let id = account.id.clone();
        update(Arc::clone(&store), 0, move |doc| {
            doc.selected_id = Some(account.id.clone());
            doc.accounts.push(account);
            Ok(())
        })
        .unwrap();
        match resolve(&store, Some("chrome".to_owned())).unwrap() {
            Some(Credential::Browser(value)) => assert_eq!(value, "firefox:work"),
            _ => panic!("选中账号必须盖过旧浏览器设置"),
        }

        update(Arc::clone(&store), 1, move |doc| {
            doc.accounts.retain(|account| account.id != id);
            Ok(())
        })
        .unwrap();
        match resolve(&store, Some("chrome".to_owned())).unwrap() {
            Some(Credential::Browser(value)) => assert_eq!(value, "chrome"),
            _ => panic!("选中账号失效时必须回退旧配置"),
        }
    }

    #[tokio::test]
    async fn account_cookies_are_written_back_and_read_back_as_a_jar() {
        let dir = Directory::new();
        let store = dir.empty_store();
        let account = prepare(
            "手机",
            AccountSource::Cookies {
                content: INITIAL.to_owned(),
            },
        )
        .unwrap();
        let id = account.id.clone();
        update(Arc::clone(&store), 0, move |doc| {
            doc.selected_id = Some(account.id.clone());
            doc.accounts.push(account);
            Ok(())
        })
        .unwrap();

        let output = run_with_store(
            Arc::clone(&store),
            dir.command("increment"),
            None,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert!(output.status.success());
        let after = status_of(&store).unwrap();
        assert_eq!(after.revision, 2);
        let doc = store.read(ACCOUNTS).unwrap().unwrap();
        assert!(doc.value["accounts"][0]["cookies"]
            .as_str()
            .unwrap()
            .ends_with("\t2\n"));
        assert_eq!(doc.value["accounts"][0]["count"], json!(1));

        let stale = writeback(&store, id, 1, INITIAL.to_owned()).unwrap_err();
        assert_eq!(stale.kind(), io::ErrorKind::WouldBlock);
        clean(&store);
    }
}
