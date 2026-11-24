//! UI 变更广播总线：纯内存、零认证耦合，供 SSE 与后台任务共同使用。
//!
//! 独立于 `app_state`，因此后续迁入 `history` 等模块时可以只依赖本模块，
//! 不会把整条 WebUI 依赖链带进来。

use std::sync::OnceLock;

use tokio::sync::broadcast;

/// 面板状态快照变化（直播状态、标题、开关）。
pub const STATUS: &'static str = "status";
/// 集群拓扑或节点状态变化。
pub const CLUSTER: &'static str = "cluster";
/// 持久化配置变化。
pub const CONFIG: &'static str = "config";
/// 直播与预告列表变化。
pub const HOLODEX: &'static str = "holodex";
/// 统计历史提交了新行。
pub const HISTORY: &'static str = "history";

const EVENT_BUS_CAPACITY: usize = 64;

static BUS: OnceLock<broadcast::Sender<&'static str>> = OnceLock::new();

fn bus() -> &'static broadcast::Sender<&'static str> {
    BUS.get_or_init(|| broadcast::channel(EVENT_BUS_CAPACITY).0)
}

/// 通知所有订阅者某类状态已变化；没有订阅者时静默丢弃，不影响调用方。
pub fn publish(kind: &'static str) {
    let _ = bus().send(kind);
}

/// 订阅变更事件；返回的接收端落后太多时会收到 `Lagged`，调用方自行跳过。
pub fn subscribe() -> broadcast::Receiver<&'static str> {
    bus().subscribe()
}

#[cfg(test)]
mod tests {
    use super::*;

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn publish_reaches_every_subscriber_with_the_exact_kind() {
        let _guard = serial();
        let mut rx = subscribe();
        publish(STATUS);
        assert_eq!(rx.try_recv().expect("status 事件"), STATUS);

        let mut history_rx = subscribe();
        publish(HISTORY);
        assert_eq!(history_rx.try_recv().expect("history 事件"), HISTORY);
        assert_eq!(rx.try_recv().expect("status 订阅端也应收到 history"), HISTORY);
    }

    #[test]
    fn publish_without_subscribers_is_silent() {
        let _guard = serial();
        publish(HOLODEX);
        publish(CLUSTER);
        publish(CONFIG);
    }

    #[test]
    fn categories_match_the_upstream_event_names() {
        let _guard = serial();
        assert_eq!(
            [STATUS, CLUSTER, CONFIG, HOLODEX, HISTORY],
            ["status", "cluster", "config", "holodex", "history"]
        );
    }
}

