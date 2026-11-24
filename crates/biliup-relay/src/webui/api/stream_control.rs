//! 直播与弹幕控制端点：开播、停播、重启、临时跳过解除与弹幕/封面/分区写入。

use std::collections::HashMap;
use std::path::Path;

use axum::extract::Json;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::status::{refresh_niconico_status, refresh_twitch_status, refresh_youtube_status};
use super::{config_save_status, validate_edit_preconditions, ApiResponse, EDIT_CONFLICT, EDIT_INVALID};
use crate::config::{load_config, Config};
use crate::plugins::{
    bili_change_live_title, bili_start_live, bili_stop_live, bili_update_area, bilibili,
    normalize_channel_id, normalize_live_id, send_danmaku as send_danmaku_to_bili,
    set_config_updated,
};
use crate::webui::state::refresh_status_cache_config_from;

#[derive(Deserialize)]
pub struct StartStreamRequest {
    platform: Option<String>,
}

pub async fn start_stream(
    Json(payload): Json<StartStreamRequest>,
) -> Result<ApiResponse<serde_json::Value>, StatusCode> {
    let mut cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let area_v2 = match payload.platform.as_deref() {
        Some("YT") => cfg.youtube.area_v2,
        Some("TW") => cfg.twitch.area_v2,
        Some("NC") => cfg.niconico.area_v2,
        _ => 235,
    };

    match bili_start_live(&mut cfg, area_v2).await {
        Ok(_) => Ok(ApiResponse {
            success: true,
            data: Some(json!({})),
            message: Some("直播已开始".to_string()),
        }),
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.starts_with("FACE_AUTH_REQUIRED:") {
                let qr_url = error_msg.strip_prefix("FACE_AUTH_REQUIRED:").unwrap_or("");
                Ok(ApiResponse {
                    success: false,
                    data: Some(json!({
                        "requires_face_auth": true,
                        "qr_url": qr_url,
                    })),
                    message: Some("需要人脸验证，请在客户端完成验证后重试".to_string()),
                })
            } else {
                Ok(ApiResponse {
                    success: false,
                    data: None,
                    message: Some(error_msg),
                })
            }
        }
    }
}

pub async fn stop_stream() -> Result<ApiResponse<()>, StatusCode> {
    let cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    bili_stop_live(&cfg)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 直播间已下播：会话的下播原因在这里记，因为这是人的主动动作。
    crate::notify::live_ended(crate::events::BroadcastEndCause::ManualStop);

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some("直播已停止".to_string()),
    })
}

pub async fn restart_stream() -> Result<ApiResponse<()>, StatusCode> {
    crate::plugins::danmaku::clear_warning_stop();

    // 先发布重启原因再停 ffmpeg，主循环才能看到完整状态。
    crate::plugins::set_manual_restart();
    set_config_updated();
    crate::cluster::clear_local_stream();

    crate::plugins::stop_ffmpeg().await;

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some("已停止当前流并重新加载配置".to_string()),
    })
}

#[derive(Deserialize)]
pub struct ReleasePriorityWarningRequest {
    channel_name: String,
    #[serde(default)]
    platform: Option<String>,
}

pub async fn release_priority_warning(
    Json(payload): Json<ReleasePriorityWarningRequest>,
) -> Result<ApiResponse<serde_json::Value>, StatusCode> {
    use crate::plugins::danmaku::warning::warning_tracker;
    let warnings = warning_tracker();
    let before_action = warnings.snapshot();
    let cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let platform = payload.platform.as_deref();
    let priority_card = is_priority_warning_card(platform);
    let release = crate::config::with_current_config(&cfg, || {
        release_card_warning_for_config(
            &warnings,
            &before_action,
            &cfg,
            &payload.channel_name,
            platform,
        )
    })
    .ok_or(StatusCode::CONFLICT)?;
    let release = match release {
        Ok(release) => release,
        Err(message) => {
            return Ok(ApiResponse {
                success: false,
                data: None,
                message: Some(message.to_owned()),
            });
        }
    };
    let released = release.is_some();
    if let Some(release) = release {
        release.log();
        // 唤醒空闲选源：这不是停/重启请求，自动重启关闭时当前源继续跑。
        set_config_updated();
    }
    refresh_status_cache_config_from(&cfg);
    Ok(ApiResponse {
        success: true,
        data: Some(json!({"released": released})),
        message: Some(
            if released {
                if priority_card {
                    "优先频道的临时跳过已解除"
                } else {
                    "临时跳过已解除"
                }
            } else if priority_card {
                "优先频道当前没有临时跳过"
            } else {
                "当前没有临时跳过"
            }
            .to_owned(),
        ),
    })
}

