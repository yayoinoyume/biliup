#![allow(dead_code)]
//! 待 Phase 2 Web 服务迁入后替换的薄适配。

use std::net::{IpAddr, Ipv4Addr};

/// 监听地址解析：`localhost` 归一为回环地址，其余必须是字面 IP。
pub fn parse_bind(input: &str) -> Result<IpAddr, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("监听地址为空".to_string());
    }
    if input.eq_ignore_ascii_case("localhost") {
        return Ok(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    input
        .parse::<IpAddr>()
        .map_err(|e| format!("invalid bind address '{input}': {e}"))
}

pub mod api {
    //! 阶段替换：待 Phase 2 迁入 webui/api/status.rs 后由真实实现替换。
    //!
    //! 数据已经是真的：直接读 plugins::ffmpeg 的实时指标，不写死常量。

    use super::state::NetworkStatus;

    /// 本地推流器的实时网络指标快照；没有本地推流时返回空快照。
    pub(crate) async fn current_network_status() -> NetworkStatus {
        // A live room may be fed by another node. Zero progress is also valid for
        // a stalled local publisher, so metrics alone cannot indicate ownership.
        if !crate::plugins::is_ffmpeg_running().await {
            return NetworkStatus::default();
        }
        let hls_cache_active = crate::plugins::is_ffmpeg_hls_cache_active();
        let network_stats = crate::plugins::get_ffmpeg_network_stats();
        NetworkStatus {
            ffmpeg_running: true,
            stream_speed: crate::plugins::get_ffmpeg_speed(),
            stream_cache_speed: if hls_cache_active {
                crate::plugins::get_ffmpeg_cache_speed()
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
}

pub mod events {
    //! 薄导出：真实总线在 `crate::ui_events`，这里只保留上游的模块路径。
    pub use crate::ui_events::*;
}

pub mod state {
    use serde::{Deserialize, Serialize};

    use crate::config::Config;

    use crate::plugins::bilibili::{BiliLiveStatus, BiliRoomLock};

    #[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
    pub struct NetworkStatus {
        #[serde(default)]
        pub ffmpeg_running: bool,
        pub stream_speed: Option<f32>,
        pub stream_cache_speed: Option<f32>,
        pub stream_bitrate_kbps: Option<f32>,
        pub stream_cache_bitrate_kbps: Option<f32>,
        pub stream_fps: Option<f32>,
        pub stream_frame: Option<u64>,
        pub stream_time_secs: Option<u32>,
        pub stream_cache_time_secs: Option<u32>,
        pub hls_cache_active: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub stream_bitrate_history: Vec<f32>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub stream_cache_bitrate_history: Vec<f32>,
    }

    #[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
    pub struct StatusData {
        pub bilibili: BiliStatus,
        pub youtube: Option<YtStatus>,
        pub twitch: Option<TwStatus>,
        #[serde(default)]
        pub niconico: Option<NicoStatus>,
        pub priority_channel: Option<PriorityChannelStatus>,
    }

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
    pub struct PriorityChannelStatus {
        pub enabled: bool,
        #[serde(default)]
        pub auto_restart: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub warning_held: bool,
        pub channel_name: String,
        pub is_live: bool,
        pub platform: Option<String>,
        pub title: Option<String>,
        pub default_area: u64,
        #[serde(default)]
        pub default_area_name: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
    pub struct BiliStatus {
        #[serde(default)]
        pub room: i64,
        pub is_live: bool,
        #[serde(default)]
        pub ffmpeg_running: bool,
        pub title: String,
        pub area_id: u64,
        pub area_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub online: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub live_start_ts: Option<i64>,
        #[serde(default, skip_serializing_if = "is_false")]
        pub room_locked: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub lock_till: Option<i64>,
        pub stream_quality: Option<String>,
        pub stream_speed: Option<f32>,
        pub stream_cache_speed: Option<f32>,
        pub stream_bitrate_kbps: Option<f32>,
        pub stream_cache_bitrate_kbps: Option<f32>,
        pub stream_fps: Option<f32>,
        pub stream_frame: Option<u64>,
        pub stream_time_secs: Option<u32>,
        pub stream_cache_time_secs: Option<u32>,
        pub hls_cache_active: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub stream_bitrate_history: Vec<f32>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub stream_cache_bitrate_history: Vec<f32>,
        pub enable_danmaku_command: bool,
    }

    pub(crate) fn is_false(value: &bool) -> bool {
        !*value
    }

    impl BiliStatus {
        /// 直播间快照写入卡片；区域名缺失时回落到统一占位文案。
        pub fn apply_live_status(&mut self, live: &BiliLiveStatus) {
            self.is_live = live.is_live;
            self.title = live.title.clone();
            self.area_id = live.area_id;
            self.area_name = if !live.area_name.is_empty() {
                live.area_name.clone()
            } else {
                crate::plugins::area_name_or_unknown(live.area_id)
            };
            self.online = live.is_live.then_some(live.online);
            self.live_start_ts = if live.is_live {
                live.live_start_ts
            } else {
                None
            };
        }
        /// 直播间定时封禁：未来 lock_till 为封禁结束，已结束的封禁清空卡片。
        pub fn apply_room_lock(&mut self, lock: &BiliRoomLock) {
            let timed = crate::plugins::timed_room_lock_wait(lock);
            self.room_locked = lock.is_locked && (lock.lock_till <= 0 || timed.is_some());
            self.lock_till = timed.map(|_| lock.lock_till);
        }

        /// 本机推流器的实时网络指标，与共享的直播间状态相互独立。
        pub fn apply_network(&mut self, network: NetworkStatus) {
            self.ffmpeg_running = network.ffmpeg_running;
            self.stream_speed = network.stream_speed;
            self.stream_cache_speed = network.stream_cache_speed;
            self.stream_bitrate_kbps = network.stream_bitrate_kbps;
            self.stream_cache_bitrate_kbps = network.stream_cache_bitrate_kbps;
            self.stream_fps = network.stream_fps;
            self.stream_frame = network.stream_frame;
            self.stream_time_secs = network.stream_time_secs;
            self.stream_cache_time_secs = network.stream_cache_time_secs;
            self.hls_cache_active = network.hls_cache_active;
            self.stream_bitrate_history = network.stream_bitrate_history;
            self.stream_cache_bitrate_history = network.stream_cache_bitrate_history;
        }
    }

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
    pub struct YtStatus {
        pub is_live: bool,
        #[serde(default)]
        pub enable_monitor: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub warning_held: bool,
        pub title: Option<String>,
        pub topic: Option<String>,
        pub channel_name: String,
        pub channel_id: String,
        pub quality: String,
        pub area_id: u64,
        pub area_name: String,
        pub crop_enabled: bool,
        pub ffmpeg_cache_enabled: bool,
        pub ffmpeg_cache_latency_secs: u64,
    }

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
    pub struct TwStatus {
        pub is_live: bool,
        #[serde(default)]
        pub enable_monitor: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub warning_held: bool,
        pub title: Option<String>,
        pub game: Option<String>,
        pub channel_name: String,
        pub channel_id: String,
        pub quality: String,
        pub area_id: u64,
        pub area_name: String,
        pub crop_enabled: bool,
        pub ffmpeg_cache_enabled: bool,
        pub ffmpeg_cache_latency_secs: u64,
    }

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
    pub struct NicoStatus {
        pub is_live: bool,
        #[serde(default)]
        pub enable_monitor: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub warning_held: bool,
        pub title: Option<String>,
        pub channel_name: String,
        pub channel_id: String,
        #[serde(default)]
        pub live_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub scheduled_start: Option<String>,
        pub quality: String,
        pub area_id: u64,
        pub area_name: String,
        pub crop_enabled: bool,
        pub ffmpeg_cache_enabled: bool,
        pub ffmpeg_cache_latency_secs: u64,
    }

    /// 读取当前缓存快照，并实时合并本机推流指标。
    pub async fn status_snapshot() -> StatusData {
        let mut status = crate::app_state::AppState::current()
            .get_status_cache()
            .unwrap_or_default();
        status
            .bilibili
            .apply_network(super::api::current_network_status().await);
        status
    }

    pub fn update_status_cache(status: StatusData) {
        crate::app_state::AppState::current().update_status_cache(status);
    }

    pub fn update_status_cache_with(update: impl FnOnce(&mut StatusData)) {
        crate::app_state::AppState::current().update_status_cache_with(update);
    }

    pub fn get_status_cache() -> Option<StatusData> {
        crate::app_state::AppState::current().get_status_cache()
    }

    /// 唤醒状态刷新 worker，让外部直播状态立刻重新抓取，而不是等下一个轮询。
    pub fn request_status_refresh() {
        crate::app_state::AppState::current().request_status_refresh();
    }

    /// 有人调用 [`request_status_refresh`] 时返回。
    pub async fn wait_status_refresh_notified() {
        crate::app_state::AppState::current()
            .status_refresh_requested()
            .await;
    }

    /// 把配置里的房间号、平台监控开关与优先频道等真实写进缓存。
    pub fn refresh_status_cache_config_from(cfg: &Config) {
        crate::config::with_current_config(cfg, || {
            update_status_cache_with(|status| apply_status_cache_config(status, cfg));
        });
    }

    pub(crate) fn apply_status_cache_config(cached_status: &mut StatusData, cfg: &Config) {
        cached_status.bilibili.room = cfg.bililive.room.into();
        cached_status.bilibili.enable_danmaku_command = cfg.bililive.enable_danmaku_command;

        if platform_channel_configured(&cfg.youtube.channel_name, &cfg.youtube.channel_id) {
            let yt_area_name = crate::plugins::area_name_or_unknown(cfg.youtube.area_v2);
            if cached_status
                .youtube
                .as_ref()
                .is_some_and(|status| status.channel_id != cfg.youtube.channel_id)
            {
                cached_status.youtube = None;
            }
            if let Some(ref mut yt_status) = cached_status.youtube {
                yt_status.enable_monitor = cfg.youtube.enable_monitor;
                yt_status.channel_name = cfg.youtube.channel_name.clone();
                yt_status.channel_id = cfg.youtube.channel_id.clone();
                yt_status.area_id = cfg.youtube.area_v2;
                yt_status.area_name = yt_area_name;
                yt_status.quality = cfg.youtube.quality.clone();
                yt_status.crop_enabled = cfg.youtube.crop.is_some();
                yt_status.ffmpeg_cache_enabled = cfg.youtube.ffmpeg_cache.enabled;
                yt_status.ffmpeg_cache_latency_secs = cfg.youtube.ffmpeg_cache.latency_secs;
                yt_status.warning_held = crate::plugins::danmaku::source_warning_held(cfg, "YT");
            } else {
                cached_status.youtube = Some(YtStatus {
                    is_live: false,
                    enable_monitor: cfg.youtube.enable_monitor,
                    warning_held: crate::plugins::danmaku::source_warning_held(cfg, "YT"),
                    title: Some("-".to_string()),
                    channel_name: cfg.youtube.channel_name.clone(),
                    channel_id: cfg.youtube.channel_id.clone(),
                    area_id: cfg.youtube.area_v2,
                    area_name: yt_area_name,
                    topic: Some("-".to_string()),
                    quality: cfg.youtube.quality.clone(),
                    crop_enabled: cfg.youtube.crop.is_some(),
                    ffmpeg_cache_enabled: cfg.youtube.ffmpeg_cache.enabled,
                    ffmpeg_cache_latency_secs: cfg.youtube.ffmpeg_cache.latency_secs,
                });
            }
        } else {
            cached_status.youtube = None;
        }

        if platform_channel_configured(&cfg.twitch.channel_name, &cfg.twitch.channel_id) {
            let tw_area_name = crate::plugins::area_name_or_unknown(cfg.twitch.area_v2);
            if cached_status
                .twitch
                .as_ref()
                .is_some_and(|status| status.channel_id != cfg.twitch.channel_id)
            {
                cached_status.twitch = None;
            }
            if let Some(ref mut tw_status) = cached_status.twitch {
                tw_status.enable_monitor = cfg.twitch.enable_monitor;
                tw_status.channel_name = cfg.twitch.channel_name.clone();
                tw_status.channel_id = cfg.twitch.channel_id.clone();
                tw_status.area_id = cfg.twitch.area_v2;
                tw_status.area_name = tw_area_name;
                tw_status.quality = cfg.twitch.quality.clone();
                tw_status.crop_enabled = cfg.twitch.crop.is_some();
                tw_status.ffmpeg_cache_enabled = cfg.twitch.ffmpeg_cache.enabled;
                tw_status.ffmpeg_cache_latency_secs = cfg.twitch.ffmpeg_cache.latency_secs;
                tw_status.warning_held = crate::plugins::danmaku::source_warning_held(cfg, "TW");
            } else {
                cached_status.twitch = Some(TwStatus {
                    is_live: false,
                    enable_monitor: cfg.twitch.enable_monitor,
                    warning_held: crate::plugins::danmaku::source_warning_held(cfg, "TW"),
                    title: Some("-".to_string()),
                    channel_name: cfg.twitch.channel_name.clone(),
                    channel_id: cfg.twitch.channel_id.clone(),
                    area_id: cfg.twitch.area_v2,
                    area_name: tw_area_name,
                    game: Some("-".to_string()),
                    quality: cfg.twitch.quality.clone(),
                    crop_enabled: cfg.twitch.crop.is_some(),
                    ffmpeg_cache_enabled: cfg.twitch.ffmpeg_cache.enabled,
                    ffmpeg_cache_latency_secs: cfg.twitch.ffmpeg_cache.latency_secs,
                });
            }
        } else {
            cached_status.twitch = None;
        }

        if cached_status
            .priority_channel
            .as_ref()
            .is_some_and(|status| status.channel_name != cfg.priority_channel.channel_name)
        {
            cached_status.priority_channel = None;
        }
        if let Some(ref mut priority_status) = cached_status.priority_channel {
            priority_status.enabled = cfg.priority_channel.enabled;
            priority_status.auto_restart = cfg.priority_channel.auto_restart;
            priority_status.warning_held = crate::plugins::danmaku::priority_warning_held(cfg);
            if !priority_status.enabled {
                priority_status.is_live = false;
                priority_status.platform = None;
                priority_status.title = None;
            }
            priority_status.channel_name = cfg.priority_channel.channel_name.clone();
            priority_status.default_area = cfg.priority_channel.default_area;
            priority_status.default_area_name =
                crate::plugins::get_area_name(cfg.priority_channel.default_area);
        } else {
            cached_status.priority_channel = Some(PriorityChannelStatus {
                enabled: cfg.priority_channel.enabled,
                auto_restart: cfg.priority_channel.auto_restart,
                warning_held: crate::plugins::danmaku::priority_warning_held(cfg),
                channel_name: cfg.priority_channel.channel_name.clone(),
                is_live: false,
                platform: None,
                title: None,
                default_area: cfg.priority_channel.default_area,
                default_area_name: crate::plugins::get_area_name(
                    cfg.priority_channel.default_area,
                ),
            });
        }

        if crate::plugins::niconico_configured(&cfg.niconico) {
            let nico_area_name = crate::plugins::area_name_or_unknown(cfg.niconico.area_v2);
            let (nico_channel_id, nico_channel_name) =
                crate::plugins::niconico_channel_identity(&cfg.niconico);
            if cached_status
                .niconico
                .as_ref()
                .is_some_and(|status| status.channel_id != nico_channel_id)
            {
                cached_status.niconico = None;
            }
            if let Some(ref mut nico_status) = cached_status.niconico {
                nico_status.enable_monitor = cfg.niconico.enable_monitor;
                nico_status.channel_name = nico_channel_name.clone();
                nico_status.channel_id = nico_channel_id.clone();
                nico_status.area_id = cfg.niconico.area_v2;
                nico_status.area_name = nico_area_name;
                nico_status.quality = cfg.niconico.quality.clone();
                nico_status.crop_enabled = cfg.niconico.crop.is_some();
                nico_status.ffmpeg_cache_enabled = cfg.niconico.ffmpeg_cache.enabled;
                nico_status.ffmpeg_cache_latency_secs = cfg.niconico.ffmpeg_cache.latency_secs;
                nico_status.warning_held = crate::plugins::danmaku::source_warning_held(cfg, "NC");
            } else {
                cached_status.niconico = Some(NicoStatus {
                    is_live: false,
                    enable_monitor: cfg.niconico.enable_monitor,
                    warning_held: crate::plugins::danmaku::source_warning_held(cfg, "NC"),
                    title: Some("-".to_string()),
                    channel_name: nico_channel_name,
                    channel_id: nico_channel_id,
                    live_id: None,
                    scheduled_start: None,
                    area_id: cfg.niconico.area_v2,
                    area_name: nico_area_name,
                    quality: cfg.niconico.quality.clone(),
                    crop_enabled: cfg.niconico.crop.is_some(),
                    ffmpeg_cache_enabled: cfg.niconico.ffmpeg_cache.enabled,
                    ffmpeg_cache_latency_secs: cfg.niconico.ffmpeg_cache.latency_secs,
                });
            }
        } else {
            cached_status.niconico = None;
        }
    }

    fn platform_channel_configured(channel_name: &str, channel_id: &str) -> bool {
        !channel_name.trim().is_empty() || !channel_id.trim().is_empty()
    }
}

pub mod stream_list {
    /// 待 Phase 2 流列表唤醒迁入后替换。
    pub fn wake() {}

    /// 待 Phase 2 流列表租约迁入后替换；单机没有面板在读列表，名册按需跳过。
    pub fn any_leased() -> bool {
        false
    }
}

pub mod listen {
    use std::net::{IpAddr, Ipv4Addr};

    /// 待 Phase 2 Web 监听迁入后替换；单机 WebSub 回调只绑回环。
    pub fn listen_bind() -> IpAddr {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    }
}

pub mod public {
    /// 待 Phase 2 公开状态页迁入后替换。
    pub fn remap_after_areas_change() {}

    pub mod snapshot {
        /// 待 Phase 2 公开状态快照迁入后替换。
        pub fn invalidate_public_status_snapshot() {}
    }
}

#[cfg(test)]
mod tests {
    use super::state::{
        apply_status_cache_config, get_status_cache, status_snapshot, update_status_cache,
        update_status_cache_with, wait_status_refresh_notified, BiliStatus, NetworkStatus,
        StatusData,
    };
    use super::state::request_status_refresh;
    use std::sync::Mutex;
    use std::time::Duration;

    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn apply_network_fills_every_stream_field() {
        let mut status = BiliStatus::default();
        status.apply_network(NetworkStatus {
            ffmpeg_running: true,
            stream_speed: Some(1.5),
            stream_cache_speed: Some(0.5),
            stream_bitrate_kbps: Some(3000.0),
            stream_cache_bitrate_kbps: Some(1200.0),
            stream_fps: Some(60.0),
            stream_frame: Some(1234),
            stream_time_secs: Some(56),
            stream_cache_time_secs: Some(12),
            hls_cache_active: true,
            stream_bitrate_history: vec![1.0, 2.0],
            stream_cache_bitrate_history: vec![3.0],
        });

        assert!(status.ffmpeg_running);
        assert_eq!(status.stream_speed, Some(1.5));
        assert_eq!(status.stream_cache_speed, Some(0.5));
        assert_eq!(status.stream_bitrate_kbps, Some(3000.0));
        assert_eq!(status.stream_cache_bitrate_kbps, Some(1200.0));
        assert_eq!(status.stream_fps, Some(60.0));
        assert_eq!(status.stream_frame, Some(1234));
        assert_eq!(status.stream_time_secs, Some(56));
        assert_eq!(status.stream_cache_time_secs, Some(12));
        assert!(status.hls_cache_active);
        assert_eq!(status.stream_bitrate_history, vec![1.0, 2.0]);
        assert_eq!(status.stream_cache_bitrate_history, vec![3.0]);
    }

    #[test]
    fn apply_status_cache_config_writes_room_and_platform_switches() {
        let _guard = serial();
        let mut cfg = crate::test_support::config("p2-01", 0);
        cfg.bililive.room = 4321;
        cfg.bililive.enable_danmaku_command = true;
        cfg.priority_channel.enabled = true;
        cfg.priority_channel.channel_name = "priority".to_string();

        let mut status = StatusData::default();
        apply_status_cache_config(&mut status, &cfg);

        assert_eq!(status.bilibili.room, 4321);
        assert!(status.bilibili.enable_danmaku_command);
        let youtube = status.youtube.expect("youtube 状态必须写入");
        assert_eq!(youtube.channel_id, "ytid");
        assert!(youtube.enable_monitor);
        assert_eq!(
            status.priority_channel.expect("优先频道状态").channel_name,
            "priority"
        );
    }

    #[tokio::test]
    async fn status_cache_round_trips_and_snapshot_merges_live_network() {
        let _guard = serial();
        update_status_cache(StatusData::default());
        update_status_cache_with(|status| {
            status.bilibili.room = 4242;
            status.bilibili.title = "单元测试房间".to_string();
        });

        let cached = get_status_cache().expect("缓存必须真实写入");
        assert_eq!(cached.bilibili.room, 4242);

        let live = super::api::current_network_status().await;
        let snapshot = status_snapshot().await;
        assert_eq!(snapshot.bilibili.room, 4242);
        assert_eq!(snapshot.bilibili.title, "单元测试房间");
        assert_eq!(snapshot.bilibili.ffmpeg_running, live.ffmpeg_running);
    }

    #[tokio::test]
    async fn wait_status_refresh_notified_wakes_on_request() {
        let _guard = serial();
        request_status_refresh();
        tokio::time::timeout(Duration::from_secs(1), wait_status_refresh_notified())
            .await
            .expect("刷新请求必须唤醒等待方");
    }
}
