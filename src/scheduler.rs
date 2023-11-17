use futures::{
    stream::{select_all, Fuse, FusedStream, FuturesUnordered, SelectAll},
    FutureExt, Stream, StreamExt,
};
use instant::Instant;
use pin_project::pin_project;
use tokio::sync::{broadcast, mpsc::UnboundedReceiver};
use tokio_stream::wrappers::UnboundedReceiverStream;

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{
    compat::{
        sleep_until, spawn_task, Sendable, SendableAnyMap, SendableFusedStream, SendableFuture,
        SendableStream, SendableWrapper, Sleep,
    },
    sink::SinkClient,
    ActorState, Transient,
};

type FuturesCollection<T> = FuturesUnordered<Pin<Box<dyn SendableFuture<Output = T>>>>;

/// Encapulates the different states a scheduler can be. Largely used to communicate how a state
/// wishes to shutdown.
enum SchedulerStatus {
    Alive,
    Marked,
    MarkedToFinish,
}

/// The primary bookkeeper for the actor. The state attach stream and queue manage futures that
/// will be managed by the Scheduler.
/// The scheduler also tracks if it is possible that no other message will be
/// yielded for the actor to process. If it finds itself in a state where all streams are closed
/// and there are no queued futures, it will close the actor; otherwise, the deadlocked actor will
/// stay in memory doing nothing.
///
/// Note: If the scheduler finds that the actor is dead but is also managing futures for it, the
/// scheduler will spawn a new async task to poll those futures to completion.
#[allow(missing_debug_implementations)]
pub struct Scheduler<A: ActorState> {
    /// The inbound streams to the actor.
    recv: SelectAll<Fuse<ActorStream<A::Message>>>,
    /// Futures that the actor has queued that will yield a message.
    queue: FuturesCollection<A::Message>,
    /// Futures that yield nothing that the scheduler will manage and poll for the actor.
    tasks: FuturesCollection<()>,
    /// The manager for outbound messages that will be broadcast from the actor.
    outbound: Option<OutboundQueue<A::Output>>,
    /// Stores edges in the form of `EdgeType`s. This is used to access connections to other actors
    /// at runtime without needing to embed them into the actor state directly.
    edges: SendableAnyMap,
    /// The number of stream that could yield a message for the actor to process. Once this and the
    /// `future_count` hit both reach zero, the actor is dead as it can no longer process any
    /// messages.
    stream_count: usize,
    /// The number of futures that could yield a message for the actor to process. Once this and
    /// the `stream_count` hit both reach zero, the actor is dead as it can no longer process any
    /// messages.
    future_count: usize,
    /// Tracks the status of the scheduler, mostly used to track how the state wants to shutdown.
    status: SchedulerStatus,
}

struct OutboundQueue<M> {
    send: broadcast::Sender<SendableWrapper<M>>,
}

impl<M: Sendable + Clone> OutboundQueue<M> {
    fn new(send: broadcast::Sender<SendableWrapper<M>>) -> Self {
        Self { send }
    }

    fn send(&mut self, msg: M) {
        let _ = self.send.send(SendableWrapper::new(msg));
    }
}

/// The container for the actor state and its scheduler. The runner polls the scheduler, aids in
/// bookkeeping if the actor is dead or not, and passes messages off to the state.
pub(crate) struct ActorRunner<A: ActorState> {
    state: A,
    scheduler: Scheduler<A>,
}

impl<A: ActorState> ActorRunner<A> {
    pub(crate) fn new(state: A, edges: SendableAnyMap) -> Self {
        let scheduler = Scheduler::new(edges);
        Self { scheduler, state }
    }

    pub(crate) fn add_broadcaster(&mut self, broad: broadcast::Sender<SendableWrapper<A::Output>>) {
        self.scheduler.outbound = Some(OutboundQueue::new(broad));
    }

    pub(crate) fn add_stream(&mut self, stream: ActorStream<A::Message>) {
        self.scheduler.attach_stream_inner(stream);
    }

    pub(crate) fn launch(self) {
        spawn_task(self.run())
    }

    async fn run(mut self) {
        self.state.start_up(&mut self.scheduler).await;
        loop {
            match self.scheduler.next().await {
                Some(msg) => self.state.process(&mut self.scheduler, msg).await,
                None => return self.close().await,
            }
        }
    }

