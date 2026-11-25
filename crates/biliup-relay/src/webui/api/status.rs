//! 面板状态端点与后台状态刷新 worker。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use super::ApiResponse;
use crate::compat::cluster;
use crate::config::{load_config, Config};
use crate::logging::safe_diagnostic;
use crate::plugins::bilibili::{get_bili_live_status, get_bili_room_lock};
use crate::plugins::niconico_session;
use crate::plugins::source_status::SourceStatus;
use crate::plugins::{
    area_name_or_unknown, get_area_name, get_ffmpeg_cache_speed, get_ffmpeg_network_stats,
    get_ffmpeg_speed, get_niconico_status, get_twitch_status, get_youtube_channel_status,
    is_ffmpeg_hls_cache_active, is_ffmpeg_running, niconico_channel_identity, niconico_configured,
    resolve_priority_channel_liveness,
};
use crate::webui::state::{
    get_status_cache, platform_channel_configured, refresh_status_cache_config_from,
    update_status_cache_with, BiliStatus, NetworkStatus, NicoStatus,
    PriorityChannelStatus, StatusData, TwStatus, YtStatus,
};

pub(crate) static STATUS_REFRESH_WORKER_STARTED: AtomicBool = AtomicBool::new(false);

pub async fn refresh_status_cache_config() {
    if let Ok(cfg) = load_config().await {
        refresh_status_cache_config_from(&cfg);
    }
}

/// 只刷新直播状态，不阻塞调用方。
pub async fn refresh_live_status_background() {
    tokio::spawn(async {
        let _ = refresh_youtube_status().await;
    });

    tokio::spawn(async {
        let _ = refresh_twitch_status().await;
    });

    tokio::spawn(async {
        let _ = refresh_niconico_status().await;
    });
}

pub struct StatusRefreshWorker(tokio::task::JoinHandle<()>);

impl Drop for StatusRefreshWorker {
    fn drop(&mut self) {
        self.0.abort();
        STATUS_REFRESH_WORKER_STARTED.store(false, Ordering::SeqCst);
    }
}

pub fn start_status_refresh_worker() -> Option<StatusRefreshWorker> {
    if STATUS_REFRESH_WORKER_STARTED.swap(true, Ordering::SeqCst) {
        return None;
    }

    Some(StatusRefreshWorker(tokio::spawn(async {
        loop {
            let interval_secs = match refresh_status_snapshot().await {
                Ok(interval_secs) => interval_secs.max(5),
                Err(e) => {
                    tracing::debug!("WebUI status refresh skipped: {}", e);
                    15
                }
            };

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {}
                _ = crate::webui::state::wait_status_refresh_notified() => {}
            }
        }
    })))
}

pub(crate) async fn refresh_status_snapshot() -> Result<u64, String> {
    let cfg = load_config().await.map_err(|e| e.to_string())?;
    refresh_status_cache_config_from(&cfg);

    if let Err(e) = refresh_bilibili_status_cache_with_config(&cfg).await {
        tracing::warn!("[B站] 状态刷新失败：{}", safe_diagnostic(&e.to_string()));
    }

    if cfg.youtube.enable_monitor && !cfg.youtube.channel_id.is_empty() {
        if let Err(e) = refresh_youtube_status_cache_with_config(&cfg).await {
            tracing::warn!("[YT] 状态刷新失败：{}", safe_diagnostic(&e.to_string()));
        }
    } else if !platform_channel_configured(&cfg.youtube.channel_name, &cfg.youtube.channel_id) {
        crate::config::with_current_config(&cfg, || {
            update_status_cache_with(|status| status.youtube = None)
        });
    }

    if cfg.twitch.enable_monitor && !cfg.twitch.channel_id.is_empty() {
        if let Err(e) = refresh_twitch_status_cache_with_config(&cfg).await {
            tracing::warn!("[TW] 状态刷新失败：{}", safe_diagnostic(&e.to_string()));
        }
    } else if !platform_channel_configured(&cfg.twitch.channel_name, &cfg.twitch.channel_id) {
        crate::config::with_current_config(&cfg, || {
            update_status_cache_with(|status| status.twitch = None)
        });
    }

    if cfg.niconico.enable_monitor && niconico_configured(&cfg.niconico) {
        if let Err(e) = refresh_niconico_status_cache_with_config(&cfg).await {
            tracing::warn!("[NC] 状态刷新失败：{}", safe_diagnostic(&e.to_string()));
        }
    } else if !niconico_configured(&cfg.niconico) {
        update_status_cache_with(|status| status.niconico = None);
    }

    if cfg.priority_channel.enabled && !cfg.priority_channel.channel_name.is_empty() {
        if let Err(e) = refresh_priority_channel_status_cache_with_config(&cfg).await {
            tracing::warn!("优先频道状态刷新失败：{}", safe_diagnostic(&e.to_string()));
        }
    }

    Ok(cfg.interval)
}

