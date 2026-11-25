//! 面板 Web 服务：状态缓存、SSE 变更流、直播与预告列表与第一批 HTTP 处理器。
//!
//! 路由挂载、鉴权中间件与静态资源由 P2-05 接入；本模块只提供处理器本体。

pub mod api;
pub mod events;
pub mod state;
pub mod stream_list;

use std::net::{IpAddr, Ipv4Addr};

/// 面板处理器共用的进程级状态：日志缓冲、状态缓存与事件总线。
///
/// 宿主只拿这一个入口，不需要认识 `app_state` 模块的内部结构。
pub fn app_state() -> crate::app_state::AppState {
    crate::app_state::AppState::current()
}

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

/// 待 P2-05 Web 监听与鉴权迁入后替换；单机 WebSub 回调只绑回环。
pub mod listen {
    use std::net::{IpAddr, Ipv4Addr};

    pub fn listen_bind() -> IpAddr {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    }
}

/// 待 Phase 2 公开状态页迁入后替换；调用方只需要这两个失效钩子。
pub mod public {
    pub fn remap_after_areas_change() {}

    pub mod snapshot {
        pub fn invalidate_public_status_snapshot() {}
    }
}

#[cfg(test)]
mod tests {
    use super::parse_bind;

    #[test]
    fn parse_bind_accepts_only_localhost_or_a_literal_ip() {
        assert_eq!(
            parse_bind(" localhost ").unwrap(),
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        );
        assert_eq!(
            parse_bind("0.0.0.0").unwrap(),
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        );
        assert!(parse_bind("").is_err());
        assert!(parse_bind("example.com").is_err());
    }
}
