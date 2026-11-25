use std::path::Path;
use std::time::Duration;

use biliup_cli::entry::{Logging, init_tracing_with};
use biliup_cli::server::errors::AppResult;
use biliup_cli::server::fleet::FleetOptions;
use biliup_cli::serve_on;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const COOKIE_FILE: &str = "cookies.json";
const HEALTH_PATH: &str = "/v1/relay/health";
/// 服务把进程当前目录当工作目录，同一二进制里的测试必须串行。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn get(port: u16, path: &str) -> Option<(u16, String)> {
    try_request(port, "GET", path, None, "")
        .await
        .map(|reply| (reply.status, reply.body))
}

struct Reply {
    status: u16,
    headers: String,
    body: String,
}

async fn request(
    port: u16,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: &str,
) -> Reply {
    try_request(port, method, path, cookie, body)
        .await
        .unwrap_or_else(|| panic!("{method} {path} 没有得到任何响应"))
}

async fn try_request(
    port: u16,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: &str,
) -> Option<Reply> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    if let Some(cookie) = cookie {
        head.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    head.push_str("\r\n");
    let mut payload = head.into_bytes();
    payload.extend_from_slice(body.as_bytes());
    let _ = stream.write_all(&payload).await;
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw).await;
    if raw.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("{method} {path} 没有状态行：{head}"));
    Some(Reply {
        status,
        headers: head.to_owned(),
        body: body.to_owned(),
    })
}

/// 大请求体：全双工并发读写，带有限重试。
///
/// 服务端检测到超限会立刻写出 413 再关连接、不排空剩下的请求体，内核因此回 RST。
/// 「先写完再读」的单线程写法会把这个 413 连同接收缓冲区一起丢掉，所以写侧跑在
/// 独立任务里、读侧在同一个连接上并发读；写侧只把 BrokenPipe / ConnectionReset /
/// ConnectionAborted 当作正常的提前收场。
///
/// 写半必须活到读侧读完响应为止：服务端回包前看到写半关闭，会当成连接已结束而
/// 直接丢弃响应（不再回任何字节），所以写任务写完先等一个放行信号再关。
///
/// RST 仍可能把已经写出的 413 在内核里丢掉（这是 TCP 的时序，不是服务端没回）。
/// 调用方把超限量压到比上限多 1 字节，剩下的尾巴很小、更不容易触发 RST；
/// 真丢了就重开一条连接重试，重试次数有限（3 次），判据不变：只有真正解析出
/// 状态行才算数，绝不把 RST 当成 413。
async fn oversized_status(port: u16, path: &str, cookie: &str, body: &str) -> u16 {
    for attempt in 1..=3 {
        if let Some(status) = oversized_status_once(port, path, cookie, body).await {
            return status;
        }
        assert!(
            attempt < 3,
            "POST {path} 连试三次都没有拿到状态行（只看到连接被重置）"
        );
    }
    unreachable!("上面的断言保证第三次失败就 panic")
}

async fn oversized_status_once(port: u16, path: &str, cookie: &str, body: &str) -> Option<u16> {
    let mut stream = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .expect("连接测试服务超时")
    .expect("连接测试服务");
    let mut head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: {}\r\nContent-Type: application/json\r\n",
        body.len()
    );
    if !cookie.is_empty() {
        head.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    head.push_str("\r\n");
    tokio::time::timeout(Duration::from_secs(10), stream.write_all(head.as_bytes()))
        .await
        .expect("写出请求头超时")
        .expect("写出请求头");
    let (mut reader, mut writer) = stream.into_split();
    let payload = body.to_owned();
    let label = path.to_owned();
    let (finish, finished) = tokio::sync::oneshot::channel::<()>();
    let writing = tokio::spawn(async move {
        let mut failure = None;
        for chunk in payload.as_bytes().chunks(64 * 1024) {
            if let Err(error) = writer.write_all(chunk).await {
                failure = Some(error);
                break;
            }
        }
        let _ = finished.await;
        drop(writer);
        failure
    });
    let raw = tokio::time::timeout(Duration::from_secs(30), async move {
        let mut raw = Vec::new();
        let mut chunk = [0u8; 16384];
        loop {
            match reader.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(len) => raw.extend_from_slice(&chunk[..len]),
            }
        }
        raw
    })
    .await
    .expect("读取响应超时");
    let _ = finish.send(());
    let written = tokio::time::timeout(Duration::from_secs(10), writing)
        .await
        .expect("写请求体任务超时");
    if let Ok(Some(error)) = written {
        match error.kind() {
            std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted => {}
            _ => panic!("{label} 写请求体意外失败：{error}"),
        }
    }
    let text = String::from_utf8_lossy(&raw).into_owned();
    text.lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse::<u16>().ok())
}

