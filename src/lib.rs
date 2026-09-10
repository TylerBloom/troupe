//! Troupe provides a high-level toolset for modelling states and data flows. Troupe actors are
//! built on top of async tasks (like those created from [`tokio::spawn`](tokio::spawn)) and help
//! you model and control the flow of information into and out of them. The main goals of `troupe`
//! are to provide:
//! - Consise ways of modelling state and state changes
//! - APIs that remove the boilerplate of creating and running async actors
//! - An easy to conceptualize data flow
//! - Ergonomic APIs devoid of magic
//! - A model that can be adopted into an existing project all at once or over time
//! - Support for both native and WASM targets
//!
//! Troupe currently supports three async runtimes: `tokio`, `async-std`, and the runtime provided
//! by the browser (via wasm-bindgen-futures). Do note that even if you are using the `async-std`
//! runtime, client-actor communication is still done via tokio channels.
//!
//! The general model of an actor is a state machine that receieves messages from surrounding
//! state, updates its state, and potentially sends messages to other actors and/or spawns more
//! actors (see the [wiki](https://en.wikipedia.org/wiki/Actor_model)). Troupe handles all of the
//! plumbing so you can focus on modelling your state. The state of an actor is modelled with the
//! [`ActorState`] trait.
//!
//! While `troupe` handles all of the plumbing for you, knowing how messages are sent and receieved
//! is key to leveraging `troupe` to its full potential. Messages can have
//!
//! Messages arriving at the actors, messages the actor sends itself, messages leaving the actors,
//!
//! Imagine a simple case where, in a web server, an actor receives messages that need to be logged
//! for telemetry. Troupe provides mechanisms for attaching [`Stream`]s to an actor's task. An
//! actor can have any number of [`Stream`]s attached, and all of them are selected on
//! indescrimiately. Meaning if two streams should yeild items one after another, that invariant is
//! not enforced by `troupe`.
//!
//! Most actors have one stream by default, their client. When spawned, a client to the actor is
//! returned. Clients can be modelled in many ways, but many provide a way to send messages into
//! the actor. In the logging actor example, the state for the rest of the server would have
//! clients to the logging actor. Those clients would then send data that could then be logged.
//!
//! Actors can receive messages from places other than their client(s). Consider that same web
//! server. Perhaps it is using [`websockets`]() somewhere in its stack. A websocket is,
//! effectively, just a [`Stream`] of messages from one of the machine's socket. Troupe provides an
//! [`ActorState`] with the ability to attach a websocket, as a [`Stream`], so the actor respond to
//! messages from the websocket just like it would respond to client messages. Similarly, perhaps
//! that actor also needs to send an HTTP request. That request can be awaited and the resulting
//! response turned into a message that the actor can process. All of this is handled by the
//! [`Scheduler`].
//!
//! NOTE: Since all [`Stream`]s *and* [`Future`]s handled by the [`Scheduler`] are selected on,
//! they need to be cancel safe.

// TODO:
// - Make ActorState::start_up return V
// - Normalize the language between docs and across methods, e.g. "ActorBuilder::launch" conflicts
// with "spawning an actor"
// - Make Unbound/backpressure clients

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

use futures::StreamExt;

use compat::MaybeSendFuture;
use compat::Sendable;
use compat::SendableFusedStream;
use scheduler::ActorRunner;
use scheduler::ActorStream;

pub use scheduler::Scheduler;

#[cfg(doc)]
use prelude::*;
#[cfg(doc)]
use sink::*;
#[cfg(doc)]
use stream::*;

/// The core abstraction of the actor model. An [`ActorState`] sits at the heart of every actor. It
/// processes messages, queues futures, and attaches streams in the [`Scheduler`], and it can
/// forward messages.
///
///
/// Actors serves two roles. They can act similarly to a
/// [`Sink`](futures::Sink) where other parts of your application (including other actors) since
/// messages into the actor. They can also act as a [`Stream`](futures::Stream) that generate
/// messages to be sent throughout your application. This role is denoted by the actor's
/// [`ActorKind`], which informs the [`ActorBuilder`] what kind of actor it is working with. For
/// sink-like actors, use the [`SinkActor`] type. For stream-like actors, use the [`StreamActor`]
/// type. For actors that function as both, use the [`JointActor`] type.
///
/// Note: When implementing this trait, all of the methods are `async` and you can use the `async
/// fn` to implement them. Their current bounds of `MaybeSendFuture` are there to abstract over the
/// different requirements for native and WASM targets.
pub trait ActorState: Sendable + Sized {
    /// The type that inbound messages to the actor will be.
    type Message: Sendable;

    /// Actors can serve different roles in the surrounding state, defined by the type of client
    /// the actor has. The [`ActorKind`] defines those clients as well as additional state the
    /// [`Scheduler`] has to interact with those clients.
    type ActorKind: ActorKind<Self>;

    /// Before starting the main loop of running the actor, this method is called to finalize any
    /// setup of the actor state, such as pulling data from a database or from over the network or
    /// simply waiting for the first few messages from the [`Scheduler`]
    ///
    /// Note: When implementing this method, you can use `async fn` instead of `impl
    /// MaybeSendFuture`.
    #[allow(unused_variables)]
    fn start_up(&mut self, scheduler: &mut Scheduler<Self>) -> impl MaybeSendFuture<Output = ()> {
        std::future::ready(())
    }

