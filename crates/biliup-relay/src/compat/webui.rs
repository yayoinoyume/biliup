//! 待 P1-02 / P1-03 迁入 Web 服务后替换。

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

pub mod events {
    /// 状态缓存变化事件名。
    pub const STATUS: &str = "status";
    /// 配置变化事件名。
    pub const CONFIG: &str = "config";

    /// 待 P1-02 Web 事件通道迁入后替换。
    pub fn publish(_kind: &'static str) {}
}

pub mod state {
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct StatusData {
        pub bilibili: BiliStatus,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct BiliStatus {
        pub title: String,
    }

    /// 待 P1-02 状态刷新信号迁入后替换。
    pub fn request_status_refresh() {}
}

pub mod stream_list {
    /// 待 P1-02 流列表唤醒迁入后替换。
    pub fn wake() {}
}

pub mod public {
    /// 待 P1-03 公开状态页迁入后替换。
    pub fn remap_after_areas_change() {}
}
