//! `/api/history/*` 统计读端点与清理：只读连接、并发准入与 ETag/304。
//!
//! 跨节点聚合通过 [`Peers`] 接缝完成；单机版本没有对等节点，
//! `Scope::Cluster` 恒为空对端。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::extract::Query as Params;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::not_configured_response;
use crate::cluster::peer_call::{PeerCallError, routes};
use crate::config::Config;
use crate::history::MissingNode;
use crate::history::query::{
    self, BadRequest, EventsPage, Query, SessionsPage, StatsParts, SummaryParts,
};

const PEER_TIMEOUT: Duration = Duration::from_secs(3);
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const QUERY_ADMISSION_TIMEOUT: Duration = Duration::from_millis(750);
pub(crate) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
// 协调请求有自己的准入上限；对端读不占这个闸门，否则两个面板会互相等死。
static COORDINATORS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
static QUERIES: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(2)));

fn install_query_deadline(
    conn: &rusqlite::Connection,
    deadline: std::time::Instant,
) -> rusqlite::Result<()> {
    // 阻塞任务与已取消的 HTTP 调用方脱钩，因此 SQLite 自己持有这个回调，
    // 取消也拿不走查询期限。
    conn.progress_handler(1000, Some(move || std::time::Instant::now() >= deadline));
    Ok(())
}

async fn acquire_db_slot(
    queries: &Arc<tokio::sync::Semaphore>,
) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    tokio::time::timeout(QUERY_ADMISSION_TIMEOUT, Arc::clone(queries).acquire_owned())
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| "统计查询繁忙".to_string())
}

async fn admitted_query(
    queries: &tokio::sync::Semaphore,
    response: impl Future<Output = Response>,
) -> Response {
    let Ok(_permit) = queries.try_acquire() else {
        return fail(StatusCode::TOO_MANY_REQUESTS, "统计查询繁忙");
    };
    response.await
}

fn ok(data: Value) -> Response {
    conditional_ok(data, &HeaderMap::new(), "")
}

/// 对合并后的真实表示做哈希，包含对端行与覆盖范围：某个对端提交了新行，
/// 未变的协调端也必须换 ETag；未结束的会话时长同样有诚实的校验值。
fn conditional_ok(data: Value, headers: &HeaderMap, query_key: &str) -> Response {
    use sha2::{Digest, Sha256};
    let bytes = match serde_json::to_vec(&json!({"success":true,"data":data})) {
        Ok(bytes) if bytes.len() <= MAX_RESPONSE_BYTES => bytes,
        Ok(_) => {
            return fail(
                StatusCode::PAYLOAD_TOO_LARGE,
                "统计结果过大，请缩小范围或减少记录数",
            );
        }
        Err(_) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "统计结果无法编码"),
    };
    let mut hash = Sha256::new();
    hash.update(crate::history::schema::SCHEMA_VERSION.to_be_bytes());
    hash.update(query_key.as_bytes());
    hash.update(&bytes);
    let tag = format!("\"{:x}\"", hash.finalize());
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|part| {
                let part = part.trim();
                part == "*" || part.strip_prefix("W/").unwrap_or(part) == tag
            })
        });
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Response::new(axum::body::Body::from(bytes))
    };
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&tag).expect("hex ETag is a header value"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    if !unchanged {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
    }
    response
}

fn fail(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({ "success": false, "message": message })),
    )
        .into_response()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Local,
    Node(String),
    Cluster,
}

pub(crate) fn local_node(cfg: &Config) -> String {
    if cfg.cluster.enabled {
        cfg.cluster.node_id.clone()
    } else {
        "local".into()
    }
}

/// `cluster`（集群下默认）、`local`（单机默认）或已知成员 id；未知 id → 400。
pub(crate) fn resolve_scope(cfg: &Config, raw: Option<&str>) -> Result<Scope, BadRequest> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty());
    match raw {
        None if cfg.cluster.enabled => Ok(Scope::Cluster),
        None | Some("local") => Ok(Scope::Local),
        Some("cluster") if cfg.cluster.enabled => Ok(Scope::Cluster),
        Some("cluster") => Ok(Scope::Local),
        Some(id) if id == local_node(cfg) => Ok(Scope::Local),
        Some(id) if cfg.cluster.enabled && cfg.cluster.peers.iter().any(|p| p.node_id == id) => {
            Ok(Scope::Node(id.to_string()))
        }
        Some(_) => Err(BadRequest("未知节点".into())),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PeerRequest {
    pub name: String,
    pub query: Query,
}

type PeerFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, MissingNode>> + Send + 'a>>;

/// 扇出接缝：生产代码签一个对端请求，测试直接用 mock 回答。
pub(crate) trait Peers: Sync {
    fn fetch<'a>(&'a self, node: &'a str, req: &'a PeerRequest) -> PeerFuture<'a>;
}

pub(crate) fn missing(node: &str, reason: &str, detail: &str) -> MissingNode {
    MissingNode {
        node_id: node.to_string(),
        reason: reason.to_string(),
        detail: Some(detail.to_string()),
    }
}

