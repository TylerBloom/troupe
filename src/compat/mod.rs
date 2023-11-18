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