fn is_priority_warning_card(platform: Option<&str>) -> bool {
    matches!(
        platform
            .map(str::trim)
            .filter(|platform| !platform.is_empty()),
        None | Some("priority")
    )
}

#[cfg(test)]
fn release_priority_warning_for_config(
    warnings: &crate::plugins::danmaku::warning::WarningTracker,
    before_action: &crate::plugins::danmaku::warning::WarningSnapshot,
    cfg: &Config,
    requested_name: &str,
) -> Result<Option<crate::plugins::danmaku::warning::WarningRelease>, &'static str> {
    release_card_warning_for_config(warnings, before_action, cfg, requested_name, None)
}

fn release_card_warning_for_config(
    warnings: &crate::plugins::danmaku::warning::WarningTracker,
    before_action: &crate::plugins::danmaku::warning::WarningSnapshot,
    cfg: &Config,
    requested_name: &str,
    platform: Option<&str>,
) -> Result<Option<crate::plugins::danmaku::warning::WarningRelease>, &'static str> {
    use crate::plugins::danmaku::warning::{BroadcastSource, WarningReleaseReason};
    let reason = WarningReleaseReason::AdminCard;
    if is_priority_warning_card(platform) {
        if requested_name.is_empty() || requested_name != cfg.priority_channel.channel_name {
            return Err("优先频道已变更，请刷新后重试");
        }
        let priority = &cfg.priority_channel;
        let priority_source = |platform, channel_id: &str| {
            BroadcastSource::new(platform, requested_name, channel_id, None)
        };
        return Ok(warnings
            .release_name(before_action, requested_name, reason)
            .or_else(|| {
                warnings.release_source(
                    before_action,
                    &priority_source("YT", &priority.youtube_channel_id),
                    reason,
                )
            })
            .or_else(|| {
                warnings.release_source(
                    before_action,
                    &priority_source("TW", &priority.twitch_channel_id),
                    reason,
                )
            }));
    }
    // 源卡片只解除它自己渲染的那个身份：release_source 也匹配显示名，
    // 因此 Twitch 的跳过不能被同一艺人的 YouTube 卡片解除。
    let code = platform.map(str::trim).unwrap_or_default();
    let source = crate::plugins::danmaku::configured_broadcast_source(cfg, code)
        .ok_or("频道已变更，请刷新后重试")?;
    if requested_name.is_empty() || requested_name != source.channel_name {
        return Err("频道已变更，请刷新后重试");
    }
    if !before_action.holds_identity(&source) {
        return Ok(None);
    }
    Ok(warnings.release_source(before_action, &source, reason))
}

#[derive(Deserialize)]
pub struct SendDanmakuRequest {
    message: String,
}

pub async fn send_danmaku(
    Json(payload): Json<SendDanmakuRequest>,
) -> Result<ApiResponse<()>, (StatusCode, String)> {
    let cfg = load_config()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match send_danmaku_to_bili(&cfg, &payload.message).await {
        Ok(_) => Ok(ApiResponse {
            success: true,
            data: None,
            message: Some("弹幕已发送".to_string()),
        }),
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("频率过快") {
                Err((
                    StatusCode::TOO_MANY_REQUESTS,
                    "发送频率过快，请稍后再试".to_string(),
                ))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, error_msg))
            }
        }
    }
}

#[derive(Deserialize)]
pub struct UpdateCoverRequest {
    // 历史字段名；只接受应用缓存里的文件名。
    image_path: String,
}

