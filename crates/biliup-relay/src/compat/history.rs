#![allow(dead_code)]
//! 统计历史（history）尚未迁入时的薄适配边界。
//!
//! B 组只依赖两类能力：向事件总线发送已脱敏的事实，以及直播汇总的
//! 解析/重试纯逻辑。真正的入库、会话序号与直播汇总调度待 Phase 2 迁入后替换。
//! 其中 `enrich` 与源仓库逐字节等价，`schema`/`writer` 是为弹幕客户端原生
//! 测试保留的最小真实 SQLite 实现。

pub use crate::events::event::{BiliTotals, EventKind};
#[allow(unused_imports)]
pub use crate::events::event::EndReason;
#[allow(unused_imports)]
pub use crate::events::event::Event;

/// 与源仓库同一份固定兜底文案；派生会话详情据此判断是否是真违规原因。
pub(crate) const STOCK_FALLBACKS: [&str; 4] =
    ["直播间警告", "直播间切断", "直播间违规", "直播间涉嫌违规"];

/// 同一次警告/切断的归并窗口，待 Phase 2 的 writer 迁入后由它定义。
pub(crate) const VIOLATION_WINDOW_SECS: i64 = 60;

/// 非阻塞转发；真正的入库等待 Phase 2 统计迁入。
pub fn emit(kind: EventKind) {
    crate::events::emit(kind);
}

pub fn emit_at(at: u64, kind: EventKind) {
    crate::events::emit_at(at, kind);
}

/// 没有统计写入线程时不存在当前会话。
pub fn open_session_seq(_room: u64) -> Option<u64> {
    None
}

pub fn recording_enabled() -> bool {
    false
}

pub fn bili_sample_secs() -> u64 {
    5
}

pub fn on_bili_started(_cfg: &crate::config::Config, _live_key: &str) {}

pub fn on_bili_stopped(_cfg: &crate::config::Config) {}

pub mod bili {
    /// 待 Phase 2 直播簿/统计迁入后替换；当前不保存任何房间状态。
    pub fn observe_room_status(_room: u64, _live: bool) {}

    pub fn observe_room_status_at(_room: u64, _live: bool, _at: u64) {}
}

pub mod capture {
    /// 待 Phase 2 调试抓帧迁入后替换；不写任何文件。
    pub fn warning_frame(_cmd: &str, _keys: &[String], _reason: Option<&str>) {}

    pub fn stop_live_data(_reply: &serde_json::Value) {}
}

pub mod event {
    #[allow(unused_imports)]
    pub use crate::events::event::Event;
}

pub mod enrich {
    //! B 站直播汇总解析与重试；与源仓库 `history/enrich.rs` 行为一致。

    use super::BiliTotals;
    use serde_json::Value;

    const STOP_LIVE_DATA_FIELDS: [&str; 6] = [
        "LiveTime",
        "AddFans",
        "NewFansClub",
        "DanmuNum",
        "MaxOnline",
        "WatchedCount",
    ];

    #[derive(Debug, PartialEq, Eq)]
    enum StopLiveDataError {
        Code(i64),
        NoData,
        Sentinel(&'static str),
    }

    fn parse_stop_live_data(reply: &Value) -> Result<BiliTotals, StopLiveDataError> {
        let code = reply.get("code").and_then(Value::as_i64).unwrap_or(-1);
        if code != 0 {
            return Err(StopLiveDataError::Code(code));
        }
        let data = reply
            .get("data")
            .and_then(Value::as_object)
            .ok_or(StopLiveDataError::NoData)?;
        let mut fields = [None; 6];
        for (slot, name) in fields.iter_mut().zip(STOP_LIVE_DATA_FIELDS) {
            let value = data.get(name);
            if value.and_then(Value::as_f64).is_some_and(|v| v < 0.0) {
                return Err(StopLiveDataError::Sentinel(name));
            }
            *slot = value.and_then(Value::as_i64);
        }
        let [live_time_secs, add_fans, new_fans_club, danmu_num, max_online, watched_count] = fields;
        Ok(BiliTotals {
            live_time_secs,
            add_fans,
            new_fans_club,
            danmu_num,
            max_online,
            watched_count,
        })
    }

    pub fn parse_room_live_key(reply: &Value) -> Option<String> {
        if reply.get("code").and_then(Value::as_i64) != Some(0) {
            return None;
        }
        let data = reply.get("data")?;
        if data.get("live_status").and_then(Value::as_i64) != Some(1) {
            return None;
        }
        ["live_key", "up_session"]
            .iter()
            .find_map(|field| parse_live_key(data.get(*field)?))
    }

    pub fn parse_live_key(value: &Value) -> Option<String> {
        let raw = match value {
            Value::String(value) => value.trim().to_string(),
            Value::Number(value) => value.as_u64()?.to_string(),
            _ => return None,
        };
        (!raw.is_empty()
            && raw != "0"
            && !raw.starts_with('<')
            && raw.len() <= 256
            && !raw.chars().any(char::is_control))
        .then_some(raw)
    }

