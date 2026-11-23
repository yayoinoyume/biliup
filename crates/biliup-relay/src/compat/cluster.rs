//! 待 Phase 2 集群同步迁入后替换。

#[derive(Debug, Default)]
pub struct ClusterState {
    pub local_draining: bool,
}

pub mod membership {
    use std::sync::Arc;

    use crate::storage::Store;

    /// 待 Phase 2 成员关系引擎迁入后替换；当前只持有存储句柄。
    pub struct Membership {
        _store: Arc<Store>,
    }

    impl Membership {
        pub fn new(store: Arc<Store>) -> Self {
            Self { _store: store }
        }

        pub fn initialize(&self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

/// 待 Phase 2 心跳豁免冷却同步迁入后替换：本机没有集群，永远不命中。
pub fn peer_exempt_cooldowns_hit(
    _platform: &str,
    _channel_id: &str,
    _episode_id: Option<&str>,
    _now: u64,
) -> bool {
    false
}
