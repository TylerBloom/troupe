//! This module contains the compatiablity layer to abstract over different async runtimes and
//! whether or not the compilation target is native or WASM.

use std::fmt::Debug;

use futures::{Future, Stream, stream::FusedStream};

#[cfg(not(target_family = "wasm"))]
mod native;
#[cfg(not(target_family = "wasm"))]
pub use native::*;

#[cfg(target_family = "wasm")]
mod wasm;
#[cfg(target_family = "wasm")]
pub use wasm::*;

pub trait SendableFuture: Sendable + Future {}

impl<T> SendableFuture for T where T: Sendable + Future {}

pub trait SendableStream: Sendable + Unpin + Stream {}

impl<T> SendableStream for T where T: Sendable + Unpin + Stream {}

pub trait SendableFusedStream: Sendable + Unpin + FusedStream {}

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
    use super::{Sendable, sleep_for, sleep_until, spawn_task};

    // Impl trait tests
}
