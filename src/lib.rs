pub mod compat;
pub mod joint;
pub mod scheduler;
pub mod sink;
pub mod stream;

pub use async_trait::async_trait;
use joint::{JointClient, JointActor};
use scheduler::{Scheduler, ActorStream, ActorRunner};
use sink::{SinkActor, SinkClient};
use stream::{StreamActor, StreamClient};
pub use tokio::sync::oneshot::{
    channel as oneshot_channel, Receiver as OneshotReceiver, Sender as OneshotSender,
};

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Future, FutureExt, StreamExt, stream::FusedStream};
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
/// can forward more messages. Actors serves two roles. They can act similarly to a [`Sink`] where
/// other parts of your application (including other actors) since messages into the actor. They
/// can also act as a [`Stream`] that generate messages to be sent throughout your application.
/// This role is denoted by the actor's `ActorType`, which informs the [`ActorBuilder`] what kind
/// of actor it is working with. For sink-like actors, use the [`SinkActor`] type. For stream-like
/// actors, use the [`StreamActor`] type. For actors that function as both, use the [`JointActor`]
/// type.
#[async_trait]
pub trait ActorState: 'static + Send + Sized {
    /// This type should be either [`SinkActor`], [`StreamActor`], or [`JointActor`]. This type is
    /// mostly a marker to inform the [`ActorBuilder`].
    type ActorType;

    /// Inbound messages to the actor must be this type. Clients will send the actor messages of
    /// this type and any queued futures or streams must yield this type.
    type Message: 'static + Send;

    /// For [`SinkActor`]s and [`JointActor`]s, this is the message type which is broadcasts.
    /// For [`StreamActor`]s, this can be `()` (unfortunately, default associated types are
    /// unstable).
    type Output: 'static + Send + Clone;

    /// Before starting the main loop of running the actor, this method is called to finalize any
    /// setup of the actor state. No inbound messages will be processed until this method is
    /// completed.
    #[allow(unused_variables)]
    async fn start_up(&mut self, scheduler: &mut Scheduler<Self>) {}

    /// The heart of the actor. This method consumes messages from clients and queued futures and
    /// streams. For [`SinkActor`]s and [`JointActor`]s, the state "responds" to messages with a
    /// [`OneshotChannel`]. The state can also queue futures and streams in the [`Scheduler`].
    /// Finally, for [`StreamActor`]s and [`JointActor`]s, any messages to forwarded can be queued
    /// in the [`Scheduler`].
    async fn process(&mut self, scheduler: &mut Scheduler<Self>, msg: Self::Message);
}

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

    pub fn add_stream<S, I>(&mut self, stream: S)
    where
        S: 'static + Send + Unpin + FusedStream<Item = I>,
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
    pub fn sink_client(&self) -> SinkClient<A::Message> {
        SinkClient::new(self.send.clone())
    }

    pub fn launch_sink(self) -> SinkClient<A::Message> {
        let Self {
            send, recv, state, ..
        } = self;
        let mut runner = ActorRunner::new(state);
        recv.into_iter()
            .for_each(|r| runner.add_stream(r.fuse()));
        runner.launch();
        SinkClient::new(send)
    }
}

/* --------- Stream actors --------- */
impl<A> ActorBuilder<A>
where
    A: ActorState<ActorType = StreamActor>,
{
    pub fn stream_client(&self) -> StreamClient<A::Output> {
        StreamClient::new(self.broadcast.as_ref().unwrap().1.resubscribe())
    }

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
        recv.into_iter()
            .for_each(|r| runner.add_stream(r.fuse()));
        runner.launch();
        StreamClient::new(sub)
    }
}

/* --------- Joint actors --------- */
impl<A> ActorBuilder<A>
where
    A: ActorState<ActorType = JointActor>,
{
    pub fn stream(&self) -> StreamClient<A::Output> {
        StreamClient::new(self.broadcast.as_ref().unwrap().1.resubscribe())
    }

    pub fn sink(&self) -> SinkClient<A::Message> {
        SinkClient::new(self.send.clone())
    }

    pub fn launch_with_stream<S>(mut self, stream: S) -> JointClient<A::Message, A::Output>
    where
        S: 'static + Send + Unpin + FusedStream<Item = A::Message>,
    {
        self.add_stream(stream);
        self.launch()
    }

    pub fn launch(self) -> JointClient<A::Message, A::Output> {
        let Self {
            send,
            recv,
            state,
            broadcast,
        } = self;
        let (broad, sub) = broadcast.unwrap_or_else(|| broadcast::channel(100));
        let mut runner = ActorRunner::new(state);
        recv.into_iter()
            .for_each(|r| runner.add_stream(r.fuse()));
        runner.add_broadcaster(broad);
        runner.launch();
        let sink = SinkClient::new(send);
        let stream = StreamClient::new(sub);
        JointClient::new(sink, stream)
    }
}

/* -------- To move -------- */

#[pin_project]
pub struct Timer<T> {
    #[pin]
    deadline: Sleep,
    msg: Option<T>,
}

impl<T> Timer<T> {
    pub fn new(deadline: Instant, msg: T) -> Self {
        Self {
            deadline: sleep_until(deadline),
            msg: Some(msg),
        }
    }
}

impl<T> Future for Timer<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.deadline
            .poll_unpin(cx)
            .map(|_| self.msg.take().unwrap())
    }
}