/// 把对端失败映射到规范原因；`detail` 是简短安全的英文（不含 URL 与密钥）。
pub(crate) fn classify(node: &str, error: &PeerCallError) -> MissingNode {
    match error {
        PeerCallError::Unreachable(m)
            if m.contains("超时") || m.to_lowercase().contains("timed out") =>
        {
            missing(node, "timeout", "no response in 3000 ms")
        }
        PeerCallError::Unreachable(_) => missing(node, "unreachable", "connection failed"),
        PeerCallError::Refused(m) if m == "HTTP 401" || m == "HTTP 403" => {
            missing(node, "auth", "signed request rejected")
        }
        PeerCallError::Refused(m) if m == "HTTP 404" => {
            missing(node, "version", "peer has no history API")
        }
        PeerCallError::Refused(_) => missing(node, "error", "invalid peer response"),
    }
}

struct LivePeers<'c> {
    cfg: &'c Config,
}

impl Peers for LivePeers<'_> {
    fn fetch<'a>(&'a self, node: &'a str, req: &'a PeerRequest) -> PeerFuture<'a> {
        Box::pin(async move {
            let call = crate::cluster::peer_call::call::<PeerRequest, Value>(
                self.cfg,
                node,
                routes::HISTORY,
                Some(req),
                PEER_TIMEOUT,
            );
            match tokio::time::timeout(PEER_TIMEOUT, call).await {
                Err(_) => Err(missing(node, "timeout", "no response in 3000 ms")),
                Ok(Err(error)) => Err(classify(node, &error)),
                Ok(Ok(envelope)) if envelope.success => envelope
                    .data
                    .ok_or_else(|| missing(node, "error", "empty peer response")),
                Ok(Ok(_)) => Err(missing(node, "error", "peer refused the query")),
            }
        })
    }
}

/// 本节点自己的库里取一份分片，形状与对端返回的一致。
pub(crate) fn local_parts(
    conn: &rusqlite::Connection,
    req: &PeerRequest,
    node: &str,
    now: i64,
) -> rusqlite::Result<Value> {
    let q = &req.query;
    // 多语句的 KPI/会话读必须看到同一个提交版本，否则保留策略可能拆掉半个会话。
    let snapshot = conn.unchecked_transaction()?;
    let value = match req.name.as_str() {
        "summary" => serde_json::to_value(query::summary(&snapshot, q, node, now)?),
        "sessions" => serde_json::to_value(query::sessions(&snapshot, q, now)?),
        "events" => serde_json::to_value(query::events(&snapshot, q)?),
        "stats" => {
            let tier = q.tier.clone().unwrap_or_else(|| "raw".into());
            serde_json::to_value(query::stats(&snapshot, q, &tier, None)?)
        }
        _ => Ok(Value::Null),
    };
    snapshot.commit()?;
    Ok(value.unwrap_or(Value::Null))
}

pub(crate) fn empty_parts(req: &PeerRequest, node: &str) -> Value {
    match req.name.as_str() {
        "summary" => serde_json::to_value(SummaryParts {
            per_node: vec![query::NodeParts {
                node_id: node.into(),
                ..Default::default()
            }],
            ..Default::default()
        }),
        "sessions" => serde_json::to_value(SessionsPage::default()),
        "events" => serde_json::to_value(EventsPage::default()),
        _ => serde_json::to_value(query::empty_stats(
            &req.query,
            req.query.tier.as_deref().unwrap_or("raw"),
        )),
    }
    .unwrap_or(Value::Null)
}

/// 本节点自己的库，走与对端相同的接缝。
struct LocalDb;

impl Peers for LocalDb {
    fn fetch<'a>(&'a self, node: &'a str, req: &'a PeerRequest) -> PeerFuture<'a> {
        Box::pin(async move {
            run_local(req.clone(), node.to_string())
                .await
                .map_err(|e| missing(node, "error", &e))
        })
    }
}

/// 本机查询走只读连接，2 秒后中断。
async fn run_local(req: PeerRequest, node: String) -> Result<Value, String> {
    let Some(shared) = crate::history::shared() else {
        return Ok(empty_parts(&req, &node));
    };
    read_database(shared.path.clone(), &QUERIES, move |conn| match conn {
        Some(conn) => local_parts(conn, &req, &node, crate::storage::now() as i64),
        None => Ok(empty_parts(&req, &node)),
    })
    .await
}

/// 共享执行边界：阻塞读取方自己持有许可与 SQLite 期限，与 HTTP 调用方生命周期无关。
async fn read_database(
    path: std::path::PathBuf,
    queries: &Arc<tokio::sync::Semaphore>,
    read: impl FnOnce(Option<&rusqlite::Connection>) -> rusqlite::Result<Value> + Send + 'static,
) -> Result<Value, String> {
    let permit = acquire_db_slot(queries).await?;
    let deadline = std::time::Instant::now() + QUERY_TIMEOUT;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let task = tokio::task::spawn_blocking(move || {
        // SQL 自己持有许可：即使 HTTP 调用方消失或超时，也不会再放一个查询进来。
        let _permit = permit;
        if !path.exists() {
            return read(None).map_err(|error| error.to_string());
        }
        let conn = crate::history::schema::open_ro(&path).map_err(|e| e.to_string())?;
        if std::time::Instant::now() >= deadline {
            return Err("统计查询超时".into());
        }
        install_query_deadline(&conn, deadline).map_err(|error| error.to_string())?;
        let _ = tx.send(conn.get_interrupt_handle());
        read(Some(&conn)).map_err(|e| {
            if e.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
                "统计查询超时".into()
            } else {
                e.to_string()
            }
        })
    });
    let interrupt = tokio::time::timeout(QUERY_TIMEOUT, rx)
        .await
        .ok()
        .and_then(Result::ok);
    match tokio::time::timeout(
        deadline.saturating_duration_since(std::time::Instant::now()),
        task,
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("统计查询失败".into()),
        Err(_) => {
            if let Some(handle) = interrupt {
                handle.interrupt();
            }
            Err("统计查询超时".into())
        }
    }
}

