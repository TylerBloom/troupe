use futures::{
    stream::{select_all, Fuse, FusedStream, FuturesUnordered, SelectAll},
    FutureExt, Stream, StreamExt,
};
use instant::Instant;
use tokio::sync::{broadcast, mpsc::UnboundedReceiver};
use tokio_stream::wrappers::UnboundedReceiverStream;

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{compat::{spawn_task, SendableFuture, SendableFusedStream, Sendable, SendableStream}, ActorState, Timer};

type FuturesCollection<T> = FuturesUnordered<Pin<Box<dyn SendableFuture<Output = T>>>>;

/// The primary bookkeeper for the actor. The state can queue and manage additional futures and
/// streams with it. The scheduler also tracks if it is possible that another message will be
/// yielded and processes by the actor. If it finds itself in a state where all streams are closed
/// and there are no queued futures, it will close the actor; otherwise, the deadlocked actor will
/// stay in memory during nothing.
pub struct Scheduler<A: ActorState> {
    /// The number of sources that could yield a message for the actor to process. Once it hits
    /// zero, the actor is dead as it can no longer process any messages.
    count: usize,
    /// The inbound streams to the actor.
    recv: SelectAll<Fuse<ActorStream<A::Message>>>,
    /// Futures that the actor has queued that will yield a message.
    queue: FuturesCollection<A::Message>,
    /// Futures that yield nothing that the scheduler will manage and poll for the actor.
    tasks: FuturesCollection<()>,
    /// The manager for outbound messages that will be broadcast from the actor.
    outbound: Option<OutboundQueue<A::Output>>,
    // TODO:
    //  - Add a queue for timers so that they are not lumped in the `queue`.
    //    - Make those timers cancelable by assoicating each on with an id (usize)
    //  - Make all messages and streams abortable with `futures::stream::Abortable`
}

struct OutboundQueue<M> {
    send: broadcast::Sender<M>,
}

impl<M: Sendable + Clone> OutboundQueue<M> {
    fn new(send: broadcast::Sender<M>) -> Self {
        Self { send }
    }

    fn send(&mut self, msg: M) {
        // TODO: Properly handle errors
        let _ = self.send.send(msg);
    }
}

/// The container for the actor state and its scheduler. The runner polls the scheduler, aids in
/// bookkeeping if the actor is dead or not, and passes messages off to the state.
pub(crate) struct ActorRunner<A: ActorState> {
    state: A,
    scheduler: Scheduler<A>,
}

impl<A: ActorState> ActorRunner<A> {
    pub(crate) fn new(state: A) -> Self {
        let scheduler = Scheduler::new();
        Self { state, scheduler }
    }

    pub(crate) fn add_broadcaster(&mut self, broad: broadcast::Sender<A::Output>) {
        self.scheduler.outbound = Some(OutboundQueue::new(broad));
    }

    pub(crate) fn add_stream<S, M>(&mut self, stream: S)
    where
        S: SendableFusedStream<Item = M>,
        M: Into<A::Message>,
    {
        self.scheduler.add_stream(stream);
    }

    pub(crate) fn launch(self) {
        spawn_task(self.run())
    }

    async fn run(mut self) {
        self.state.start_up(&mut self.scheduler).await;
        loop {
            if self.scheduler.is_dead() {
                panic!("Scheduler is dead!!!");
            }
            let msg = self.scheduler.next().await;
            self.state.process(&mut self.scheduler, msg).await;
        }
    }
}

impl<A: ActorState> Scheduler<A> {
    /// The constructor for the scheduler.
    fn new() -> Self {
        let recv = select_all([]);
        let queue = FuturesCollection::new();
        let tasks = FuturesCollection::new();
        Self {
            recv,
            queue,
            tasks,
            count: 0,
            outbound: None,
        }
    }

    /// Returns if the actor is dead and should be dropped.
    fn is_dead(&self) -> bool {
        self.count == 0
    }

