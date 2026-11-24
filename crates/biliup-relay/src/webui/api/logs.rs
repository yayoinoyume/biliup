//! 日志、版本与外部依赖探测：只读端点，绝不触发任何下载或自更新。

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use axum::extract::Json;
use serde::Serialize;

use super::ApiResponse;
use crate::app_state::AppState;

/// 单个外部命令探测的期限；超时即按不可用处理。
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const PROBE_POLL: Duration = Duration::from_millis(10);

#[derive(Serialize)]
pub struct LogsResponse {
    success: bool,
    logs: String,
}

pub async fn get_logs_endpoint() -> Json<LogsResponse> {
    let logs = AppState::current().get_logs();
    Json(LogsResponse {
        success: true,
        logs: logs.join("\n"),
    })
}

#[derive(Serialize)]
pub struct VersionInfo {
    version: String,
}

pub async fn get_version() -> Json<ApiResponse<VersionInfo>> {
    Json(ApiResponse {
        success: true,
        data: Some(VersionInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
        }),
        message: None,
    })
}

/// 宿主环境里外部工具的只读可用性；本进程从不下载这些二进制。
#[derive(Serialize, Debug, PartialEq, Eq, Default)]
pub struct DepsStatus {
    ffmpeg: bool,
    yt_dlp: bool,
    streamlink: bool,
    ready: bool,
}

/// 在期限内以 0 退出的才算可用。超时或等待失败一律杀掉并 `wait` 回收，
/// 绝不在宿主里留下僵尸进程，结果按不可用处理。
fn probe_with(program: &Path, timeout: Duration) -> bool {
    let Ok(mut child) = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(PROBE_POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn probe(program: &Path) -> bool {
    probe_with(program, PROBE_TIMEOUT)
}

fn deps_status() -> DepsStatus {
    let ffmpeg = probe(Path::new("ffmpeg"));
    let yt_dlp = probe(Path::new("yt-dlp"));
    let streamlink = probe(Path::new("streamlink"));
    DepsStatus {
        ffmpeg,
        yt_dlp,
        streamlink,
        ready: ffmpeg && (yt_dlp || streamlink),
    }
}

pub async fn get_deps_status() -> Json<DepsStatus> {
    // 外部进程探测是阻塞 I/O：绝不占用 tokio worker 线程。
    Json(tokio::task::spawn_blocking(deps_status).await.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn logs_endpoint_returns_the_lines_the_app_state_buffer_holds() {
        let _guard = crate::storage::lock_test_global();
        let state = AppState::current();
        state.init_log_buffer();
        state.add_log_line("实测第一行".to_string());
        state.add_log_line("实测第二行".to_string());

        let Json(response) = get_logs_endpoint().await;
        assert!(response.success);
        assert_eq!(response.logs, "实测第一行\n实测第二行");
    }

    #[tokio::test]
    async fn version_endpoint_reports_the_crate_version_and_no_self_update() {
        let Json(response) = get_version().await;
        assert!(response.success);
        let data = response.data.expect("版本必须返回");
        assert_eq!(data.version, env!("CARGO_PKG_VERSION"));
        let wire = serde_json::to_value(ApiResponse {
            success: true,
            data: Some(data),
            message: None,
        })
        .unwrap();
        for absent in ["latest", "updater", "download_url", "update_available"] {
            assert!(
                !wire.to_string().contains(absent),
                "版本端点不得带自更新字段 {absent}"
            );
        }
    }

    #[tokio::test]
    async fn deps_probe_is_read_only_and_reports_executables_only() {
        let dir = tempfile::tempdir().unwrap();
        let runnable = dir.path().join("probe-ok.sh");
        std::fs::write(&runnable, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&runnable, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(probe(&runnable), "以 0 退出的脚本必须判为可用");
        }
        assert!(!probe(&dir.path().join("missing-tool")));

        let status = deps_status();
        let wire = serde_json::to_value(&status).unwrap();
        let keys: Vec<&str> = wire
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["ffmpeg", "ready", "streamlink", "yt_dlp"]);
        assert_eq!(status.ready, status.ffmpeg && (status.yt_dlp || status.streamlink));
    }

    /// 慢命令必须在期限附近返回 false，且子进程被回收（不是僵尸）。
    #[cfg(unix)]
    #[test]
    fn a_slow_probe_times_out_without_leaving_a_zombie() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let slow = dir.path().join("probe-slow.sh");
        std::fs::write(
            &slow,
            format!("#!/bin/sh\necho $$ > '{}'\nsleep 30\n", pid_file.display()),
        )
        .unwrap();
        std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();

        let timeout = Duration::from_millis(300);
        let started = Instant::now();
        assert!(!probe_with(&slow, timeout), "慢命令必须判为不可用");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "探测必须在有界时间内返回，实际 {elapsed:?}"
        );

        let pid: u32 = std::fs::read_to_string(&pid_file)
            .expect("子进程必须写下自己的 pid")
            .trim()
            .parse()
            .expect("pid 数字");
        #[cfg(target_os = "linux")]
        {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok();
            let state = stat.as_deref().and_then(|stat| stat.rsplit(')').next());
            assert!(
                state.is_none_or(|rest| !rest.trim_start().starts_with('Z')),
                "超时的子进程必须被回收，不得留成僵尸：{stat:?}"
            );
        }
        assert!(
            std::fs::metadata(format!("/proc/{pid}")).is_err() || cfg!(not(target_os = "linux")),
            "子进程必须已经退出"
        );
    }
}