pub(crate) async fn refresh_bilibili_status_cache_with_config(cfg: &Config) -> Result<(), String> {
    // 房间号是配置而不是直播事实：先写进卡片，B 站不可达时概览仍能跳转。
    crate::config::with_current_config(cfg, || {
        update_status_cache_with(|status| {
            status.bilibili.room = cfg.bililive.room.into();
        })
    });

    let live = get_bili_live_status(cfg.bililive.room)
        .await
        .map_err(|e| e.to_string())?;
    // get_info 没有封禁时钟：room_init 失败时保留上一次的封禁状态。
    let room_lock = if cfg.bililive.room > 0 {
        get_bili_room_lock(cfg.bililive.room).await.ok()
    } else {
        None
    };

    crate::config::with_current_config(cfg, || {
        update_status_cache_with(|status| {
            status.bilibili.apply_live_status(&live);
            status.bilibili.enable_danmaku_command = cfg.bililive.enable_danmaku_command;
            if let Some(lock) = &room_lock {
                status.bilibili.apply_room_lock(lock);
            }
        })
    });

    Ok(())
}

pub(crate) async fn refresh_youtube_status_cache_with_config(cfg: &Config) -> Result<(), String> {
    if cfg.youtube.channel_id.is_empty() {
        return Err("尚未配置 YouTube 频道".to_string());
    }

    let source = get_youtube_channel_status(&cfg.youtube.channel_id)
        .await
        .map_err(|e| e.to_string())?;

    let area_name = area_name_or_unknown(cfg.youtube.area_v2);

    crate::config::with_current_config(cfg, || {
        update_status_cache_with(|status| {
            status.youtube = Some(YtStatus {
                is_live: source.is_live,
                enable_monitor: cfg.youtube.enable_monitor,
                warning_held: crate::plugins::danmaku::source_warning_held(cfg, "YT"),
                title: source.title,
                topic: source.topic,
                channel_name: cfg.youtube.channel_name.clone(),
                channel_id: cfg.youtube.channel_id.clone(),
                quality: cfg.youtube.quality.clone(),
                area_id: cfg.youtube.area_v2,
                area_name,
                crop_enabled: cfg.youtube.crop.is_some(),
                ffmpeg_cache_enabled: cfg.youtube.ffmpeg_cache.enabled,
                ffmpeg_cache_latency_secs: cfg.youtube.ffmpeg_cache.latency_secs,
            });
        })
    });

    Ok(())
}