    /// Yields the next message to be processed by the actor state.
    async fn next(&mut self) -> A::Message {
        loop {
            tokio::select! {
                msg = self.recv.next() => {
                    // TODO: This causes an issue where, if `recv` is out of valid streams, the
                    // count goes to zero. This is a problem because it ignores the `queue`. To
                    // manage this, we should split the stream count from the main count and add a
                    // condition to this branch that checks that count.
                    match msg {
                        Some(msg) => return msg,
                        None => {
                            self.count -= 1;
                        }
                    }
                },
                msg = self.queue.next(), if !self.queue.is_empty() => {
                    self.count -= 1;
                    return msg.unwrap();
                },
                _ = self.tasks.next(), if !self.tasks.is_empty() => {},
            }
        }
    }

    /// Adds a future to the scheduler that will be managed and polled by the [`Scheduler`]. The
    /// output from the future must be convertible into a message for the actor to later process.
    /// If you like the scheduler to just poll and manage the future, see the [`manage_future`].
    ///
    /// Note: There is no ordering to the collection of futures. Moreover, there is no ordering
    /// between the queued futures and queued streams. The first to yield an item is the first to
    /// be processed. For this reason, the futures queued this way must be `'static`.
    pub fn add_task<F, I>(&mut self, fut: F)
    where
        F: Sendable + Future<Output = I>,
        I: 'static + Into<A::Message>,
    {
        self.count += 1;
        self.queue.push(Box::pin(fut.map(Into::into)));
    }

    /// Adds the given future to an internal queue of futures that the scheduler will manage;
    /// however, anything that the future yields will be dropped immediately. If the item yielded
    /// by the future can be turned into a message for the actor and you would the actor to process
    /// it, see the [`add_task`] method.
    ///
    /// Note: Managed futures are polled at the same time as the queued futures that yield messages
    /// and the queued stream. For this reason, the managed futures must be `'static`.
    pub fn manage_future<F>(&mut self, fut: F)
    where
        F: Sendable + Future<Output = ()>,
    {
        self.tasks.push(Box::pin(fut));
    }

    /// Adds a stream that will be polled and managed by the scheduler. Messages yielded by the
    /// streams will be processed by the actor. The given stream must be a [`FusedStream`];
    /// however, the scheduler will mark a stream as done once it yields its first `None` and will
    /// never poll that stream again. If you would like to add a non-fused stream, see
    /// [`add_endless_stream`].
    pub fn add_stream<S, I>(&mut self, stream: S)
    where
        S: SendableStream<Item = I> + FusedStream,
        I: Into<A::Message>,
    {
        self.count += 1;
        self.recv
            .push(ActorStream::Secondary(Box::new(stream.map(|m| m.into()))).fuse());
    }

    pub fn schedule<M>(&mut self, deadline: Instant, msg: M)
    where
        M: 'static + Into<A::Message>,
    {
        self.count += 1;
        self.queue.push(Box::pin(Timer::new(deadline, msg.into())));
    }

    /// Broadcasts a message to all listening clients. If the message fails to send, the msg will
    /// be dropped.
    ///
    /// Note: This method does nothing if the actor is a [`SinkActor`]. [`StreamActor`]s and
    /// [`JointActor`] will be able to broadcast.
    // TODO: Return back the message or in some way handle failures for the user.
    pub fn broadcast<M>(&mut self, msg: M)
    where
        M: Into<A::Output>,
    {
        if let Some(out) = self.outbound.as_mut() {
            out.send(msg.into())
        }
    }
}

impl<M: Sendable> From<UnboundedReceiver<M>> for ActorStream<M> {
    fn from(value: UnboundedReceiver<M>) -> Self {
        Self::Main(UnboundedReceiverStream::new(value))
    }
}

pub(crate) enum ActorStream<M> {
    Main(UnboundedReceiverStream<M>),
    Secondary(Box<dyn SendableFusedStream<Item = M>>),
}

impl<M: Sendable> Stream for ActorStream<M> {
    type Item = M;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match *self {
            ActorStream::Main(ref mut stream) => Pin::new(stream).poll_next(cx),
            ActorStream::Secondary(ref mut stream) => Pin::new(stream).poll_next(cx),
        }
    }
}
