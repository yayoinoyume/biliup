#![allow(dead_code)]
//! 待 Phase 2 集群同步迁入后替换。
//! 本阶段忽略 cfg.cluster.*，恒按单机语义回答。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::{ClusterConfig, Config};
use crate::plugins::stream_item::StreamItem;
use crate::plugins::youtube_data::YtVideo;

#[derive(Debug, Default)]
pub struct ClusterState {
    pub local_draining: bool,
}

/// 待 Phase 3 集群成员生命周期迁入后替换：单机只有独立节点一种状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterLifecycle {
    Standalone,
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

    /// 待 Phase 3 集群成员关系迁入后替换：单机恒为独立节点。
    pub fn current_lifecycle() -> crate::compat::cluster::ClusterLifecycle {
        crate::compat::cluster::ClusterLifecycle::Standalone
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

/// 监控相关字段的归一化哈希；与源仓库 cluster/version.rs 的 payload 逐字段一致，
/// 因此同一份监控配置在两个阶段之间会得到相同的版本号。
pub fn monitored_config_version(config: &Config) -> String {
    let value = serde_json::json!({
        "youtube": {
            "channel_name": config.youtube.channel_name,
            "channel_id": config.youtube.channel_id,
        },
        "twitch": {
            "channel_name": config.twitch.channel_name,
            "channel_id": config.twitch.channel_id,
        },
        "priority_channel": {
            "channel_name": config.priority_channel.channel_name,
            "youtube_channel_id": config.priority_channel.youtube_channel_id,
            "twitch_channel_id": config.priority_channel.twitch_channel_id,
            "default_area": config.priority_channel.default_area,
        },
        "niconico": {
            "channel_name": config.niconico.channel_name,
            "channel_id": config.niconico.channel_id,
            "live_id": config.niconico.live_id,
            "area_v2": config.niconico.area_v2,
        },
        "channels_json": read_json_file("channels.json"),
        "areas_json": read_json_file("areas.json"),
    });
    canonical_value_hash(&value)
}

/// 待 Phase 2 集群监控配置同步迁入后替换：单机无需同步。
pub async fn sync_monitored_config_after_change(_config: &Config) -> String {
    String::new()
}

/// 待 Phase 5 集群直播簿同步迁入后替换：单机没有对等节点，恒为空。
pub fn peer_live_books(_node_id: &str) -> Vec<crate::history::live_book::BiliLiveBook> {
    Vec::new()
}

/// 单机没有集群：本节点始终可以监控/推流。
pub fn local_monitoring_allowed(config: &Config) -> bool {
    local_monitoring_block_reason(config).is_none()
}

/// 单机没有集群归属：None 表示本节点可以继续监控/推流。
pub fn local_monitoring_block_reason(_config: &Config) -> Option<String> {
    None
}

/// 单机条件始终就绪，立即返回；签名与源仓库一致，供 runtime 统一调用。
pub async fn wait_for_local_monitoring(_config: &Config) {}

/// 单机不统计失败重启次数。
pub fn record_stream_exit(_success: bool) {}

/// 单机没有集群所有权：永远允许本机推流。
pub fn local_may_push(_config: &Config, _stream: Option<ClusterStreamIdentity>) -> bool {
    true
}

/// 单机没有集群所有权需要清除。
pub fn clear_local_stream() {}

/// 源仓库 cluster/types.rs 的流身份；本阶段只出现在所有权判断签名与日志里。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ClusterStreamIdentity {
    pub platform: String,
    pub channel_name: String,
    pub channel_id: String,
    pub stream_id: Option<String>,
    pub title: Option<String>,
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

/// 待 Phase 2 集群状态库迁入后替换：版本哈希只覆盖真正的共享监控字段，
/// 与源仓库 cluster/version.rs 的 monitored_sync_value_from_config 同形。
fn read_json_file(name: &str) -> Option<serde_json::Value> {
    crate::storage::read_document(name).ok()
}

fn canonical_value_hash(value: &serde_json::Value) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    canonical_json(value).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn canonical_json(value: &serde_json::Value) -> String {
    let mut output = String::new();
    write_canonical_json(value, &mut output);
    output
}

fn write_canonical_json(value: &serde_json::Value, output: &mut String) {
    match value {
        serde_json::Value::Null => output.push_str("null"),
        serde_json::Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        serde_json::Value::Number(value) => output.push_str(&value.to_string()),
        serde_json::Value::String(value) => {
            output.push_str(&serde_json::to_string(value).unwrap_or_default());
        }
        serde_json::Value::Array(values) => {
            output.push('[');
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json(item, output);
            }
            output.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            output.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).unwrap_or_default());
                output.push(':');
                write_canonical_json(&map[key], output);
            }
            output.push('}');
        }
    }
}

/// 待 Phase 3 集群状态迁入后替换：单机恒为本机的活动节点。
pub fn local_node_is_active_owner(_config: &Config) -> bool {
    true
}

