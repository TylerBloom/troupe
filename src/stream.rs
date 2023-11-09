use std::{
    future::Future,
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

// This type is a placeholder for tokio_stream's broadcast stream. Since we can't create new stream
// handles from an existing broadcast stream, we will have to manually implement `Stream` until
// that is addressed.
#[pin_project]
struct BroadcastStream<M>(#[pin] broadcast::Receiver<M>);

impl<M> StreamClient<M> {
    pub(crate) fn new(recv: broadcast::Receiver<M>) -> Self {
        Self {
            recv: BroadcastStream(recv),
        }
    }

    pub fn builder<A>(state: A) -> ActorBuilder<A>
    where
        A: ActorState<ActorType = StreamActor, Output = M>,
    {
        ActorBuilder::new(state)
    }

    pub fn try_recv(&mut self) -> Option<M>
    where
        M: Clone,
    {
        self.recv.0.try_recv().ok()
    }
}

impl<M: Clone> Clone for StreamClient<M> {
    fn clone(&self) -> Self {
        let recv = BroadcastStream(self.recv.0.resubscribe());
        Self { recv }
    }
}

impl<M: Clone> Stream for StreamClient<M> {
    type Item = M;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().recv.poll_next(cx)
    }
}

impl<M: Clone> Stream for BroadcastStream<M> {
    type Item = M;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        pin!(self.project().0.recv()).poll(cx).map(Result::ok)
    }
}
