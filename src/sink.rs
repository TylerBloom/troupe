use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::sync::mpsc::UnboundedSender;

use crate::{oneshot_channel, ActorBuilder, ActorState, OneshotReceiver, OneshotSender};

/// A marker type used by the [`ActorBuilder`] to know what kind of [`ActorState`] it is dealing
/// with. A sink actor is one that receives messages from other parts of the application. By adding
/// a [`OneshotChannel`] into the message, the actor can respond with a particular piece of data.
/// This allows for type-safe communication between different parts of your application.
///
/// The client of a [`SinkActor`] is the [`SinkClient`]. This client implements methods that allow
/// for the sending of messages to this client. Communication between a sink client and
/// sink actor can be roughly modelled with an MPSC-style channel (see [`mpsc::channel`]).
pub struct SinkActor;

pub struct SinkClient<M> {
    send: UnboundedSender<M>,
}

pub struct Tracker<T> {
    recv: OneshotReceiver<T>,
}

impl<M> SinkClient<M> {
    pub(crate) fn new(send: UnboundedSender<M>) -> Self {
        Self { send }
    }

    pub fn builder<A>(state: A) -> ActorBuilder<A>
    where
        A: ActorState<Message = M, ActorType = SinkActor>,
    {
        ActorBuilder::new(state)
    }

    pub fn send(&self, msg: impl Into<M>) {
        // This returns a result. It only errors when the connected actor panics. Should we "bubble
        // up" that panic?
        let _ = self.send.send(msg.into());
    }

    pub fn track<T, R>(&self, msg: T) -> Tracker<R>
    where
        M: From<(T, OneshotSender<R>)>,
    {
        let (send, recv) = oneshot_channel();
        let msg = M::from((msg, send));
        self.send(msg);
        Tracker::new(recv)
    }
}

impl<M> Clone for SinkClient<M> {
    fn clone(&self) -> Self {
        Self::new(self.send.clone())
    }
}

impl<T> Tracker<T> {
    pub fn new(recv: OneshotReceiver<T>) -> Self {
        Self { recv }
    }
}

impl<T> Future for Tracker<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.recv).poll(cx).map(Result::unwrap)
    }
}