async fn read_cached_cover(path: &Path) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    #[cfg(windows)]
    options.custom_flags(0x0020_0000);
    let file = options.open(path).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::other("封面必须是普通图片文件"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(std::io::Error::other("封面不能是链接"));
        }
    }
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).await?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err(std::io::Error::other("封面为空或过大"));
    }
    Ok(bytes)
}

pub async fn update_cover(
    Json(payload): Json<UpdateCoverRequest>,
) -> Result<ApiResponse<()>, StatusCode> {
    // 先解析名字再读凭据、连 B 站：路径穿越在校验处就断掉。
    if !matches!(
        payload.image_path.as_str(),
        "cover.jpg" | "pic_for_crop.jpg"
    ) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = crate::storage::cache_file(&payload.image_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let bytes = read_cached_cover(&path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        }
    })?;
    let cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    bilibili::bili_change_cover_bytes(&cfg, bytes)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some("封面已更新".to_string()),
    })
}

#[derive(Deserialize)]
pub struct UpdateAreaRequest {
    area_id: u64,
}

pub async fn update_area(
    Json(payload): Json<UpdateAreaRequest>,
) -> Result<ApiResponse<()>, StatusCode> {
    let cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    bili_update_area(&cfg, payload.area_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some("分区已更新".to_string()),
    })
}

#[derive(Deserialize)]
pub struct UpdateTitleRequest {
    title: String,
}

pub async fn update_title(
    Json(payload): Json<UpdateTitleRequest>,
) -> Result<ApiResponse<()>, StatusCode> {
    let cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    bili_change_live_title(&cfg, &payload.title)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some("直播标题已更新".to_string()),
    })
}

#[derive(Deserialize)]
pub struct UpdateChannelRequest {
    platform: String,
    channel_id: Option<String>,
    channel_name: Option<String>,
    live_id: Option<String>,
    area_id: Option<u64>,
    quality: Option<String>,
    riot_api_key: Option<String>,
    cookies_file: Option<String>,
    cookies_from_browser: Option<String>,
}

pub async fn update_channel(
    Json(payload): Json<UpdateChannelRequest>,
) -> Result<ApiResponse<()>, StatusCode> {
    let mut cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let previous_cfg = cfg.clone();
    let old_monitored_config_version = crate::cluster::monitored_config_version(&cfg);

    match payload.platform.as_str() {
        "youtube" => {
            if let Some(channel_id) = payload.channel_id {
                cfg.youtube.channel_id = channel_id;
            }
            if let Some(channel_name) = payload.channel_name {
                cfg.youtube.channel_name = channel_name;
            }
            if let Some(area_id) = payload.area_id {
                cfg.youtube.area_v2 = area_id;
                if area_id == 86 {
                    if let Some(riot_api_key) = payload.riot_api_key {
                        if !riot_api_key.is_empty() {
                            cfg.riot_api_key = Some(riot_api_key);
                        }
                    }
                }
            }
            if let Some(quality) = payload.quality {
                cfg.youtube.quality = quality;
            }
            if let Some(cookies_file) = payload.cookies_file {
                cfg.youtube.cookies_file = if cookies_file.is_empty() {
                    None
                } else {
                    Some(cookies_file)
                };
            }
            if let Some(cookies_from_browser) = payload.cookies_from_browser {
                cfg.youtube.cookies_from_browser = if cookies_from_browser.is_empty() {
                    None
                } else {
                    Some(cookies_from_browser)
                };
            }
        }
        "twitch" => {
            if let Some(channel_id) = payload.channel_id {
                cfg.twitch.channel_id = channel_id;
            }
            if let Some(channel_name) = payload.channel_name {
                cfg.twitch.channel_name = channel_name;
            }
            if let Some(area_id) = payload.area_id {
                cfg.twitch.area_v2 = area_id;
                if area_id == 86 {
                    if let Some(riot_api_key) = payload.riot_api_key {
                        if !riot_api_key.is_empty() {
                            cfg.riot_api_key = Some(riot_api_key);
                        }
                    }
                }
            }
            if let Some(quality) = payload.quality {
                cfg.twitch.quality = quality;
            }
        }
        "niconico" => {
            if let Some(channel_id) = payload.channel_id {
                cfg.niconico.channel_id = normalize_channel_id(&channel_id);
            }
            if let Some(channel_name) = payload.channel_name {
                cfg.niconico.channel_name = channel_name;
            }
            if let Some(live_id) = payload.live_id {
                cfg.niconico.live_id = normalize_live_id(&live_id);
            }
            if let Some(area_id) = payload.area_id {
                cfg.niconico.area_v2 = area_id;
                if area_id == 86 {
                    if let Some(riot_api_key) = payload.riot_api_key {
                        if !riot_api_key.is_empty() {
                            cfg.riot_api_key = Some(riot_api_key);
                        }
                    }
                }
            }
            if let Some(quality) = payload.quality {
                cfg.niconico.quality = quality;
            }
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    }

    crate::config::save_config(&mut cfg)
        .await
        .map_err(config_save_status)?;

    let refresh_youtube = youtube_monitor_reload_needed(&previous_cfg, &cfg);
    let refresh_twitch = twitch_monitor_reload_needed(&previous_cfg, &cfg);
    let refresh_niconico = niconico_monitor_reload_needed(&previous_cfg, &cfg);

    if refresh_youtube || refresh_twitch || refresh_niconico {
        set_config_updated();
    }

    refresh_status_cache_config_from(&cfg);

    // 只有活动监听目标变了才在后台重取直播状态。
    if refresh_youtube || refresh_twitch || refresh_niconico {
        tokio::spawn(async move {
            if refresh_youtube {
                let _ = refresh_youtube_status().await;
            }
            if refresh_twitch {
                let _ = refresh_twitch_status().await;
            }
            if refresh_niconico {
                let _ = refresh_niconico_status().await;
            }
        });
    }

    let sync_message = if old_monitored_config_version != crate::cluster::monitored_config_version(&cfg)
    {
        crate::cluster::sync_monitored_config_after_change(&cfg).await
    } else {
        String::new()
    };

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some(format!("{} 频道已更新{}", payload.platform, sync_message)),
    })
}