/// 被裁剪的对端在回复里加的键；协调端读取后会移除它。
pub(crate) const PEER_CLAMP_KEY: &str = "clamped_to_days";

pub(crate) async fn gather(
    cfg: &Config,
    scope: &Scope,
    peers: &dyn Peers,
    req: &PeerRequest,
    local: &dyn Peers,
    partial: &std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
    now: i64,
) -> Result<(Vec<Value>, Vec<MissingNode>), String> {
    let targets: Vec<String> = match scope {
        Scope::Local => Vec::new(),
        Scope::Node(id) => vec![id.clone()],
        Scope::Cluster => cfg
            .cluster
            .peers
            .iter()
            .map(|p| p.node_id.clone())
            .filter(|id| *id != cfg.cluster.node_id)
            .collect(),
    };
    let remote = futures_util::future::join_all(targets.iter().map(|node| peers.fetch(node, req)));
    let mut missing_nodes = Vec::new();
    let (local_part, remote) = if matches!(scope, Scope::Node(_)) {
        (None, remote.await)
    } else {
        let node = local_node(cfg);
        let mut local_req = req.clone();
        check_peer_request(&mut local_req, cfg, now).map_err(|e| e.0)?;
        let (l, r) = futures_util::future::join(local.fetch(&node, &local_req), remote).await;
        let l = match l {
            Ok(v) => Some(v),
            Err(m) if *scope == Scope::Cluster => {
                missing_nodes.push(m);
                None
            }
            Err(m) => return Err(m.detail.unwrap_or(m.reason)),
        };
        (l, r)
    };
    let mut parts: Vec<Value> = local_part.into_iter().collect();
    for (node, result) in targets.iter().zip(remote) {
        match result {
            Ok(mut v) => {
                if req.name == "stats"
                    && !serde_json::from_value::<StatsParts>(v.clone())
                        .is_ok_and(|p| query::stats_matches_query(&p, &req.query))
                {
                    missing_nodes.push(missing(node, "error", "incompatible history grid"));
                    continue;
                }
                // 保留期更短的对端返回的是裁剪过的范围：算部分覆盖而不是缺失。
                let clamp = v
                    .as_object_mut()
                    .and_then(|m| m.remove(PEER_CLAMP_KEY))
                    .and_then(|d| d.as_u64());
                if let Some(days) = clamp {
                    let mut partial = partial.lock().unwrap_or_else(|e| e.into_inner());
                    let entry = partial.entry(node.clone()).or_insert(days);
                    *entry = (*entry).min(days);
                }
                parts.push(v)
            }
            Err(m) => missing_nodes.push(m),
        }
    }
    Ok((parts, missing_nodes))
}

fn decode<T: serde::de::DeserializeOwned>(
    parts: Vec<Value>,
    missing_nodes: &mut Vec<MissingNode>,
) -> Vec<T> {
    parts
        .into_iter()
        .filter_map(|v| match serde_json::from_value(v) {
            Ok(t) => Some(t),
            Err(_) => {
                missing_nodes.push(missing("unknown", "error", "invalid peer response"));
                None
            }
        })
        .collect()
}

/// 四个数据路由共用的请求处理。
pub(crate) async fn answer(
    cfg: &Config,
    name: &str,
    params: &HashMap<String, String>,
    peers: &dyn Peers,
    local: &dyn Peers,
    now: i64,
) -> Result<Value, (StatusCode, String)> {
    let bad = |e: BadRequest| (StatusCode::BAD_REQUEST, e.0);
    let retention = cfg.history.retention.clone();
    let default_limit = if name == "sessions" { 50 } else { 20 };
    // 先校验一次调用方范围再选来源：协调端较短的保留期只限制自己的 SQL 窗口。
    let mut q = query::parse(params, now, query::MAX_RANGE_DAYS, default_limit).map_err(bad)?;
    let mut scope = resolve_scope(cfg, params.get("scope").map(String::as_str)).map_err(bad)?;
    // 这条提示只描述本机来源：远端范围不继承它，会话统计的范围来自那一行。
    let clamped = if matches!(scope, Scope::Node(_)) || (name == "stats" && q.session.is_some()) {
        None
    } else {
        query::clamp_peer(&mut q.clone(), retention.rollup_days).or(q.clamped_to_days)
    };
    let partial = std::sync::Mutex::new(std::collections::BTreeMap::new());
    let sources = Sources {
        peers,
        local,
        partial: &partial,
    };
    let mut out = answer_inner(cfg, name, &mut q, &mut scope, &sources, now).await?;
    if let Some(map) = out.as_object_mut() {
        if let Some(days) = clamped {
            map.insert("clamped_to_days".into(), json!(days));
            if scope == Scope::Cluster {
                map.insert("clamp_scope".into(), json!("local"));
            }
        }
        let partial = partial.into_inner().unwrap_or_else(|e| e.into_inner());
        if !partial.is_empty() {
            let nodes: Vec<Value> = partial
                .into_iter()
                .map(|(node, days)| json!({"node": node, "clamped_to_days": days}))
                .collect();
            map.insert("partial_nodes".into(), Value::Array(nodes));
        }
    }
    Ok(out)
}

