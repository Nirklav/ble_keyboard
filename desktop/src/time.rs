//! Timers for both builds: tokio natively, the browser ones in wasm.

use std::future::Future;
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await
}

#[cfg(target_arch = "wasm32")]
pub async fn sleep(duration: Duration) {
    gloo_timers::future::sleep(duration).await
}

/// `None` if `future` doesn't finish within `duration`.
#[cfg(not(target_arch = "wasm32"))]
pub async fn timeout<F: Future>(duration: Duration, future: F) -> Option<F::Output> {
    tokio::time::timeout(duration, future).await.ok()
}

/// `None` if `future` doesn't finish within `duration`.
#[cfg(target_arch = "wasm32")]
pub async fn timeout<F: Future>(duration: Duration, future: F) -> Option<F::Output> {
    use futures::future::{Either, select};
    match select(std::pin::pin!(future), std::pin::pin!(sleep(duration))).await {
        Either::Left((output, _)) => Some(output),
        Either::Right(_) => None,
    }
}