pub async fn get_channels() -> Result<Json<serde_json::Value>, StatusCode> {
    crate::storage::read_document("channels.json")
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn get_areas() -> Result<Json<serde_json::Value>, StatusCode> {
    crate::storage::read_document("areas.json")
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(Serialize)]
pub struct BannedKeywordsResponse {
    danmaku_banned_keywords: Vec<String>,
    streaming_banned_keywords: Vec<String>,
}

pub async fn get_banned_keywords() -> Result<Json<BannedKeywordsResponse>, StatusCode> {
    let data: serde_json::Value = crate::storage::read_document("areas.json")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let danmaku_banned = data["banned_keywords"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let streaming_banned = data["streaming_banned_keywords"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(BannedKeywordsResponse {
        danmaku_banned_keywords: danmaku_banned,
        streaming_banned_keywords: streaming_banned,
    }))
}

#[derive(Deserialize, Serialize)]
pub struct UpdateBannedKeywordsRequest {
    #[serde(skip_serializing)]
    expected: Option<HashMap<String, serde_json::Value>>,
    danmaku_banned_keywords: Option<Vec<String>>,
    streaming_banned_keywords: Option<Vec<String>>,
}

pub async fn update_banned_keywords(
    Json(payload): Json<UpdateBannedKeywordsRequest>,
) -> Result<ApiResponse<()>, StatusCode> {
    let patch = serde_json::to_value(&payload).map_err(|_| StatusCode::BAD_REQUEST)?;
    let result = crate::config::mutate_document("areas.json", move |data: &mut serde_json::Value| {
        let current = json!({
            "danmaku_banned_keywords": data.get("banned_keywords").cloned().unwrap_or_else(|| json!([])),
            "streaming_banned_keywords": data.get("streaming_banned_keywords").cloned().unwrap_or_else(|| json!([])),
        });
        validate_edit_preconditions(&current, &patch, payload.expected.as_ref())
            .map_err(str::to_owned)?;
        if let Some(keywords) = payload.danmaku_banned_keywords {
            data["banned_keywords"] = serde_json::json!(keywords);
        }
        if let Some(keywords) = payload.streaming_banned_keywords {
            data["streaming_banned_keywords"] = serde_json::json!(keywords);
        }
        Ok(())
    })
    .await;
    if let Err(error) = result {
        return Err(if error == EDIT_CONFLICT {
            StatusCode::CONFLICT
        } else if error == EDIT_INVALID {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        });
    }
    set_config_updated();

    crate::webui::public::remap_after_areas_change();
    // Phase 3 集群文档同步迁入后替换：单机没有对端需要同步。
    let sync_message = String::new();

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some(format!("禁用关键词已更新{sync_message}")),
    })
}

