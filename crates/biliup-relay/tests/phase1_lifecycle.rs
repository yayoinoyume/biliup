//! Phase 1 收官端到端本地集成测试：纯本地临时目录，零外网、零真实凭据。
//!
//! 三条路径都走 RelayService 真实的监控线程、停止通道与退出清理代码：
//! 1. `shutdown()` 真实停机（弹幕 + 推流 + 存储）后同进程重启；
//! 2. 先 `stop_monitor_loop()` 再 `shutdown()` 不丢清理；
//! 3. 直接 `drop(RelayService)` 由 `Drop` 兜底停掉监控线程与引擎。
//!
//! 引擎用零外网替身：推流侧登记一个真实 `sleep` 子进程（`stop_ffmpeg()` 按正常
//! 路径终止它），弹幕侧只走真实的运行标志与停止信号协议。

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use biliup_relay::config::{load_config, save_config, Config};
use biliup_relay::plugins::priority::{
    store_prefetched_playable_stream, take_prefetched_playable_stream, PrefetchedPlayableStream,
    PriorityChannelPlatform,
};
use biliup_relay::{local_publisher_running, RelayInitConfig, RelayService};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

struct Harness {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    init: RelayInitConfig,
}

async fn boot() -> (Harness, RelayService) {
    let dir = tempfile::tempdir().expect("临时目录");
    let data_dir = dir.path().join("relay");
    let key_file = dir.path().join("keys/master.key");
    let init = RelayInitConfig {
        data_dir: data_dir.clone(),
        key_file: Some(key_file.clone()),
        ffmpeg_path: None,
    };
    let relay = RelayService::new();
    relay.start(init.clone()).await.expect("启动应成功");
    assert!(key_file.exists(), "显式 key_file 必须落在数据目录之外");
    let seeded: Config =
        serde_json::from_value(seed_config_json()).expect("初始配置必须能反序列化");
    biliup_relay::storage::write_document("config.json", &seeded).expect("写入初始配置");
    (
        Harness {
            _dir: dir,
            data_dir,
            init,
        },
        relay,
    )
}

fn process_exited(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(_) => true,
        Ok(stat) => stat
            .rsplit(')')
            .next()
            .is_some_and(|rest| rest.trim_start().starts_with('Z')),
    }
}

async fn wait_until<F, Fut>(what: &str, mut probe: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if probe().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    eprintln!("等待超时：{what}");
    false
}

/// 弹幕客户端的零外网替身：只跑真实的运行标志与停止信号协议。
fn start_danmaku_stand_in() -> tokio::task::JoinHandle<()> {
    biliup_relay::plugins::set_danmaku_stop_signal(false);
    biliup_relay::plugins::set_danmaku_running(true);
    tokio::spawn(async move {
        while !biliup_relay::plugins::should_stop_danmaku() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        biliup_relay::plugins::set_danmaku_running(false);
    })
}

/// 启动一个只登记本地占位推流进程的监控任务，并回报该进程 PID。
fn start_placeholder_engine(relay: &RelayService) -> Arc<Mutex<Option<u32>>> {
    let pid_slot: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&pid_slot);
    relay.spawn_monitor_loop_with(move || async move {
        let pid = biliup_relay::plugins::ffmpeg::track_local_publisher_for_test()
            .await
            .expect("登记占位推流进程");
        *slot.lock().unwrap_or_else(|error| error.into_inner()) = Some(pid);
        std::future::pending::<()>().await;
    });
    pid_slot
}

async fn engine_is_up(pid_slot: &Arc<Mutex<Option<u32>>>) -> bool {
    let slot = Arc::clone(pid_slot);
    let started = wait_until("监控任务登记占位推流进程", move || {
        let slot = Arc::clone(&slot);
        async move {
            slot.lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_some()
        }
    })
    .await;
    if !started || !wait_until("本地推流可观测", local_publisher_running).await {
        return false;
    }
    assert!(
        biliup_relay::plugins::is_danmaku_running(),
        "弹幕替身必须在运行，停弹幕才有可观测的状态变化"
    );
    true
}

fn taken_pid(pid_slot: &Arc<Mutex<Option<u32>>>) -> u32 {
    pid_slot
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .expect("占位推流进程 PID")
}

async fn assert_engine_stopped(pid: u32) {
    assert!(
        wait_until("弹幕替身收到停止信号", || async {
            !biliup_relay::plugins::is_danmaku_running()
        })
        .await,
        "停弹幕必须真实执行"
    );
    assert!(
        biliup_relay::plugins::should_stop_danmaku(),
        "停弹幕必须发出停止信号"
    );
    assert!(
        wait_until("推流进程被停止", || async {
            !biliup_relay::plugins::is_ffmpeg_running().await
        })
        .await,
        "停推流必须真实执行"
    );
    assert!(!local_publisher_running().await, "空闲态不应有本地推流");
    assert!(
        wait_until("占位子进程退出", move || async move { process_exited(pid) }).await,
        "占位推流子进程必须退出"
    );
}

