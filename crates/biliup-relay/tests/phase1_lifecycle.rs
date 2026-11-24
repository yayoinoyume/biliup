//! Phase 1 收官端到端本地集成测试：纯本地临时目录，零外网、零真实凭据。
//!
//! 覆盖四件事：
//! 1. `RelayService::start` 用独立临时 `data_dir` 与数据目录之外的 `key_file` 初始化加密存储；
//! 2. 配置写入与重载真实生效，并触发已迁入的真实优先频道预取槽清理；
//! 3. 空闲态没有本地推流；
//! 4. `shutdown` 停弹幕、停推流、释放存储后，同进程可以干净重启且数据完整保留。

use biliup_relay::compat::webui::api::current_network_status;
use biliup_relay::config::{load_config, save_config, Config};
use biliup_relay::plugins::priority::{
    store_prefetched_playable_stream, take_prefetched_playable_stream, PrefetchedPlayableStream,
    PriorityChannelPlatform,
};
use biliup_relay::{RelayInitConfig, RelayService};

const CHANNEL_ID: &str = "UC_phase1_lifecycle";

/// 只写没有 serde 默认值的字段，其余交给 Config 的默认值。
fn seed_config_json() -> serde_json::Value {
    serde_json::json!({
        "auto_cover": false,
        "enable_anti_collision": false,
        "interval": 60,
        "bililive": {
            "enable_danmaku_command": false,
            "room": 1,
            "bili_rtmp_url": "",
            "bili_rtmp_key": ""
        },
        "twitch": {},
        "youtube": {},
        "holodex_api_key": null,
        "riot_api_key": null,
        "enable_lol_monitor": false,
        "lol_monitor_interval": null,
        "anti_collision_list": {}
    })
}

fn prefetched() -> PrefetchedPlayableStream {
    PrefetchedPlayableStream {
        platform: PriorityChannelPlatform::Youtube,
        channel_id: CHANNEL_ID.to_string(),
        topic: None,
        title: Some("示例直播".to_string()),
        m3u8_url: "https://example.invalid/phase1.m3u8".to_string(),
        stream_id: Some("phase1-video".to_string()),
    }
}

#[tokio::test]
async fn relay_lifecycle_persists_config_and_clears_prefetch() {
    let dir = tempfile::tempdir().expect("临时目录");
    let data_dir = dir.path().join("relay");
    let key_file = dir.path().join("keys/master.key");
    let init = RelayInitConfig {
        data_dir: data_dir.clone(),
        key_file: Some(key_file.clone()),
        ffmpeg_path: None,
    };

    let relay = RelayService::new();
    relay.start(init.clone()).await.expect("第一次启动应成功");
    assert!(key_file.exists(), "显式 key_file 必须落在数据目录之外");

    // 1) 加密存储里的配置写入与重载。
    let seeded: Config =
        serde_json::from_value(seed_config_json()).expect("初始配置必须能反序列化");
    biliup_relay::storage::write_document("config.json", &seeded).expect("写入初始配置");
    let mut cfg = load_config().await.expect("读取配置");
    cfg.interval = 42;
    save_config(&mut cfg).await.expect("写入配置");
    let reloaded = load_config().await.expect("重载配置");
    assert_eq!(reloaded.interval, 42, "配置必须真实落盘并可重载");

    // 2) 预取槽由已迁入的真实 priority 插件持有：store 之后立刻能 take 到。
    store_prefetched_playable_stream(prefetched(), &reloaded);
    assert!(
        take_prefetched_playable_stream(PriorityChannelPlatform::Youtube, CHANNEL_ID).is_some(),
        "预取槽必须是真实实现，而非空桩"
    );

    // 3) 再次预取后写配置：旧桩已删除，config.rs 走真实清理函数。
    store_prefetched_playable_stream(prefetched(), &reloaded);
    let mut cfg = reloaded;
    cfg.interval = 60;
    save_config(&mut cfg).await.expect("第二次写入配置");
    assert!(
        take_prefetched_playable_stream(PriorityChannelPlatform::Youtube, CHANNEL_ID).is_none(),
        "配置写入后旧预取必须失效"
    );
    assert_eq!(
        load_config().await.expect("再次重载配置").interval,
        60,
        "第二次写入同样要真实落盘"
    );

    // 4) 空闲态没有本地推流。
    let status = current_network_status().await;
    assert!(!status.ffmpeg_running, "空闲态不应有本地推流");

    // 5) 停机：停弹幕、停推流、释放存储。
    relay.shutdown().await;
    let health = relay.health();
    assert_eq!(health.state, "stopped");
    assert!(!health.enabled);
    assert!(!health.storage_ready);
    assert!(!biliup_relay::plugins::is_ffmpeg_running().await);
    assert!(!biliup_relay::plugins::is_danmaku_running());

    // 6) 同进程干净重启，数据完整保留。
    relay.start(init).await.expect("第二次启动应成功");
    let after = load_config().await.expect("重启后读取配置");
    assert_eq!(after.interval, 60, "重启后配置必须保留");
    assert!(data_dir.join(biliup_relay::DB_FILE).exists());
    relay.shutdown().await;
}
