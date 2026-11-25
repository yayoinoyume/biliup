//! 转播底座的 `/v1/relay/*` 全量路由与宿主鉴权桥接。
//!
//! 路由一律用完整字面量声明：`permissions.rs` 的路由扫描器只认每条路由的完整路径，
//! `.nest()` 会让它只看到短路径而漏扫。

use crate::server::api::access::Caller;
use crate::server::infrastructure::permissions::Permission;
use crate::server::infrastructure::policy::Subject;
use crate::server::infrastructure::service_register::ServiceRegister;
use crate::server::infrastructure::users::{AuthSession, Backend};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use axum_login::{AuthUser, AuthnBackend};
use biliup_relay::plugins::youtube_websub;
use biliup_relay::webui::api;
use biliup_relay::RelayHealthSnapshot;
use futures::future::BoxFuture;
use std::time::Duration;

/// 存储恢复体的上限；只有这一条路由放开，其余仍是 Axum 默认值。
const STORAGE_RESTORE_BODY_LIMIT: usize = 48 * 1024 * 1024;

pub async fn health(State(services): State<ServiceRegister>) -> Json<RelayHealthSnapshot> {
    Json(services.relay.health())
}

/// 受保护的全部 `/v1/relay/*` 路由：由 `app.rs` 合进 `protected_routes`，
/// 因此处在本进程的 access 中间件之内（默认拒绝），不经过 fleet 守卫。
pub fn router(services: ServiceRegister) -> Router<()> {
    Router::new()
        .route("/v1/relay/health", get(health))
        .route("/v1/relay/version", get(api::logs::get_version))
        .route("/v1/relay/status", get(api::status::get_status))
        .route("/v1/relay/events", get(events))
        .route("/v1/relay/network-status", get(api::status::get_network_status))
        .route("/v1/relay/logs", get(api::logs::get_logs_endpoint))
        .route("/v1/relay/deps/status", get(api::logs::get_deps_status))
        .route(
            "/v1/relay/config",
            get(api::config::get_config).post(api::config::update_config),
        )
        .route(
            "/v1/relay/priority-channel",
            post(api::config::update_priority_channel),
        )
        .route(
            "/v1/relay/priority-channel/release-warning",
            post(api::stream_control::release_priority_warning),
        )
        .route("/v1/relay/start", post(api::stream_control::start_stream))
        .route("/v1/relay/stop", post(api::stream_control::stop_stream))
        .route("/v1/relay/restart", post(api::stream_control::restart_stream))
        .route("/v1/relay/danmaku", post(api::stream_control::send_danmaku))
        .route("/v1/relay/cover", post(api::stream_control::update_cover))
        .route("/v1/relay/area", post(api::stream_control::update_area))
        .route("/v1/relay/title", post(api::stream_control::update_title))
        .route("/v1/relay/channel", post(api::stream_control::update_channel))
        .route("/v1/relay/channels", get(api::stream_control::get_channels))
        .route(
            "/v1/relay/channels/resolve-youtube",
            post(api::setup_data::resolve_youtube_channel),
        )
        .route("/v1/relay/areas", get(api::stream_control::get_areas))
        .route("/v1/relay/areas/catalog", get(api::setup_data::get_area_catalog))
        .route(
            "/v1/relay/banned-keywords",
            get(api::stream_control::get_banned_keywords)
                .post(api::stream_control::update_banned_keywords),
        )
        .route("/v1/relay/setup-status", get(api::setup::check_setup))
        .route(
            "/v1/relay/setup/save-config",
            post(api::setup::save_setup_config),
        )
        .route(
            "/v1/relay/setup/holodex-favorites",
            post(api::setup_data::preview_setup_favorites),
        )
        .route(
            "/v1/relay/setup/login-status",
            get(api::setup::check_login_status),
        )
        .route("/v1/relay/setup/qrcode", get(api::setup::get_qr_code))
        .route("/v1/relay/setup/poll-login", post(api::setup::poll_login))
        .route("/v1/relay/setup/bili-room", get(api::setup::resolve_bili_room))
        .route(
            "/v1/relay/toggle-youtube-monitor",
            post(api::stream_control::toggle_youtube_monitor),
        )
        .route(
            "/v1/relay/toggle-twitch-monitor",
            post(api::stream_control::toggle_twitch_monitor),
        )
        .route(
            "/v1/relay/toggle-niconico-monitor",
            post(api::stream_control::toggle_niconico_monitor),
        )
        .route("/v1/relay/holodex/switch", post(api::stream_switch::switch_stream))
        .route(
            "/v1/relay/holodex/streams",
            get(api::admin_stream_list::get_stream_list),
        )
        .route(
            "/v1/relay/holodex/auth/status",
            get(api::holodex_auth::auth_status),
        )
        .route(
            "/v1/relay/youtube/keys",
            get(api::youtube_api_status::get_status),
        )
        .route(
            "/v1/relay/refresh/youtube",
            get(api::status::refresh_youtube_status),
        )
        .route(
            "/v1/relay/refresh/twitch",
            get(api::status::refresh_twitch_status),
        )
        .route(
            "/v1/relay/refresh/niconico",
            get(api::status::refresh_niconico_status),
        )
        .route(
            "/v1/relay/refresh/priority-channel",
            get(api::status::refresh_priority_channel_status),
        )
        .route(
            "/v1/relay/niconico/session",
            get(api::status::niconico_session_status),
        )
        .route(
            "/v1/relay/niconico/session/check",
            post(api::status::check_niconico_session),
        )
        .route(
            "/v1/relay/history/status",
            get(api::history::status),
        )
        .route(
            "/v1/relay/history/summary",
            get(api::history::summary),
        )
        .route(
            "/v1/relay/history/sessions",
            get(api::history::sessions),
        )
        .route(
            "/v1/relay/history/events",
            get(api::history::events),
        )
        .route("/v1/relay/history/stats", get(api::history::stats))
        .route("/v1/relay/history/clean", post(api::history::clean))
        .route("/v1/relay/storage", get(api::storage::storage_status))
        .route(
            "/v1/relay/storage/backup",
            post(api::storage::export_storage_backup),
        )
        .route(
            "/v1/relay/storage/restore",
            post(api::storage::restore_storage_backup)
                .layer(DefaultBodyLimit::max(STORAGE_RESTORE_BODY_LIMIT)),
        )
        .route(
            "/v1/relay/youtube/cookies",
            get(api::storage::youtube_cookie_status)
                .post(api::storage::import_youtube_cookies)
                .delete(api::storage::clear_youtube_cookies),
        )
        .route(
            "/v1/relay/player-filter",
            get(api::storage::get_player_filter).post(api::storage::save_player_filter),
        )
        .route(
            "/v1/relay/manage/areas",
            get(api::manage::get_areas_manage)
                .post(api::manage::add_area)
                .put(api::manage::update_area_manage),
        )
        .route(
            "/v1/relay/manage/areas/{id}",
            delete(api::manage::delete_area),
        )
        .route(
            "/v1/relay/manage/channels",
            get(api::manage::get_channels_manage)
                .post(api::manage::add_channel)
                .put(api::manage::update_channel_manage),
        )
        .route(
            "/v1/relay/manage/channels/{name}",
            delete(api::manage::delete_channel),
        )
        .route(
            "/v1/relay/crop/capture/{platform}",
            post(api::crop::capture_frame),
        )
        .route("/v1/relay/crop/update", post(api::crop::update_crop))
        .route(
            "/v1/relay/ffmpeg-cache/update",
            post(api::crop::update_ffmpeg_cache),
        )
        .route(
            "/v1/relay/crop/{platform}",
            get(
                |Path(platform): Path<String>| async move {
                    match api::crop::get_crop(platform).await {
                        Ok(response) => response.into_response(),
                        Err(status) => status.into_response(),
                    }
                },
            ),
        )
        .route(
            "/v1/relay/ffmpeg-cache/{platform}",
            get(
                |Path(platform): Path<String>| async move {
                    match api::crop::get_ffmpeg_cache(platform).await {
                        Ok(response) => response.into_response(),
                        Err(status) => status.into_response(),
                    }
                },
            ),
        )
        .with_state(services)
}

