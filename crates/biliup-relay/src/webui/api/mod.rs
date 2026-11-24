//! 第一批业务 API 处理器：状态、控制、切流、预告列表、历史与日志。
//!
//! 每个子模块用显式 `use` 声明依赖，不共享 `use super::*` 的隐式命名空间。

pub mod admin_stream_list;
pub mod history;
pub mod logs;
pub mod status;
pub mod stream_control;
pub mod stream_switch;

pub(crate) use status::current_network_status;

use std::collections::HashMap;

use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// 配置保存的失败边界：写锁被占用回报冲突，其余脱敏记错后回报 500。
pub(crate) fn config_save_status(error: Box<dyn std::error::Error>) -> StatusCode {
    if error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::WouldBlock)
    {
        StatusCode::CONFLICT
    } else {
        tracing::error!(
            "配置保存失败：{}",
            crate::logging::safe_diagnostic(&error.to_string())
        );
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

#[derive(Serialize)]
pub struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    message: Option<String>,
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

pub(crate) const EDIT_CONFLICT: &str = "配置已被其他操作修改，请重新加载后再保存";
pub(crate) const EDIT_INVALID: &str = "配置修改缺少有效的原始值";

/// 只校验浏览器真正编辑过的键：保存本身在写锁里再检测一次竞争。
pub(crate) fn validate_edit_preconditions(
    current: &serde_json::Value,
    patch: &serde_json::Value,
    expected: Option<&HashMap<String, serde_json::Value>>,
) -> Result<(), &'static str> {
    let Some(expected) = expected else {
        return Ok(());
    };
    for (key, value) in patch.as_object().ok_or(EDIT_INVALID)? {
        if value.is_null() {
            continue;
        }
        let before = expected.get(key).ok_or(EDIT_INVALID)?;
        let now = current.get(key).ok_or(EDIT_INVALID)?;
        if before != now {
            return Err(EDIT_CONFLICT);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::status::apply_effective_local_monitor_state;
    use super::stream_control::{twitch_monitor_toggle_matches, youtube_monitor_toggle_matches};
    use super::{validate_edit_preconditions, EDIT_CONFLICT, EDIT_INVALID};
    use crate::webui::state::{
        BiliStatus, NicoStatus, PriorityChannelStatus, StatusData, TwStatus, YtStatus,
    };
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn browser_edits_merge_unrelated_changes_but_reject_stale_fields() {
        let expected = HashMap::from([("interval".to_string(), json!(30))]);
        let patch = json!({"interval": 75, "auto_cover": null});
        assert!(
            validate_edit_preconditions(
                &json!({"interval": 30, "auto_cover": true}),
                &patch,
                Some(&expected)
            )
            .is_ok()
        );
        assert_eq!(
            validate_edit_preconditions(
                &json!({"interval": 45, "auto_cover": true}),
                &patch,
                Some(&expected)
            ),
            Err(EDIT_CONFLICT)
        );
        assert_eq!(
            validate_edit_preconditions(
                &json!({"interval": 30, "auto_cover": true}),
                &json!({"interval": 75, "auto_cover": false}),
                Some(&expected)
            ),
            Err(EDIT_INVALID)
        );
        assert!(validate_edit_preconditions(&json!({}), &patch, None).is_ok());
    }

    #[test]
    fn keyword_preconditions_compare_array_contents() {
        let expected = HashMap::from([("streaming_banned_keywords".to_string(), json!(["old"]))]);
        let patch = json!({"streaming_banned_keywords": []});
        assert_eq!(
            validate_edit_preconditions(
                &json!({"streaming_banned_keywords": ["new"]}),
                &patch,
                Some(&expected)
            ),
            Err(EDIT_CONFLICT)
        );
        assert!(
            validate_edit_preconditions(
                &json!({"streaming_banned_keywords": ["old"]}),
                &patch,
                Some(&expected)
            )
            .is_ok()
        );
    }

    #[test]
    fn active_owner_status_preserves_configured_monitor_toggles() {
        let expected = enabled_status_toggles();
        let mut status = expected.clone();

        apply_effective_local_monitor_state(&mut status, true);

        assert_eq!(
            status.bilibili.enable_danmaku_command,
            expected.bilibili.enable_danmaku_command
        );
        assert_eq!(
            status.youtube.as_ref().unwrap().enable_monitor,
            expected.youtube.as_ref().unwrap().enable_monitor
        );
        assert_eq!(
            status.twitch.as_ref().unwrap().enable_monitor,
            expected.twitch.as_ref().unwrap().enable_monitor
        );
        assert_eq!(
            status.niconico.as_ref().unwrap().enable_monitor,
            expected.niconico.as_ref().unwrap().enable_monitor
        );
        assert_eq!(
            status.priority_channel.as_ref().unwrap().enabled,
            expected.priority_channel.as_ref().unwrap().enabled
        );
        assert_eq!(
            status.priority_channel.as_ref().unwrap().auto_restart,
            expected.priority_channel.as_ref().unwrap().auto_restart
        );
    }

    #[test]
    fn standby_status_hides_every_runtime_monitor_toggle() {
        let mut status = enabled_status_toggles();

        apply_effective_local_monitor_state(&mut status, false);

        assert!(!status.bilibili.enable_danmaku_command);
        assert!(!status.youtube.unwrap().enable_monitor);
        assert!(!status.twitch.unwrap().enable_monitor);
        assert!(!status.niconico.unwrap().enable_monitor);
        let priority = status.priority_channel.unwrap();
        assert!(!priority.enabled);
        assert!(!priority.auto_restart);
    }

    fn enabled_status_toggles() -> StatusData {
        StatusData {
            bilibili: BiliStatus {
                enable_danmaku_command: true,
                ..BiliStatus::default()
            },
            youtube: Some(YtStatus {
                is_live: false,
                enable_monitor: true,
                warning_held: false,
                title: None,
                topic: None,
                channel_name: "youtube".to_string(),
                channel_id: "youtube-id".to_string(),
                quality: "best".to_string(),
                area_id: 1,
                area_name: "area".to_string(),
                crop_enabled: false,
                ffmpeg_cache_enabled: false,
                ffmpeg_cache_latency_secs: 0,
            }),
            twitch: Some(TwStatus {
                is_live: false,
                enable_monitor: true,
                warning_held: false,
                title: None,
                game: None,
                channel_name: "twitch".to_string(),
                channel_id: "twitch-id".to_string(),
                quality: "best".to_string(),
                area_id: 1,
                area_name: "area".to_string(),
                crop_enabled: false,
                ffmpeg_cache_enabled: false,
                ffmpeg_cache_latency_secs: 0,
            }),
            niconico: Some(NicoStatus {
                is_live: false,
                enable_monitor: true,
                warning_held: false,
                title: None,
                channel_name: "niconico".to_string(),
                channel_id: "vspo".to_string(),
                live_id: None,
                scheduled_start: None,
                quality: "best".to_string(),
                area_id: 1,
                area_name: "area".to_string(),
                crop_enabled: false,
                ffmpeg_cache_enabled: false,
                ffmpeg_cache_latency_secs: 0,
            }),
            priority_channel: Some(PriorityChannelStatus {
                default_area_name: Some("测试分区".into()),
                enabled: true,
                auto_restart: true,
                warning_held: false,
                channel_name: "priority".to_string(),
                is_live: false,
                platform: None,
                title: None,
                default_area: 1,
            }),
        }
    }

    #[test]
    fn monitor_toggle_match_requires_top_level_and_platform_flags() {
        let mut cfg = monitor_toggle_test_config();
        cfg.youtube.enable_monitor = true;
        cfg.enable_youtube_monitor = false;
        cfg.twitch.enable_monitor = false;
        cfg.enable_twitch_monitor = true;

        assert!(!youtube_monitor_toggle_matches(&cfg, true));
        assert!(!twitch_monitor_toggle_matches(&cfg, false));

        cfg.enable_youtube_monitor = true;
        cfg.enable_twitch_monitor = false;

        assert!(youtube_monitor_toggle_matches(&cfg, true));
        assert!(twitch_monitor_toggle_matches(&cfg, false));
    }

    fn monitor_toggle_test_config() -> crate::config::Config {
        let mut cfg = crate::test_support::config("status-cache", 0);
        cfg.cluster = crate::config::ClusterConfig::default();
        cfg.youtube.enable_monitor = false;
        cfg.twitch.enable_monitor = false;
        cfg.youtube.channel_name = "yt-channel".to_string();
        cfg.twitch.channel_name = "tw-channel".to_string();
        cfg.enable_youtube_monitor = false;
        cfg.enable_twitch_monitor = false;
        cfg
    }
}