    /// Closes the actor state.
    async fn close(self) {
        let Self {
            state,
            mut scheduler,
            ..
        } = self;
        state.finalize(&mut scheduler).await;
        scheduler.finalize();
    }
}

impl<A: ActorState> Scheduler<A> {
    /// The constructor for the scheduler.
    fn new(edges: SendableAnyMap) -> Self {
        let recv = select_all([]);
        let queue = FuturesCollection::new();
        let tasks = FuturesCollection::new();
        Self {
            recv,
            queue,
            tasks,
            edges,
            outbound: None,
            stream_count: 0,
            future_count: 0,
            status: SchedulerStatus::Alive,
        }
    }

    /// Returns if the actor is dead and should be dropped.
    fn is_dead(&self) -> bool {
        self.stream_count + self.future_count == 0
            || matches!(
                self.status,
                SchedulerStatus::Marked | SchedulerStatus::MarkedToFinish
            )
    }

    /// Performs that final actions before closing the actor process.
    fn finalize(self) {
        if matches!(
            self.status,
            SchedulerStatus::Alive | SchedulerStatus::MarkedToFinish
        ) {
            spawn_task(poll_to_completion(self.tasks))
        }
    }

    /// Yields the next message to be processed by the actor state.
    async fn next(&mut self) -> Option<A::Message> {
        loop {
            if self.is_dead() {
                return None;
            }
            tokio::select! {
                msg = self.recv.next(), if self.stream_count != 0 => {
                    match msg {
                        Some(msg) => return Some(msg),
                        None => {
                            self.stream_count -= 1;
                        }
                    }
                },
                msg = self.queue.next(), if self.future_count != 0 => {
                    self.future_count -= 1;
                    return msg
                },
                _ = self.tasks.next(), if !self.tasks.is_empty() => {},
                else => {
                    return None
                }
            }
        }
    }

    /// Queues a future in the scheduler that it will manage and poll. The output from the future
    /// must be convertible into a message for the actor to process. If you'd like the scheduler to
    /// just manage and poll the future, see the [`manage_future`](Scheduler::manage_future)
    /// method.
    ///
    /// Note: There is no ordering to the collection of futures. Moreover, there is no ordering
    /// between the queued futures and attached streams. The first to yield an item is the first to
    /// be processed. For this reason, the futures queued this way must be `'static`, i.e. they
    /// can't reference the actor's state.
    pub fn queue_task<F, I>(&mut self, fut: F)
    where
        F: Sendable + Future<Output = I>,
        I: 'static + Into<A::Message>,
    {
        self.future_count += 1;
        self.queue.push(Box::pin(fut.map(Into::into)));
    }

    /// Adds the given future to an internal queue of futures that the scheduler will manage;
    /// however, anything that the future yields will be dropped immediately. If the item yielded
    /// by the future can be turned into a message for the actor and you would the actor to process
    /// it, see the [`queue_task`](Scheduler::queue_task) method.
    ///
    /// Note: Managed futures are polled at the same time as the queued futures that yield messages
    /// and the attached streams. For this reason, the futures managed this way must be `'static`,
    /// i.e. they can't reference the actor's state.
    pub fn manage_future<F, T>(&mut self, fut: F)
    where
        F: Sendable + Future<Output = T>,
        T: Sendable,
    {
        self.tasks.push(Box::pin(fut.map(drop)));
    }

    /// Attaches a stream that will be polled and managed by the scheduler. Messages yielded by the
    /// streams must be able to be converted into the actor's message type so that the actor can
    /// process it. The given stream must be a [`FusedStream`]; however, the scheduler requires a
    /// stronger invariant than that given by `FusedStream`. The scheduler will mark a stream as
    /// "done" once the stream yields its first `None`. After that, the scheduler will never poll
    /// that stream again.
    pub fn attach_stream<S, I>(&mut self, stream: S)
    where
        S: SendableStream<Item = I> + FusedStream,
        I: Into<A::Message>,
    {
        let stream = ActorStream::Secondary(Box::new(stream.map(|m| m.into())));
        self.attach_stream_inner(stream)
    }

    /// Adds an actor stream to the scheduler.
    fn attach_stream_inner(&mut self, stream: ActorStream<A::Message>) {
        self.stream_count += 1;
        self.recv.push(stream.fuse());
    }

    /// Schedules a message to be given to the actor to process at a given time.
    pub fn schedule<M>(&mut self, deadline: Instant, msg: M)
    where
        M: Sendable + Into<A::Message>,
    {
        self.future_count += 1;
        self.queue.push(Box::pin(Timer::new(deadline, msg.into())));
    }

