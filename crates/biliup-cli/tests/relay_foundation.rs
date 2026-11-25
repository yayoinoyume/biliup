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

/// 大请求体走真正的 HTTP 客户端：413 时服务会先回包再断开，裸 TCP 读不到。
async fn post_status(port: u16, path: &str, cookie: &str, body: &str) -> u16 {
    reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header(reqwest::header::COOKIE, cookie)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_owned())
        .send()
        .await
        .expect("发送大请求")
        .status()
        .as_u16()
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

struct KeyFileEnv(Option<std::ffi::OsString>);

impl KeyFileEnv {
    fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("BILISTREAM_KEY_FILE");
        unsafe { std::env::set_var("BILISTREAM_KEY_FILE", path) };
        Self(previous)
    }
}

impl Drop for KeyFileEnv {
    fn drop(&mut self) {
        unsafe {
            match self.0.take() {
                Some(value) => std::env::set_var("BILISTREAM_KEY_FILE", value),
                None => std::env::remove_var("BILISTREAM_KEY_FILE"),
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
    let _key_env = KeyFileEnv::set(&key_file);

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
    let _key_env = KeyFileEnv::set(&key_dir.path().join("master.key"));
    let work_dir = tempfile::tempdir().unwrap();
    let (port, stop, task) = spawn_server(true, work_dir.path(), &logging);
    let (status, _) = wait_for_answer(port).await;
    assert_eq!(status, 401, "开启 --auth 后未登录访问 /v1/relay/health 应被拦截");

    let admin = bootstrap_admin(port, "boss-password").await;
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
        "/v1/relay/config",
        "/v1/relay/history/status",
        "/v1/relay/manage/areas",
        "/v1/relay/setup-status",
    ] {
        let reply = request(port, "GET", path, Some(&viewer), "").await;
        assert!(
            reply.status != 401 && reply.status != 403,
            "只读角色访问 {path} 被拒：{}",
            reply.status
        );
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

    for (method, path) in [
        ("GET", "/v1/relay/status"),
        ("POST", "/v1/relay/stop"),
        ("POST", "/v1/relay/history/clean"),
        ("GET", "/v1/relay/refresh/youtube"),
    ] {
        let reply = request(port, method, path, Some(&operator), "").await;
        assert!(
            reply.status != 401 && reply.status != 403,
            "操作员访问 {method} {path} 被拒：{}",
            reply.status
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
            reply.status != 401 && reply.status != 403,
            "超管访问 {method} {path} 被拒：{}",
            reply.status
        );
    }
    for (method, path) in [
        ("POST", "/v1/relay/cluster/v1/history"),
        ("GET", "/v1/relay/update/check"),
    ] {
        let reply = request(port, method, path, Some(&admin), "").await;
        assert_eq!(reply.status, 404, "未挂载的 {method} {path} 必须 404");
    }

    let oversized = format!(
        "{{\"password\":\"boss-password\",\"backup_base64\":\"{}\"}}",
        "A".repeat(3 * 1024 * 1024)
    );
    let restore = post_status(port, "/v1/relay/storage/restore", &admin, &oversized).await;
    assert_ne!(restore, 413, "存储恢复必须放行 48MiB 以内的请求体");
    let rejected = post_status(port, "/v1/relay/config", &admin, &oversized).await;
    assert_eq!(rejected, 413, "其它路由不得跟着放宽 body limit");

    stop_and_assert_released(port, stop, task).await;
    assert_eq!(home_key_files(), home_keys_before, "测试不得写入真实密钥目录");
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_events_stream_ends_when_the_session_is_revoked() {
    let _serial = SERIAL.lock().await;
    let logging = init_tracing_with("info", false);
    let key_dir = tempfile::tempdir().unwrap();
    let _key_env = KeyFileEnv::set(&key_dir.path().join("master.key"));
    let work_dir = tempfile::tempdir().unwrap();
    let (port, stop, task) = spawn_server(true, work_dir.path(), &logging);
    wait_for_answer(port).await;
    let admin = bootstrap_admin(port, "boss-password").await;
    let viewer_id = create_user(port, &admin, "ro", "ro-password", "viewer").await;
    let viewer = login(port, "ro", "ro-password").await;

    // 生产用 30 秒复查；这里把 SSE 会话复查压到 100ms，才可能在秒级断言断流。
    unsafe { std::env::set_var("BILISTREAM_SSE_RECHECK_MS", "100") };
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
    unsafe { std::env::remove_var("BILISTREAM_SSE_RECHECK_MS") };

    stop_and_assert_released(port, stop, task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn websub_callback_is_mounted_outside_the_login_guard() {
    let _serial = SERIAL.lock().await;
    let logging = init_tracing_with("info", false);
    let key_dir = tempfile::tempdir().unwrap();
    let _key_env = KeyFileEnv::set(&key_dir.path().join("master.key"));
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
    let oversized_push = "x".repeat(300 * 1024);
    let limited = request(port, "POST", "/v1/relay/websub/youtube", None, &oversized_push).await;
    assert_eq!(
        limited.status, 413,
        "请求必须到 WebSub 处理器（它自己的 256KiB 上限），而不是被登录墙拦下"
    );

    let push = request(port, "POST", "/v1/relay/websub/youtube", None, "<feed/>").await;
    assert_eq!(
        push.status, 401,
        "缺 X-Hub-Signature 的推送必须被 WebSub 自己的 HMAC 校验拒绝"
    );

    stop_and_assert_released(port, stop, task).await;
}
