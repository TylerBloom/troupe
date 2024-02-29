//! The compatability layer between async runtimes as well as native vs WASM targets.

use std::fmt::Debug;

use futures::{stream::FusedStream, Future, Stream};

#[cfg(not(target_family = "wasm"))]
mod native;
#[cfg(not(target_family = "wasm"))]
pub use native::*;

#[cfg(target_family = "wasm")]
mod wasm;
#[cfg(target_family = "wasm")]
pub use wasm::*;

/// This trait abstracts over the requirements for spawning a task. In native async runtimes, a
/// task might be ran in a different thread, so the future must be `'static + Send`. In WASM, you
/// are always running in a single thread, so spawning a task only requires that the future that
/// the future is `'static`. This concept is used throughout `troupe` to make writing actors in
/// WASM as easy as possible.
pub trait Sendable: 'static + MaybeSend {}

impl<T> Sendable for T where T: 'static + Send {}

/// For native targets, troupe requires nearly every future to be `Send`. For WASM targets, nothing
/// needs to be `Send` because nothing can be sent across threads.
pub trait MaybeSendFuture: MaybeSend + Future {}

impl<T> MaybeSendFuture for T where T: MaybeSend + Future {}

/// A trait to abstract over if a future can be processed in a seperate async process.
pub trait SendableFuture: Sendable + Future {}

impl<T> SendableFuture for T where T: Sendable + Future {}

/// A trait to abstract over if a stream can be managed by the [`Scheduler`](crate::Scheduler).
pub trait SendableStream: Sendable + Unpin + Stream {}

impl<T> SendableStream for T where T: Sendable + Unpin + Stream {}

/// A trait to abstract over if a fused stream can be managed by the
/// [`Scheduler`](crate::Scheduler).
pub trait SendableFusedStream: SendableStream + FusedStream {}

impl<T> SendableFusedStream for T where T: SendableStream + FusedStream {}

impl Debug for Sleep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sleep(..)")
    }
}

#[cfg(test)]
mod test {
    // Import tests
    #[allow(unused_imports)]
    use super::{sleep_for, sleep_until, spawn_task, Sendable};

    // Impl trait tests
}