    /// The heart of the actor. This method consumes messages yielded by attached streams and
    /// queued futures. As part of processing a message, an actor can spawn other actors, provide
    /// streams and futures to the [`Scheduler`] to be managed, send messages other actors via
    /// clients the state has, and, if applicable, interact with outbound streams provided by the
    /// actor's [`ActorKind`].
    ///
    /// Note: When implementing this method, you can use `async fn` instead of `impl
    /// MaybeSendFuture`.
    fn process(
        &mut self,
        scheduler: &mut Scheduler<Self>,
        msg: Self::Message,
    ) -> impl MaybeSendFuture<Output = ()>;

    /// Once the actor has completed, this method is called to allow the actor to clean up anything
    /// that remains.
    ///
    /// Note: When implementing this method, you can use `async fn` instead of `impl
    /// MaybeSendFuture`.
    #[allow(unused_variables)]
    fn finalize(self, scheduler: &mut Scheduler<Self>) -> impl MaybeSendFuture<Output = ()> {
        std::future::ready(())
    }
}

/// Where [`ActorState`] describes how an actor reacts to messages, the `ActorKind` trait describes
/// how messages get into and out of that actor. The three kinds provided by this crate are
/// [`SinkActor`], [`StreamActor`], and [`JointActor`], but these can be extended into other
/// message-passing patterns (e.g. back pressure, single-consumer clients, etc.) without changing
/// the [`Scheduler`].
///
/// A kind is constructed when the actor is launched where it does three jobs:
/// 1. Builds the [`Client`](ActorKind::Client) that [`ActorBuilder::launch`] returns.
/// 2. Attaches whatever inbound streams that client needs. A [`SinkActor`], for example,
///    attaches the receiving half of the [`mpsc-style`](tokio::sync::mpsc) channel that backs its
///    [`SinkClient`].
/// 3. Keeps whatever state is needed to send messages back out. A [`StreamActor`], for example,
///    holds the [`broadcast`](tokio::sync::broadcast) sender that its [`StreamClient`]s listen to.
///
/// That third piece is stored in the [`Scheduler`], which [`Deref`](std::ops::Deref)s to the kind.
/// Any inherent method a kind defines is therefore callable straight off the `scheduler` that an
/// [`ActorState`] is handed, which is how [`StreamActor::broadcast`] is reached.
pub trait ActorKind<S: ActorState>: Sized + Sendable {
    /// The client that is returned to the caller of [`ActorBuilder::launch`].
    type Client;

    /// The data that this needs in order to be constructed. Kinds that need no config should use
    /// `()`, so [`ActorBuilder::config`] does not need to be called. Otherwise, a value of this
    /// type must be given to the builder via [`ActorBuilder::config`] before launching.
    type Config;

    /// Uses the given config to construct `Self`, its client, and a callback that finishes wiring
    /// up the [`Scheduler`].
    ///
    /// The kind is needed to build the scheduler, so anything that must be registered *on* the
    /// scheduler (most notably inbound streams, which are attached via
    /// [`Scheduler::attach_stream`]) can not be done here. Instead, that work is done by the
    /// returned closure once the scheduler exists.
    fn construct(
        config: Self::Config,
    ) -> (Self, Self::Client, impl 'static + FnOnce(&mut Scheduler<S>));
}

/// Holds a type that implements [`ActorState`], helps aggregate all data that the actor needs, and
/// then spawns the actor in an async task. After spawning the task, a client is returned
/// to the caller. That client's type is the [`Client`](ActorKind::Client) of the state's
/// [`ActorKind`].
#[allow(missing_debug_implementations)]
pub struct ActorBuilder<A: ActorState, C> {
    recv: Vec<ActorStream<A::Message>>,
    config: C,
    state: A,
}

/* --------- All actors --------- */
impl<A: ActorState> ActorBuilder<A, ()> {
    /// Constructs a new builder for an actor that uses the given state.
    pub fn new(state: A) -> Self {
        Self {
            state,
            recv: vec![],
            config: (),
        }
    }
}

impl<A: ActorState, C> ActorBuilder<A, C> {
    /// Attaches a stream that will be used by the actor once its spawned. No messages will be
    /// processed until after the actor is launched.
    pub fn attach_stream<S, I>(mut self, stream: S) -> Self
    where
        S: SendableFusedStream<Item = I>,
        I: Into<A::Message>,
    {
        self.recv.push(Box::new(stream.map(|m| m.into())));
        self
    }

    /// Provides the config that the state's [`ActorKind`] needs in order to be constructed at
    /// launch. Kinds whose [`Config`](ActorKind::Config) is a unit do not need this method to be
    /// called. Calling this method more than once drops the previous config.
    pub fn config(
        self,
        config: <A::ActorKind as ActorKind<A>>::Config,
    ) -> ActorBuilder<A, <A::ActorKind as ActorKind<A>>::Config> {
        let Self {
            recv,
            state,
            config: _,
        } = self;
        ActorBuilder {
            recv,
            config,
            state,
        }
    }
}

impl<A, K, C> ActorBuilder<A, C>
where
    K: ActorKind<A, Config = C>,
    A: ActorState<ActorKind = K>,
{
    /// Spawns the actor in a separate async task and returns the client of the actor's
    /// [`ActorKind`], through which messages can be sent and/or received.
    pub fn spawn(self) -> K::Client {
        let Self {
            recv,
            config,
            state,
        } = self;
        let (kind, client, init) = A::ActorKind::construct(config);
        let mut runner = ActorRunner::new(state, kind);
        init(&mut runner.scheduler);
        recv.into_iter().for_each(|r| runner.attach_stream(r));
        runner.spawn();
        client
    }
}
