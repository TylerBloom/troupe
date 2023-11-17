use futures::FutureExt;
use gloo_timers::future::{sleep, TimeoutFuture};
use instant::{Duration, Instant};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use super::SendableFuture;
use send_wrapper::SendWrapper;

/* ------ Send workarounds ------ */

/// This trait abstracts over the requirements for spawning a task. In native async runtimes, a
/// task might be ran in a different thread, so the future must be `'static + Send`. In WASM, you
/// are always running in a single thread, so spawning a task only requires that the future that
/// the future is `'static`. This concept is used throughout `troupe` to make writing actors in
/// WASM as easy as possible.
pub trait Sendable: 'static {}

impl<T> Sendable for T where T: 'static {}

/// Because `async_trait` requires that trait futures are [`Send`] and the non-`Send` alternative
/// is worse, both the [`ActorState`](crate::Scheduler) and [`Scheduler`](crate::Scheduler) must be
/// `Send`. This is a problem for WASM. This wrapper provides a uniform interfaces between WASM and
/// non-WASM targets through which a `Send` workaround can be implemented.
///
/// For WASM targets, this wrapper is just [`SendWrapper`]. This is completely safe to use as WASM
/// applications are strictly bound to a single thread, so this wrapper will never panic.
pub type SendableWrapper<T> = SendWrapper<T>;

/* ------ General Utils ------ */

/// A wrapper around `wasm-bindgen-future`'s `spawn_local` function, which spawns a future tha
/// will execute in the background
pub(crate) fn spawn_task<F, T>(fut: F)
where
    F: SendableFuture<Output = T>,
    T: Sendable,
{
    wasm_bindgen_futures::spawn_local(fut.map(drop));
}

/// A future that will sleep for a period of time before waking up.
pub struct Sleep(TimeoutFuture);

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.0.poll_unpin(cx)
    }
}

/// Creates an instance of [`Sleep`] that will sleep for at least as long as the given duration.
pub fn sleep_for(dur: Duration) -> Sleep {
    Sleep(sleep(dur))
}

/// Creates an instance of [`Sleep`] that will sleep at least until the given point in time.
pub fn sleep_until(deadline: Instant) -> Sleep {
    sleep_for(deadline - Instant::now())
}
