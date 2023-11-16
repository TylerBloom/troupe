//! Troupe provides a high-level toolset for modelling crates with actors. Troupe actors are built
//! on top of async process, like those created from `tokio::spawn`, and help you model and control
//! the flow of information in and out of them. The main goals of `troupe` are to provide:
//! - An easy to conceptualize data flow
//! - Simple access to concurrently processing of futures within actors
//! - A model that can be adopted into an existing project all at once or over time
//! - An ergonomic API devoid of magic
//!
//! At the core of every actor is an [`ActorState`]. These are the building blocks used to model
//! your program with `troupe`. This state is fully isolated from the rest of your application and
//! can only be reached by attaching a stream of messages. For many actors, a stream is provided by
//! `troupe` in the form of an [`mpsc-style`](tokio::sync::mpsc) tokio channel. All attached
//! streams are managed by a [`Scheduler`]. The state can attach new streams, queue futures that
//! yield message, or hand off futures that yield nothing to the scheduler.
//!
//! Communication to and from an actor is managed by a client. Each actor state defines how its
//! clients should function via the [`ActorState`]'s `ActorType`. Conceptually, every actor is
//! either something that consumes messages, i.e. a `Sink`, or something to broadcasts messages,
//! i.e. a `Stream`, or both. An actor that largely receive messages from other parts of our
//! program is a [`SinkActor`], which use [`SinkClient`]s. An actor that processes messages from a
//! source (for example a Websocket) and then broadcast these messages is a [`StreamActor`], which
//! use [`StreamClient`]s. If an actor does both of these, it is a [`JointActor`] and uses
//! [`JointClient`]s.
//!
//! Troupe currently supports three async runtimes: `tokio`, `async-std`, and the runtime provided by
//! the browser (via wasm-bindgen-futures). Do note that even if you are using the `async-std`
//! runtime, client-actor communication is still done via tokio channels.

#![warn(rust_2018_idioms)]
#![deny(
    missing_docs,
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_rust_codeblocks,
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

pub mod compat;
pub mod joint;
pub mod prelude;
pub(crate) mod scheduler;
pub mod sink;
pub mod stream;

use compat::{Sendable, SendableAnyMap, SendableFusedStream, SendableFuture, SendableWrapper};
use joint::{JointActor, JointClient};
pub use scheduler::Scheduler;
use scheduler::{ActorRunner, ActorStream};
use sink::{SinkActor, SinkClient};
use stream::{StreamActor, StreamClient};
pub use tokio::sync::oneshot::{
    channel as oneshot_channel, Receiver as OneshotReceiver, Sender as OneshotSender,
};

use std::marker::PhantomData;

use futures::StreamExt;
use tokio::sync::{
    broadcast,
    mpsc::{unbounded_channel, UnboundedSender},
};

/// The core abstraction of the actor model. An [`ActorState`] sits at the heart of every actor. It
/// processes messages, queues futures, and attaches streams in the [`Scheduler`], and it can
/// forward messages. Actors serves two roles. They can act similarly to a
/// [`Sink`](futures::Sink) where other parts of your application (including other actors) since
/// messages into the actor. They can also act as a [`Stream`](futures::Stream) that generate
/// messages to be sent throughout your application. This role is denoted by the actor's
/// `ActorType`, which informs the [`ActorBuilder`] what kind of actor it is working with. For
/// sink-like actors, use the [`SinkActor`] type. For stream-like actors, use the [`StreamActor`]
/// type. For actors that function as both, use the [`JointActor`] type.
pub trait ActorState: Sendable + Sized {
    /// This type should either be [`SinkActor`], [`StreamActor`], or [`JointActor`]. This type is
    /// mostly a marker to inform the [`ActorBuilder`].
    type ActorType;

    /// This type should either be [`Permanent`] or [`Transient`]. This type is mostly a marker
    /// type to inform the actor's client(s) if it should expect the actor to shutdown at any
    /// point.
    type Permanence;

