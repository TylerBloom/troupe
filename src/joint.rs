//! Actors that both can be sent messages and broadcast messages.

use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use futures::Stream;
use pin_project::pin_project;
use tokio::sync::broadcast;

use crate::compat::Sendable;
use crate::sink::SinkActor;
use crate::sink::SinkClient;
use crate::sink::Tracker;
use crate::stream::Broadcastee;
use crate::stream::StreamActor;
use crate::stream::StreamClient;
use crate::ActorKind;
use crate::ActorState;
use crate::Scheduler;

use crate::OneshotSender;

/// The [`ActorKind`] for actors that both receive and broadcast messages. A joint actor is one
/// that acts as both a [`SinkActor`] and a [`StreamActor`]. Its clients, [`JointClient`]s, can
/// both send messages into the actor and recieve messages forwarded by the actor.
///
/// Note that the actor's inbound and outbound message types are distinct. Inbound messages are the
/// state's [`Message`](ActorState::Message); outbound messages are this kind's `M`.
///
/// Constructing this kind does the work of both of its halves: it creates the MPSC channel that
/// backs the client's sink side and attaches the receiving end to the [`Scheduler`], and it holds
/// the broadcast sender that backs the client's stream side. Since the kind lives in the
/// scheduler, which derefs to it, an [`ActorState`] broadcasts by calling
/// [`broadcast`](JointActor::broadcast) on the scheduler it is given.
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
    ///
    /// This is normally reached through the [`Scheduler`], which derefs to this type, rather than
    /// on a `JointActor` directly.
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
/// [`StreamClient`]. `I` is the type of message sent *into* the actor and `O` is the type of
/// message broadcast *out* of it.
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

    /// Sends a request-response style message to an actor. The given data is paired with a
    /// one-time use channel and sent to the actor. A [`Tracker`] that will receive a response from
    /// the actor is returned.
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
