//! Actors that broadcast messages.

use std::{
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
};

#[cfg(target_family = "wasm")]
use send_wrapper::SendWrapper;

use futures::{ready, Stream, StreamExt};
use tokio::sync::broadcast;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::compat::Sendable;

/// A marker type used by the [`ActorBuilder`](crate::ActorBuilder) to know what kind of
/// [`ActorState`](crate::ActorState) it is dealing with. A stream actor is one that receives
/// messages from one or more streams and then forwards messages to its clients.
///
/// The client of a [`StreamActor`] is the [`StreamClient`]. This client implements methods for
/// receiving methods that are "forwarded" by the actor. Unlike the
/// [`SinkActor`](crate::sink::SinkActor), stream actors and clients don't directly support
/// request/response style communication. Communication between a stream actor and client(s) can be
/// modelled with a broadcast-style channel (see [`broadcast::channel`]).
#[derive(Debug)]
pub struct StreamActor;

/// A client that receives messages from an actor that broadcasts them.
#[derive(Debug)]
pub struct StreamClient<M> {
    recv: BroadcastStream<M>,
}

/// Because of how broadcast streams are implemented in `tokio_streams`, we can not create a
/// broadcast stream from another broadcast stream. Because of this, we must track a second, inner
/// receiver.
struct BroadcastStream<M> {
    /// A copy of the original channel, used for cloning the client.
    #[cfg(not(target_family = "wasm"))]
    copy: broadcast::Receiver<M>,
    #[cfg(target_family = "wasm")]
    copy: broadcast::Receiver<SendWrapper<M>>,
    #[cfg(not(target_family = "wasm"))]
    /// The stream that is polled.
    inner: tokio_stream::wrappers::BroadcastStream<M>,
    #[cfg(target_family = "wasm")]
    inner: tokio_stream::wrappers::BroadcastStream<SendWrapper<M>>,
}

impl<M> StreamClient<M>
where
    M: Sendable + Clone,
{
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn new(recv: broadcast::Receiver<M>) -> Self {
        Self {
            recv: BroadcastStream::new(recv),
        }
    }

    #[cfg(target_family = "wasm")]
    pub(crate) fn new(recv: broadcast::Receiver<SendWrapper<M>>) -> Self {
        Self {
            recv: BroadcastStream::new(recv),
        }
    }
}

impl<M> BroadcastStream<M>
where
    M: Sendable + Clone,
{
    #[cfg(not(target_family = "wasm"))]
    fn new(stream: broadcast::Receiver<M>) -> Self {
        let copy = stream.resubscribe();
        let inner = tokio_stream::wrappers::BroadcastStream::new(stream);
        Self { copy, inner }
    }

    #[cfg(target_family = "wasm")]
    fn new(stream: broadcast::Receiver<SendWrapper<M>>) -> Self {
        let copy = stream.resubscribe();
        let inner = tokio_stream::wrappers::BroadcastStream::new(stream);
        Self { copy, inner }
    }
}

impl<M> Clone for StreamClient<M>
where
    M: Sendable + Clone,
{
    fn clone(&self) -> Self {
        let recv = BroadcastStream::new(self.recv.copy.resubscribe());
        Self { recv }
    }
}

impl<M> Stream for StreamClient<M>
where
    M: Sendable + Clone,
{
    type Item = Result<M, u64>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let digest = Pin::new(&mut self.get_mut().recv).poll_next(cx);
        digest
    }
}

impl<M> Stream for BroadcastStream<M>
where
    M: Sendable + Clone,
{
    type Item = Result<M, u64>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let done = ready!(self.inner.poll_next_unpin(cx));
        drop(self.copy.try_recv());
        match done {
            Some(Ok(val)) => {
                #[cfg(target_family = "wasm")]
                let val = val.take();
                Poll::Ready(Some(Ok(val)))
            }
            Some(Err(BroadcastStreamRecvError::Lagged(count))) => Poll::Ready(Some(Err(count))),
            None => Poll::Ready(None),
        }
    }
}

impl<M> Debug for BroadcastStream<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BroadcastStream({:?})", self.copy)
    }
}