/// YouTube Hub 推送不带浏览器 Cookie，这条路由必须挂在登录校验之外：
/// Hub 的 GET 验证与 POST 推送只由 `verify_token` 与 `X-Hub-Signature` HMAC 鉴权。
pub fn websub_router() -> Router<()> {
    Router::new().route(
        "/v1/relay/websub/youtube",
        youtube_websub::callback_method_router(),
    )
}

/// 面板事件流：宿主中间件会在会话失效时截断响应体，这里再挂一个更短的
/// 复查，把改密 / 删号 / 降权后的断流压到一次复查间隔之内。
async fn events(caller: Caller, auth_session: AuthSession, headers: HeaderMap) -> Response {
    let shutdown = session_revocation(caller, auth_session);
    biliup_relay::webui::events::sse_stream_until(
        biliup_relay::webui::app_state(),
        headers,
        shutdown,
    )
    .into_response()
}

/// 会话撤销等待端。
///
/// 默认只等宿主的会话通道（access 里 30 秒一次的复查），不额外起第二条轮询；
/// 只有显式配了更短的复查间隔，才并联一次 watch_session，先到者结束流。
fn session_revocation(caller: Caller, auth_session: AuthSession) -> BoxFuture<'static, ()> {
    let host = caller.revoked();
    // --auth 关闭时是零鉴权超管：旧 cookie 没有意义，更不能让它断流。
    if !caller.subject.auth_enabled {
        return host;
    }
    let Some(user) = auth_session.user else {
        return host;
    };
    let Some(interval) = sse_recheck_override() else {
        return host;
    };
    let backend = auth_session.backend;
    let user_id = user.id;
    let auth_hash = user.session_auth_hash().to_vec();
    Box::pin(async move {
        tokio::select! {
            _ = host => {}
            _ = watch_session(backend, user_id, auth_hash, interval) => {}
        }
    })
}

/// 只认 `--auth` 下的真实会话：用户被删、被禁用、改密或强制下线后返回。
async fn watch_session(backend: Backend, user_id: i64, auth_hash: Vec<u8>, interval: Duration) {
    loop {
        tokio::time::sleep(interval).await;
        match backend.get_user(&user_id).await {
            Ok(Some(user))
                if user.session_auth_hash() == auth_hash.as_slice()
                    && Subject::user(user.id, user.role).can(Permission::RelayView) => {}
            Ok(_) => return,
            Err(error) => tracing::warn!(?error, "SSE 会话复查失败，下一轮再试"),
        }
    }
}

/// 更短的会话复查间隔：只在显式设置 `BILISTREAM_SSE_RECHECK_MS` 时生效。
///
/// 下限 100ms：再快就是紧循环狂查数据库；上限 30s：再慢不比宿主自己的复查快，
/// 没有意义。区间外一律忽略，回落默认（只走宿主的会话通道）。
fn sse_recheck_override() -> Option<Duration> {
    let millis = std::env::var("BILISTREAM_SSE_RECHECK_MS")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    (100..=30_000)
        .contains(&millis)
        .then(|| Duration::from_millis(millis))
}