#[derive(Deserialize)]
pub struct ToggleMonitorRequest {
    enabled: bool,
}

pub(crate) fn youtube_monitor_toggle_matches(cfg: &Config, enabled: bool) -> bool {
    cfg.youtube.enable_monitor == enabled && cfg.enable_youtube_monitor == enabled
}

pub(crate) fn twitch_monitor_toggle_matches(cfg: &Config, enabled: bool) -> bool {
    cfg.twitch.enable_monitor == enabled && cfg.enable_twitch_monitor == enabled
}

pub async fn toggle_youtube_monitor(
    Json(payload): Json<ToggleMonitorRequest>,
) -> Result<ApiResponse<()>, StatusCode> {
    let mut cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if payload.enabled && !crate::cluster::local_node_can_enable_monitor_toggles(&cfg) {
        return Ok(monitor_toggle_enable_rejected_response());
    }

    if youtube_monitor_toggle_matches(&cfg, payload.enabled) {
        refresh_status_cache_config_from(&cfg);
        return Ok(ApiResponse {
            success: true,
            data: None,
            message: Some(format!(
                "YouTube监控已是{}",
                if payload.enabled { "启用" } else { "禁用" }
            )),
        });
    }

    cfg.youtube.enable_monitor = payload.enabled;
    cfg.enable_youtube_monitor = payload.enabled;

    crate::config::save_config(&mut cfg)
        .await
        .map_err(config_save_status)?;

    set_config_updated();
    refresh_status_cache_config_from(&cfg);
    crate::webui::state::request_status_refresh();
    let toggle_sync_message =
        crate::cluster::schedule_active_monitor_state_sync_after_toggle_change(&cfg);

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some(format!(
            "YouTube监控已{}{}",
            if payload.enabled { "启用" } else { "禁用" },
            toggle_sync_message
        )),
    })
}

pub async fn toggle_twitch_monitor(
    Json(payload): Json<ToggleMonitorRequest>,
) -> Result<ApiResponse<()>, StatusCode> {
    let mut cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if payload.enabled && !crate::cluster::local_node_can_enable_monitor_toggles(&cfg) {
        return Ok(monitor_toggle_enable_rejected_response());
    }

    if twitch_monitor_toggle_matches(&cfg, payload.enabled) {
        refresh_status_cache_config_from(&cfg);
        return Ok(ApiResponse {
            success: true,
            data: None,
            message: Some(format!(
                "Twitch监控已是{}",
                if payload.enabled { "启用" } else { "禁用" }
            )),
        });
    }

    cfg.twitch.enable_monitor = payload.enabled;
    cfg.enable_twitch_monitor = payload.enabled;

    crate::config::save_config(&mut cfg)
        .await
        .map_err(config_save_status)?;

    set_config_updated();
    refresh_status_cache_config_from(&cfg);
    crate::webui::state::request_status_refresh();
    let toggle_sync_message =
        crate::cluster::schedule_active_monitor_state_sync_after_toggle_change(&cfg);

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some(format!(
            "Twitch监控已{}{}",
            if payload.enabled { "启用" } else { "禁用" },
            toggle_sync_message
        )),
    })
}

pub(crate) fn niconico_monitor_toggle_matches(cfg: &Config, enabled: bool) -> bool {
    cfg.niconico.enable_monitor == enabled
}

