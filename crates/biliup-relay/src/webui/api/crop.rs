//! 裁切与 HLS 缓存设置：抓帧要起外部进程，全部经带期限的监督者回收。

use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::Response;
use base64::Engine as _;
use serde::Deserialize;

use super::ApiResponse;
use super::config_read_response;
use super::config_save_status;
use crate::config::{CropConfig, FfmpegCache, load_config};
use crate::plugins::set_config_updated;
use crate::webui::state::refresh_status_cache_config_from;

/// 抓一帧的期限：比 yt-dlp 取流短，避免一次点击占住进程太久。
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(45);

/// 起一个外部命令并在期限内返回；超时由监督者 kill + wait 回收，绝不留僵尸。
async fn run_tool(
    command: Command,
    limit: Duration,
    label: &'static str,
) -> std::io::Result<std::process::Output> {
    crate::plugins::subprocess::command_output_with_timeout(command, limit, label).await
}

fn twitch_proxy_playlist(proxy_region: &str) -> &'static str {
    match proxy_region {
        "na" => "--twitch-proxy-playlist=https://lb-na.cdn-perfprod.com",
        "eu" => "--twitch-proxy-playlist=https://lb-eu.cdn-perfprod.com",
        "eu2" => "--twitch-proxy-playlist=https://lb-eu2.cdn-perfprod.com",
        "eu3" => "--twitch-proxy-playlist=https://lb-eu3.cdn-perfprod.com",
        "eu4" => "--twitch-proxy-playlist=https://lb-eu4.cdn-perfprod.com",
        "eu5" => "--twitch-proxy-playlist=https://lb-eu5.cdn-perfprod.com",
        "as" => "--twitch-proxy-playlist=https://lb-as.cdn-perfprod.com",
        "sa" => "--twitch-proxy-playlist=https://lb-sa.cdn-perfprod.com",
        "eul" => "--twitch-proxy-playlist=https://eu.luminous.dev",
        "eu2l" => "--twitch-proxy-playlist=https://eu2.luminous.dev",
        "asl" => "--twitch-proxy-playlist=https://as.luminous.dev",
        "" => "",
        _ => "--twitch-proxy-playlist=https://as.luminous.dev",
    }
}

