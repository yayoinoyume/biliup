//! 薄适配边界：为尚未迁入本 crate 的模块提供最小实现。
//!
//! 每个子模块只保留已迁入代码真正调用到的那一小撮 API，行为尽量保持真实
//! （监听地址校验、Netscape Cookie 格式校验都是真的），不写空过假校验。
//! 替换时机：
//! - `webui`：P1-02 / P1-03 迁入 Web 服务后删除；
//! - `cluster`：Phase 2 集群同步迁入后删除；
//! - `youtube_cookies` / `priority` / `bilibili`：Phase 2 对应插件迁入后删除。

pub(crate) mod bilibili;
pub(crate) mod cluster;
pub(crate) mod priority;
pub(crate) mod webui;
pub(crate) mod youtube_cookies;
