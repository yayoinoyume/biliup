//! 通知层尚未迁入时的薄适配；待 Phase 4 通知迁入后替换。
//!
//! 源仓库的 live_source / live_ended 本身只是把选源与下播事实转发到事件总线，
//! 因此这里保留同一真实语义，QQ 机器人消费与去重待 Phase 4 接入。

use crate::events::{BroadcastEndCause, EndDetail, EventKind};

pub mod live_events {
    //! 运行时选源时已知的源事实；不含任何 B 站推流密钥。

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct LiveSource {
        pub stream_key: String,
        pub platform: String,
        pub channel: String,
        pub title: String,
    }
}

/// 待 Phase 4 通知选举迁入后替换：单机永远不是集群的 recorder 角色。
pub fn is_recorder() -> bool {
    false
}

/// 运行时选中该源且 B 站直播间在线：开播或会话内换台；QQ 机器人消费待 Phase 4。
pub fn live_source(info: live_events::LiveSource) {
    crate::events::emit(EventKind::BroadcastStarted {
        stream_key: crate::logging::safe_text(&info.stream_key, 128),
        platform: crate::logging::safe_text(&info.platform, 20),
        channel: crate::logging::safe_text(&info.channel, 80),
        title: crate::logging::safe_text(&info.title, 160),
    });
}

/// B 站直播间下播：只有已开的会话才结束；通知发送待 Phase 4。
pub fn live_ended(cause: BroadcastEndCause) {
    live_ended_with(cause, None);
}

/// 带 B 站警告/切断原因的下播边界。
pub fn live_ended_with(cause: BroadcastEndCause, detail: Option<EndDetail>) {
    crate::events::emit(EventKind::BroadcastEnded { cause, detail });
}

/// 待 Phase 4 通知选举迁入后替换：单机没有集群 recorder 角色。
pub(crate) fn recorder_status() -> &'static str {
    "none"
}

/// 待 Phase 4 通知迁入后替换：本阶段没有指令/网关队列。
pub(crate) fn dropped() -> serde_json::Value {
    serde_json::json!({"commands": 0, "gateway": 0, "events": 0})
}
