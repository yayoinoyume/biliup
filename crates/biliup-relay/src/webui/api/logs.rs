//! 日志、版本与外部依赖探测：只读端点，绝不触发任何下载或自更新。

use std::path::Path;
use std::process::{Command, Stdio};

use axum::extract::Json;
use serde::Serialize;

use super::ApiResponse;
use crate::app_state::AppState;

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
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct DepsStatus {
    ffmpeg: bool,
    yt_dlp: bool,
    streamlink: bool,
    ready: bool,
}

/// 可执行且 `--version` 成功才算可用；缺失或不可执行一律为 false。
fn probe(program: &Path) -> bool {
    Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
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
    Json(deps_status())
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
}
