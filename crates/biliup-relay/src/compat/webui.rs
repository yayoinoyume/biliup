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

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct StatusData {
        pub bilibili: BiliStatus,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct BiliStatus {
        pub title: String,
        pub is_live: bool,
    }

    static CACHE: Mutex<Option<StatusData>> = Mutex::new(None);

    /// 待 Phase 2 状态缓存迁入后替换；单进程内保留最近一次写入。
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
}

pub mod public {
    /// 待 Phase 2 公开状态页迁入后替换。
    pub fn remap_after_areas_change() {}

    pub mod snapshot {
        /// 待 Phase 2 公开状态快照迁入后替换。
        pub fn invalidate_public_status_snapshot() {}
    }
}
