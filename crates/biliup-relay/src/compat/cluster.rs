#![allow(dead_code)]
//! 待 Phase 2 集群同步迁入后替换。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::config::Config;
use crate::plugins::stream_item::StreamItem;
use crate::plugins::youtube_data::YtVideo;

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

/// 待 Phase 2 监控配置版本迁入后替换：单机没有需要同步的版本。
pub fn monitored_config_version(_config: &Config) -> String {
    String::new()
}

/// 待 Phase 2 集群监控配置同步迁入后替换：单机无需同步。
pub async fn sync_monitored_config_after_change(_config: &Config) -> String {
    String::new()
}

/// 待 Phase 2 集群所有权判断迁入后替换：单机永远允许本机监控。
pub fn local_monitoring_allowed(_config: &Config) -> bool {
    true
}

/// 待 Phase 5 集群 YouTube 索引迁入后替换：本机如何回答 YouTube 问题。
#[derive(Clone, Debug, Default)]
pub(crate) enum YtIndexRole {
    /// 只替自己回答，等同无集群。
    #[default]
    Standalone,
    /// 替整个集群回答：公开状态节点且持有可用 key。
    Owner,
    /// 从 owner 的索引回答，不调用 Google。
    Peer(Arc<YtIndexPayload>),
}

impl YtIndexRole {
    pub(crate) fn is_peer(&self) -> bool {
        matches!(self, Self::Peer(_))
    }
}

/// 待 Phase 5 集群 YouTube 索引迁入后替换：owner 发布给 peer 的索引载荷。
#[derive(Clone, Debug, Default)]
pub(crate) struct YtIndexPayload {
    pub(crate) version: String,
    /// videos.list 按视频 ID 的回答；None 表示 YouTube 没给出该 ID。
    pub(crate) videos: BTreeMap<String, Option<YtVideo>>,
    /// 发现流里新近直播的行，由 owner 并入 Holodex 结果。
    pub(crate) discovered: Vec<StreamItem>,
}

impl YtIndexPayload {
    /// owner 对 ids 的回答；没有回答的 ID 留在外面，覆盖层保留 Holodex 的值。
    pub(crate) fn answers<'a>(
        &self,
        ids: impl IntoIterator<Item = &'a String>,
    ) -> HashMap<String, YtVideo> {
        ids.into_iter()
            .filter_map(|id| Some((id.clone(), self.videos.get(id)?.clone()?)))
            .collect()
    }
}

#[cfg(test)]
tokio::task_local! {
    static TEST_ROLE: YtIndexRole;
}

/// 待 Phase 5 集群 YouTube 索引迁入后替换：单机没有集群，永远是 Standalone。
pub(crate) fn yt_index_role() -> YtIndexRole {
    #[cfg(test)]
    if let Ok(role) = TEST_ROLE.try_with(YtIndexRole::clone) {
        return role;
    }
    YtIndexRole::Standalone
}

/// 待 Phase 5 集群 YouTube 索引迁入后替换；测试里假装本机持有 role。
#[cfg(test)]
pub(crate) async fn with_yt_index_role<F: std::future::Future>(
    role: YtIndexRole,
    f: F,
) -> F::Output {
    TEST_ROLE.scope(role, f).await
}

/// 待 Phase 5 集群 YouTube 索引迁入后替换：单机只按本机配置的频道监控。
pub(crate) fn cluster_monitored_channels(cfg: &Config) -> HashSet<String> {
    crate::plugins::youtube::monitored_channels(cfg)
}

/// 待 Phase 5 集群 WebSub 归属迁入后替换：单机没有集群归属，订阅边界始终放行。
pub(crate) fn may_subscribe_websub(_cfg: &Config) -> bool {
    true
}
