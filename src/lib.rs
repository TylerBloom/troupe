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
//! clients should function via the [`ActorState`]'s [`ActorKind`]. Conceptually, every actor is
//! either something that consumes messages, i.e. a `Sink`, or something to broadcasts messages,
//! i.e. a `Stream`, or both. An actor that largely receive messages from other parts of our
//! program is a [`SinkActor`], which use [`SinkClient`]s. An actor that processes messages from a
//! source (for example a Websocket) and then broadcast these messages is a [`StreamActor`], which
//! use [`StreamClient`]s. If an actor does both of these, it is a [`JointActor`] and uses
//! [`JointClient`]s.
//!
//! An [`ActorKind`] is more than a label, though. When the actor is launched, its kind is
//! constructed. That construction produces the client that is handed back to the caller, any
//! streams that feed the actor (such as the [`mpsc-style`](tokio::sync::mpsc) channel behind a
//! [`SinkClient`]), and whatever state the kind needs in order to talk back to its clients (such
//! as the broadcast channel behind a [`StreamClient`]). That state lives inside the [`Scheduler`],
//! which [`Deref`](std::ops::Deref)s to it, so an actor broadcasts by calling
//! [`StreamActor::broadcast`] directly on its scheduler.
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

use futures::StreamExt;

use compat::MaybeSendFuture;
use compat::Sendable;
use compat::SendableFusedStream;
use scheduler::ActorRunner;
use scheduler::ActorStream;

#[cfg(doc)]
use prelude::*;
#[cfg(doc)]
use sink::*;
#[cfg(doc)]
use stream::*;

pub use scheduler::Scheduler;
pub use tokio::sync::oneshot::channel as oneshot_channel;
pub use tokio::sync::oneshot::Receiver as OneshotReceiver;
pub use tokio::sync::oneshot::Sender as OneshotSender;

/// The core abstraction of the actor model. An [`ActorState`] sits at the heart of every actor. It
/// processes messages, queues futures, and attaches streams in the [`Scheduler`], and it can
/// forward messages. Actors serves two roles. They can act similarly to a
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
    /// The kind of actor that this state models, such as [`SinkActor`], [`StreamActor`], or
    /// [`JointActor`]. The kind determines the client type that the [`ActorBuilder`] hands back on
    /// launch, the config that the builder needs, and how messages flow into and out of the actor.
    ///
    /// The kind is not a bare marker. Its state is stored in the [`Scheduler`], which
    /// [`Deref`](std::ops::Deref)s to it, so the methods a kind exposes (such as
    /// [`StreamActor::broadcast`]) are callable directly on the `scheduler` given to
    /// [`start_up`](ActorState::start_up), [`process`](ActorState::process), and
    /// [`finalize`](ActorState::finalize).
    type ActorKind: ActorKind<Self>;

    /// Inbound messages to the actor must be this type. Clients will send the actor messages of
    /// this type and any queued futures or streams must yield this type.
    type Message: Sendable;

    /// Before starting the main loop of running the actor, this method is called to finalize any
    /// setup of the actor state, such as pulling data from a database or from over the network. No
    /// inbound messages will be processed until this method is completed.
    ///
    /// Note: When implementing this method, you can use `async fn` instead of `impl
    /// MaybeSendFuture`.
    #[allow(unused_variables)]
    fn start_up(&mut self, scheduler: &mut Scheduler<Self>) -> impl MaybeSendFuture<Output = ()> {
        std::future::ready(())
    }

    /// The heart of the actor. This method consumes messages attached streams and queued futures
    /// and streams. For [`SinkActor`]s and [`JointActor`]s, the state can "respond" to messages
    /// containing a [`OneshotChannel`](tokio::sync::oneshot::channel) sender. The state can also
    /// queue futures and attach streams in the [`Scheduler`]. Finally, for [`StreamActor`]s and
    /// [`JointActor`]s, the state can broadcast messages by calling
    /// [`broadcast`](StreamActor::broadcast) on the [`Scheduler`], which derefs to the actor's
    /// [`ActorKind`].
    ///
    /// Note: When implementing this method, you can use `async fn` instead of `impl
    /// MaybeSendFuture`.
    fn process(
        &mut self,
        scheduler: &mut Scheduler<Self>,
        msg: Self::Message,
    ) -> impl MaybeSendFuture<Output = ()>;

    /// Once the actor has died, this method is called to allow the actor to clean up anything that
    /// remains.
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
/// [`SinkActor`], [`StreamActor`], and [`JointActor`], but this trait is public so that other
/// message-passing patterns (back pressure, single-consumer clients, etc.) can be modelled without
/// changing the [`Scheduler`].
///
/// A kind is constructed when the actor is launched, and it does three jobs at that point:
/// 1. It builds the [`Client`](ActorKind::Client) that [`ActorBuilder::launch`] returns.
/// 2. It attaches whatever inbound streams that client needs. A [`SinkActor`], for example,
///    attaches the receiving half of the [`mpsc-style`](tokio::sync::mpsc) channel that backs its
///    [`SinkClient`].
/// 3. It keeps whatever state is needed to send messages back out. A [`StreamActor`], for example,
///    holds the [`broadcast`](tokio::sync::broadcast) sender that its [`StreamClient`]s listen to.
///
/// That third piece is stored in the [`Scheduler`], which [`Deref`](std::ops::Deref)s to the kind.
/// Any inherent method a kind defines is therefore callable straight off the `scheduler` that an
/// [`ActorState`] is handed, which is how [`StreamActor::broadcast`] is reached.
pub trait ActorKind<S: ActorState>: Sized + Sendable {
    /// The client that is returned to the caller of [`ActorBuilder::launch`].
    type Client;