/// 头部名大小写不敏感；值原样返回（会话 id 是大小写敏感的 base64）。
fn header_value(reply: &Reply, name: &str) -> Option<String> {
    reply.headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

fn session_cookie(reply: &Reply) -> String {
    header_value(reply, "set-cookie")
        .and_then(|value| value.split(';').next().map(str::to_string))
        .expect("响应必须带会话 Cookie")
}

async fn bootstrap_admin(port: u16, password: &str) -> String {
    let body = format!("{{\"username\":\"biliup\",\"password\":\"{password}\"}}");
    let reply = request(port, "POST", "/v1/users/register", None, &body).await;
    assert_eq!(reply.status, 200, "管理员注册失败：{}", reply.body);
    session_cookie(&reply)
}

async fn create_user(
    port: u16,
    admin: &str,
    username: &str,
    password: &str,
    role: &str,
) -> i64 {
    let body = format!(
        "{{\"username\":\"{username}\",\"password\":\"{password}\",\"role\":\"{role}\"}}"
    );
    let reply = request(port, "POST", "/v1/web-users", Some(admin), &body).await;
    assert_eq!(reply.status, 201, "创建 {role} 失败：{}", reply.body);
    serde_json::from_str::<serde_json::Value>(&reply.body).unwrap()["id"]
        .as_i64()
        .expect("新建用户必须返回 id")
}

/// 直接写一份最小配置：只为了让需要 config.json 的只读端点有东西可读，
/// 且不触发任何后台刷新（向导路由会顺带唤醒状态轮询去联网）。
fn seed_min_config() {
    let seeded: biliup_relay::Config = serde_json::from_value(serde_json::json!({
        "auto_cover": false,
        "enable_anti_collision": false,
        "interval": 3600,
        "bililive": {
            "enable_danmaku_command": false,
            "room": 1,
            "bili_rtmp_url": "",
            "bili_rtmp_key": ""
        },
        "twitch": {},
        "youtube": {},
        "holodex_api_key": null,
        "riot_api_key": null,
        "enable_lol_monitor": false,
        "lol_monitor_interval": null,
        "anti_collision_list": {}
    }))
    .expect("最小配置必须能反序列化");
    biliup_relay::storage::write_document("config.json", &seeded).expect("写入最小配置");
}

/// `manage/*` 的写端点要读 areas.json：缺文档时一律 500（数据不存在）。
/// 补一份最小的合法文档，写入路径才是真的可写、而不是靠 500 蒙过去。
fn seed_min_areas() {
    biliup_relay::storage::write_document("areas.json", &serde_json::json!({ "areas": [] }))
        .expect("写入最小分区文档");
}

async fn login(port: u16, username: &str, password: &str) -> String {
    let body = format!("{{\"username\":\"{username}\",\"password\":\"{password}\"}}");
    let reply = request(port, "POST", "/v1/users/login", None, &body).await;
    assert_eq!(reply.status, 200, "登录 {username} 失败：{}", reply.body);
    session_cookie(&reply)
}

async fn open_sse(port: u16, cookie: &str) -> (String, tokio::net::TcpStream) {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("连接测试服务");
    let head = format!(
        "GET /v1/relay/events HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes()).await.expect("写出 SSE 请求");
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.expect("读 SSE 响应头");
        assert!(read > 0, "SSE 响应头没读完连接就结束了");
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        assert!(buffer.len() < 64 * 1024, "SSE 响应头过大");
    }
    (String::from_utf8_lossy(&buffer).into_owned(), stream)
}

async fn stream_closed(stream: &mut tokio::net::TcpStream, wait: Duration) -> bool {
    tokio::time::timeout(wait, async {
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await
    .is_ok()
}

fn spawn_server(
    auth: bool,
    work_dir: &Path,
    logging: &Logging,
) -> (u16, tokio::sync::oneshot::Sender<()>, tokio::task::JoinHandle<AppResult<()>>) {
    std::env::set_current_dir(work_dir).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve_on(
        listener,
        auth,
        false,
        logging.handle(),
        None,
        COOKIE_FILE.into(),
        FleetOptions::default(),
        Some(Box::pin(async move {
            let _ = stopped.await;
        })),
    ));
    (port, stop, task)
}

async fn wait_for_answer(port: u16) -> (u16, String) {
    for _ in 0..200 {
        if let Some(response) = get(port, HEALTH_PATH).await {
            return response;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("服务没有在预绑定端口 {port} 上应答");
}

async fn stop_and_assert_released(
    port: u16,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<AppResult<()>>,
) {
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(20), task)
        .await
        .expect("优雅关闭信号发出后服务没有停下")
        .unwrap()
        .unwrap();
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "关闭后 {port} 仍然可连"
    );
}

/// 环境变量守卫：断言失败也要恢复原值，别把设置漏给同进程的其它测试。
struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn key_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        dirs.push(
            std::path::PathBuf::from(xdg)
                .join("bilistream")
                .join("keys"),
        );
    }
    if let Some(home) = std::env::var_os("HOME") {
        let dir = std::path::PathBuf::from(home)
            .join(".config")
            .join("bilistream")
            .join("keys");
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

fn home_key_files() -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for dir in key_dirs() {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            files.push(entry.path());
        }
    }
    files.sort();
    files
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_health_is_ready_behind_optional_auth() {
    let _serial = SERIAL.lock().await;
    let logging = init_tracing_with("info", false);
    let key_dir = tempfile::tempdir().unwrap();
    let key_file = key_dir.path().join("master.key");
    let home_keys_before = home_key_files();
    let _key_env = EnvVarGuard::set("BILISTREAM_KEY_FILE", &key_file);

    let open_dir = tempfile::tempdir().unwrap();
    let (port, stop, task) = spawn_server(false, open_dir.path(), &logging);
    let (status, body) = wait_for_answer(port).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"state\":\"ready\""), "{body}");
    assert!(body.contains("\"storage_ready\":true"), "{body}");
    assert!(body.contains("\"enabled\":true"), "{body}");
    assert!(body.contains("\"sqlite_driver\":\"rusqlite/bundled\""), "{body}");
    assert!(
        open_dir.path().join("data/relay/relay.db").exists(),
        "data/relay 下应生成加密库文件"
    );
    assert!(
        !std::fs::read_dir(open_dir.path().join("data/relay"))
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".key")),
        "密钥文件不得写进数据目录"
    );
    stop_and_assert_released(port, stop, task).await;

    let guarded_dir = tempfile::tempdir().unwrap();
    let (port, stop, task) = spawn_server(true, guarded_dir.path(), &logging);
    let (status, body) = wait_for_answer(port).await;
    assert_eq!(status, 401, "开启 --auth 后未登录应被拦截：{body}");
    stop_and_assert_released(port, stop, task).await;

    assert!(key_file.exists(), "测试密钥必须落在临时目录");
    assert_eq!(
        home_key_files(),
        home_keys_before,
        "测试不得向真实 $HOME/.config/bilistream/keys 写入新密钥"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_routes_enforce_the_three_role_matrix_and_the_restore_body_limit() {
    let _serial = SERIAL.lock().await;
    let logging = init_tracing_with("info", false);
    let key_dir = tempfile::tempdir().unwrap();
    let home_keys_before = home_key_files();
    let _key_env = EnvVarGuard::set("BILISTREAM_KEY_FILE", key_dir.path().join("master.key"));
    let work_dir = tempfile::tempdir().unwrap();
    let (port, stop, task) = spawn_server(true, work_dir.path(), &logging);
    let (status, _) = wait_for_answer(port).await;
    assert_eq!(status, 401, "开启 --auth 后未登录访问 /v1/relay/health 应被拦截");

    let admin = bootstrap_admin(port, "boss-password").await;
    // /v1/relay/history/status 要读 config.json。直接写文档而不走向导路由：
    // 向导会 request_status_refresh() 唤醒状态轮询，进而真的去问 B 站（测试必须离线）。
    seed_min_config();
    seed_min_areas();
    create_user(port, &admin, "op", "op-password", "operator").await;
    create_user(port, &admin, "ro", "ro-password", "viewer").await;
    let operator = login(port, "op", "op-password").await;
    let viewer = login(port, "ro", "ro-password").await;

    for (method, path) in [
        ("GET", "/v1/relay/status"),
        ("POST", "/v1/relay/stop"),
        ("POST", "/v1/relay/config"),
    ] {
        let reply = request(port, method, path, None, "").await;
        assert_eq!(reply.status, 401, "匿名 {method} {path}");
    }

    for path in [
        "/v1/relay/status",
        "/v1/relay/history/status",
        "/v1/relay/setup-status",
    ] {
        let reply = request(port, "GET", path, Some(&viewer), "").await;
        assert!(
            (200..300).contains(&reply.status),
            "不依赖外部配置的只读端点 {path} 必须是 2xx：{} {}",
            reply.status,
            reply.body
        );
    }
    // seed 之后这两个端点有东西可读：必须真的 200，不能靠“不是 4xx”蒙过去。
    for path in ["/v1/relay/config", "/v1/relay/manage/areas"] {
        let reply = request(port, "GET", path, Some(&viewer), "").await;
        assert_eq!(reply.status, 200, "只读角色访问 {path}：{}", reply.body);
    }
    for (method, path) in [
        ("POST", "/v1/relay/stop"),
        ("POST", "/v1/relay/history/clean"),
        ("POST", "/v1/relay/config"),
        ("GET", "/v1/relay/storage"),
        ("GET", "/v1/relay/setup/login-status"),
        ("GET", "/v1/relay/setup/qrcode"),
    ] {
        let reply = request(port, method, path, Some(&viewer), "").await;
        assert_eq!(reply.status, 403, "只读角色 {method} {path}");
    }

    // 停播会真的外呼 B 站 stopLive：离线只要求它走到处理器（200 成功 / 500 外呼失败），
    // 被权限墙拦下才是 401/403/404/405。
    let halted = request(port, "POST", "/v1/relay/stop", Some(&operator), "").await;
    assert!(
        matches!(halted.status, 200 | 500),
        "操作员访问 POST /v1/relay/stop：{} {}",
        halted.status,
        halted.body
    );
    for (method, path) in [
        ("GET", "/v1/relay/status"),
        ("POST", "/v1/relay/history/clean"),
        ("GET", "/v1/relay/refresh/youtube"),
    ] {
        let reply = request(port, method, path, Some(&operator), "").await;
        assert!(
            !matches!(reply.status, 401 | 403 | 404 | 405 | 500),
            "操作员访问 {method} {path} 被拒或报错：{} {}",
            reply.status,
            reply.body
        );
    }
    for (method, path) in [
        ("POST", "/v1/relay/config"),
        ("POST", "/v1/relay/storage/restore"),
        ("POST", "/v1/relay/manage/areas"),
        ("GET", "/v1/relay/setup/qrcode"),
        ("POST", "/v1/relay/setup/save-config"),
    ] {
        let reply = request(port, method, path, Some(&operator), "").await;
        assert_eq!(reply.status, 403, "操作员 {method} {path}");
    }

    for (method, path, body) in [
        ("GET", "/v1/relay/storage", ""),
        ("GET", "/v1/relay/setup/login-status", ""),
        ("GET", "/v1/relay/player-filter", ""),
        ("POST", "/v1/relay/config", "{}"),
        ("POST", "/v1/relay/manage/areas", "{}"),
        ("POST", "/v1/relay/banned-keywords", "{}"),
        ("POST", "/v1/relay/crop/update", "{}"),
        ("POST", "/v1/relay/ffmpeg-cache/update", "{}"),
        ("POST", "/v1/relay/setup/save-config", "{}"),
    ] {
        let reply = request(port, method, path, Some(&admin), body).await;
        assert!(
            !matches!(reply.status, 401 | 403 | 404 | 405 | 500),
            "超管访问 {method} {path} 被拒或报错：{} {}",
            reply.status,
            reply.body
        );
    }
    for (method, path) in [
        ("POST", "/v1/relay/cluster/v1/history/clean"),
        ("GET", "/v1/relay/update/check"),
    ] {
        let reply = request(port, method, path, Some(&admin), "").await;
        assert_eq!(reply.status, 404, "未挂载的 {method} {path} 必须 404");
    }

    let oversized = format!(
        "{{\"password\":\"boss-password\",\"backup_base64\":\"{}\"}}",
        "A".repeat(3 * 1024 * 1024)
    );
    let restore = oversized_status(port, "/v1/relay/storage/restore", &admin, &oversized).await;
    // 3MiB 体必须真的进到处理器：它自己按“备份格式无效”回 400（不是 413、也不是 500）。
    assert_eq!(
        restore, 400,
        "存储恢复必须放行 48MiB 以内的请求体并由处理器拒绝无效备份"
    );
    // 只比默认上限多一个字节：服务端会把整个请求体读掉才报 413，关连接时它的
    // 接收缓冲区是空的，于是正常 FIN 而不会用 RST 把 413 打掉。多送 1MiB 时
    // 剩余字节会让服务端以 RST 收场，响应可能被内核丢掉（r1 放行失败就是这个）。
    let mut over_limit = format!(
        "{{\"password\":\"boss-password\",\"backup_base64\":\"{}\"}}",
        "A".repeat(2 * 1024 * 1024)
    );
    over_limit.truncate(2 * 1024 * 1024 + 1);
    let rejected = oversized_status(port, "/v1/relay/config", &admin, &over_limit).await;
    assert_eq!(rejected, 413, "其它路由不得跟着放宽 body limit");

    stop_and_assert_released(port, stop, task).await;
    assert_eq!(home_key_files(), home_keys_before, "测试不得写入真实密钥目录");
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_events_stream_ends_when_the_session_is_revoked() {
    let _serial = SERIAL.lock().await;
    let logging = init_tracing_with("info", false);
    let key_dir = tempfile::tempdir().unwrap();
    let _key_env = EnvVarGuard::set("BILISTREAM_KEY_FILE", key_dir.path().join("master.key"));
    let work_dir = tempfile::tempdir().unwrap();
    let (port, stop, task) = spawn_server(true, work_dir.path(), &logging);
    wait_for_answer(port).await;
    let admin = bootstrap_admin(port, "boss-password").await;
    let viewer_id = create_user(port, &admin, "ro", "ro-password", "viewer").await;
    let viewer = login(port, "ro", "ro-password").await;

    // 生产用 30 秒复查；这里把 SSE 会话复查压到 100ms，才可能在秒级断言断流。
    let _sse_env = EnvVarGuard::set("BILISTREAM_SSE_RECHECK_MS", "100");
    let (head, mut stream) = open_sse(port, &viewer).await;
    let lowered = head.to_lowercase();
    assert!(lowered.contains(" 200 "), "SSE 建流失败：{head}");
    assert!(lowered.contains("content-type: text/event-stream"), "{head}");
    assert!(
        !stream_closed(&mut stream, Duration::from_millis(200)).await,
        "刚建立的流不能立刻结束"
    );

    let revoked = request(
        port,
        "POST",
        &format!("/v1/web-users/{viewer_id}/logout-all"),
        Some(&admin),
        "",
    )
    .await;
    assert_eq!(revoked.status, 204, "强制下线：{}", revoked.body);
    assert!(
        stream_closed(&mut stream, Duration::from_secs(2)).await,
        "会话撤销后 SSE 必须在一个复查周期内干净结束"
    );
    stop_and_assert_released(port, stop, task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn websub_callback_is_mounted_outside_the_login_guard() {
    let _serial = SERIAL.lock().await;
    let logging = init_tracing_with("info", false);
    let key_dir = tempfile::tempdir().unwrap();
    let _key_env = EnvVarGuard::set("BILISTREAM_KEY_FILE", key_dir.path().join("master.key"));
    let work_dir = tempfile::tempdir().unwrap();
    let (port, stop, task) = spawn_server(true, work_dir.path(), &logging);
    wait_for_answer(port).await;

    let verify = request(
        port,
        "GET",
        "/v1/relay/websub/youtube?hub.mode=subscribe&hub.topic=https%3A%2F%2Fwww.youtube.com%2Fxml%2Ffeeds%2Fvideos.xml%3Fchannel_id%3DUCprobe&hub.challenge=probe",
        None,
        "",
    )
    .await;
    assert_ne!(verify.status, 401, "Hub 验证不带 Cookie，不能被登录墙拦住");
    assert_eq!(verify.status, 404, "没有待验证订阅时由 WebSub 自己拒绝");

    // 超过 WebSub 自己的 256KiB 上限：登录墙会先回 401，走到处理器才会是 413。
    // 与 2MiB 用例同理：只超出 256KiB 上限一个字节，服务端读干净再回 413。
    let oversized_push = "x".repeat(256 * 1024 + 1);
    let limited = oversized_status(port, "/v1/relay/websub/youtube", "", &oversized_push).await;
    assert_eq!(
        limited, 413,
        "请求必须到 WebSub 处理器（它自己的 256KiB 上限），而不是被登录墙拦下"
    );

    let push = request(port, "POST", "/v1/relay/websub/youtube", None, "<feed/>").await;
    assert_eq!(
        push.status, 401,
        "缺 X-Hub-Signature 的推送必须被 WebSub 自己的 HMAC 校验拒绝"
    );

    stop_and_assert_released(port, stop, task).await;
}
