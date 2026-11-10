//! Neo: Counts MCP POSTs across both rmcp handoffs: awaiting the first response and producing
//! its body. Shutdown waits for reply ownership to end, subject to the consumer grace.
//! GET streams are excluded because they need transport cancellation to end.
//!
//! A counted POST's reply ownership ends when hyper polls its body to `Ready(None)` (every
//! frame handed to hyper's connection writer, before the final chunk is buffered or flushed)
//! or when the body is dropped (client disconnect, connection error, or an unread response).
//! Only `Ready(None)` is a server-side handoff, and neither is client receipt.
//! `transport.cancel()` follows every ownership end unless `REPLY_DELIVERY_GRACE` expired
//! first, and the grace exists only for consumers that stop reading. Client receipt of the
//! complete reply is asserted end to end by T1, T4, T6 and T7. Socket flush timing, TCP receipt
//! and the order in which a client observes two connections are not server guarantees.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use http_body::{Body as HttpBody, Frame, SizeHint};
use tokio::time;
use tokio_util::task::TaskTracker;
use tokio_util::task::task_tracker::TaskTrackerToken;

/// Neo: Only stalled reply consumers are bounded; run work and runtime cleanup stay unbounded.
pub(crate) const REPLY_DELIVERY_GRACE: Duration = Duration::from_secs(5);

/// Shared by all transport clones. A token owns one POST until its HTTP body ends or drops.
#[derive(Clone, Default)]
pub(crate) struct ReplyTracker(TaskTracker);

impl ReplyTracker {
    pub(crate) async fn track(
        State(replies): State<Self>,
        request: Request,
        next: Next,
    ) -> Response {
        // Keep this local across the await: taking ownership only after rmcp returns would
        // leave its first-message select unprotected. Cancellation before handoff drops it.
        let token = (request.method() == Method::POST).then(|| replies.0.token());
        let response = next.run(request).await;
        let Some(token) = token else {
            return response;
        };
        let (parts, body) = response.into_parts();
        Response::from_parts(
            parts,
            Body::new(Tracked {
                body,
                token: Some(token),
            }),
        )
    }

    /// Called after owned run work ends. Closing does not forbid new tokens: POSTs arriving
    /// during the drain can extend it within `grace`. Arrivals after it resolves are refused
    /// shutdown requests and have no in-flight run delivery guarantee.
    pub(crate) async fn drain(&self, grace: Duration) -> bool {
        self.0.close();
        time::timeout(grace, self.0.wait()).await.is_ok()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

/// The token follows the body into hyper, not merely into the handler's response value.
/// EOF releases it; drop also releases it for unpolled responses and disconnected clients.
struct Tracked {
    body: Body,
    token: Option<TaskTrackerToken>,
}

impl HttpBody for Tracked {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let frame = Pin::new(&mut self.body).poll_frame(cx);
        if matches!(&frame, Poll::Ready(None)) {
            drop(self.token.take());
        }
        frame
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::Router;
    use axum::body::{Body, Bytes};
    use axum::http::{Method, Request};
    use axum::middleware;
    use futures_util::future;
    use futures_util::stream;
    use futures_util::{StreamExt, poll};
    use http_body::Body as HttpBody;
    use tokio::sync::{Barrier, mpsc};
    use tokio::time;
    use tower::ServiceExt;

    use super::{REPLY_DELIVERY_GRACE, ReplyTracker, Tracked};

    fn request(method: Method) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri("/")
            .body(Body::empty())
            .unwrap()
    }

    fn streaming_router(
        replies: &ReplyTracker,
        receiver: mpsc::UnboundedReceiver<Result<Bytes, io::Error>>,
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
    ) -> Router {
        let receiver = Arc::new(Mutex::new(Some(receiver)));
        Router::new()
            .fallback(move || {
                let receiver = receiver.lock().unwrap().take().unwrap();
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release);
                async move {
                    entered.wait().await;
                    release.wait().await;
                    Body::from_stream(stream::unfold(receiver, |mut receiver| async move {
                        receiver.recv().await.map(|chunk| (chunk, receiver))
                    }))
                }
            })
            .layer(middleware::from_fn_with_state(
                replies.clone(),
                ReplyTracker::track,
            ))
    }