pub async fn toggle_niconico_monitor(
    Json(payload): Json<ToggleMonitorRequest>,
) -> Result<ApiResponse<()>, StatusCode> {
    let mut cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if payload.enabled && !crate::cluster::local_node_can_enable_monitor_toggles(&cfg) {
        return Ok(monitor_toggle_enable_rejected_response());
    }

    if niconico_monitor_toggle_matches(&cfg, payload.enabled) {
        refresh_status_cache_config_from(&cfg);
        return Ok(ApiResponse {
            success: true,
            data: None,
            message: Some(format!(
                "Niconico监控已是{}",
                if payload.enabled { "启用" } else { "禁用" }
            )),
        });
    }

    cfg.niconico.enable_monitor = payload.enabled;

    crate::config::save_config(&mut cfg)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    set_config_updated();
    refresh_status_cache_config_from(&cfg);
    crate::webui::state::request_status_refresh();
    let toggle_sync_message =
        crate::cluster::schedule_active_monitor_state_sync_after_toggle_change(&cfg);

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some(format!(
            "Niconico监控已{}{}",
            if payload.enabled { "启用" } else { "禁用" },
            toggle_sync_message
        )),
    })
}

pub(super) fn monitor_toggle_enable_rejected_response() -> ApiResponse<()> {
    ApiResponse {
        success: false,
        data: None,
        message: Some("只有活跃节点可启用监控开关".to_string()),
    }
}

pub(crate) fn monitor_target_reload_needed(
    previous_enabled: bool,
    current_enabled: bool,
    previous_channel_name: &str,
    current_channel_name: &str,
    previous_channel_id: &str,
    current_channel_id: &str,
) -> bool {
    previous_enabled != current_enabled
        || (current_enabled
            && (previous_channel_name != current_channel_name
                || previous_channel_id != current_channel_id))
}

pub(crate) fn youtube_monitor_reload_needed(previous: &Config, current: &Config) -> bool {
    monitor_target_reload_needed(
        previous.youtube.enable_monitor,
        current.youtube.enable_monitor,
        &previous.youtube.channel_name,
        &current.youtube.channel_name,
        &previous.youtube.channel_id,
        &current.youtube.channel_id,
    )
}

pub(crate) fn twitch_monitor_reload_needed(previous: &Config, current: &Config) -> bool {
    monitor_target_reload_needed(
        previous.twitch.enable_monitor,
        current.twitch.enable_monitor,
        &previous.twitch.channel_name,
        &current.twitch.channel_name,
        &previous.twitch.channel_id,
        &current.twitch.channel_id,
    )
}

pub(crate) fn niconico_monitor_reload_needed(previous: &Config, current: &Config) -> bool {
    previous.niconico.enable_monitor != current.niconico.enable_monitor
        || (current.niconico.enable_monitor
            && (previous.niconico.channel_name != current.niconico.channel_name
                || previous.niconico.channel_id != current.niconico.channel_id
                || previous.niconico.live_id != current.niconico.live_id))
}

#[cfg(test)]
mod priority_warning_tests {
    use super::*;
    use crate::plugins::danmaku::warning::{BroadcastSource, WarningTracker};
    use std::sync::Arc;

    fn priority_config(auto_restart: bool) -> Config {
        let mut cfg = crate::test_support::config("priority-admin-test", 0);
        cfg.priority_channel.enabled = true;
        cfg.priority_channel.auto_restart = auto_restart;
        cfg.priority_channel.channel_name = "示例频道 A".into();
        cfg.priority_channel.youtube_channel_id = "UC-held".into();
        cfg.priority_channel.twitch_channel_id = "held_login".into();
        cfg
    }