/// 一份答案的分片来源，以及哪些对端做了裁剪。
struct Sources<'a> {
    peers: &'a dyn Peers,
    local: &'a dyn Peers,
    partial: &'a std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
}

async fn answer_inner(
    cfg: &Config,
    name: &str,
    q: &mut Query,
    scope: &mut Scope,
    sources: &Sources<'_>,
    now: i64,
) -> Result<Value, (StatusCode, String)> {
    let Sources {
        peers,
        local,
        partial,
    } = *sources;
    let internal = |e: String| (StatusCode::INTERNAL_SERVER_ERROR, e);
    let retention = cfg.history.retention.clone();
    let node = local_node(cfg);
    let run = |req: PeerRequest, scope: Scope| async move {
        gather(cfg, &scope, peers, &req, local, partial, now).await
    };
    match name {
        "summary" => {
            let req = PeerRequest {
                name: name.into(),
                query: q.clone(),
            };
            let (parts, mut missing_nodes) = run(req, scope.clone()).await.map_err(internal)?;
            let parts: Vec<SummaryParts> = decode(parts, &mut missing_nodes);
            Ok(query::render_summary(
                &query::merge_summary(parts),
                &missing_nodes,
            ))
        }
        "sessions" => {
            let limit = q.limit;
            let req = PeerRequest {
                name: name.into(),
                query: q.clone(),
            };
            let (parts, mut missing_nodes) = run(req, scope.clone()).await.map_err(internal)?;
            let pages: Vec<SessionsPage> = decode(parts, &mut missing_nodes);
            let mut page = query::merge_sessions(pages, limit);
            page.missing_nodes = missing_nodes;
            Ok(serde_json::to_value(page).unwrap_or(Value::Null))
        }
        "events" => {
            let limit = q.limit;
            let req = PeerRequest {
                name: name.into(),
                query: q.clone(),
            };
            let (parts, mut missing_nodes) = run(req, scope.clone()).await.map_err(internal)?;
            let pages: Vec<EventsPage> = decode(parts, &mut missing_nodes);
            let mut page = query::merge_events(pages, limit);
            page.missing_nodes = missing_nodes;
            Ok(serde_json::to_value(page).unwrap_or(Value::Null))
        }
        "stats" => {
            let mut missing_nodes = Vec::new();
            if let Some(ids) = q.session.clone() {
                // 先解析成员：样本归它们所在的节点所有。
                let mut lookup = q.clone();
                lookup.id = Some(ids);
                lookup.session = None;
                lookup.limit = 500;
                let req = PeerRequest {
                    name: "sessions".into(),
                    query: lookup,
                };
                let (parts, mut m) = run(req, scope.clone()).await.map_err(internal)?;
                let pages: Vec<SessionsPage> = decode(parts, &mut m);
                let members: Vec<_> = pages.into_iter().flat_map(|p| p.items).collect();
                if members.is_empty() {
                    return Ok(
                        json!({ "resolution": "raw", "ts": [], "series": {}, "missing_nodes": m }),
                    );
                }
                q.from = members.iter().map(|s| s.started_at).min().unwrap_or(q.from);
                q.to = members
                    .iter()
                    .map(|s| s.ended_at.unwrap_or(now).max(s.started_at + 1))
                    .max()
                    .unwrap_or(q.to);
                // 一条跨节点的接力会话只问最后一个节点会丢掉交接前的样本。
                let mut nodes: std::collections::BTreeSet<&str> =
                    members.iter().map(|s| s.node.as_str()).collect();
                if nodes.len() == 1 {
                    let only = nodes.pop_first().unwrap_or_default();
                    *scope = if only == node {
                        Scope::Local
                    } else {
                        Scope::Node(only.to_string())
                    };
                }
                missing_nodes.extend(m);
            }
            q.tier = Some(query::choose_tier(q.from, q.to, now, retention.raw_days).into());
            let req = PeerRequest {
                name: name.into(),
                query: q.clone(),
            };
            let (parts, mut m) = run(req, scope.clone()).await.map_err(internal)?;
            missing_nodes.append(&mut m);
            let parts: Vec<StatsParts> = decode(parts, &mut missing_nodes);
            let mut out = match query::merge_stats(parts) {
                Some(p) => query::render_stats(&p),
                None => json!({ "resolution": "raw", "ts": [], "series": {} }),
            };
            out["missing_nodes"] = json!(missing_nodes);
            Ok(out)
        }
        _ => Err((StatusCode::NOT_FOUND, "Unknown route".into())),
    }
}

async fn handle(name: &str, params: HashMap<String, String>, headers: HeaderMap) -> Response {
    admitted_query(&COORDINATORS, handle_admitted(name, params, headers)).await
}

async fn handle_admitted(
    name: &str,
    params: HashMap<String, String>,
    headers: HeaderMap,
) -> Response {
    let cfg = match crate::config::load_config().await {
        Ok(cfg) => cfg,
        Err(e) if crate::config::is_not_configured_error(&*e) => return not_configured_response(),
        Err(_) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "读取配置失败"),
    };
    let peers = LivePeers { cfg: &cfg };
    match answer(
        &cfg,
        name,
        &params,
        &peers,
        &LocalDb,
        crate::storage::now() as i64,
    )
    .await
    {
        Ok(data) => {
            let sorted: std::collections::BTreeMap<_, _> = params.iter().collect();
            conditional_ok(data, &headers, &format!("{name}:{sorted:?}"))
        }
        Err((status, message)) => fail(status, &message),
    }
}