    // T3: deterministic W2 proof, including arrival/handler handoff and partial body delivery.
    #[tokio::test]
    async fn post_ownership_spans_the_handler_await_and_every_body_chunk() {
        let replies = ReplyTracker::default();
        let (sender, receiver) = mpsc::unbounded_channel();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let app = streaming_router(
            &replies,
            receiver,
            Arc::clone(&entered),
            Arc::clone(&release),
        );
        let pending = tokio::spawn(app.oneshot(request(Method::POST)));
        entered.wait().await;
        assert_eq!(
            replies.len(),
            1,
            "arrival is counted before the handler returns"
        );
        let drain = replies.drain(REPLY_DELIVERY_GRACE);
        tokio::pin!(drain);
        assert!(poll!(drain.as_mut()).is_pending());
        release.wait().await;
        let response = pending.await.unwrap().unwrap();
        assert_eq!(replies.len(), 1, "an unpolled body owns the token");
        assert!(poll!(drain.as_mut()).is_pending());
        let mut body = response.into_body().into_data_stream();
        sender.send(Ok(Bytes::from_static(b"first"))).unwrap();
        assert_eq!(body.next().await.unwrap().unwrap(), "first");
        assert!(poll!(drain.as_mut()).is_pending(), "one chunk is not EOF");
        drop(sender);
        assert!(body.next().await.is_none());
        assert!(drain.await);
        assert_eq!(replies.len(), 0);
    }

    #[tokio::test]
    async fn unread_body_drop_and_handler_cancellation_release_ownership() {
        for cancel_handler in [false, true] {
            let replies = ReplyTracker::default();
            let (_sender, receiver) = mpsc::unbounded_channel();
            let entered = Arc::new(Barrier::new(2));
            let release = Arc::new(Barrier::new(2));
            let app = streaming_router(
                &replies,
                receiver,
                Arc::clone(&entered),
                Arc::clone(&release),
            );
            let pending = tokio::spawn(app.oneshot(request(Method::POST)));
            entered.wait().await;
            assert_eq!(replies.len(), 1);
            if cancel_handler {
                pending.abort();
                assert!(pending.await.unwrap_err().is_cancelled());
            } else {
                release.wait().await;
                let response = pending.await.unwrap().unwrap();
                assert_eq!(replies.len(), 1);
                drop(response);
            }
            assert!(replies.drain(REPLY_DELIVERY_GRACE).await);
            assert_eq!(replies.len(), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn consumer_grace_expires_without_cancelling_or_timing_out_run_work() {
        let replies = ReplyTracker::default();
        let (_sender, receiver) = mpsc::unbounded_channel();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let app = streaming_router(
            &replies,
            receiver,
            Arc::clone(&entered),
            Arc::clone(&release),
        );
        let pending = tokio::spawn(app.oneshot(request(Method::POST)));
        entered.wait().await;
        release.wait().await;
        let response = pending.await.unwrap().unwrap();
        let drain = replies.drain(REPLY_DELIVERY_GRACE);
        tokio::pin!(drain);
        assert!(poll!(drain.as_mut()).is_pending());
        time::advance(
            REPLY_DELIVERY_GRACE
                .checked_sub(Duration::from_millis(1))
                .unwrap(),
        )
        .await;
        assert!(poll!(drain.as_mut()).is_pending());
        time::advance(Duration::from_millis(1)).await;
        assert!(
            !drain.await,
            "only the stalled body consumer exhausts the grace"
        );
        assert_eq!(replies.len(), 1, "the body remains owned until drop");
        drop(response);
        assert_eq!(replies.len(), 0);
    }

    #[tokio::test]
    async fn get_streams_are_excluded_and_full_body_hints_are_preserved() {
        let replies = ReplyTracker::default();
        let app = Router::new().fallback(|| async { Body::empty() }).layer(
            middleware::from_fn_with_state(replies.clone(), ReplyTracker::track),
        );
        let response = app.oneshot(request(Method::GET)).await.unwrap();
        assert_eq!(
            replies.len(),
            0,
            "GET streams must not deadlock the POST drain"
        );
        assert!(replies.drain(REPLY_DELIVERY_GRACE).await);
        drop(response);

        let mut body = Tracked {
            body: Body::from("x"),
            token: Some(replies.0.token()),
        };
        assert_eq!(body.size_hint().exact(), Some(1));
        assert!(!body.is_end_stream());
        let frame = future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await;
        assert_eq!(frame.unwrap().unwrap().into_data().unwrap(), "x");
        assert!(body.is_end_stream());
        assert_eq!(replies.len(), 1, "Full may finish without a final EOF poll");
        drop(body);
        assert_eq!(replies.len(), 0);
    }
}