    #[test]
    fn priority_release_preserves_current_source_and_saved_targets_in_both_restart_modes() {
        for auto_restart in [false, true] {
            let cfg = priority_config(auto_restart);
            let saved = serde_json::to_value(&cfg).unwrap();
            let warnings = Arc::new(WarningTracker::default());
            let held = BroadcastSource::new("YT", "示例频道 A", "UC-held", Some("episode-a"));
            let _held_owner = warnings.own_source(71, held.clone());
            let mut event = warnings.capture_event();
            warnings.mark_event(&mut event, None).unwrap();
            let running =
                BroadcastSource::new("TW", "示例频道 B", "other_login", Some("episode-b"));
            let _running_owner = warnings.own_source(72, running.clone());

            assert!(
                release_priority_warning_for_config(
                    &warnings,
                    &warnings.snapshot(),
                    &cfg,
                    &cfg.priority_channel.channel_name,
                )
                .unwrap()
                .is_some()
            );
            assert!(!warnings.snapshot().holds(&held));
            assert_eq!(warnings.active_source(), Some(running));
            assert_eq!(serde_json::to_value(&cfg).unwrap(), saved);
            assert!(
                release_priority_warning_for_config(
                    &warnings,
                    &warnings.snapshot(),
                    &cfg,
                    &cfg.priority_channel.channel_name,
                )
                .unwrap()
                .is_none(),
                "重复解除不该产生副作用"
            );
        }
    }

