//! `entry::serve` with a host-bound listener and a host-owned shutdown, as the
//! desktop app uses it. Kept in its own test binary because `work_dir`
//! switches the working directory of the whole process.

use std::time::Duration;

use biliup_cli::entry::{ServeOptions, init_tracing_with, serve};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

async fn get_status_line(port: u16) -> Option<String> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.ok()?;
    let response = String::from_utf8_lossy(&response);
    response.lines().next().map(str::to_owned)
}

#[tokio::test(flavor = "multi_thread")]
async fn serves_on_prebound_listener_until_host_shutdown() {
    let work_dir = tempfile::tempdir().unwrap();
    // 密钥文件必须落在 data 目录之外、也不能是真实的 ~/.config/bilistream：
    // 服务找不到默认目录时会往用户家目录写新 .key，测试不能留下这种东西。
    let key_dir = tempfile::tempdir().unwrap();
    let key_file = key_dir.path().join("relay.key");
    let home_keys_before = home_key_files();
    let _key_env = EnvVarGuard::set("BILISTREAM_KEY_FILE", &key_file);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let logging = init_tracing_with("info", false);

    let server = tokio::spawn(serve(ServeOptions {
        bind: "127.0.0.1".into(),
        port: 1,
        auth: false,
        secure_session_cookie: false,
        config: None,
        user_cookie: "cookies.json".into(),
        work_dir: Some(work_dir.path().to_path_buf()),
        log_handle: logging.handle(),
        listener: Some(listener),
        shutdown: Some(Box::pin(async move {
            let _ = stopped.await;
        })),
        ffmpeg: None,
        render_tools_dir: None,
        fleet: Default::default(),
    }));

    let mut status = None;
    for _ in 0..100 {
        status = get_status_line(port).await;
        if status.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let status = status.expect("server never answered on the pre-bound port");
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    assert!(work_dir.path().join("data/data.sqlite3").exists());

    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("server did not stop after the host shutdown fired")
        .unwrap()
        .unwrap();
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "listener still open after shutdown"
    );
    assert!(key_file.exists(), "测试密钥必须落在临时目录");
    assert_eq!(
        home_key_files(),
        home_keys_before,
        "测试不得向真实 $HOME/.config/bilistream/keys 新增 .key"
    );
}