pub(crate) async fn refresh_twitch_status_cache_with_config(cfg: &Config) -> Result<(), String> {
    if cfg.twitch.channel_id.is_empty() {
        return Err("尚未配置 Twitch 频道".to_string());
    }

    let (is_live, game, title, _) = get_twitch_status(&cfg.twitch.channel_id)
        .await
        .map_err(|e| e.to_string())?;
    let area_name = area_name_or_unknown(cfg.twitch.area_v2);

    crate::config::with_current_config(cfg, || {
        update_status_cache_with(|status| {
            status.twitch = Some(TwStatus {
                is_live,
                enable_monitor: cfg.twitch.enable_monitor,
                warning_held: crate::plugins::danmaku::source_warning_held(cfg, "TW"),
                title,
                game,
                channel_name: cfg.twitch.channel_name.clone(),
                channel_id: cfg.twitch.channel_id.clone(),
                quality: cfg.twitch.quality.clone(),
                area_id: cfg.twitch.area_v2,
                area_name,
                crop_enabled: cfg.twitch.crop.is_some(),
                ffmpeg_cache_enabled: cfg.twitch.ffmpeg_cache.enabled,
                ffmpeg_cache_latency_secs: cfg.twitch.ffmpeg_cache.latency_secs,
            });
        })
    });

    Ok(())
}

pub(crate) async fn refresh_niconico_status_cache_with_config(cfg: &Config) -> Result<(), String> {
    if !niconico_configured(&cfg.niconico) {
        return Err("尚未配置 Niconico 频道".to_string());
    }

    let SourceStatus {
        is_live,
        title,
        start_time: scheduled_start,
        video_id: live_id,
        ..
    } = get_niconico_status(&cfg.niconico)
        .await
        .map_err(|e| e.to_string())?;
    let area_name = area_name_or_unknown(cfg.niconico.area_v2);
    let (channel_id, channel_name) = niconico_channel_identity(&cfg.niconico);

    update_status_cache_with(|status| {
        status.niconico = Some(NicoStatus {
            is_live,
            enable_monitor: cfg.niconico.enable_monitor,
            warning_held: crate::plugins::danmaku::source_warning_held(cfg, "NC"),
            title,
            channel_name,
            channel_id,
            live_id,
            scheduled_start: scheduled_start.map(|start| start.to_rfc3339()),
            quality: cfg.niconico.quality.clone(),
            area_id: cfg.niconico.area_v2,
            area_name,
            crop_enabled: cfg.niconico.crop.is_some(),
            ffmpeg_cache_enabled: cfg.niconico.ffmpeg_cache.enabled,
            ffmpeg_cache_latency_secs: cfg.niconico.ffmpeg_cache.latency_secs,
        });
    });

    Ok(())
}

pub(crate) async fn refresh_priority_channel_status_cache_with_config(
    cfg: &Config,
) -> Result<(), String> {
    if !cfg.priority_channel.enabled {
        return Err("优先频道未启用".to_string());
    }

    if cfg.priority_channel.channel_name.is_empty() {
        return Err("尚未配置优先频道".to_string());
    }

    // 只取元数据：后台监控还要确认可播放地址才会切换，因此面板可能
    // 比实际切换早一点点显示直播中。
    let liveness = resolve_priority_channel_liveness(cfg).await;
    let platform = liveness
        .platform
        .map(|platform| platform.api_name().to_string());
    let title = liveness.title;

    crate::config::with_current_config(cfg, || {
        update_status_cache_with(|status| {
            status.priority_channel = Some(PriorityChannelStatus {
                enabled: cfg.priority_channel.enabled,
                auto_restart: cfg.priority_channel.auto_restart,
                warning_held: crate::plugins::danmaku::priority_warning_held(cfg),
                channel_name: cfg.priority_channel.channel_name.clone(),
                is_live: platform.is_some(),
                platform,
                title,
                default_area: cfg.priority_channel.default_area,
                default_area_name: get_area_name(cfg.priority_channel.default_area),
            });
        })
    });

    Ok(())
}

