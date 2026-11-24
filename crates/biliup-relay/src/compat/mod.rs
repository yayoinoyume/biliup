//! 薄适配边界：为尚未迁入本 crate 的模块提供最小实现。
//!
//! 每个子模块只保留已迁入代码真正调用到的那一小撮 API，行为尽量保持真实
//! （监听地址校验、单机集群角色、面板租约都是真的），不写空过假校验。
//! 替换时机：
//! - `webui`：Phase 2 Web 服务迁入后删除；
//! - `cluster`：Phase 2 集群同步迁入后删除；
//! - `notify`：Phase 4 通知迁入后删除。

pub(crate) mod cluster;
pub(crate) mod notify;
pub(crate) mod webui;