    /// The data that this kind needs in order to be constructed. Kinds that need nothing use `()`,
    /// in which case [`ActorBuilder::config`] does not need to be called. Otherwise, a value of
    /// this type must be given to the builder via [`ActorBuilder::config`] before launching.
    type Config;

    /// Uses the given config to construct the kind, its client, and a callback that finishes
    /// wiring up the [`Scheduler`].
    ///
    /// The kind itself is needed to build the scheduler, so anything that must be registered *on*
    /// the scheduler (most notably inbound streams, which are attached with
    /// [`Scheduler::attach_stream`]) can not be done here. Instead, that work is returned as a
    /// closure that the builder runs once the scheduler exists.
    fn construct(
        config: Self::Config,
    ) -> (Self, Self::Client, impl 'static + FnOnce(&mut Scheduler<S>));
}

/// Holds a type that implements [`ActorState`], helps aggregate all data that the actor needs, and
/// then launches the async actor process. When the actor process is launched, a client is returned
/// to the caller. That client's type is the [`Client`](ActorKind::Client) of the state's
/// [`ActorKind`].
///
/// The `C` parameter tracks the config that has been given to the builder so far. A builder starts
/// out as `ActorBuilder<A, ()>` and, if the state's kind needs a config, is moved to
/// `ActorBuilder<A, Config>` by [`config`](ActorBuilder::config). Only a builder whose `C` matches
/// its kind's [`Config`](ActorKind::Config) can be launched, so a missing config is a compile
/// error rather than a runtime one. Kinds whose config is `()` (all three provided by this crate)
/// can be launched straight after [`new`](ActorBuilder::new).
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
    ///
    /// This is in addition to whatever streams the state's [`ActorKind`] attaches for itself, such
    /// as the channel behind a [`SinkClient`]. It can be called any number of times.
    pub fn attach_stream<S, I>(mut self, stream: S) -> Self
    where
        S: SendableFusedStream<Item = I>,
        I: Into<A::Message>,
    {
        self.recv.push(Box::new(stream.map(|m| m.into())));
        self
    }

    /// Provides the config that the state's [`ActorKind`] needs in order to be constructed at
    /// launch. Kinds whose [`Config`](ActorKind::Config) is `()` do not need this method to be
    /// called. Calling this method more than once discards the previous config.
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
    /// Launches the actor in a separate async task and returns the client for the actor's
    /// [`ActorKind`], through which messages can be sent and/or received.
    ///
    /// No message is processed until after the actor is launched and [`ActorState::start_up`] has
    /// completed, including messages yielded by streams given to
    /// [`attach_stream`](ActorBuilder::attach_stream).
    pub fn launch(self) -> K::Client {
        let Self {
            recv,
            config,
            state,
        } = self;
        let (kind, client, init) = A::ActorKind::construct(config);
        let mut runner = ActorRunner::new(state, kind);
        init(&mut runner.scheduler);
        recv.into_iter().for_each(|r| runner.add_stream(r));
        runner.launch();
        client
    }
}