pub async fn capture_frame(
    Path(platform): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<ApiResponse<()>> {
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

    let m3u8_url: String;
    let proxy: Option<String>;

    match platform.as_str() {
        "youtube" => {
            let channel_id = params
                .get("channel_id")
                .map(|s| s.as_str())
                .unwrap_or(&cfg.youtube.channel_id);
            let channel_url = format!("https://www.youtube.com/channel/{}/live", channel_id);

            let mut cmd = Command::new(crate::plugins::subprocess::executable_command(
                "yt-dlp.exe",
                "yt-dlp",
            ));
            crate::plugins::subprocess::configure_no_window(&mut cmd);
            cmd.arg("-f")
                .arg(&cfg.youtube.quality)
                .arg("-g")
                .arg(&channel_url);

            if let Some(ref proxy_url) = cfg.youtube.proxy {
                cmd.arg("--proxy").arg(proxy_url);
            }

            match crate::plugins::youtube_cookies::run(
                cmd,
                cfg.youtube.cookies_from_browser.clone(),
                CAPTURE_TIMEOUT,
            )
            .await
            {
                Ok(output) if output.status.success() => {
                    m3u8_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if m3u8_url.is_empty() {
                        return Json(ApiResponse {
                            success: false,
                            data: None,
                            message: Some("未取到 YouTube 直播流地址，请确认正在直播".to_string()),
                        });
                    }
                    proxy = cfg.youtube.proxy.clone();
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::error!(
                        "yt-dlp 取流失败：{}",
                        crate::logging::safe_diagnostic(&stderr)
                    );

                    let message = if stderr.contains("will begin in")
                        || stderr.contains("not live")
                        || stderr.contains("Premieres in")
                    {
                        "该频道未在直播".to_string()
                    } else {
                        format!(
                            "获取直播流失败：{}",
                            stderr.lines().next().unwrap_or("未知错误")
                        )
                    };

                    return Json(ApiResponse {
                        success: false,
                        data: None,
                        message: Some(message),
                    });
                }
                Err(e) => {
                    tracing::error!(
                        "yt-dlp 启动失败：{}",
                        crate::logging::safe_diagnostic(&e.to_string())
                    );
                    return Json(ApiResponse {
                        success: false,
                        data: None,
                        message: Some(format!(
                            "yt-dlp 启动失败：{}；请确认已安装 yt-dlp 并位于 PATH 中",
                            e
                        )),
                    });
                }
            }
        }
        "twitch" => {
            let proxy_url = twitch_proxy_playlist(&cfg.twitch.proxy_region);
            let channel_id = params
                .get("channel_id")
                .filter(|id| !id.is_empty())
                .map(|s| s.as_str())
                .unwrap_or(&cfg.twitch.channel_id);
            let channel_url = format!("https://twitch.tv/{}", channel_id);

            let mut cmd = Command::new(crate::plugins::subprocess::executable_command(
                "streamlink.exe",
                "streamlink",
            ));
            crate::plugins::subprocess::configure_no_window(&mut cmd);
            if !proxy_url.is_empty() {
                cmd.arg(proxy_url);
            }
            cmd.arg("--stream-url")
                .arg(&channel_url)
                .arg(&cfg.twitch.quality);
            if let Some(ref http_proxy) = cfg.twitch.proxy {
                cmd.arg("--http-proxy").arg(http_proxy);
            }

            match run_tool(cmd, CAPTURE_TIMEOUT, "streamlink").await {
                Ok(output) if output.status.success() => {
                    m3u8_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if m3u8_url.is_empty() {
                        return Json(ApiResponse {
                            success: false,
                            data: None,
                            message: Some("未取到 Twitch 直播流地址，请确认正在直播".to_string()),
                        });
                    }
                    proxy = cfg.twitch.proxy.clone();
                }
                _ => {
                    return Json(ApiResponse {
                        success: false,
                        data: None,
                        message: Some(
                            "未取到 Twitch 直播流地址，请确认已安装 streamlink 与 streamlink-ttvlol 插件"
                                .to_string(),
                        ),
                    });
                }
            }
        }
        "niconico" => {
            return Json(ApiResponse {
                success: false,
                data: None,
                message: Some(
                    "Niconico 无法从 HLS 抓帧，请手动上传一张静帧图片进行裁剪".to_string(),
                ),
            });
        }
        _ => {
            return Json(ApiResponse {
                success: false,
                data: None,
                message: Some("不支持的平台".to_string()),
            });
        }
    }

    let output_path = match crate::storage::cache_file("pic_for_crop.jpg") {
        Ok(path) => path,
        Err(_) => {
            return Json(ApiResponse {
                success: false,
                data: None,
                message: Some("数据目录不可用".to_string()),
            });
        }
    };

    let mut cmd = Command::new(crate::plugins::subprocess::executable_command(
        "ffmpeg.exe",
        "ffmpeg",
    ));
    crate::plugins::subprocess::configure_no_window(&mut cmd);
    cmd.arg("-y")
        .arg("-live_start_index")
        .arg("-1")
        .arg("-i")
        .arg(&m3u8_url)
        .arg("-vframes")
        .arg("1")
        .arg(&output_path);

    if let Some(proxy_url) = proxy {
        cmd.arg("-http_proxy").arg(proxy_url);
    }

    let output = match run_tool(cmd, CAPTURE_TIMEOUT, "ffmpeg").await {
        Ok(o) => o,
        Err(e) => {
            return Json(ApiResponse {
                success: false,
                data: None,
                message: Some(format!("ffmpeg 执行失败：{}", e)),
            });
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Json(ApiResponse {
            success: false,
            data: None,
            message: Some(format!(
                "ffmpeg 取帧失败：{}",
                crate::logging::safe_diagnostic(&stderr)
            )),
        });
    }

    let image_data = match tokio::fs::read(&output_path).await {
        Ok(data) => data,
        Err(e) => {
            return Json(ApiResponse {
                success: false,
                data: None,
                message: Some(format!("读取截取的画面失败：{}", e)),
            });
        }
    };

    let base64_image = base64::engine::general_purpose::STANDARD.encode(&image_data);

    Json(ApiResponse {
        success: true,
        data: Some(()),
        message: Some(format!("data:image/jpeg;base64,{}", base64_image)),
    })
}

#[derive(Deserialize)]
pub struct UpdateCropRequest {
    pub platform: String,
    pub enabled: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub x: Option<u32>,
    pub y: Option<u32>,
}

pub async fn update_crop(
    Json(payload): Json<UpdateCropRequest>,
) -> Result<ApiResponse<()>, StatusCode> {
    let mut cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let crop_config = match crop_config_from_update(&payload) {
        Ok(crop_config) => crop_config,
        Err(message) => {
            return Ok(ApiResponse {
                success: false,
                data: None,
                message: Some(message),
            });
        }
    };

    match payload.platform.as_str() {
        "youtube" => {
            cfg.youtube.crop = crop_config;
        }
        "twitch" => {
            cfg.twitch.crop = crop_config;
        }
        "niconico" => {
            cfg.niconico.crop = crop_config;
        }
        _ => {
            return Ok(ApiResponse {
                success: false,
                data: None,
                message: Some("不支持的平台".to_string()),
            });
        }
    }

    crate::config::save_config(&mut cfg)
        .await
        .map_err(config_save_status)?;

    set_config_updated();

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some("裁剪配置已更新".to_string()),
    })
}

pub(crate) fn crop_config_from_update(
    payload: &UpdateCropRequest,
) -> Result<Option<CropConfig>, String> {
    if !payload.enabled {
        return Ok(None);
    }

    let (Some(width), Some(height), Some(x), Some(y)) =
        (payload.width, payload.height, payload.x, payload.y)
    else {
        return Err("启用裁剪时需要填写宽高与坐标".to_string());
    };

    if width == 0 || height == 0 {
        return Err("裁剪宽高需大于 0".to_string());
    }

    Ok(Some(CropConfig {
        width,
        height,
        x,
        y,
    }))
}

