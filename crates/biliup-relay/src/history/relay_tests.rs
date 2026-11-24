//! Relay 生命周期与统计引擎的联动测试。
//!
//! 每个用例都走真实的 RelayService::start -> 事件总线 -> 单写线程 ->
//! shutdown 落盘路径，只使用临时目录与合成事实，零外网、零真实凭据。

use super::{EndReason, SessionGuard};
use crate::config::{load_config, Config};
use crate::storage;
use crate::{RelayInitConfig, RelayService};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// 这些用例共用进程级存储、事件总线与统计句柄，必须串行。
static SERIAL: Mutex<()> = Mutex::const_new(());

struct Harness {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
}

fn seed(room: i32) -> Config {
    let mut cfg = crate::test_support::config("local", 0);
    cfg.bililive.room = room;
    cfg.history.enabled = true;
    cfg
}

/// 先建库写配置，再启动服务：统计引擎在 start 时读取同一份配置。
async fn boot(room: i32) -> (Harness, RelayService) {
    let dir = tempfile::tempdir().expect("临时目录");
    let data_dir = dir.path().join("relay");
    let key_file = dir.path().join("keys/master.key");
    storage::init(&data_dir, Some(&key_file)).expect("打开存储");
    storage::write_document("config.json", &seed(room)).expect("写入配置");
    let relay = RelayService::new();
    relay
        .start(RelayInitConfig {
            data_dir: data_dir.clone(),
            key_file: Some(key_file),
            ffmpeg_path: None,
        })
        .await
        .expect("启动应成功");
    (Harness { _dir: dir, data_dir }, relay)
}

fn lock_store() -> std::sync::MutexGuard<'static, ()> {
    storage::TEST_STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn rows(db: &str, sql: &str, seq: u64) -> i64 {
    super::schema::open_ro(std::path::Path::new(db))
        .expect("只读打开统计库")
        .query_row(sql, [seq as i64], |row| row.get(0))
        .expect("查询统计库")
}

/// 统计写入线程是唯一按名字登记的后台线程；停机后必须归零。
#[cfg(target_os = "linux")]
fn writer_threads() -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir("/proc/self/task") {
        for entry in entries.flatten() {
            if let Ok(name) = std::fs::read_to_string(entry.path().join("comm")) {
                if name.trim() == "history-writer" {
                    count += 1;
                }
            }
        }
    }
    count
}

#[cfg(not(target_os = "linux"))]
fn writer_threads() -> usize {
    0
}