/// 本机推流器的实时网络指标快照；没有本地推流时返回空快照。
pub(crate) async fn current_network_status() -> NetworkStatus {
    // 直播间可能由别的节点推流；卡住的本地推流器同样是零进度，
    // 因此指标本身不能用来判断归属。
    if !is_ffmpeg_running().await {
        return NetworkStatus::default();
    }
    let hls_cache_active = is_ffmpeg_hls_cache_active();
    let network_stats = get_ffmpeg_network_stats();
    NetworkStatus {
        ffmpeg_running: true,
        stream_speed: get_ffmpeg_speed(),
        stream_cache_speed: if hls_cache_active {
            get_ffmpeg_cache_speed()
        } else {
            None
        },
        stream_bitrate_kbps: network_stats.push_bitrate_kbps,
        stream_cache_bitrate_kbps: if hls_cache_active {
            network_stats.cache_bitrate_kbps
        } else {
            None
        },
        stream_fps: network_stats.push_fps,
        stream_frame: network_stats.push_frame,
        stream_time_secs: network_stats.push_time_secs,
        stream_cache_time_secs: if hls_cache_active {
            network_stats.cache_time_secs
        } else {
            None
        },
        hls_cache_active,
        stream_bitrate_history: network_stats.push_bitrate_history,
        stream_cache_bitrate_history: if hls_cache_active {
            network_stats.cache_bitrate_history
        } else {
            Vec::new()
        },
    }
}

pub(crate) async fn apply_realtime_stream_metrics(bili: &mut BiliStatus) {
    let network = current_network_status().await;
    bili.stream_quality = if network.ffmpeg_running {
        network.stream_speed.map(|speed| {
            if speed > 0.97 {
                "流畅".to_string()
            } else if speed > 0.94 {
                "波动".to_string()
            } else {
                "卡顿".to_string()
            }
        })
    } else {
        None
    };
    bili.apply_network(network);
}

pub async fn get_status() -> impl IntoResponse {
    let local_node_is_active_owner = match load_config().await {
        Ok(cfg) => {
            refresh_status_cache_config_from(&cfg);
            cluster::local_node_is_active_owner(&cfg)
        }
        Err(e) => {
            if get_status_cache().is_some() {
                tracing::debug!("Skipped status config refresh: {}", e);
            } else {
                let is_not_found = e.to_string().contains("No such file");
                if !is_not_found {
                    tracing::error!("配置加载失败：{}", safe_diagnostic(&e.to_string()));
                }

                let error_msg = if e.to_string().contains("Permission denied") {
                    format!("配置文件权限错误：{}。请确保 config.json 文件存在且有读取权限，或在可执行文件所在目录运行程序。", e)
                } else if is_not_found {
                    "配置文件不存在，请完成首次设置".to_string()
                } else {
                    format!("配置加载失败：{}", e)
                };
                return (
                    StatusCode::OK,
                    Json(ApiResponse::<()> {
                        success: false,
                        data: None,
                        message: Some(error_msg),
                    }),
                )
                    .into_response();
            }

            // 配置读不出来时无法可靠解析本机集群身份，保留缓存视图。
            true
        }
    };

    let mut status = get_status_cache().unwrap_or_default();
    apply_effective_local_monitor_state(&mut status, local_node_is_active_owner);
    apply_realtime_stream_metrics(&mut status.bilibili).await;

    (
        StatusCode::OK,
        Json(ApiResponse {
            success: true,
            data: Some(status),
            message: None,
        }),
    )
        .into_response()
}

/// 备用节点不跑任何监控：配置里的开关在这里一律按关闭展示。
pub(crate) fn apply_effective_local_monitor_state(status: &mut StatusData, is_active_owner: bool) {
    if is_active_owner {
        return;
    }

    status.bilibili.enable_danmaku_command = false;
    if let Some(youtube) = status.youtube.as_mut() {
        youtube.enable_monitor = false;
    }
    if let Some(twitch) = status.twitch.as_mut() {
        twitch.enable_monitor = false;
    }
    if let Some(niconico) = status.niconico.as_mut() {
        niconico.enable_monitor = false;
    }
    if let Some(priority) = status.priority_channel.as_mut() {
        priority.enabled = false;
        priority.auto_restart = false;
    }
}

pub async fn get_network_status() -> Json<ApiResponse<NetworkStatus>> {
    Json(ApiResponse {
        success: true,
        data: Some(current_network_status().await),
        message: None,
    })
}

