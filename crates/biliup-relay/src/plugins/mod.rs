//! 通用基础插件 A 组。
pub mod banned_keywords;
pub(crate) mod channel_id;
pub mod http;
pub(crate) mod mask_pairing;
pub(crate) mod source_status;
pub(crate) mod stream_item;
pub mod subprocess;
pub(crate) mod violation_notice;
pub mod warning_exempt;
pub mod wbi;

use std::sync::atomic::{AtomicBool, Ordering};

static CONFIG_UPDATED: AtomicBool = AtomicBool::new(false);

/// 标记配置已更新，监控循环据此跳过本轮等待（消费方在 Phase 2 迁入）。
pub fn set_config_updated() {
    CONFIG_UPDATED.store(true, Ordering::SeqCst);
}

pub fn is_config_updated() -> bool {
    CONFIG_UPDATED.load(Ordering::SeqCst)
}

pub fn clear_config_updated() {
    CONFIG_UPDATED.store(false, Ordering::SeqCst);
}
