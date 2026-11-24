//! SSE 变更流：服务端状态一变，面板立刻重取，不必等轮询。
//!
//! 撤权检查通过 [`SseAuth`] 注入，因此本模块不依赖 `biliup-cli` 的登录态，
//! 也就不存在反向依赖。

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Extension;
use futures_util::stream::Stream;
use tokio::sync::{broadcast, watch};

pub use crate::ui_events::{CLUSTER, CONFIG, HISTORY, HOLODEX, STATUS};
use crate::app_state::AppState;

/// 两次撤权复查之间最长的心跳间隔：没有事件时也要发现连接已被撤销。
const AUTH_RECHECK: Duration = Duration::from_secs(30);

type CheckFuture = Pin<Box<dyn Future<Output = bool> + Send>>;
type Check = dyn Fn(Arc<HeaderMap>) -> CheckFuture + Send + Sync;

/// 连接撤权钩子：判定 + 唤醒。
///
/// 判定在每次唤醒时执行，返回 `false` 表示该连接已被撤销、必须立刻断流。
/// [`SseAuth::revoke`] 让在流上的连接立刻复查，不必等到下一个事件。
#[derive(Clone)]
pub struct SseAuth {
    check: Arc<Check>,
    revision: Arc<watch::Sender<u64>>,
}

impl SseAuth {
    /// 默认允许：路由没有装鉴权层时（P2-05 接线前）流照常工作。
    pub fn allow_all() -> Self {
        Self::with_check(|_| Box::pin(async { true }))
    }

    pub fn with_check<F>(check: F) -> Self
    where
        F: Fn(Arc<HeaderMap>) -> CheckFuture + Send + Sync + 'static,
    {
        let (revision, _) = watch::channel(0);
        Self {
            check: Arc::new(check),
            revision: Arc::new(revision),
        }
    }