    /// Inbound messages to the actor must be this type. Clients will send the actor messages of
    /// this type and any queued futures or streams must yield this type.
    type Message: Sendable;

    /// For [`SinkActor`]s and [`JointActor`]s, this is the message type which is broadcasts.
    /// For [`StreamActor`]s, this can be `()` (unfortunately, default associated types are
    /// unstable).
    type Output: Sendable + Clone;

    /// Before starting the main loop of running the actor, this method is called to finalize any
    /// setup of the actor state, such as pulling data from a database or from over the network. No
    /// inbound messages will be processed until this method is completed.
    #[allow(unused_variables)]
    fn start_up(&mut self, scheduler: &mut Scheduler<Self>) -> impl SendableFuture<Output = ()> {
        std::future::ready(())
    }

    /// The heart of the actor. This method consumes messages attached streams and queued futures
    /// and streams. For [`SinkActor`]s and [`JointActor`]s, the state can "respond" to messages
    /// containing a [`OneshotChannel`](tokio::sync::oneshot::channel) sender. The state can also
    /// queue futures and attach streams in the [`Scheduler`]. Finally, for [`StreamActor`]s and
    /// [`JointActor`]s, the state can broadcast messages via [`Scheduler`].
    fn process(
        &mut self,
        scheduler: &mut Scheduler<Self>,
        msg: Self::Message,
    ) -> impl SendableFuture<Output = ()>;

    /// Once the actor has died, this method is called to allow the actor to clean up anything that
    /// remains. Note that this method is also called even for [`Permanent`] actors that have
    /// expired.
    #[allow(unused_variables)]
    fn finalize(self, scheduler: &mut Scheduler<Self>) -> impl SendableFuture<Output = ()> {
        std::future::ready(())
    }
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

/// Holds a type that implements [`ActorState`], helps aggregate all data that the actor needs, and
/// then launches the async actor process. When the actor process is launched, a client is returned
/// to the caller. This client's type depends on the actor's type.
#[allow(missing_debug_implementations)]
pub struct ActorBuilder<T, A: ActorState> {
    /// The type of actor that is being built. This is the same as `A::ActorType` but
    /// specialization is not yet supported.
    ty: PhantomData<T>,
    send: UnboundedSender<A::Message>,
    edges: SendableAnyMap,
    #[allow(clippy::type_complexity)]
    broadcast: Option<(
        broadcast::Sender<SendableWrapper<A::Output>>,
        broadcast::Receiver<SendableWrapper<A::Output>>,
    )>,
    recv: Vec<ActorStream<A::Message>>,
    state: A,
}

/* --------- All actors --------- */
impl<T, A> ActorBuilder<T, A>
where
    A: ActorState<ActorType = T>,
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
            ty: PhantomData,
            edges: SendableAnyMap::new(),
        }
    }

    /// Attaches a stream that will be used by the actor once its spawned. No messages will be
    /// processed until after the actor is launched.
    pub fn attach_stream<S, I>(&mut self, stream: S)
    where
        S: SendableFusedStream<Item = I>,
        I: Into<A::Message>,
    {
        self.recv
            .push(ActorStream::Secondary(Box::new(stream.map(|m| m.into()))));
    }

    /// Adds a client to the builder, which the state can access later.
    pub fn add_edge<P: 'static + Send, M: 'static + Send>(&mut self, client: SinkClient<P, M>) {
        _ = self.edges.insert(client);
    }

    /// Adds an arbitrary data to the builder, which the state can access later. This method is
    /// intended to be used with containers hold that multiple clients of the same type.
    ///
    /// For example, you can attach a series of actor clients that are indexed using a hashmap.
    pub fn add_multi_edge<C: 'static + Send>(&mut self, container: C) {
        _ = self.edges.insert(container);
    }
}