pub async fn summary(
    Params(params): Params<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    handle("summary", params, headers).await
}

pub async fn sessions(
    Params(params): Params<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    handle("sessions", params, headers).await
}

pub async fn events(
    Params(params): Params<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    handle("events", params, headers).await
}

pub async fn stats(
    Params(params): Params<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    handle("stats", params, headers).await
}

/// 写入线程为一次清理预留的时间预算。
const CLEAN_TIMEOUT: Duration = Duration::from_secs(30);

/// 破坏性维护：清掉本节点已记录的行。每个归档只由自己的写入线程清理，
/// 正在转播的节点会拒绝。已经清空的归档再清一次是无操作，因此重复点击会收敛。
pub async fn clean() -> Response {
    let cfg = match crate::config::load_config().await {
        Ok(cfg) => cfg,
        Err(_) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "读取配置失败"),
    };
    let local = match crate::history::clear(CLEAN_TIMEOUT).await {
        Ok(rows) => rows,
        Err(message) => return fail(StatusCode::CONFLICT, &message),
    };
    let me = local_node(&cfg);
    crate::webui::events::publish(crate::webui::events::HISTORY);
    ok(clean_report(&me, local, Vec::new()))
}

/// 按钮读的那份报告：总数、每个清过的节点，以及每个留着行的节点和原因。
/// 本机永远排第一个，于是单机安装的读法与集群一致。
pub(crate) fn clean_report(
    me: &str,
    local: u64,
    remote: Vec<(String, Result<u64, String>)>,
) -> Value {
    let mut rows = local;
    let mut nodes = vec![json!({ "node_id": me, "rows": local })];
    let mut failed = Vec::new();
    for (node, result) in remote {
        match result {
            Ok(peer_rows) => {
                rows += peer_rows;
                nodes.push(json!({ "node_id": node, "rows": peer_rows }));
            }
            Err(reason) => failed.push(json!({ "node_id": node, "reason": reason })),
        }
    }
    json!({ "rows": rows, "nodes": nodes, "failed": failed })
}

pub(crate) fn status_value(cfg: &Config) -> Value {
    let node = local_node(cfg);
    let mut cluster_nodes = vec![node.clone()];
    if cfg.cluster.enabled {
        cluster_nodes.extend(cfg.cluster.peers.iter().map(|peer| peer.node_id.clone()));
        cluster_nodes.sort();
        cluster_nodes.dedup();
    }
    let retention = &cfg.history.retention;
    let retention = json!({
        "raw_days": retention.raw_days, "rollup_days": retention.rollup_days,
        "event_days": retention.event_days, "event_rows": retention.event_rows,
        "max_mb":retention.max_mb,
    });
    let recorder = crate::notify::recorder_status();
    let mut dropped: std::collections::BTreeMap<&str, u64> =
        crate::events::dropped().into_iter().collect();
    *dropped.entry("notify").or_default() +=
        crate::notify::dropped()["commands"].as_u64().unwrap_or(0);
    dropped.entry("history").or_default();
    let Some(shared) = crate::history::shared() else {
        return json!({
            "node_id": node, "cluster_nodes": cluster_nodes,
            "enabled": cfg.history.enabled, "schema_version": crate::history::schema::SCHEMA_VERSION, "oldest_at": null,
            "configured_enabled":cfg.history.enabled,"restart_required":false,
            "recording_since": null, "db_bytes": 0,
            "rows": {"sessions": 0, "runs": 0, "events": 0, "samples_raw": 0, "samples_1m": 0, "notify_sends": 0},
            "dropped": dropped,
            "writer": {"state": "stopped", "last_batch_at": null, "last_error": null},
            "retention": retention, "recorder": recorder, "missing_nodes": [],
        });
    };
    let size = |p: &std::path::Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let wal = shared.path.with_extension("sqlite-wal");
    let (rows, oldest, partial_before) = match crate::history::schema::open_ro(&shared.path)
        .and_then(|conn| {
            install_query_deadline(&conn, std::time::Instant::now() + QUERY_TIMEOUT)?;
            Ok(conn)
        }) {
        Ok(conn) => (
            query::status_rows(&conn).unwrap_or(Value::Null),
            query::oldest_at(&conn).ok().and_then(|o| o.0),
            query::retention_floor(&conn).ok().flatten(),
        ),
        Err(_) => (Value::Null, None, None),
    };
    let writer = shared
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    *dropped.entry("history").or_default() +=
        shared.dropped.load(std::sync::atomic::Ordering::Relaxed);
    let db_bytes = size(&shared.path) + size(&wal);
    json!({
        "node_id": node, "cluster_nodes": cluster_nodes,
        "enabled": shared.enabled,
        "configured_enabled":cfg.history.enabled,"restart_required":shared.enabled!=cfg.history.enabled,
        "schema_version": crate::history::schema::SCHEMA_VERSION,
        "oldest_at": oldest,
        "recording_since": shared.enabled.then_some(shared.recording_since),
        "db_bytes": db_bytes,
        "wal_bytes":size(&wal),
        "space_pressure": db_bytes>=cfg.history.retention.max_mb.max(1).saturating_mul(1024*1024),
        "partial_before":partial_before,
        "rows": rows,
        "dropped": dropped,
        // 写入错误是 SQLite 的措辞；不带路径也不带密钥。
        "writer": {"state": writer.state, "last_batch_at": writer.last_batch_at,
                   "last_error": writer.last_error.map(|e| crate::logging::safe_text(&e, 120))},
        "retention": retention,
        "recorder": recorder,
        "missing_nodes": [],
    })
}

