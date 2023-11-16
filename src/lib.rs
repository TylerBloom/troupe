//! Troupe provides a high-level toolset for modelling crates with actors. Troupe actors are built
//! on top of async process, like those created from `tokio::spawn`, and help you model and control
//! the flow of information in and out of them. The main goals of `troupe` are to provide:
//! - An easy to conceptualize data flow
//! - Simple access to concurrent processing of futures within actors
//! - A model that can be adopted into an existing project all at once or over time
//! - An ergonomic API devoid of magic
//!
//! At the core of every actor is an [`ActorState`]. These are the building blocks of modeling your
//! application with `troupe`. This state fully isolated from the rest of your application and can
//! only be reached by attaching a stream of inbound messages. For most actors, this stream is
//! provided by `troupe` in the form of an MSPC-style tokio channel. Regardless of the stream, all
//! inbound messages pass through the [`Scheduler`], which the state has access to while processing
//! messages. The state can add new streams of messages, queue futures that yield message, or hand
//! off futures that yield nothing to the scheduler.
//!
//! Communication from/to an actor is generally managed by a client. Each actor can define what its
//! clients should function via the [`ActorState`]'s `ActorType`. Conceptually, every actor is
//! either a `Stream` or `Sink` (or both). An actor that largely receive messages from other parts
//! of our application is a [`SinkActor`], which use [`SinkClient`]s. An actor that recieves
//! messages from a different type of stream (for example a Websocket) and then broadcast these
//! messages is a [`StreamActor`], which use [`StreamClient`]s. If an actor does both of these, it
//! is a [`JointActor`] and uses [`JointClient`]s.
//!
//! Troupe also supports WASM by using `wasm-bindgen-futures` to run actors.

#![warn(rust_2018_idioms)]
#![deny(
    missing_docs,
    rustdoc::broken_intra_doc_links,
    missing_debug_implementations,
    unreachable_pub,
    unreachable_patterns,
    unused,
    unused_results,
    unused_qualifications,
    while_true,
    trivial_casts,
    trivial_bounds,
    trivial_numeric_casts,
    unconditional_panic,
    unsafe_code,
    clippy::all
)]

/// The compatability layer between async runtime and native vs WASM targets.
pub mod compat;
/// Actors that both can be sent messages and broadcast messages.
pub mod joint;
/// Re-exports of commonly used items.
pub mod prelude;
pub(crate) mod scheduler;
/// Actors that are only sent messages (either fire-and-forget messages or request-response
/// messages).
pub mod sink;
/// Actors that broadcast messages.
pub mod stream;

pub use async_trait::async_trait;
use compat::{Sendable, SendableStream};
use joint::{JointActor, JointClient};
pub use scheduler::Scheduler;
use scheduler::{ActorRunner, ActorStream};
use sink::{SinkActor, SinkClient};
use stream::{StreamActor, StreamClient};
pub use tokio::sync::oneshot::{
    channel as oneshot_channel, Receiver as OneshotReceiver, Sender as OneshotSender,
};

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{stream::FusedStream, Future, StreamExt};
use instant::Instant;
use pin_project::pin_project;
use tokio::sync::{
    broadcast,
    mpsc::{unbounded_channel, UnboundedSender},
};

use crate::compat::{sleep_until, Sleep};