async fn assert_restart_reads_back(harness: &Harness, relay: &RelayService, interval: u64) {
    relay
        .start(harness.init.clone())
        .await
        .expect("重启应成功");
    let reloaded = load_config().await.expect("重启后读取配置");
    assert_eq!(reloaded.interval, interval, "重启后配置必须保留");
    assert!(harness.data_dir.join(biliup_relay::DB_FILE).exists());
    relay.shutdown().await;
}

#[tokio::test]
async fn shutdown_stops_a_real_local_engine_and_releases_storage() {
    let _serial = SERIAL.lock().await;
    let (harness, relay) = boot().await;

    let mut cfg = load_config().await.expect("读取配置");
    cfg.interval = 42;
    save_config(&mut cfg).await.expect("写入配置");
    let reloaded = load_config().await.expect("重载配置");
    assert_eq!(reloaded.interval, 42, "配置必须真实落盘并可重载");

    // 预取槽由已迁入的真实 priority 插件持有。
    store_prefetched_playable_stream(prefetched(), &reloaded);
    assert!(
        take_prefetched_playable_stream(PriorityChannelPlatform::Youtube, CHANNEL_ID).is_some(),
        "预取槽必须是真实实现，而非空桩"
    );
    // 再次预取后写配置：旧桩已删除，config.rs 走真实清理函数。
    store_prefetched_playable_stream(prefetched(), &reloaded);
    let mut cfg = reloaded;
    cfg.interval = 60;
    save_config(&mut cfg).await.expect("第二次写入配置");
    assert!(
        take_prefetched_playable_stream(PriorityChannelPlatform::Youtube, CHANNEL_ID).is_none(),
        "配置写入后旧预取必须失效"
    );

    // 真实拉起零外网引擎替身。
    let danmaku = start_danmaku_stand_in();
    let pid_slot = start_placeholder_engine(&relay);
    assert!(engine_is_up(&pid_slot).await, "监控任务必须真的把引擎跑起来");
    assert!(biliup_relay::plugins::is_danmaku_running());
    let pid = taken_pid(&pid_slot);

    relay.shutdown().await;

    assert_engine_stopped(pid).await;
    danmaku.await.expect("弹幕替身应正常退出");
    let health = relay.health();
    assert_eq!(health.state, "stopped");
    assert!(!health.enabled);
    assert!(!health.storage_ready);

    assert_restart_reads_back(&harness, &relay, 60).await;
}

#[tokio::test]
async fn stop_then_shutdown_does_not_lose_the_engine_cleanup() {
    let _serial = SERIAL.lock().await;
    let (harness, relay) = boot().await;

    let danmaku = start_danmaku_stand_in();
    let pid_slot = start_placeholder_engine(&relay);
    assert!(engine_is_up(&pid_slot).await, "监控任务必须真的把引擎跑起来");
    let pid = taken_pid(&pid_slot);

    assert!(
        relay.stop_monitor_loop().await,
        "监控线程应当自己完成收尾"
    );
    // 引擎由监控线程停掉；之后的 shutdown 仍必须干净释放存储。
    assert_engine_stopped(pid).await;
    relay.shutdown().await;
    danmaku.await.expect("弹幕替身应正常退出");
    assert!(!relay.health().storage_ready);

    assert_restart_reads_back(&harness, &relay, 60).await;
}

#[tokio::test]
async fn dropping_the_service_still_stops_the_monitor_thread_and_engine() {
    let _serial = SERIAL.lock().await;
    let (harness, relay) = boot().await;

    let danmaku = start_danmaku_stand_in();
    let pid_slot = start_placeholder_engine(&relay);
    assert!(engine_is_up(&pid_slot).await, "监控任务必须真的把引擎跑起来");
    let pid = taken_pid(&pid_slot);

    drop(relay);

    assert_engine_stopped(pid).await;
    danmaku.await.expect("弹幕替身应正常退出");

    // 引擎清理期间 store 不被动，测试自己收尾后必须能重新打开同一份数据。
    biliup_relay::storage::shutdown();
    let reopened = RelayService::new();
    reopened
        .start(harness.init.clone())
        .await
        .expect("drop 之后应能重新打开存储");
    let reloaded = load_config().await.expect("重新打开后读取配置");
    assert_eq!(reloaded.interval, 60);
    assert!(Path::new(&harness.data_dir).exists());
    reopened.shutdown().await;
}
