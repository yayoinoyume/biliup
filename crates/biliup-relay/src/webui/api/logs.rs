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

/// 每个工具只认自己的版本开关：ffmpeg 用单横线，yt-dlp / streamlink 用双横线。
/// ffmpeg 对 `--version` 返回退出码 8，用错就会永远判为不可用。
const FFMPEG_VERSION_ARGS: &[&str] = &["-version"];
const TOOL_VERSION_ARGS: &[&str] = &["--version"];

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
fn probe_with(program: &Path, args: &[&str], timeout: Duration) -> bool {
    let Ok(mut child) = Command::new(program)
        .args(args)
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
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Err(error) => {
                tracing::debug!("依赖探测等待失败：{error}");
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn probe(program: &Path, args: &[&str]) -> bool {
    probe_with(program, args, PROBE_TIMEOUT)
}

fn deps_status_with(ffmpeg: &Path, yt_dlp: &Path, streamlink: &Path) -> DepsStatus {
    let ffmpeg = probe(ffmpeg, FFMPEG_VERSION_ARGS);
    let yt_dlp = probe(yt_dlp, TOOL_VERSION_ARGS);
    let streamlink = probe(streamlink, TOOL_VERSION_ARGS);
    DepsStatus {
        ffmpeg,
        yt_dlp,
        streamlink,
        ready: ffmpeg && (yt_dlp || streamlink),
    }
}

fn deps_status() -> DepsStatus {
    deps_status_with(
        Path::new("ffmpeg"),
        Path::new("yt-dlp"),
        Path::new("streamlink"),
    )
}

pub async fn get_deps_status() -> Json<DepsStatus> {
    // 外部进程探测是阻塞 I/O：绝不占用 tokio worker 线程。
    Json(tokio::task::spawn_blocking(deps_status).await.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    fn on_path(name: &str) -> bool {
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(name).is_file())
        })
    }

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
            assert!(
                probe(&runnable, TOOL_VERSION_ARGS),
                "以 0 退出的脚本必须判为可用"
            );
        }
        assert!(!probe(&dir.path().join("missing-tool"), TOOL_VERSION_ARGS));

        let status = deps_status();
        let wire = serde_json::to_value(&status).unwrap();
        let keys: Vec<&str> = wire
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["ffmpeg", "ready", "streamlink", "yt_dlp"]);
        assert_eq!(
            status.ready,
            status.ffmpeg && (status.yt_dlp || status.streamlink)
        );
    }

    /// ffmpeg 只认单横线版本开关：`--version` 会以 8 退出，用错就永远不可用。
    #[cfg(unix)]
    #[test]
    fn ffmpeg_is_probed_with_its_single_dash_version_flag() {
        let dir = tempfile::tempdir().unwrap();
        let ffmpeg = script(
            dir.path(),
            "ffmpeg-fake.sh",
            "#!/bin/sh\n[ \"$1\" = \"-version\" ] && exit 0\nexit 8\n",
        );
        let yt_dlp = script(
            dir.path(),
            "yt-dlp-fake.sh",
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && exit 0\nexit 2\n",
        );
        let streamlink = dir.path().join("no-streamlink");

        let status = deps_status_with(&ffmpeg, &yt_dlp, &streamlink);
        assert!(status.ffmpeg, "ffmpeg 必须以 -version 判为可用");
        assert!(status.yt_dlp, "yt-dlp 必须以 --version 判为可用");
        assert!(!status.streamlink, "不存在的命令必须判为不可用");
        assert!(status.ready, "ffmpeg 与 yt-dlp 都可用时 ready 必须为 true");
    }

    /// 本机装了真实 ffmpeg 时再核对一次真实退出码；没装就跳过（不失败、不 ignore）。
    #[cfg(unix)]
    #[test]
    fn a_real_ffmpeg_on_path_answers_its_single_dash_version_flag() {
        if !on_path("ffmpeg") {
            return;
        }
        assert!(
            probe_with(Path::new("ffmpeg"), FFMPEG_VERSION_ARGS, PROBE_TIMEOUT),
            "真实 ffmpeg 必须能以 -version 探测成功"
        );
    }

    /// 慢命令必须在期限附近返回 false，且子进程被回收（不是僵尸）。
    #[cfg(unix)]
    #[test]
    fn a_slow_probe_times_out_without_leaving_a_zombie() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let slow = script(
            dir.path(),
            "probe-slow.sh",
            &format!(
                "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
                pid_file.display()
            ),
        );

        let timeout = Duration::from_millis(300);
        let started = Instant::now();
        assert!(
            !probe_with(&slow, TOOL_VERSION_ARGS, timeout),
            "慢命令必须判为不可用"
        );
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
            assert!(
                state.is_none_or(|rest| !rest.trim_start().starts_with('S')),
                "超时后不得留下仍在运行的 `exec sleep` 子进程：{stat:?}"
            );
        }
        #[cfg(not(target_os = "linux"))]
        let _ = pid;
    }
}