pub async fn get_crop(
    platform: String,
) -> Result<Json<ApiResponse<Option<CropConfig>>>, Response> {
    let cfg = load_config().await.map_err(|e| config_read_response(&*e))?;

    let crop = match platform.as_str() {
        "youtube" => cfg.youtube.crop,
        "twitch" => cfg.twitch.crop,
        "niconico" => cfg.niconico.crop,
        _ => {
            return Ok(Json(ApiResponse {
                success: false,
                data: None,
                message: Some("不支持的平台".to_string()),
            }));
        }
    };

    Ok(Json(ApiResponse {
        success: true,
        data: Some(crop),
        message: None,
    }))
}

#[derive(Deserialize)]
pub struct UpdateFfmpegCacheRequest {
    pub platform: String,
    pub enabled: bool,
    pub latency_secs: Option<u64>,
}

pub async fn update_ffmpeg_cache(
    Json(payload): Json<UpdateFfmpegCacheRequest>,
) -> Result<ApiResponse<()>, StatusCode> {
    let mut cfg = load_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let cache = match payload.platform.as_str() {
        "youtube" => &mut cfg.youtube.ffmpeg_cache,
        "twitch" => &mut cfg.twitch.ffmpeg_cache,
        "niconico" => &mut cfg.niconico.ffmpeg_cache,
        _ => {
            return Ok(ApiResponse {
                success: false,
                data: None,
                message: Some("不支持的平台".to_string()),
            });
        }
    };

    cache.enabled = payload.enabled;
    if let Some(latency_secs) = payload.latency_secs {
        cache.latency_secs = latency_secs.clamp(1, 60);
    }

    crate::config::save_config(&mut cfg)
        .await
        .map_err(config_save_status)?;

    set_config_updated();
    refresh_status_cache_config_from(&cfg);

    Ok(ApiResponse {
        success: true,
        data: None,
        message: Some("HLS 缓存配置已更新".to_string()),
    })
}

pub async fn get_ffmpeg_cache(
    platform: String,
) -> Result<Json<ApiResponse<FfmpegCache>>, Response> {
    let cfg = load_config().await.map_err(|e| config_read_response(&*e))?;

    let cache = match platform.as_str() {
        "youtube" => cfg.youtube.ffmpeg_cache.clone(),
        "twitch" => cfg.twitch.ffmpeg_cache.clone(),
        "niconico" => cfg.niconico.ffmpeg_cache.clone(),
        _ => {
            return Ok(Json(ApiResponse {
                success: false,
                data: None,
                message: Some("不支持的平台".to_string()),
            }));
        }
    };

    Ok(Json(ApiResponse {
        success: true,
        data: Some(cache),
        message: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ffmpeg_cache_latency_is_clamped_and_persisted() {
        let _guard = crate::storage::lock_test_global();
        let dir = tempfile::tempdir().unwrap();
        crate::storage::init(&dir.path().join("data"), Some(&dir.path().join("key")))
            .unwrap();
        let mut cfg = crate::test_support::config("crop", 0);
        cfg.youtube.ffmpeg_cache.latency_secs = 10;
        crate::config::save_config(&mut cfg).await.unwrap();

        for (requested, expected) in [(0u64, 1u64), (1, 1), (30, 30), (60, 60), (99, 60)] {
            let response = update_ffmpeg_cache(Json(UpdateFfmpegCacheRequest {
                platform: "youtube".into(),
                enabled: true,
                latency_secs: Some(requested),
            }))
            .await
            .unwrap();
            assert!(response.success);
            let saved = crate::config::load_config().await.unwrap();
            assert_eq!(
                saved.youtube.ffmpeg_cache.latency_secs, expected,
                "请求 {requested} 秒必须收敛到 {expected}"
            );
            assert!(saved.youtube.ffmpeg_cache.enabled);
        }

        let unsupported = update_ffmpeg_cache(Json(UpdateFfmpegCacheRequest {
            platform: "unknown".into(),
            enabled: true,
            latency_secs: Some(10),
        }))
        .await
        .unwrap();
        assert!(!unsupported.success, "未知平台必须被拒绝");

        crate::storage::shutdown();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_tool_times_out_and_reaps_child_process() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let script = dir.path().join("slow-tool.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut cmd = Command::new(&script);
        crate::plugins::subprocess::configure_no_window(&mut cmd);
        let started = std::time::Instant::now();
        let error = run_tool(cmd, Duration::from_millis(300), "crop-test")
            .await
            .expect_err("慢命令必须在期限后失败");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "抓帧工具必须在有界时间内返回，实际 {:?}",
            started.elapsed()
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
                "超时后不得留下仍在运行的子进程：{stat:?}"
            );
        }
        #[cfg(not(target_os = "linux"))]
        let _ = pid;
    }
}