    #[test]
    fn priority_release_rejects_stale_card_names_and_cannot_clear_a_newer_warning() {
        let mut cfg = priority_config(false);
        let warnings = Arc::new(WarningTracker::default());
        let held = BroadcastSource::new("YT", "示例频道 A", "UC-held", Some("episode-a"));
        let _owner = warnings.own_source(81, held.clone());
        let mut event = warnings.capture_event();
        warnings.mark_event(&mut event, None).unwrap();
        let old_click = warnings.snapshot();

        for requested in ["", "已经更换的频道"] {
            assert!(
                release_priority_warning_for_config(&warnings, &old_click, &cfg, requested)
                    .is_err()
            );
            assert!(warnings.snapshot().holds(&held));
        }
        cfg.priority_channel.channel_name = "示例频道 B".into();
        cfg.priority_channel.youtube_channel_id = "UC-other".into();
        cfg.priority_channel.twitch_channel_id = "other_login".into();
        assert!(
            release_priority_warning_for_config(&warnings, &old_click, &cfg, "示例频道 A",)
                .is_err()
        );
        assert!(
            release_priority_warning_for_config(&warnings, &old_click, &cfg, "示例频道 B",)
                .unwrap()
                .is_none(),
            "当前卡片不能解除无关的跳过"
        );
        assert!(warnings.snapshot().holds(&held));

        let cfg = priority_config(false);
        let mut newer = warnings.capture_event();
        warnings.mark_event(&mut newer, None).unwrap();
        assert!(
            release_priority_warning_for_config(&warnings, &old_click, &cfg, "示例频道 A",)
                .unwrap()
                .is_none()
        );
        assert!(warnings.snapshot().holds(&held));
        assert!(
            release_priority_warning_for_config(
                &warnings,
                &warnings.snapshot(),
                &cfg,
                "示例频道 A",
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn priority_release_matches_a_changed_alias_by_canonical_provider_id() {
        for (platform, id) in [("YT", "UC-held"), ("TW", "HELD_LOGIN")] {
            let cfg = priority_config(false);
            let warnings = Arc::new(WarningTracker::default());
            let held = BroadcastSource::new(platform, "频道旧显示名", id, Some("episode-a"));
            let _owner = warnings.own_source(91, held.clone());
            let mut event = warnings.capture_event();
            warnings.mark_event(&mut event, None).unwrap();
            assert!(
                release_priority_warning_for_config(
                    &warnings,
                    &warnings.snapshot(),
                    &cfg,
                    "示例频道 A",
                )
                .unwrap()
                .is_some()
            );
            assert!(!warnings.snapshot().holds(&held));
            assert_eq!(warnings.active_source(), Some(held));
        }
    }

    #[test]
    fn ordinary_twitch_card_releases_that_live_id_without_lighting_youtube() {
        let mut cfg = priority_config(false);
        cfg.youtube.channel_name = "示例频道 A".into();
        cfg.youtube.channel_id = "UC-other".into();
        cfg.twitch.channel_name = "示例频道 A".into();
        cfg.twitch.channel_id = "held_login".into();
        let warnings = Arc::new(WarningTracker::default());
        let held = BroadcastSource::new("TW", "示例频道 A", "held_login", Some("tw-stream-1"));
        let _owner = warnings.own_source(93, held.clone());
        let mut event = warnings.capture_event();
        warnings.mark_event(&mut event, None).unwrap();
        assert!(!warnings.snapshot().holds_identity(&BroadcastSource::new(
            "YT",
            "示例频道 A",
            "UC-other",
            None,
        )));
        assert!(
            release_card_warning_for_config(
                &warnings,
                &warnings.snapshot(),
                &cfg,
                "示例频道 A",
                Some("YT"),
            )
            .unwrap()
            .is_none(),
            "YouTube 卡片不是这条跳过"
        );
        assert!(warnings.snapshot().holds(&held));
        assert!(
            release_card_warning_for_config(
                &warnings,
                &warnings.snapshot(),
                &cfg,
                "示例频道 A",
                Some("TW"),
            )
            .unwrap()
            .is_some()
        );
        assert!(!warnings.snapshot().holds(&held));
    }

    #[test]
    fn a_card_platform_resolves_to_the_source_its_config_names() {
        let mut cfg = priority_config(false);
        cfg.youtube.channel_name = "示例频道 A".into();
        cfg.youtube.channel_id = "UC-other".into();
        cfg.twitch.channel_name = "示例频道 A".into();
        cfg.twitch.channel_id = "Held_Login".into();
        let yt = crate::plugins::danmaku::configured_broadcast_source(&cfg, "YT").unwrap();
        let tw = crate::plugins::danmaku::configured_broadcast_source(&cfg, "TW").unwrap();
        assert_eq!(
            (yt.platform.as_str(), yt.channel_id.as_str()),
            ("YT", "UC-other")
        );
        // Twitch 的 id 大小写不敏感，卡片仍然匹配这条跳过。
        assert_eq!(tw.channel_id, "held_login");
        assert!(crate::plugins::danmaku::configured_broadcast_source(&cfg, "priority").is_none());
        assert!(crate::plugins::danmaku::configured_broadcast_source(&cfg, "").is_none());
    }
}

#[cfg(test)]
mod cover_access_tests {
    use super::*;

    #[tokio::test]
    async fn cover_api_rejects_paths_before_loading_configuration() {
        for image_path in [
            "/etc/passwd",
            "../../etc/passwd",
            "data/bilistream.db",
            "data/cache/cover.jpg",
            "C:\\Users\\user\\secret.txt",
            "cover.jpg/../cookies.json",
        ] {
            let result = update_cover(Json(UpdateCoverRequest {
                image_path: image_path.into(),
            }))
            .await;
            assert_eq!(result.err(), Some(StatusCode::BAD_REQUEST), "{image_path}");
        }
    }

    #[tokio::test]
    async fn cached_cover_reads_a_regular_file_and_refuses_a_link() {
        let root =
            std::env::temp_dir().join(format!("bilistream-cover-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("cover.jpg");
        std::fs::write(&image, b"synthetic-cover-bytes").unwrap();
        assert_eq!(
            read_cached_cover(&image).await.unwrap(),
            b"synthetic-cover-bytes"
        );
        assert!(read_cached_cover(&root).await.is_err());
        #[cfg(unix)]
        {
            let link = root.join("pic_for_crop.jpg");
            std::os::unix::fs::symlink(&image, &link).unwrap();
            assert!(read_cached_cover(&link).await.is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod monitor_target_tests {
    use super::monitor_target_reload_needed;

    #[test]
    fn monitor_target_reload_needed_only_for_enable_or_enabled_channel_change() {
        assert!(!monitor_target_reload_needed(
            true, true, "Channel", "Channel", "id", "id",
        ));
        assert!(monitor_target_reload_needed(
            true, false, "Channel", "Channel", "id", "id",
        ));
        assert!(monitor_target_reload_needed(
            false, true, "Channel", "Channel", "id", "id",
        ));
        assert!(monitor_target_reload_needed(
            true, true, "Channel", "Other", "id", "id",
        ));
        assert!(monitor_target_reload_needed(
            true, true, "Channel", "Channel", "id", "other-id",
        ));
        assert!(!monitor_target_reload_needed(
            false, false, "Channel", "Other", "id", "other-id",
        ));
    }
}
