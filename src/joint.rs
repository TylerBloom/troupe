use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use pin_project::pin_project;

use crate::{
    sink::{self, SinkClient},
    stream::StreamClient,
    ActorBuilder, ActorState, Permanent, Transient,
};

use crate::OneshotSender;

/// A marker type used by the [`ActorBuilder`] to know what kind of [`ActorState`] it is dealing
/// with. A joint actor is one that acts as both a [`SinkActor`] and a [`StreamActor`]. Its clients
/// can both send messages into the actor and recieve messages forwarded by the actor.
pub struct JointActor;

#[pin_project]
pub struct JointClient<T, I, O> {
    send: SinkClient<T, I>,
    #[pin]
    recv: StreamClient<O>,
}

impl<T, I, O: 'static + Send + Clone> JointClient<T, I, O> {
    pub(crate) fn new(send: SinkClient<T, I>, recv: StreamClient<O>) -> Self {
        Self { send, recv }
    }

    pub fn builder<A>(state: A) -> ActorBuilder<A>
    where
        A: ActorState<Message = I, ActorType = JointActor, Output = O>,
    {
        ActorBuilder::new(state)
    }

    pub fn split(self) -> (SinkClient<T, I>, StreamClient<O>) {
        let Self { send, recv } = self;
        (send, recv)
    }

    pub fn sink(&self) -> SinkClient<T, I> {
        self.send.clone()
    }

    pub fn stream(&self) -> StreamClient<O>
    where
        O: Clone,
    {
        self.recv.clone()
    }

    pub fn send(&self, msg: impl Into<I>) -> bool {
        self.send.send(msg)
    }
}

impl<I, O> JointClient<Permanent, I, O> {
    pub fn track<M, R>(&self, msg: M) -> sink::permanent::Tracker<R>
    where
        I: From<(M, OneshotSender<R>)>,
    {
        self.send.track(msg)
    }
}

impl<I, O> JointClient<Transient, I, O> {
    pub fn track<M, R>(&self, msg: M) -> sink::transient::Tracker<R>
    where
        I: From<(M, OneshotSender<R>)>,
    {
        self.send.track(msg)
    }
}

impl<T, I, O> Clone for JointClient<T, I, O>
where
    O: 'static + Send + Clone,
{
    fn clone(&self) -> Self {
        Self {
            send: self.send.clone(),
            recv: self.recv.clone(),
        }
    }
}

impl<T, I, O> Stream for JointClient<T, I, O>
where
    O: 'static + Send + Clone,
{
    type Item = O;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().recv.poll_next(cx)
    }
}