pub async fn status(headers: HeaderMap) -> Response {
    let cfg = match crate::config::load_config().await {
        Ok(cfg) => cfg,
        Err(e) if crate::config::is_not_configured_error(&*e) => return not_configured_response(),
        Err(_) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "读取配置失败"),
    };
    let permit = match acquire_db_slot(&QUERIES).await {
        Ok(p) => p,
        Err(e) => return fail(StatusCode::TOO_MANY_REQUESTS, &e),
    };
    let value = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        status_value(&cfg)
    })
    .await
    .unwrap_or(Value::Null);
    conditional_ok(value, &headers, "status")
}

/// 对端路由的准入规则：名字必须是四个数据查询之一，范围必须合法。
/// 比本节点 rollup_days 更长的范围会被裁剪，返回值是裁剪了几天。
pub(crate) fn check_peer_request(
    req: &mut PeerRequest,
    cfg: &Config,
    now: i64,
) -> Result<Option<u64>, BadRequest> {
    if !matches!(
        req.name.as_str(),
        "summary" | "sessions" | "events" | "stats"
    ) {
        return Err(BadRequest("未知统计查询".into()));
    }
    crate::history::query::validate_peer(&req.query, now)
        .map_err(|e| BadRequest(format!("统计查询无效：{}", e.0)))?;
    if req.name == "stats" {
        let cutoff = now - cfg.history.retention.raw_days as i64 * 86_400;
        req.query.raw_from = (req.query.from < cutoff).then(|| (cutoff + 59).div_euclid(60) * 60);
    }
    Ok(crate::history::query::clamp_peer(
        &mut req.query,
        cfg.history.retention.rollup_days,
    ))
}


