use std::path::Path;
use std::time::Duration;

use biliup_cli::entry::{Logging, init_tracing_with};
use biliup_cli::server::errors::AppResult;
use biliup_cli::server::fleet::FleetOptions;
use biliup_cli::serve_on;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const COOKIE_FILE: &str = "cookies.json";
const HEALTH_PATH: &str = "/v1/relay/health";

async fn get(port: u16, path: &str) -> Option<(u16, String)> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.ok()?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text.lines().next()?.split(' ').nth(1)?.parse().ok()?;
    let body = text.split("\r\n\r\n").nth(1).unwrap_or_default().to_owned();
    Some((status, body))
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

fn home_key_files() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let dir = std::path::PathBuf::from(home)
        .join(".config")
        .join("bilistream")
        .join("keys");
    let mut names = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_health_is_ready_behind_optional_auth() {
    let logging = init_tracing_with("info", false);
    let key_dir = tempfile::tempdir().unwrap();
    let key_file = key_dir.path().join("master.key");
    let home_keys_before = home_key_files();
    unsafe { std::env::set_var("BILISTREAM_KEY_FILE", &key_file) };

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
