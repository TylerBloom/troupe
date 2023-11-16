use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use pin_project::pin_project;

use crate::{
    sink::{self, SinkClient},
    stream::StreamClient,
    Permanent, Transient,
};

use crate::OneshotSender;

/// A marker type used by the [`ActorBuilder`](crate::ActorBuilder) to know what kind of [`ActorState`](crate::ActorState) it is dealing
/// with. A joint actor is one that acts as both a [`SinkActor`](crate::sink::SinkActor) and a [`StreamActor`](crate::stream::StreamActor). Its clients
/// can both send messages into the actor and recieve messages forwarded by the actor.
#[derive(Debug)]
pub struct JointActor;

/// A client to an actor. This client is a combination of the ['SinkClient`] and the
/// [`StreamClient`].
#[pin_project]
#[derive(Debug)]
pub struct JointClient<T, I, O> {
    send: SinkClient<T, I>,
    #[pin]
    recv: StreamClient<O>,
}

impl<T, I, O: 'static + Send + Clone> JointClient<T, I, O> {
    /// A constuctor for the client.
    pub(crate) fn new(send: SinkClient<T, I>, recv: StreamClient<O>) -> Self {
        Self { send, recv }
    }

    /// Consumes the client and return the constituent sink and stream clients.
    pub fn split(self) -> (SinkClient<T, I>, StreamClient<O>) {
        let Self { send, recv } = self;
        (send, recv)
    }

    /// Returns a clone of this client's sink client.
    pub fn sink(&self) -> SinkClient<T, I> {
        self.send.clone()
    }

    /// Returns a clone of this client's stream client.
    pub fn stream(&self) -> StreamClient<O>
    where
        O: Clone,
    {
        self.recv.clone()
    }

    /// Returns if the actor that the client is connected to is dead or not.
    pub fn is_closed(&self) -> bool {
        self.send.is_closed()
    }

    /// Sends a fire-and-forget style message to the actor and returns if the message was sent
    /// successfully.
    pub fn send(&self, msg: impl Into<I>) -> bool {
        self.send.send(msg)
    }
}

impl<I, O> JointClient<Permanent, I, O> {
    /// Sends a request-response style message to a [`Permanent`] actor. The given data is paired
    /// with a one-time use channel and sent to the actor. A
    /// [`Tracker`](crate::sink::permanent::Tracker) that will receive a
    /// response from the actor is returned.
    ///
    /// Note: Since this client is one for a permanent actor, there is an implicit unwrap once the
    /// tracker receives a message from the actor. If the actor drops the other half of the channel
    /// or has died somehow (likely from a panic), the returned tracker will panic too.
    pub fn track<M, R>(&self, msg: M) -> sink::permanent::Tracker<R>
    where
        I: From<(M, OneshotSender<R>)>,
    {
        self.send.track(msg)
    }
}

impl<I, O> JointClient<Transient, I, O> {
    /// Sends a request-response style message to a [`Transient`] actor.
    /// The given data is paired with a one-time use channel and sent to the actor.
    /// A [`Tracker`](crate::sink::transient::Tracker) that will receive a response from the actor is returned.
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