/* --------- Sink actors --------- */
impl<A> ActorBuilder<SinkActor, A>
where
    A: ActorState<ActorType = SinkActor>,
{
    /// Returns a client for the actor that will be spawned. While the returned client will be able
    /// to send messages, those messages will not be processed until after the actor is launched by
    /// the builder.
    pub fn client(&self) -> SinkClient<A::Permanence, A::Message> {
        SinkClient::new(self.send.clone())
    }

    /// Launches an actor that uses the given state and returns a client to the actor.
    pub fn launch(self) -> SinkClient<A::Permanence, A::Message> {
        let Self {
            send,
            recv,
            state,
            edges,
            ..
        } = self;
        let mut runner = ActorRunner::new(state, edges);
        recv.into_iter().for_each(|r| runner.add_stream(r));
        runner.launch();
        SinkClient::new(send)
    }
}

/* --------- Stream actors --------- */
impl<A> ActorBuilder<StreamActor, A>
where
    A: ActorState<ActorType = StreamActor>,
{
    /// Returns a client for the actor that will be spawned. The client will not yield any messages
    /// until after the actor is launched and has sent a message.
    pub fn client(&mut self) -> StreamClient<A::Output> {
        let (_, broad) = self
            .broadcast
            .get_or_insert_with(|| broadcast::channel(100));
        StreamClient::new(broad.resubscribe())
    }

    /// Launches an actor that uses the given state. Returns a client to the actor.
    pub fn launch<S>(self, stream: S) -> StreamClient<A::Output>
    where
        S: SendableFusedStream<Item = A::Message>,
    {
        let Self {
            mut recv,
            state,
            broadcast,
            edges,
            ..
        } = self;
        let (broad, sub) = broadcast.unwrap_or_else(|| broadcast::channel(100));
        recv.push(ActorStream::Secondary(Box::new(stream)));
        let mut runner = ActorRunner::new(state, edges);
        runner.add_broadcaster(broad);
        recv.into_iter().for_each(|r| runner.add_stream(r));
        runner.launch();
        StreamClient::new(sub)
    }
}

/* --------- Joint actors --------- */
impl<A> ActorBuilder<JointActor, A>
where
    A: ActorState<ActorType = JointActor>,
{
    /// Returns a stream client for the actor that will be spawned. The client will not yield any
    /// messages until after the actor is launched and has sent a message.
    pub fn stream_client(&self) -> StreamClient<A::Output> {
        StreamClient::new(self.broadcast.as_ref().unwrap().1.resubscribe())
    }

    /// Returns a sink client for the actor that will be spawned. While the returned client will be
    /// able to send messages, those messages will not be processed until after the actor is
    /// launched by the builder.
    pub fn sink_client(&self) -> SinkClient<A::Permanence, A::Message> {
        SinkClient::new(self.send.clone())
    }

    /// Returns a joint client for the actor that will be spawned. While the returned client will be
    /// able to send messages, those messages will not be processed until after the actor is
    /// launched by the builder. The client will also not yield any messages until after the actor
    /// is launched and has sent a message.
    pub fn client(&self) -> SinkClient<A::Permanence, A::Message> {
        SinkClient::new(self.send.clone())
    }

    /// Launches an actor that uses the given state and stream. Returns a client to the actor.
    pub fn launch_with_stream<S>(
        mut self,
        stream: S,
    ) -> JointClient<A::Permanence, A::Message, A::Output>
    where
        S: SendableFusedStream<Item = A::Message>,
    {
        self.attach_stream(stream);
        self.launch()
    }

    /// Launches an actor that uses the given state. Returns a client to the actor.
    pub fn launch(self) -> JointClient<A::Permanence, A::Message, A::Output> {
        let Self {
            send,
            recv,
            state,
            broadcast,
            edges,
            ..
        } = self;
        let (broad, sub) = broadcast.unwrap_or_else(|| broadcast::channel(100));
        let mut runner = ActorRunner::new(state, edges);
        recv.into_iter().for_each(|r| runner.add_stream(r));
        runner.add_broadcaster(broad);
        runner.launch();
        let sink = SinkClient::new(send);
        let stream = StreamClient::new(sub);
        JointClient::new(sink, stream)
    }
}
