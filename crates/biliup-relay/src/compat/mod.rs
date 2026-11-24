//! 薄适配边界：为尚未迁入本 crate 的模块提供最小实现。
//!
//! 每个子模块只保留已迁入代码真正调用到的那一小撮 API，行为尽量保持真实
//! （单机集群角色、通知下播事实都是真的），不写空过假校验。
//! 替换时机：
//! - `cluster`：Phase 3 集群同步迁入后删除；
//! - `notify`：Phase 4 通知迁入后删除。
//!
//! WebUI 已在 P2-03 换成真实 `crate::webui` 模块树，不再是适配层。

pub(crate) mod cluster;
pub(crate) mod notify;