/// 待 Phase 3 集群状态迁入后替换：单机永远不是备用节点。
pub fn is_standby(_config: &Config) -> bool {
    false
}

/// 待 Phase 3 集群状态迁入后替换：单机是活动节点，监控开关永远可以打开。
pub fn local_node_can_enable_monitor_toggles(_config: &Config) -> bool {
    true
}

/// 待 Phase 3 集群监控开关同步迁入后替换：单机没有对端需要同步。
pub fn schedule_active_monitor_state_sync_after_toggle_change(_config: &Config) -> String {
    String::new()
}

/// 待 Phase 3 集群对等调用迁入后替换：单机没有对等节点，所有调用一律失败。
pub mod peer_call {
    use std::time::Duration;

    use serde::Serialize;
    use serde::de::DeserializeOwned;

    use crate::config::Config;

    pub mod routes {
        pub const HISTORY: &str = "/api/cluster/v1/history";
        pub const HISTORY_CLEAN: &str = "/api/cluster/v1/history/clean";
    }

    #[derive(Debug)]
    pub enum PeerCallError {
        Unreachable(String),
        Refused(String),
    }

    #[derive(Debug)]
    pub struct Envelope<T> {
        pub success: bool,
        pub data: Option<T>,
        pub message: Option<String>,
    }

    /// 单机没有对等节点：调用恒为「不可达」，调用方按缺失节点上报。
    pub async fn call<T, R>(
        _config: &Config,
        _node: &str,
        _route: &str,
        _body: Option<&T>,
        _timeout: Duration,
    ) -> Result<Envelope<R>, PeerCallError>
    where
        T: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        Err(PeerCallError::Unreachable("单机没有对等节点".to_string()))
    }
}

/// 待 Phase 3 集群设置围栏迁入后替换：单机没有成员关系，任何编辑都直接接受。
pub fn fence_cluster_edit(
    _current: &ClusterConfig,
    next: ClusterConfig,
    _lifecycle: ClusterLifecycle,
) -> Result<ClusterConfig, &'static str> {
    Ok(next)
}

/// 待 Phase 3 集群共享设置版本迁入后替换：单机没有对端比较版本。
pub fn shared_settings_version(_config: &Config) -> String {
    String::new()
}

/// 待 Phase 3 集群设置同步迁入后替换：单机没有对端需要下发。
pub async fn push_cluster_settings_to_peers(_config: &Config) -> Result<(), String> {
    Ok(())
}

/// 待 Phase 3 集群监控配置同步迁入后替换：单机没有对端需要下发。
pub async fn push_monitored_config_to_peers(_config: &Config) -> Result<(), String> {
    Ok(())
}

/// 待 Phase 3 集群弹幕命令运行时同步迁入后替换：单机直接改本地门闸与客户端。
pub async fn apply_danmaku_command_runtime_state(enabled: bool) {
    crate::plugins::enable_danmaku_commands(enabled);
    if enabled {
        if !crate::plugins::is_danmaku_running() {
            crate::plugins::run_danmaku();
        }
    } else if crate::plugins::is_danmaku_running() {
        crate::plugins::stop_danmaku().await;
    }
}

/// 待 Phase 3 集群围栏迁入后替换：单机不发围栏承诺，所以能力表不带该字段。
pub fn membership_fencing_capability() -> Option<serde_json::Value> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 恢复门闸与运行态，断言失败也不把全局状态留给其它测试。
    struct GateReset {
        commands_enabled: bool,
        running: bool,
    }

    impl Drop for GateReset {
        fn drop(&mut self) {
            crate::plugins::set_danmaku_commands_enabled(self.commands_enabled);
            crate::plugins::set_danmaku_running(self.running);
            crate::plugins::set_danmaku_stop_signal(false);
        }
    }

    /// 只走关闭方向：开启方向会真的启动弹幕客户端线程并联网，单测不得触发。
    #[tokio::test]
    async fn apply_danmaku_command_runtime_state_flips_the_gate_and_stops_cleanly_offline() {
        let _guard = crate::storage::lock_test_global();
        let _reset = GateReset {
            commands_enabled: crate::plugins::is_danmaku_commands_enabled(),
            running: crate::plugins::is_danmaku_running(),
        };
        crate::plugins::set_danmaku_running(false);
        crate::plugins::set_danmaku_commands_enabled(true);

        apply_danmaku_command_runtime_state(false).await;

        assert!(
            !crate::plugins::is_danmaku_commands_enabled(),
            "关闭弹幕指令必须立即翻转门闸，不能等下一轮 run_bilistream"
        );
        assert!(
            !crate::plugins::is_danmaku_running(),
            "关闭路径不得启动弹幕客户端"
        );
        assert!(
            !crate::plugins::should_stop_danmaku(),
            "没有客户端在跑时不得留下停止信号"
        );
    }
}
