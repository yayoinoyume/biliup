#![allow(dead_code)]
//! 待 Phase 2 Web 服务迁入后替换的薄适配。

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;

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
    /// 状态缓存变化事件名。
    pub const STATUS: &str = "status";
    /// 配置变化事件名。
    pub const CONFIG: &str = "config";

    /// 待 Phase 2 Web 事件通道迁入后替换。
    pub fn publish(_kind: &'static str) {}
}

pub mod state {
    use super::*;
    use serde::{Deserialize, Serialize};

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

    static CACHE: Mutex<Option<StatusData>> = Mutex::new(None);

    /// 待 Phase 2 状态缓存迁入后替换；单进程内保留最近一次写入。
    pub fn update_status_cache(status: StatusData) {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        *cache = Some(status);
    }

    pub fn update_status_cache_with(update: impl FnOnce(&mut StatusData)) {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        update(cache.get_or_insert_with(StatusData::default));
    }

    /// 待 Phase 2 状态刷新信号迁入后替换。
    pub fn request_status_refresh() {}
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
