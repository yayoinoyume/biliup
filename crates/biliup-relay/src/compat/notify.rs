//! 通知层尚未迁入时的薄适配；待 Phase 4 通知迁入后替换。
//!
//! 源仓库的 `live_ended_with` 本身只是把下播原因转发到事件总线，
//! 因此这里保留同一真实语义，通知发送/去重仍由 Phase 4 负责。

use crate::events::{BroadcastEndCause, EndDetail, EventKind};

pub fn live_ended_with(cause: BroadcastEndCause, detail: Option<EndDetail>) {
    crate::events::emit(EventKind::BroadcastEnded { cause, detail });
}
