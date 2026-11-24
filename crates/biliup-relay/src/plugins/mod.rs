//! 通用插件：A 组基础能力 + B 组 B 站直播控制与弹幕客户端。

pub mod banned_keywords;
pub mod bilibili;
pub(crate) mod channel_id;
pub(crate) mod danmaku;
pub(crate) mod danmaku_client;
pub mod http;
pub(crate) mod mask_pairing;
pub(crate) mod source_status;
pub(crate) mod stream_item;
pub mod subprocess;
pub(crate) mod violation_notice;
pub mod warning_exempt;
pub mod wbi;

pub use bilibili::*;
pub use danmaku::*;
pub use danmaku_client::*;