    /// Broadcasts a message to all listening clients. If the message fails to send, the message
    /// will be dropped.
    ///
    /// Note: This method does nothing if the actor is a [`SinkActor`](crate::sink::SinkActor).
    /// [`StreamActor`](crate::stream::StreamActor)s and [`JointActor`](crate::joint::JointActor)
    /// will be able to broadcast.
    pub fn broadcast<M>(&mut self, msg: M)
    where
        M: Into<A::Output>,
    {
        if let Some(out) = self.outbound.as_mut() {
            out.send(msg.into())
        }
    }

    /// Adds a client to the set of connections to other actors, which the state can access later.
    pub fn add_edge<P: 'static + Send, M: 'static + Send>(&mut self, client: SinkClient<P, M>) {
        _ = self.edges.insert(client);
    }

    /// Gets a reference to a sink client to another actor.
    pub fn get_edge<P: 'static + Send, M: 'static + Send>(&self) -> Option<&SinkClient<P, M>> {
        self.edges.get::<SinkClient<P, M>>()
    }

    /// Gets mutable reference to a sink client to another actor.
    pub fn get_edge_mut<P: 'static + Send, M: 'static + Send>(
        &mut self,
    ) -> Option<&mut SinkClient<P, M>> {
        self.edges.get_mut::<SinkClient<P, M>>()
    }

    /// Removes a client to the set of connections to other actors.
    pub fn remove_edge<P: 'static + Send, M: 'static + Send>(&mut self) {
        _ = self.edges.remove::<SinkClient<P, M>>();
    }

    /// Adds an arbitrary data to the set of connections to other actors, which the state can
    /// access later. This method is intended to be used with containers hold that multiple clients
    /// of the same type.
    ///
    /// For example, you can attach a series of actor clients that are indexed using a hashmap.
    pub fn add_multi_edge<C: 'static + Send>(&mut self, container: C) {
        _ = self.edges.insert(container);
    }

    /// Gets a reference to an arbitary type held in the container that holds connections to other
    /// actors. This method is intended to be used with containers that hold multiple clients of
    /// the same type.
    ///
    /// For example, you can store and access a series of actor clients that are indexed using a
    /// hashmap.
    pub fn get_multi_edge<C: 'static + Send>(&self) -> Option<&C> {
        self.edges.get::<C>()
    }

    /// Gets a mutable reference to an arbitary type held in the container that holds connections to other
    /// actors. This method is intended to be used with containers that hold multiple clients of
    /// the same type.
    pub fn get_multi_edge_mut<C: 'static + Send>(&mut self) -> Option<&mut C> {
        self.edges.get_mut::<C>()
    }

    /// Adds a piece of arbitrary data from the set of connections to other actors.
    pub fn remove_multi_edge<C: 'static + Send>(&mut self) {
        _ = self.edges.remove::<C>();
    }
}

impl<A> Scheduler<A>
where
    A: ActorState<Permanence = Transient>,
{
    /// Marks the actor as ready to shutdown. After the state finishes processing the current
    /// message, actor process will shutdown. Any unprocessed messages will be dropped, all
    /// attached streams will be closed, all futures that will yield a message will be cancelled,
    /// and all managed futures will dropped. If you would like the managed (non-message) futures
    /// to be processed, use the [`shutdown_and_finish`](Scheduler::shutdown_and_finish) method
    /// instead.
    pub fn shutdown(&mut self) {
        self.status = SchedulerStatus::Marked;
    }

    /// Marks the actor as ready to shutdown. After the state finishes processing the current
    /// message, it will shutdown the actor processes. Any unprocessed messages will be dropped,
    /// all attached streams will be closed, all futures that will yield a message will be
    /// cancelled, but all managed futures will be polled to completion in a new async process. If
    /// you would like for the managed futures and streams to be dropped instead, use the
    /// [`shutdown`](Scheduler::shutdown) method.
    pub fn shutdown_and_finish(&mut self) {
        self.status = SchedulerStatus::MarkedToFinish;
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

/// A simple function to poll a stream until it is closed. Largely used when the scheduler closes.
async fn poll_to_completion<S>(mut stream: S)
where
    S: SendableFusedStream,
{
    loop {
        if stream.next().await.is_none() {
            return;
        }
    }
}

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
