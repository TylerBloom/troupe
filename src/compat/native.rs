use futures::Stream;
use pin_project::pin_project;
use std::{
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
};

/* ------ Send workarounds ------ */

/// This trait abstracts over the requirements for spawning a task. In native async runtimes, a
/// task might be ran in a different thread, so the future must be `'static + Send`. In WASM, you
/// are always running in a single thread, so spawning a task only requires that the future that
/// the future is `'static`. This concept is used throughout `troupe` to make writing actors in
/// WASM as easy as possible.
pub trait Sendable: 'static + Send {}

impl<T> Sendable for T where T: 'static + Send {}

/// Because [`async_trait`](async_trait::async_trait) requires that trait futures are [`Send`]*,
/// both the [`ActorState`](crate::ActorState) and [`Scheduler`](crate::Scheduler) must be `Send`.
/// This can be a problem for WASM, so this wrapper provides a uniform interfaces between WASM and
/// non-WASM targets through which a `Send` workaround can be implemented.
///
/// For non-WASM targets, this wrapper is a transparent wrapper.
///
/// *`async_trait` allows futures to be `!Send`, this can not easily be done based on the
/// compilation target, and, generally speaking, `!Send` futures are more difficult to work with
/// that `Send` futures.
#[derive(Debug)]
#[pin_project]
pub struct SendableWrapper<T>(#[pin] T);

impl<T> SendableWrapper<T>
where
    T: Sendable,
{
    /// Constructs a wrapper around the given value.
    pub fn new(inner: T) -> Self {
        Self(inner)
    }

    /// Removes the inner value from the wrapper.
    pub fn take(self) -> T {
        self.0
    }
}

impl<T: Sendable> Deref for SendableWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Sendable> DerefMut for SendableWrapper<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Sendable + Clone> Clone for SendableWrapper<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: SendableFuture> Future for SendableWrapper<T> {
    type Output = <T as Future>::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().0.poll(cx)
    }
}

impl<T: SendableStream> Stream for SendableWrapper<T> {
    type Item = <T as Stream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().0.poll_next(cx)
    }
}

/* ------ General Utils ------ */

#[cfg(all(feature = "tokio", feature = "async-std"))]
compile_error!("You can not enable both the 'tokio' and 'async-std' features. This leads to namespace collisions");

#[cfg(feature = "tokio")]
pub use tokio::*;

#[cfg(feature = "async-std")]
pub use async_std::*;

use super::{SendableFuture, SendableStream};

#[cfg(feature = "tokio")]
mod tokio {
    use crate::compat::SendableFuture;
    use instant::{Duration, Instant};
    use pin_project::pin_project;
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    use super::Sendable;

    /// A wrapper around the async runtime which spawns a future that will execute in the
    /// background.
    pub fn spawn_task<F, T>(fut: F)
    where
        F: SendableFuture<Output = T>,
        T: Sendable,
    {
        drop(tokio::spawn(fut));
    }

    /// A future that will sleep for a period of time before waking up. Created by the
    /// [`sleep_for`] and [`sleep_until`] fuctions.
    #[pin_project]
    pub struct Sleep(#[pin] tokio::time::Sleep);

    impl Future for Sleep {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            self.project().0.poll(cx)
        }
    }

    /// Creates an instance of [`Sleep`] that will sleep for at least as long as the given duration.
    pub fn sleep_for(dur: Duration) -> Sleep {
        Sleep(tokio::time::sleep(dur))
    }

    /// Creates an instance of [`Sleep`] that will sleep at least until the given point in time.
    pub fn sleep_until(deadline: Instant) -> Sleep {
        Sleep(tokio::time::sleep_until(deadline.into()))
    }
}

#[cfg(feature = "async-std")]
mod async_std {
    use crate::compat::SendableFuture;
    use futures::FutureExt;
    use instant::{Duration, Instant};
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    use super::Sendable;

    /// A wrapper around the async runtime which spawns a future that will execute in the
    /// background.
    pub fn spawn_task<F, T>(fut: F)
    where
        F: SendableFuture<Output = T>,
        T: Sendable,
    {
        drop(async_std::task::spawn(fut));
    }

    /// A future that will sleep for a period of time before waking up. Created by the
    /// [`sleep_for`] and [`sleep_until`] fuctions.
    pub struct Sleep(Pin<Box<dyn 'static + Send + Future<Output = ()>>>);

    impl Future for Sleep {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            self.0.poll_unpin(cx)
        }
    }

    /// Creates an instance of [`Sleep`] that will sleep for at least as long as the given duration.
    pub fn sleep_for(dur: Duration) -> Sleep {
        Sleep(Box::pin(async_std::task::sleep(dur)))
    }

    /// Creates an instance of [`Sleep`] that will sleep at least until the given point in time.
    pub fn sleep_until(deadline: Instant) -> Sleep {
        sleep_for(deadline - Instant::now())
    }
}
