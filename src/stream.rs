use std::{
    pin::{pin, Pin},
    task::{Context, Poll},
};

use futures::Stream;
use pin_project::pin_project;
use tokio::sync::broadcast;

use crate::{ActorBuilder, ActorState};

/// A marker type used by the [`ActorBuilder`] to know what kind of [`ActorState`] it is dealing
/// with. A stream actor is one that receives messages from one or streams and then forwards
/// messages to its clients.
///
/// The client of a [`StreamActor`] is the [`StreamClient`]. This client implements methods for
/// receiving methods that are "forwarded" by the actor. Unlike the [`SinkActor`], stream actors and
/// clients don't directly support request/response style communication. Communication between a
/// stream actor and client(s) can be modelled with a broadcast-style channel (see [`broadcast::channel`]).
pub struct StreamActor;

#[pin_project]
pub struct StreamClient<M> {
    #[pin]
    recv: BroadcastStream<M>,
}

/// Because of how broadcast streams are implemented in `tokio_streams`, we can not create a
/// broadcast stream from another broadcast stream. Because of this, we must track a second, inner
/// receiver.
#[pin_project]
struct BroadcastStream<M> {
    /// A copy of the original channel, used for cloning the client.
    copy: broadcast::Receiver<M>,
    /// The stream that is polled.
    #[pin]
    inner: tokio_stream::wrappers::BroadcastStream<M>,
}

impl<M: Send + Clone> StreamClient<M> {
    pub(crate) fn new(recv: broadcast::Receiver<M>) -> Self {
        Self {
            recv: BroadcastStream::new(recv),
        }
    }

    pub fn builder<A>(state: A) -> ActorBuilder<A>
    where
        A: ActorState<ActorType = StreamActor, Output = M>,
    {
        ActorBuilder::new(state)
    }
}

impl<M: Send + Clone> BroadcastStream<M> {
    fn new(stream: broadcast::Receiver<M>) -> Self {
        let copy = stream.resubscribe();
        let inner = tokio_stream::wrappers::BroadcastStream::new(stream);
        Self { copy, inner }
    }
}

impl<M: Send + Clone> Clone for StreamClient<M> {
    fn clone(&self) -> Self {
        let recv = BroadcastStream::new(self.recv.copy.resubscribe());
        Self { recv }
    }
}

impl<M: Send + Clone> Stream for StreamClient<M> {
    type Item = M;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().recv.poll_next(cx)
    }
}

impl<M: Send + Clone> Stream for BroadcastStream<M> {
    type Item = M;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        let digest = this.inner.poll_next(cx).map(|res| res.transpose().ok().flatten());
        if digest.is_ready() {
            drop(this.copy.try_recv());
        }
        digest
    }
}