    pub fn redact_room_info(reply: &Value) -> Value {
        serde_json::json!({
            "code": reply.get("code").and_then(Value::as_i64),
            "data": {
                "live_status": reply.pointer("/data/live_status").and_then(Value::as_i64),
                "has_live_key": parse_room_live_key(reply).is_some(),
            }
        })
    }

    pub const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    #[derive(Debug, PartialEq, Eq)]
    pub enum FetchResult {
        Finished(Option<BiliTotals>),
        Cancelled,
    }

    pub async fn fetch_with_retry_guarded<F, Fut, G>(
        mut fetch: F,
        first: std::time::Duration,
        retry: std::time::Duration,
        mut owns_claim: G,
    ) -> FetchResult
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Value, String>>,
        G: FnMut() -> bool,
    {
        tokio::time::sleep(first).await;
        for attempt in 0..2 {
            if attempt == 1 {
                tokio::time::sleep(retry).await;
            }
            if !owns_claim() {
                return FetchResult::Cancelled;
            }
            let reply = fetch().await;
            if !owns_claim() {
                return FetchResult::Cancelled;
            }
            match reply {
                Ok(reply) => {
                    super::capture::stop_live_data(&reply);
                    match parse_stop_live_data(&reply) {
                        Ok(totals) => return FetchResult::Finished(Some(totals)),
                        Err(error) => {
                            tracing::warn!("B站直播汇总无效（第 {} 次）：{error:?}", attempt + 1)
                        }
                    }
                }
                Err(error) => tracing::warn!(
                    "B站直播汇总请求失败（第 {} 次）：{}",
                    attempt + 1,
                    safe_fetch_failure(&error)
                ),
            }
        }
        FetchResult::Finished(None)
    }

    fn safe_fetch_failure(error: &str) -> &str {
        if matches!(error, "timeout" | "invalid json" | "request failed")
            || error.strip_prefix("http ").is_some_and(|code| {
                code.parse::<u16>()
                    .is_ok_and(|code| (400..600).contains(&code))
            })
        {
            error
        } else {
            "request failed"
        }
    }
}

pub mod schema {
    //! 待 Phase 2 统计库 schema 迁入后替换；当前只建弹幕测试用到的两张表。
    use std::path::Path;

    use rusqlite::Connection;

    pub fn open_rw(path: &Path) -> rusqlite::Result<Connection> {
        let conn = Connection::open(path)?;
        super::writer::create_tables(&conn)?;
        Ok(conn)
    }
}

pub mod writer {
    //! 待 Phase 2 统计写入线程迁入后替换的最小真实实现。
    //!
    //! 只覆盖弹幕客户端原生测试需要的语义：会话行、BiliCounts 样本，
    //! 以及按 (seq, room) 归属，避免把另一房间的观察写进本会话。

    use super::{Event, EventKind};
    use rusqlite::{params, Connection, OptionalExtension};

    pub(crate) fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions(
                 id INTEGER PRIMARY KEY,
                 seq INTEGER NOT NULL,
                 room INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS samples_raw(
                 id INTEGER PRIMARY KEY,
                 session_id INTEGER NOT NULL,
                 metric INTEGER NOT NULL,
                 max REAL NOT NULL,
                 at INTEGER NOT NULL
             );",
        )
    }

    pub struct Writer {
        pub conn: Connection,
        _node_id: String,
        _boot_id: i64,
    }

    impl Writer {
        pub fn new(conn: Connection, node_id: &str, boot_id: i64) -> Self {
            create_tables(&conn).expect("创建统计测试表");
            Self {
                conn,
                _node_id: node_id.to_owned(),
                _boot_id: boot_id,
            }
        }

        pub fn commit(&mut self, events: &[Event]) -> rusqlite::Result<()> {
            for event in events {
                if let EventKind::SessionStarted { seq, room, .. } = &event.kind {
                    self.conn.execute(
                        "INSERT INTO sessions(seq, room) VALUES (?1, ?2)",
                        params![seq, room],
                    )?;
                }
            }
            Ok(())
        }

        pub fn apply(&mut self, event: &Event) -> rusqlite::Result<()> {
            let EventKind::BiliCounts {
                seq,
                room,
                online,
                watched,
            } = &event.kind
            else {
                return Ok(());
            };
            let session: Option<i64> = self
                .conn
                .query_row(
                    "SELECT id FROM sessions WHERE seq=?1 AND room=?2 ORDER BY id DESC LIMIT 1",
                    params![seq, room],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(session) = session else {
                return Ok(());
            };
            for (metric, value) in [(10, *online), (11, *watched)] {
                let Some(value) = value else {
                    continue;
                };
                self.conn.execute(
                    "INSERT INTO samples_raw(session_id, metric, max, at) VALUES (?1, ?2, ?3, ?4)",
                    params![session, metric, value as f64, event.at],
                )?;
            }
            Ok(())
        }
    }
}