// This state needs to be send because of constraints of `async_trait`. Ideally, it would be
// `Sendable`.
/// The core abstraction of the actor model. An [`ActorState`] sits at the heart of every actor. It
/// processes messages, queues futures and streams in the [`Scheduler`] that yield messages, and it
/// can forward more messages. Actors serves two roles. They can act similarly to a [`Sink`](futures::Sink) where
/// other parts of your application (including other actors) since messages into the actor. They
/// can also act as a [`Stream`](futures::Stream) that generate messages to be sent throughout your application.
/// This role is denoted by the actor's `ActorType`, which informs the [`ActorBuilder`] what kind
/// of actor it is working with. For sink-like actors, use the [`SinkActor`] type. For stream-like
/// actors, use the [`StreamActor`] type. For actors that function as both, use the [`JointActor`]
/// type.
#[async_trait]
pub trait ActorState: 'static + Send + Sized {
    /// This type should either be [`SinkActor`], [`StreamActor`], or [`JointActor`]. This type is
    /// mostly a marker to inform the [`ActorBuilder`].
    type ActorType;

    /// This type should either be [`Permanent`] or [`Transient`]. This type is mostly a marker
    /// type to inform the actor's client(s).
    type Permanence;

    /// Inbound messages to the actor must be this type. Clients will send the actor messages of
    /// this type and any queued futures or streams must yield this type.
    type Message: Sendable;

    /// For [`SinkActor`]s and [`JointActor`]s, this is the message type which is broadcasts.
    /// For [`StreamActor`]s, this can be `()` (unfortunately, default associated types are
    /// unstable).
    type Output: Sendable + Clone;

    /// Before starting the main loop of running the actor, this method is called to finalize any
    /// setup of the actor state. No inbound messages will be processed until this method is
    /// completed.
    #[allow(unused_variables)]
    async fn start_up(&mut self, scheduler: &mut Scheduler<Self>) {}

    /// The heart of the actor. This method consumes messages from clients and queued futures and
    /// streams. For [`SinkActor`]s and [`JointActor`]s, the state "responds" to messages with a
    /// [`OneshotChannel`](tokio::sync::oneshot::channel). The state can also queue futures and
    /// streams in the [`Scheduler`]. Finally, for [`StreamActor`]s and [`JointActor`]s, any
    /// messages to forwarded can be queued in the [`Scheduler`].
    async fn process(&mut self, scheduler: &mut Scheduler<Self>, msg: Self::Message);
}

/// A marker type used in the [`ActorState`]. It communicates that the actor should never die. As
/// such, the [`Scheduler`] will not provide the actor state a method to shutdown. Also, the
/// [`Tracker`](crate::sink::permanent::Tracker)s for request-response style messages will implictly unwrap responses from their
/// oneshot channels.
#[derive(Debug)]
pub struct Permanent;

/// A marker type used in the [`ActorState`]. It communicates that the actor should exist for a
/// non-infinite amount of time. The [`Scheduler`] will provide the actor state a method to
/// shutdown. Also, the [`Tracker`](crate::sink::permanent::Tracker)s for request-response style messages will not implictly unwrap
/// responses from their oneshot channels.
#[derive(Debug)]
pub struct Transient;

/// Holds a type that implements [`ActorState`], helps aggregate all data needed by the actor, and
/// then launches the actor process. When the actor process is launched, a client is returned to
/// the caller. This client's type depends on the actor's type.
#[allow(missing_debug_implementations)]
pub struct ActorBuilder<A: ActorState> {
    send: UnboundedSender<A::Message>,
    #[allow(clippy::type_complexity)]
    broadcast: Option<(broadcast::Sender<A::Output>, broadcast::Receiver<A::Output>)>,
    recv: Vec<ActorStream<A::Message>>,
    state: A,
}

/* --------- All actors --------- */
impl<A> ActorBuilder<A>
where
    A: ActorState,
{
    /// Constructs a new builder for an actor that uses the given state.
    pub fn new(state: A) -> Self {
        let (send, recv) = unbounded_channel();
        let recv = vec![recv.into()];
        Self {
            state,
            send,
            recv,
            broadcast: None,
        }
    }

    /// Attaches a stream that will be used by the actor once its spawned. No messages will be
    /// processed until after the actor is launched.
    pub fn add_stream<S, I>(&mut self, stream: S)
    where
        S: SendableStream<Item = I> + FusedStream,
        I: Into<A::Message>,
    {
        self.recv
            .push(ActorStream::Secondary(Box::new(stream.map(|m| m.into()))));
    }
}

/* --------- Sink actors --------- */
impl<A> ActorBuilder<A>
where
    A: ActorState<ActorType = SinkActor>,
{
    /// Returns a client for the actor that will be spawned. While the returned client will be able
    /// to send messages, those messages will not be processed until after the actor is launched by
    /// the builder.
    pub fn sink_client(&self) -> SinkClient<A::Permanence, A::Message> {
        SinkClient::new(self.send.clone())
    }

    /// Launches an actor that uses the given state and returns a client to the actor.
    pub fn launch_sink(self) -> SinkClient<A::Permanence, A::Message> {
        let Self {
            send, recv, state, ..
        } = self;
        let mut runner = ActorRunner::new(state);
        recv.into_iter().for_each(|r| runner.add_stream(r.fuse()));
        runner.launch();
        SinkClient::new(send)
    }
}

