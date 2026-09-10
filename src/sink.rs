//! Actors that are only sent messages (either fire-and-forget messages or request-response
//! messages).
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use futures::stream::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::ActorKind;
use crate::ActorState;
use crate::Scheduler;

/// The [`ActorKind`] for actors that only receive messages. A sink actor is one that receives
/// messages from other parts of the application. By adding a oneshot channel to the message,
/// the actor can respond with a particular piece of data. This allows for type-safe communication
/// between different parts of your program.
///
/// The client of a [`SinkActor`] is the [`SinkClient`]. This client implements methods that allow
/// for the sending of messages to this client. Communication between a sink client and sink actor
/// uses an MPSC-style channel (see [`mpsc::channel`](tokio::sync::mpsc)); constructing this kind
/// creates that channel, hands the sending half to the client, and attaches the receiving half to
/// the actor's [`Scheduler`].
///
/// Unlike the other kinds, a sink actor sends nothing back to its clients outside of the oneshot
/// channels carried by its own messages, so this type holds no state of its own.
#[derive(Debug)]
pub struct SinkActor {}

impl<S: ActorState> ActorKind<S> for SinkActor {
    type Client = SinkClient<S::Message>;
    type Config = ();

    fn construct(
        (): Self::Config,
    ) -> (Self, Self::Client, impl 'static + FnOnce(&mut Scheduler<S>)) {
        let this = Self {};

        let (send, recv) = tokio::sync::mpsc::unbounded_channel();
        let client = SinkClient::new(send);
        let func = move |scheduler: &mut Scheduler<S>| {
            scheduler.attach_stream(UnboundedReceiverStream::new(recv).fuse());
        };
        (this, client, func)
    }
}

/// A client to an actor. This client sends messages to the actor and supports two styles of
/// messaging. The first is fire-and-forget messages. These messages are sent to the client
/// immediately (no `.await` needed). The actor will process them eventually. The second kind is
/// request-response or "trackable" messages. These messages are identical to the last kind except
/// they contain a one-time use channel that the actor will use to send a message back.
///
/// It is helpful to use the [`derive_more`](https://crates.io/crates/derive_more) crate's
/// [`From`](https://jeltef.github.io/derive_more/derive_more/from.html) derive macro with a sink
/// actor's message type. The [`send`](SinkClient::send) and [`track`](SinkClient::track) methods
/// of the `SinkClient` perform automatic convertion between the provided data and the actor's
/// message type. Say you have an actor like the one below. You can send messages to that actor
/// like so:
/// ```ignore
/// # use std::collections::HashMap;
/// # use troupe::prelude::*;
/// # use derive_more::From;
/// #[derive(Default)]
/// struct CacheState(HashMap<usize, String>);
///
/// #[derive(From)]
/// enum CacheCommand {
///     Insert(usize, String),
///     Get(usize, OneshotSender<Option<String>>),
///     Delete(usize),
/// }
///
/// # impl ActorState for CacheState {
/// #   type ActorKind = SinkActor;
/// #   type Message = CacheCommand;
/// #
/// #   async fn process(&mut self, scheduler: &mut Scheduler<Self>, msg: Self::Message) { () }
/// # }
/// // `SinkActor`'s config is `()`, so the builder can be launched directly.
/// let client: SinkClient<CacheCommand> = ActorBuilder::new(CacheState::default()).launch();
///
/// // Sends CacheCommand::Insert(42, "Hello world")
/// client.send((42, String::from("Hello World")));
/// // Sends CacheCommand::Get(42, OneshotSender) and returns a tracker which will listen for a
/// // response from the actor.
/// let tracker = client.track(42);
/// // Sends CacheCommand::Delete(42)
/// client.send(42);
/// ```
#[derive(Debug)]
pub struct SinkClient<M> {
    send: UnboundedSender<M>,
}

impl<M> SinkClient<M> {
    pub(crate) fn new(send: UnboundedSender<M>) -> Self {
        Self { send }
    }

    /// Returns if the actor that the client is connected to is dead or not.
    pub fn is_closed(&self) -> bool {
        self.send.is_closed()
    }

    /// Sends a fire-and-forget style message to the actor and returns if the message was sent
    /// successfully.
    pub fn send(&self, msg: impl Into<M>) -> bool {
        self.send.send(msg.into()).is_ok()
    }

    /// Sends a request-response style message to an actor. The given data is paired with a
    /// one-time use channel and sent to the actor. A [`Tracker`] that will receive a response from
    /// the actor is returned.
    pub fn track<I, O>(&self, msg: I) -> Tracker<O>
    where
        M: From<(I, oneshot::Sender<O>)>,
    {
        let (send, recv) = oneshot::channel();
        let msg = M::from((msg, send));
        let _ = self.send(msg);
        Tracker::new(recv)
    }
}

impl<M> Clone for SinkClient<M> {
    fn clone(&self) -> Self {
        Self::new(self.send.clone())
    }
}

/// A tracker for a request-response style message sent to an actor.
///
/// Note: This tracker might be created after a failed attempt to send a message to a dead
/// actor. This means that the tracker will return `None` when polled; however, that does not
/// mean that the message was successfully received by the actor.
#[derive(Debug)]
pub struct Tracker<T> {
    recv: oneshot::Receiver<T>,
}

impl<T> Tracker<T> {
    /// A constuctor for the tracker.
    pub(crate) fn new(recv: oneshot::Receiver<T>) -> Self {
        Self { recv }
    }
}

impl<T> Future for Tracker<T> {
    type Output = Option<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.recv).poll(cx).map(Result::ok)
    }
}
