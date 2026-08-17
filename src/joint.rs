//! Actors that both can be sent messages and broadcast messages.

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use pin_project::pin_project;
use tokio::sync::broadcast;

use crate::{
    compat::Sendable,
    sink::{SinkActor, SinkClient, Tracker},
    stream::{Broadcastee, StreamActor, StreamClient},
    ActorKind, ActorState, Scheduler,
};

use crate::OneshotSender;

/// A marker type used by the [`ActorBuilder`](crate::ActorBuilder) to know what kind of
/// [`ActorState`](crate::ActorState) it is dealing with. A joint actor is one that acts as both a
/// [`SinkActor`](crate::sink::SinkActor) and a [`StreamActor`](crate::stream::StreamActor). Its
/// clients, [`JointClient`]s, can both send messages into the actor and recieve messages forwarded
/// by the actor.
#[derive(Debug)]
pub struct JointActor<M> {
    broadcast: broadcast::Sender<Broadcastee<M>>,
}

impl<M: Sendable + Clone, A: ActorState> ActorKind<A> for JointActor<M> {
    type Client = JointClient<A::Message, M>;
    type Config = ();

    fn construct((): Self::Config) -> (Self, Self::Client, impl FnOnce(&mut Scheduler<A>)) {
        let (SinkActor {}, send, init_one) = SinkActor::construct(());
        let (StreamActor { broadcast }, recv, init_two) = StreamActor::construct(());

        let this = Self { broadcast };
        let client = JointClient { send, recv };
        let init = move |scheduler: &mut Scheduler<A>| {
            init_one(scheduler);
            init_two(scheduler);
        };

        (this, client, init)
    }
}

impl<M: Sendable + Clone> JointActor<M> {
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

/// A client to an actor. This client is a combination of the [`SinkClient`] and the
/// [`StreamClient`].
#[pin_project]
#[derive(Debug)]
pub struct JointClient<I, O> {
    send: SinkClient<I>,
    #[pin]
    recv: StreamClient<O>,
}

impl<I, O: Sendable + Clone> JointClient<I, O> {
    /// Consumes the client and return the constituent sink and stream clients.
    pub fn split(self) -> (SinkClient<I>, StreamClient<O>) {
        let Self { send, recv } = self;
        (send, recv)
    }

    /// Returns a clone of this client's sink client.
    pub fn sink(&self) -> SinkClient<I> {
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

    /// Sends a request-response style message to a [`Permanent`] actor. The given data is paired
    /// with a one-time use channel and sent to the actor. A
    /// [`Tracker`](crate::sink::permanent::Tracker) that will receive a response from the actor is
    /// returned.
    ///
    /// Note: Since this client is one for a permanent actor, there is an implicit unwrap once the
    /// tracker receives a message from the actor. If the actor drops the other half of the channel
    /// or has died somehow (likely from a panic), the returned tracker will panic too. So, it is
    /// important that the actor always sends back a message
    pub fn track<M, R>(&self, msg: M) -> Tracker<R>
    where
        I: From<(M, OneshotSender<R>)>,
    {
        self.send.track(msg)
    }
}

impl<I, O> Clone for JointClient<I, O>
where
    O: Sendable + Clone,
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
    O: Sendable + Clone,
{
    type Item = Result<O, u64>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().recv.poll_next(cx)
    }
}