    /// 供每个连接独立使用的唤醒端；之后调用 [`SseAuth::revoke`] 都能到达它。
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    /// 权限可能变了：让所有在流上的连接立刻复查。
    pub fn revoke(&self) {
        self.revision
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    async fn allowed(&self, headers: &Arc<HeaderMap>) -> bool {
        (self.check)(Arc::clone(headers)).await
    }
}

/// 通知所有已连接的面板：`kind` 变了。没有订阅者时静默丢弃，不阻塞调用方。
pub fn publish(kind: &'static str) {
    AppState::current().publish(kind);
}

/// SSE 端点：服务端状态变化时发出一条同名事件。
pub async fn sse_events(
    State(state): State<AppState>,
    auth: Option<Extension<Arc<SseAuth>>>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // 先订阅再判权：判权与流构造之间发生的变更不会漏掉已鉴权的流。
    let rx = state.subscribe_events();
    let auth = auth
        .map(|Extension(auth)| auth)
        .unwrap_or_else(|| Arc::new(SseAuth::allow_all()));
    Sse::new(sse_stream(rx, auth, Arc::new(headers))).keep_alive(KeepAlive::default())
}

/// 事件流本体：先判权，再等事件；每次唤醒都复查一次撤权钩子。
pub(crate) fn sse_stream(
    rx: broadcast::Receiver<&'static str>,
    auth: Arc<SseAuth>,
    headers: Arc<HeaderMap>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let changed = auth.subscribe();
    sse_stream_with(rx, auth, headers, changed)
}

/// 事件流本体；唤醒端由调用方给出，测试可以直接观察钩子释放时的行为。
fn sse_stream_with(
    rx: broadcast::Receiver<&'static str>,
    auth: Arc<SseAuth>,
    headers: Arc<HeaderMap>,
    changed: watch::Receiver<u64>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures_util::stream::unfold(
        (rx, auth, headers, changed),
        |(mut rx, auth, headers, mut changed)| async move {
            loop {
                if !auth.allowed(&headers).await {
                    return None;
                }
                let event = tokio::select! {
                    result = changed.changed() => {
                        // 钩子已释放：不会再有撤权通知，断开比静默长连更安全。
                        if result.is_err() {
                            return None;
                        }
                        continue;
                    }
                    _ = tokio::time::sleep(AUTH_RECHECK) => continue,
                    event = rx.recv() => event,
                };
                if !auth.allowed(&headers).await {
                    return None;
                }
                let event = match event {
                    Ok(kind) => Event::default().event(kind).data("changed"),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        Event::default().event("refresh").data("all")
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                };
                return Some((Ok(event), (rx, auth, headers, changed)));
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, BodyDataStream};
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use futures_util::StreamExt;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tower::ServiceExt;

    fn router(state: AppState, auth: Arc<SseAuth>) -> Router {
        Router::new()
            .route("/api/events", get(sse_events))
            .layer(Extension(auth))
            .with_state(state)
    }

    async fn open(app: Router) -> BodyDataStream {
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.into_body().into_data_stream()
    }

    /// 读到 want 里每一条都出现为止；超时即判定流没送达。
    async fn collect_until(stream: &mut BodyDataStream, want: &[&str]) -> String {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let mut seen = String::new();
        loop {
            assert!(
                tokio::time::Instant::now() < deadline,
                "事件未在期限内到达，已收到：{seen}"
            );
            let remaining = deadline - tokio::time::Instant::now();
            let Ok(next) = tokio::time::timeout(remaining, stream.next()).await else {
                panic!("事件流超时，已收到：{seen}");
            };
            let Some(Ok(chunk)) = next else {
                panic!("事件流提前结束，已收到：{seen}");
            };
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if want.iter().all(|kind| seen.contains(kind)) {
                return seen;
            }
        }
    }

    async fn closed_within(stream: &mut BodyDataStream, wait: Duration) -> bool {
        matches!(tokio::time::timeout(wait, stream.next()).await, Ok(None))
    }

    #[tokio::test]
    async fn authority_is_checked_after_subscribing_so_the_first_event_is_not_lost() {
        let state = AppState::new();
        let checks = Arc::new(AtomicUsize::new(0));
        let publishing = {
            let state = state.clone();
            let checks = Arc::clone(&checks);
            move |_headers: Arc<HeaderMap>| -> CheckFuture {
                if checks.fetch_add(1, Ordering::SeqCst) == 0 {
                    // 判权期间发生的变更：只有先订阅才拿得到。
                    state.publish(STATUS);
                }
                Box::pin(async { true })
            }
        };
        let mut stream = open(router(state, Arc::new(SseAuth::with_check(publishing)))).await;

        let seen = collect_until(&mut stream, &["event: status"]).await;
        assert!(seen.contains("data: changed"), "事件必须带 data：{seen}");
        assert!(checks.load(Ordering::SeqCst) >= 1, "判权必须执行过");
    }

    #[tokio::test]
    async fn every_published_category_reaches_the_stream() {
        let state = AppState::new();
        let mut stream = open(router(state.clone(), Arc::new(SseAuth::allow_all()))).await;
        for kind in [STATUS, CLUSTER, CONFIG, HOLODEX, HISTORY] {
            state.publish(kind);
        }
        let want: Vec<String> = [STATUS, CLUSTER, CONFIG, HOLODEX, HISTORY]
            .iter()
            .map(|kind| format!("event: {kind}"))
            .collect();
        let want: Vec<&str> = want.iter().map(String::as_str).collect();
        collect_until(&mut stream, &want).await;
    }

    #[tokio::test]
    async fn a_lagged_subscriber_is_told_to_refresh_everything() {
        let state = AppState::new();
        let mut stream = open(router(state.clone(), Arc::new(SseAuth::allow_all()))).await;
        for _ in 0..96 {
            state.publish(HOLODEX);
        }
        let seen = collect_until(&mut stream, &["event: refresh"]).await;
        assert!(seen.contains("data: all"), "落后补发必须是全量刷新：{seen}");
    }

    #[tokio::test]
    async fn revoking_the_connection_ends_an_idle_stream() {
        let state = AppState::new();
        let allowed = Arc::new(AtomicBool::new(true));
        let gate = Arc::clone(&allowed);
        let auth = Arc::new(SseAuth::with_check(
            move |_headers: Arc<HeaderMap>| -> CheckFuture {
                let gate = Arc::clone(&gate);
                Box::pin(async move { gate.load(Ordering::SeqCst) })
            },
        ));
        let mut stream = open(router(state.clone(), Arc::clone(&auth))).await;

        state.publish(STATUS);
        collect_until(&mut stream, &["event: status"]).await;
        assert!(
            !closed_within(&mut stream, Duration::from_millis(100)).await,
            "撤权前的空闲流必须保持连接"
        );

        allowed.store(false, Ordering::SeqCst);
        auth.revoke();
        assert!(
            closed_within(&mut stream, Duration::from_secs(1)).await,
            "撤权钩子翻 false 后必须立刻断流"
        );
    }

    /// 撤权钩子的唤醒端被释放后，流必须立刻结束而不是空转。
    #[tokio::test]
    async fn a_dropped_watch_sender_ends_the_stream() {
        use axum::response::IntoResponse;

        let state = AppState::new();
        let auth = Arc::new(SseAuth::allow_all());
        let (sender, changed) = watch::channel(0u64);
        let stream = sse_stream_with(
            state.subscribe_events(),
            Arc::clone(&auth),
            Arc::new(HeaderMap::new()),
            changed,
        );
        let mut body = Sse::new(stream)
            .into_response()
            .into_body()
            .into_data_stream();
        sender.send_modify(|revision| *revision += 1);
        drop(sender);
        assert!(
            closed_within(&mut body, Duration::from_secs(1)).await,
            "watch sender 释放后流必须立刻结束"
        );
    }
}