#[tokio::test]
async fn session_guard_facts_reach_disk_before_shutdown_returns() {
    let _serial = SERIAL.lock().await;
    let _store = lock_store();
    let (harness, relay) = boot(987_654).await;
    assert!(
        super::recording_enabled(),
        "统计引擎必须在事件总线封口前启动"
    );
    let cfg = load_config().await.expect("读取配置");

    super::on_bili_started(&cfg, "synthetic-relay-live");
    let guard = SessionGuard::start(
        &cfg,
        "YT",
        "UC_history_relay",
        "统计频道",
        Some("统计标题"),
    );
    let seq = guard.seq();
    let exit_at = crate::storage::now().saturating_sub(5);
    guard.run_started(10);
    guard.run_exited_with_diagnostic_at(
        exit_at,
        10,
        Some(1),
        false,
        true,
        Some("synthetic diagnostic"),
    );
    guard.finish(EndReason::SourceEnded);

    relay.shutdown().await;

    let db = harness
        .data_dir
        .join("history.sqlite")
        .display()
        .to_string();
    assert_eq!(
        rows(
            &db,
            "SELECT count(*) FROM sessions WHERE seq=?1 AND channel_id='UC_history_relay'",
            seq
        ),
        1,
        "会话必须在 shutdown 返回前落盘"
    );
    let conn = super::schema::open_ro(std::path::Path::new(&db)).expect("只读打开");
    let (live_key, end_reason, runs, failures): (Option<String>, Option<String>, i64, i64) = conn
        .query_row(
            "SELECT live_key,end_reason,runs,failures FROM sessions WHERE seq=?1",
            [seq as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("会话行");
    assert_eq!(live_key.as_deref(), Some("synthetic-relay-live"));
    assert_eq!(end_reason.as_deref(), Some("source_ended"));
    assert_eq!((runs, failures), (1, 1));
    let (ended_at, exit_code, success, counted): (i64, Option<i64>, i64, i64) = conn
        .query_row(
            "SELECT ended_at,exit_code,success,counted_failure FROM runs WHERE run=10",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("运行行");
    assert_eq!(ended_at, exit_at as i64, "run_exited_at 必须真实落库");
    assert_eq!((exit_code, success, counted), (Some(1), 0, 1));
    drop(conn);
    drop(relay);
    assert_eq!(writer_threads(), 0, "shutdown 后不得残留统计写入线程");
}

#[tokio::test]
async fn two_data_directories_in_one_process_stay_independent() {
    let _serial = SERIAL.lock().await;
    let _store = lock_store();

    let (dir_a, relay_a) = boot(700_001).await;
    let cfg = load_config().await.expect("读取配置");
    let guard = SessionGuard::start(&cfg, "YT", "UC_history_dir_a", "目录A", None);
    let seq_a = guard.seq();
    guard.finish(EndReason::SourceEnded);
    relay_a.shutdown().await;
    drop(relay_a);
    assert_eq!(writer_threads(), 0, "第一次停机必须回收写入线程");

    let (dir_b, relay_b) = boot(700_002).await;
    let cfg = load_config().await.expect("读取配置 B");
    let guard = SessionGuard::start(&cfg, "YT", "UC_history_dir_b", "目录B", None);
    let seq_b = guard.seq();
    guard.finish(EndReason::UserStop);
    relay_b.shutdown().await;
    drop(relay_b);
    assert_eq!(writer_threads(), 0, "第二次停机必须回收写入线程");
    assert_ne!(seq_a, seq_b, "每次会话序号必须唯一");

    let db_a = dir_a.data_dir.join("history.sqlite").display().to_string();
    let db_b = dir_b.data_dir.join("history.sqlite").display().to_string();
    assert!(std::path::Path::new(&db_a).exists() && std::path::Path::new(&db_b).exists());
    assert_eq!(
        rows(&db_a, "SELECT count(*) FROM sessions WHERE seq=?1", seq_a),
        1,
        "目录 A 必须只保留自己的会话"
    );
    assert_eq!(
        rows(&db_a, "SELECT count(*) FROM sessions WHERE seq=?1", seq_b),
        0,
        "目录 A 不得出现目录 B 的会话"
    );
    assert_eq!(
        rows(&db_b, "SELECT count(*) FROM sessions WHERE seq=?1", seq_b),
        1
    );
    assert_eq!(
        rows(&db_b, "SELECT count(*) FROM sessions WHERE seq=?1", seq_a),
        0,
        "目录 B 不得出现目录 A 的会话"
    );
    let channel: String = super::schema::open_ro(std::path::Path::new(&db_b))
        .expect("只读打开 B")
        .query_row("SELECT channel_id FROM sessions WHERE seq=?1", [seq_b as i64], |row| {
            row.get(0)
        })
        .expect("B 的会话行");
    assert_eq!(channel, "UC_history_dir_b");
}

#[tokio::test]
async fn a_committed_batch_publishes_the_history_ui_event() {
    let _serial = SERIAL.lock().await;
    let _store = lock_store();
    let (harness, relay) = boot(700_003).await;
    let mut rx = crate::ui_events::subscribe();
    let cfg = load_config().await.expect("读取配置");
    let guard = SessionGuard::start(&cfg, "YT", "UC_history_ui", "界面频道", None);
    guard.run_started(10);

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut published = false;
    while !published && Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Ok(kind)) if kind == crate::ui_events::HISTORY => published = true,
            Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {}
        }
    }
    assert!(published, "写入线程提交后必须发出 HISTORY 变更信号");
    guard.finish(EndReason::UserStop);
    relay.shutdown().await;
    drop(harness);
}

#[tokio::test]
async fn clear_refuses_an_open_relay_and_succeeds_after_it_ends() {
    let _serial = SERIAL.lock().await;
    let _store = lock_store();
    let (harness, relay) = boot(700_004).await;
    let cfg = load_config().await.expect("读取配置");
    let guard = SessionGuard::start(&cfg, "YT", "UC_history_clear", "清理频道", None);
    let seq = guard.seq();

    let refused = super::clear(Duration::from_secs(2)).await;
    let reason = refused.expect_err("转播进行中必须拒绝清理");
    assert!(reason.contains("正在转播"), "{reason}");
    guard.finish(EndReason::UserStop);

    let deleted = super::clear(Duration::from_secs(5)).await.expect("停机后应能清理");
    assert!(deleted > 0, "清理必须真的删掉已落盘行");
    let db = harness.data_dir.join("history.sqlite").display().to_string();
    assert_eq!(
        rows(&db, "SELECT count(*) FROM sessions WHERE seq=?1", seq),
        0,
        "清理后本会话的行必须消失"
    );
    relay.shutdown().await;
}
