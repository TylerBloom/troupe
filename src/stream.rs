//! Actors that broadcast messages.

use std::{
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{ready, Stream, StreamExt};
use tokio::sync::broadcast;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::{compat::Sendable, ActorKind, ActorState, Scheduler};

#[cfg(not(target_family = "wasm"))]
pub(crate) type Broadcastee<M> = M;
#[cfg(target_family = "wasm")]
pub(crate) type Broadcastee<M> = send_wrapper::SendWrapper<M>;

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
pub struct StreamActor<M> {
    pub(crate) broadcast: broadcast::Sender<Broadcastee<M>>,
}

impl<M: Sendable + Clone, A: ActorState> ActorKind<A> for StreamActor<M> {
    type Client = StreamClient<M>;
    type Config = ();

    fn construct(
        (): Self::Config,
    ) -> (Self, Self::Client, impl 'static + FnOnce(&mut Scheduler<A>)) {
        let (send, recv) = broadcast::channel(10);

        let client = StreamClient::new(recv);
        let this = Self { broadcast: send };
        let init = move |_scheduler: &mut Scheduler<A>| {};

        (this, client, init)
    }
}

impl<M: Sendable + Clone> StreamActor<M> {
    /// Broadcasts a message to all listening clients. If the message fails to send, the message
    /// will be dropped.
    pub fn broadcast(&mut self, msg: impl Into<M>) {
        #[cfg(not(target_family = "wasm"))]
        let _ = self.broadcast.send(msg.into());
        #[cfg(target_family = "wasm")]
        let _ = self
            .broadcast
            .send(send_wrapper::SendWrapper::new(msg.into()));
    }
}

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
    copy: broadcast::Receiver<Broadcastee<M>>,
    /// The stream that is polled.
    inner: tokio_stream::wrappers::BroadcastStream<Broadcastee<M>>,
}

impl<M> StreamClient<M>
where
    M: Sendable + Clone,
{
    pub(crate) fn new(recv: broadcast::Receiver<Broadcastee<M>>) -> Self {
        Self {
            recv: BroadcastStream::new(recv),
        }
    }
}

impl<M> BroadcastStream<M>
where
    M: Sendable + Clone,
{
    fn new(stream: broadcast::Receiver<Broadcastee<M>>) -> Self {
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