pub async fn refresh_youtube_status() -> Json<ApiResponse<()>> {
    let cfg = match load_config().await {
        Ok(c) => c,
        Err(e) => {
            return Json(ApiResponse {
                success: false,
                data: None,
                message: Some(format!("配置读取失败：{}", e)),
            });
        }
    };

    if cfg.youtube.channel_id.is_empty() {
        return Json(ApiResponse {
            success: false,
            data: None,
            message: Some("尚未配置 YouTube 频道".to_string()),
        });
    }

    match refresh_youtube_status_cache_with_config(&cfg).await {
        Ok(()) => Json(ApiResponse {
            success: true,
            data: Some(()),
            message: Some("YouTube 状态已刷新".to_string()),
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            message: Some(format!("获取 YouTube 状态失败：{}", e)),
        }),
    }
}

pub async fn refresh_twitch_status() -> Json<ApiResponse<()>> {
    let cfg = match load_config().await {
        Ok(c) => c,
        Err(e) => {
            return Json(ApiResponse {
                success: false,
                data: None,
                message: Some(format!("配置读取失败：{}", e)),
            });
        }
    };

    if cfg.twitch.channel_id.is_empty() {
        return Json(ApiResponse {
            success: false,
            data: None,
            message: Some("尚未配置 Twitch 频道".to_string()),
        });
    }

    match refresh_twitch_status_cache_with_config(&cfg).await {
        Ok(()) => Json(ApiResponse {
            success: true,
            data: Some(()),
            message: Some("Twitch 状态已刷新".to_string()),
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            message: Some(format!("获取 Twitch 状态失败：{}", e)),
        }),
    }
}

pub async fn refresh_niconico_status() -> Json<ApiResponse<()>> {
    let cfg = match load_config().await {
        Ok(c) => c,
        Err(e) => {
            return Json(ApiResponse {
                success: false,
                data: None,
                message: Some(format!("配置读取失败：{}", e)),
            });
        }
    };

    if !niconico_configured(&cfg.niconico) {
        return Json(ApiResponse {
            success: false,
            data: None,
            message: Some("尚未配置 Niconico 频道".to_string()),
        });
    }

    match refresh_niconico_status_cache_with_config(&cfg).await {
        Ok(()) => Json(ApiResponse {
            success: true,
            data: Some(()),
            message: Some("Niconico 状态已刷新".to_string()),
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            message: Some(format!("获取 Niconico 状态失败：{}", e)),
        }),
    }
}

pub async fn refresh_priority_channel_status() -> Json<ApiResponse<()>> {
    let cfg = match load_config().await {
        Ok(c) => c,
        Err(e) => {
            return Json(ApiResponse {
                success: false,
                data: None,
                message: Some(format!("配置读取失败：{}", e)),
            });
        }
    };

    if !cfg.priority_channel.enabled {
        return Json(ApiResponse {
            success: false,
            data: None,
            message: Some("优先频道未启用".to_string()),
        });
    }

    if cfg.priority_channel.channel_name.is_empty() {
        return Json(ApiResponse {
            success: false,
            data: None,
            message: Some("尚未配置优先频道".to_string()),
        });
    }

    match refresh_priority_channel_status_cache_with_config(&cfg).await {
        Ok(()) => Json(ApiResponse {
            success: true,
            data: Some(()),
            message: Some("优先频道状态已刷新".to_string()),
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            message: Some(format!("获取优先频道状态失败：{}", e)),
        }),
    }
}

pub async fn niconico_session_status(
) -> Result<Json<ApiResponse<niconico_session::SessionStatus>>, StatusCode> {
    let cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(ApiResponse {
        success: true,
        data: Some(niconico_session::status(&cfg)),
        message: None,
    }))
}

pub async fn check_niconico_session(
) -> Result<Json<ApiResponse<niconico_session::SessionStatus>>, StatusCode> {
    let cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(ApiResponse {
        success: true,
        data: Some(niconico_session::check(&cfg, true).await),
        message: None,
    }))
}