/* --------- Stream actors --------- */
impl<A> ActorBuilder<A>
where
    A: ActorState<ActorType = StreamActor>,
{
    /// Returns a client for the actor that will be spawned. The client will not yield any messages
    /// until after the actor is launched and has sent a message.
    pub fn stream_client(&mut self) -> StreamClient<A::Output> {
        let (_, broad) = self.broadcast.get_or_insert_with(|| broadcast::channel(100));
        StreamClient::new(broad.resubscribe())
    }

    /// Launches an actor that uses the given state and stream and returns a client to the actor.
    pub fn launch_stream<S>(self, stream: S) -> StreamClient<A::Output>
    where
        S: 'static + Send + Unpin + FusedStream<Item = A::Message>,
    {
        let Self {
            mut recv,
            state,
            broadcast,
            ..
        } = self;
        let (broad, sub) = broadcast.unwrap_or_else(|| broadcast::channel(100));
        recv.push(ActorStream::Secondary(Box::new(stream)));
        let mut runner = ActorRunner::new(state);
        runner.add_broadcaster(broad);
        recv.into_iter().for_each(|r| runner.add_stream(r.fuse()));
        runner.launch();
        StreamClient::new(sub)
    }
}

/* --------- Joint actors --------- */
impl<A> ActorBuilder<A>
where
    A: ActorState<ActorType = JointActor>,
{
    /// Returns a client for the actor that will be spawned. The client will not yield any messages
    /// until after the actor is launched and has sent a message.
    pub fn stream(&self) -> StreamClient<A::Output> {
        StreamClient::new(self.broadcast.as_ref().unwrap().1.resubscribe())
    }

    /// Returns a sink client for the actor that will be spawned. While the returned client will be
    /// able to send messages, those messages will not be processed until after the actor is
    /// launched by the builder.
    pub fn sink(&self) -> SinkClient<A::Permanence, A::Message> {
        SinkClient::new(self.send.clone())
    }

    /// Launches an actor that uses the given state and stream and returns a client to the actor.
    pub fn launch_with_stream<S>(mut self, stream: S) -> JointClient<A::Permanence, A::Message, A::Output>
    where
        S: 'static + Send + Unpin + FusedStream<Item = A::Message>,
    {
        self.add_stream(stream);
        self.launch()
    }

    /// Launches an actor that uses the given state and returns a client to the actor.
    pub fn launch(self) -> JointClient<A::Permanence, A::Message, A::Output> {
        let Self {
            send,
            recv,
            state,
            broadcast,
        } = self;
        let (broad, sub) = broadcast.unwrap_or_else(|| broadcast::channel(100));
        let mut runner = ActorRunner::new(state);
        recv.into_iter().for_each(|r| runner.add_stream(r.fuse()));
        runner.add_broadcaster(broad);
        runner.launch();
        let sink = SinkClient::new(send);
        let stream = StreamClient::new(sub);
        JointClient::new(sink, stream)
    }
}

/* -------- To move -------- */

/// A message and sleep timer pair. Once the timer has elapsed and is polled, the message is taken
/// from the inner option and returned. After that point, the timer should not be polled again.
#[pin_project]
#[allow(missing_debug_implementations)]
pub(crate) struct Timer<T> {
    #[pin]
    deadline: Sleep,
    msg: Option<T>,
}

impl<T> Timer<T> {
    pub(crate) fn new(deadline: Instant, msg: T) -> Self {
        Self {
            deadline: sleep_until(deadline),
            msg: Some(msg),
        }
    }
}

impl<T> Future for Timer<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.deadline.poll(cx) {
            Poll::Ready(()) => Poll::Ready(this.msg.take().unwrap()),
            Poll::Pending => Poll::Pending,
        }
    }
}
