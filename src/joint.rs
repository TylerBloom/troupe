use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use pin_project::pin_project;

use crate::{
    sink::{SinkClient, Tracker},
    stream::StreamClient,
    ActorBuilder, ActorState,
};

use crate::OneshotSender;

/// A marker type used by the [`ActorBuilder`] to know what kind of [`ActorState`] it is dealing
/// with. A joint actor is one that acts as both a [`SinkActor`] and a [`StreamActor`]. Its clients
/// can both send messages into the actor and recieve messages forwarded by the actor.
pub struct JointActor;

#[pin_project]
pub struct JointClient<I, O> {
    send: SinkClient<I>,
    #[pin]
    recv: StreamClient<O>,
}

impl<I, O: Send + Clone> JointClient<I, O> {
    pub(crate) fn new(send: SinkClient<I>, recv: StreamClient<O>) -> Self {
        Self { send, recv }
    }

    pub fn builder<A>(state: A) -> ActorBuilder<A>
    where
        A: ActorState<Message = I, ActorType = JointActor, Output = O>,
    {
        ActorBuilder::new(state)
    }

    pub fn split(self) -> (SinkClient<I>, StreamClient<O>) {
        let Self { send, recv } = self;
        (send, recv)
    }

    pub fn sink(&self) -> SinkClient<I> {
        self.send.clone()
    }

    pub fn stream(&self) -> StreamClient<O>
    where
        O: Clone,
    {
        self.recv.clone()
    }

    pub fn send(&self, msg: impl Into<I>) {
        self.send.send(msg)
    }

    pub fn track<T, R>(&self, msg: T) -> Tracker<R>
    where
        I: From<(T, OneshotSender<R>)>,
    {
        self.send.track(msg)
    }
}

impl<I, O> Clone for JointClient<I, O>
where
    O: Send + Clone,
{
    fn clone(&self) -> Self {
        Self {
            send: self.send.clone(),
            recv: self.recv.clone(),
        }
    }
}

impl<I, O> Stream for JointClient<I, O>
where
    O: Send + Clone,
{
    type Item = O;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().recv.poll_next(cx)
    }
}