/// 测试用的临时库连同 WAL 伴生文件一起删掉，不在 /tmp 留残留。
#[cfg(test)]
fn remove_sqlite(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

#[cfg(test)]
mod conditional_tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn conditional_reply_uses_merged_rows_coverage_and_query_identity() {
        let original = json!({"items":[{"eid":"a:1:1"}],"missing_nodes":["b"]});
        let response = conditional_ok(original.clone(), &HeaderMap::new(), "sessions:cluster");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-cache"
        );
        let tag = response.headers()[header::ETAG]
            .to_str()
            .unwrap()
            .to_string();
        let mut request = HeaderMap::new();
        request.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&format!("\"unrelated\", W/{tag}")).unwrap(),
        );
        let unchanged = conditional_ok(original.clone(), &request, "sessions:cluster");
        assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED);
        assert!(to_bytes(unchanged.into_body(), MAX_RESPONSE_BYTES)
            .await
            .unwrap()
            .is_empty());
        let peer_arrived = conditional_ok(
            json!({"items":[{"eid":"a:1:1"},{"eid":"b:2:3"}],"missing_nodes":[]}),
            &request,
            "sessions:cluster",
        );
        assert_eq!(peer_arrived.status(), StatusCode::OK);
        assert_ne!(peer_arrived.headers()[header::ETAG], tag);
        assert_eq!(
            conditional_ok(original, &request, "sessions:local").status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn response_cap_counts_encoded_bytes_and_never_truncates_json() {
        let response = conditional_ok(
            json!({"detail":"界".repeat(MAX_RESPONSE_BYTES/2)}),
            &HeaderMap::new(),
            "events",
        );
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(response.headers().get(header::ETAG).is_none());
        let bytes = to_bytes(response.into_body(), MAX_RESPONSE_BYTES)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["success"], false);
        assert!(body["message"].as_str().unwrap().contains("缩小范围"));
    }

    #[tokio::test]
    async fn sqlite_deadline_survives_caller_cancellation_and_releases_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "bilistream-history-cancel-deadline-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let writer = crate::history::schema::open_rw(&path).unwrap();
        writer.execute("INSERT INTO events(eid,at,node,kind,severity,title,detail) VALUES('initial',1,'local','test','info','','')",[]).unwrap();
        let queries = Arc::new(tokio::sync::Semaphore::new(1));
        let worker_queries = Arc::clone(&queries);
        let worker_path = path.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (begin_tx, begin_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let caller = tokio::spawn(async move {
            read_database(worker_path,&worker_queries,move |conn| {
                let conn=conn.unwrap();
                conn.execute_batch("BEGIN; SELECT count(*) FROM events;")?;
                let _=ready_tx.send(());
                begin_rx.recv().unwrap();
                let result=conn.query_row(
                    "WITH RECURSIVE work(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM work WHERE n<1000000000) SELECT sum(n) FROM work",
                    [], |row|row.get::<_,i64>(0),
                );
                let _=done_tx.send(result.as_ref().err().and_then(|e|e.sqlite_error_code()));
                result.map(|value|json!(value))
            }).await
        });
        tokio::time::timeout(Duration::from_secs(1), ready_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(queries.available_permits(), 0);
        writer.execute("INSERT INTO events(eid,at,node,kind,severity,title,detail) VALUES('newer',2,'local','test','info','','')",[]).unwrap();
        writer.busy_timeout(Duration::ZERO).unwrap();
        assert_eq!(
            writer
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1,
            "读取方钉住了更早的已提交快照"
        );
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        assert_eq!(
            queries.available_permits(),
            0,
            "调用方取消不会释放 SQL 准入"
        );
        begin_tx.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(4), done_rx)
                .await
                .unwrap()
                .unwrap(),
            Some(rusqlite::ErrorCode::OperationInterrupted)
        );
        let released =
            tokio::time::timeout(Duration::from_secs(1), Arc::clone(&queries).acquire_owned())
                .await
                .unwrap()
                .unwrap();
        drop(released);
        assert_eq!(queries.available_permits(), 1);
        assert_eq!(
            writer
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            std::fs::metadata(path.with_extension("sqlite-wal"))
                .unwrap()
                .len(),
            0
        );
        remove_sqlite(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
        routing::get,
        Router,
    };
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    struct DelayedLocal {
        conn: Mutex<rusqlite::Connection>,
        started: tokio::sync::Semaphore,
        release: tokio::sync::Semaphore,
        now: i64,
    }

    impl Peers for DelayedLocal {
        fn fetch<'a>(&'a self, node: &'a str, req: &'a PeerRequest) -> PeerFuture<'a> {
            Box::pin(async move {
                self.started.add_permits(1);
                self.release.acquire().await.unwrap().forget();
                local_parts(&self.conn.lock().unwrap(), req, node, self.now)
                    .map_err(|_| missing(node, "error", "query failed"))
            })
        }
    }

    struct ConcurrentNode {
        coordinators: tokio::sync::Semaphore,
        queries: Arc<tokio::sync::Semaphore>,
        conn: Mutex<rusqlite::Connection>,
        active: std::sync::atomic::AtomicUsize,
        peak: std::sync::atomic::AtomicUsize,
        now: i64,
    }

    impl Peers for ConcurrentNode {
        fn fetch<'a>(&'a self, node: &'a str, req: &'a PeerRequest) -> PeerFuture<'a> {
            Box::pin(async move {
                let _permit = acquire_db_slot(&self.queries)
                    .await
                    .map_err(|e| missing(node, "error", &e))?;
                let active = self
                    .active
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                self.peak
                    .fetch_max(active, std::sync::atomic::Ordering::SeqCst);
                // 留足时间让两个面板的互相读取争真正的两个库槽位。
                tokio::time::sleep(Duration::from_millis(20)).await;
                let out = local_parts(&self.conn.lock().unwrap(), req, node, self.now)
                    .map_err(|_| missing(node, "error", "query failed"));
                self.active
                    .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                out
            })
        }
    }

    #[tokio::test]
    async fn reciprocal_dashboard_fanout_does_not_reserve_peer_database_slots() {
        let now = crate::storage::now() as i64;
        let make = |node: &str| {
            let path = std::env::temp_dir().join(format!(
                "bilistream-history-fanout-{node}-{}.sqlite",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            let conn = crate::history::schema::open_rw(&path).unwrap();
            conn.execute("INSERT INTO sessions (eid,node,boot_id,seq,platform,channel_id,channel_name,started_at,last_seen_at,ended_at,end_reason)
                VALUES (?1,?2,1,1,'YT','test','test',?3,?4,?4,'source_ended')", rusqlite::params![format!("{node}:1:1"), node, now - 200, now - 100]).unwrap();
            Arc::new(ConcurrentNode {
                coordinators: tokio::sync::Semaphore::new(2),
                queries: Arc::new(tokio::sync::Semaphore::new(2)),
                conn: Mutex::new(conn),
                active: 0.into(),
                peak: 0.into(),
                now,
            })
        };
        let a = make("a");
        let b = make("b");
        let barrier = Arc::new(tokio::sync::Barrier::new(4));
        let dashboard = |node: Arc<ConcurrentNode>,
                         peer: Arc<ConcurrentNode>,
                         id: &'static str,
                         other: &'static str| {
            let barrier = Arc::clone(&barrier);
            async move {
                let mut cfg = crate::test_support::config(id, 0);
                cfg.cluster.enabled = true;
                cfg.cluster.peers = vec![crate::config::ClusterPeer {
                    node_id: other.into(),
                    name: String::new(),
                    api_url: format!("http://{other}.example"),
                    priority: 0,
                }];
                admitted_query(&node.coordinators, async {
                    // 四个协调许可先占满再开始对端读，复现原先的环形互等。
                    barrier.wait().await;
                    match answer(&cfg, "sessions", &HashMap::new(), &*peer, &*node, now).await {
                        Ok(data) => ok(data),
                        Err((status, message)) => fail(status, &message),
                    }
                })
                .await
            }
        };
        let responses = tokio::time::timeout(
            Duration::from_secs(2),
            futures_util::future::join_all([
                dashboard(Arc::clone(&a), Arc::clone(&b), "a", "b"),
                dashboard(Arc::clone(&a), Arc::clone(&b), "a", "b"),
                dashboard(Arc::clone(&b), Arc::clone(&a), "b", "a"),
                dashboard(Arc::clone(&b), Arc::clone(&a), "b", "a"),
            ]),
        )
        .await
        .unwrap();
        for response in responses {
            assert_eq!(response.status(), StatusCode::OK);
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                    .unwrap();
            assert_eq!(body["data"]["items"].as_array().unwrap().len(), 2);
            assert_eq!(body["data"]["missing_nodes"], json!([]));
        }
        for node in [a, b] {
            assert_eq!(node.peak.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert_eq!(node.active.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
        for node in ["a", "b"] {
            remove_sqlite(
                &std::env::temp_dir()
                    .join(format!("bilistream-history-fanout-{node}-{}.sqlite", std::process::id())),
            );
        }
    }

    #[tokio::test]
    async fn cancelling_a_caller_does_not_release_its_blocking_database_slot() {
        let queries = Arc::new(tokio::sync::Semaphore::new(2));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let caller = tokio::spawn({
            let queries = Arc::clone(&queries);
            async move {
                let permit = acquire_db_slot(&queries).await.unwrap();
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let _ = started_tx.send(());
                    release_rx.recv().unwrap();
                })
                .await
                .unwrap();
            }
        });
        started_rx.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        assert_eq!(
            queries.available_permits(),
            1,
            "HTTP 取消后 SQL 仍占着一个槽位"
        );
        let second = acquire_db_slot(&queries).await.unwrap();
        assert!(
            queries.try_acquire().is_err(),
            "第三个库查询不得开始"
        );
        release_tx.send(()).unwrap();
        let released = tokio::time::timeout(Duration::from_secs(1), acquire_db_slot(&queries))
            .await
            .unwrap()
            .unwrap();
        drop((second, released));
        assert_eq!(queries.available_permits(), 2);
    }

    #[tokio::test]
    async fn handler_keeps_two_query_admission_until_the_response_finishes() {
        let path = std::env::temp_dir().join(format!(
            "bilistream-history-admission-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let conn = crate::history::schema::open_rw(&path).unwrap();
        let now = crate::storage::now() as i64;
        conn.execute("INSERT INTO sessions (eid,node,boot_id,seq,platform,channel_id,channel_name,started_at,last_seen_at,ended_at,end_reason)
            VALUES ('local:1:1','local',1,1,'YT','test','test',?1,?2,?2,'source_ended')", rusqlite::params![now - 200, now - 100]).unwrap();
        let local = Arc::new(DelayedLocal {
            conn: Mutex::new(conn),
            started: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
            now,
        });
        let queries = Arc::new(tokio::sync::Semaphore::new(2));
        let mut cfg = crate::test_support::config("local", 0);
        cfg.cluster.enabled = false;
        let cfg = Arc::new(cfg);
        let app = Router::new().route(
            "/api/history/sessions",
            get({
                let local = Arc::clone(&local);
                move || {
                    let local = Arc::clone(&local);
                    let queries = Arc::clone(&queries);
                    let cfg = Arc::clone(&cfg);
                    async move {
                        admitted_query(&queries, async {
                            match answer(&cfg, "sessions", &HashMap::new(), &*local, &*local, now)
                                .await
                            {
                                Ok(data) => ok(data),
                                Err((status, message)) => fail(status, &message),
                            }
                        })
                        .await
                    }
                }
            }),
        );
        let request = || {
            Request::builder()
                .uri("/api/history/sessions")
                .body(Body::empty())
                .unwrap()
        };
        let a = tokio::spawn(app.clone().oneshot(request()));
        let b = tokio::spawn(app.clone().oneshot(request()));
        tokio::time::timeout(Duration::from_secs(1), local.started.acquire_many(2))
            .await
            .unwrap()
            .unwrap()
            .forget();
        let refused = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            local.started.available_permits(),
            0,
            "第三个请求不得开始库/对端读"
        );
        let refusal: Value =
            serde_json::from_slice(&to_bytes(refused.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(refusal["success"], false);
        local.release.add_permits(2);
        for response in [a.await.unwrap().unwrap(), b.await.unwrap().unwrap()] {
            assert_eq!(response.status(), StatusCode::OK);
            let data: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                    .unwrap();
            assert_eq!(data["data"]["items"][0]["id"], "local:1:1");
        }
        // 完成后准入就被释放，后来的客户端还能读。
        local.release.add_permits(1);
        assert_eq!(
            app.oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
        drop(local);
        remove_sqlite(&path);
    }

    /// 按钮读的那份报告：总数、本机排第一，以及每个留着行的节点和原因。
    #[test]
    fn clean_report_sums_every_node_and_names_the_ones_that_kept_rows() {
        let report = clean_report(
            "local",
            10,
            vec![
                ("tokyo-1".into(), Ok(5)),
                (
                    "sg-3".into(),
                    Err("正在转播，请先停止转播再清理历史".into()),
                ),
            ],
        );
        assert_eq!(report["rows"], 15);
        assert_eq!(
            report["nodes"][0],
            json!({ "node_id": "local", "rows": 10 })
        );
        assert_eq!(
            report["nodes"][1],
            json!({ "node_id": "tokyo-1", "rows": 5 })
        );
        assert_eq!(report["failed"][0]["node_id"], "sg-3");
        assert_eq!(
            report["failed"][0]["reason"],
            "正在转播，请先停止转播再清理历史"
        );
        let solo = clean_report("local", 0, Vec::new());
        assert_eq!(solo["rows"], 0);
        assert_eq!(solo["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(solo["failed"].as_array().unwrap().len(), 0);
    }


}
