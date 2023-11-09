use futures::{
    stream::{select_all, FusedStream, FuturesUnordered, SelectAll},
    Stream, StreamExt,
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

use crate::{compat::spawn_task, ActorState};

type FuturesCollection<T> = FuturesUnordered<Pin<Box<dyn Future<Output = T>>>>;

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
    recv: SelectAll<ValvedStream<ActorStream<A::Message>>>,
    /// Futures that the actor has queued that will yield a message.
    queue: FuturesCollection<A::Message>,
    /// Futures that yield nothing that the scheduler will manage and poll for the actor.
    tasks: FuturesCollection<()>,
    // TODO:
    //  - Add a queue for timers so that they are not lumped in the `queue`.
    //    - Make those timers cancelable by assoicating each on with an id (usize)
    //  - Make all messages and streams abortable with `futures::stream::Abortable`
}

/// The container for the actor state and its scheduler. The runner polls the scheduler, aids in
/// bookkeeping if the actor is dead or not, and passes messages off to the state.
struct ActorRunner<A: ActorState> {
    state: A,
    scheduler: Scheduler<A>,
}

/// A stream wraps another stream and implements [`FusedStream`] but with a stronger guarantee.
/// Once the inner stream yields `None`, the stream closes and will never poll the inner stream
/// again. This is used for bookkeeping in the [`Scheduler`]. The scheduler needs to track the
/// number of active streams and futures in order to track when the actor is dead.
#[pin_project]
struct ValvedStream<S> {
    /// The inner stream.
    #[pin]
    inner: S,
    /// The tracker that marks the stream as done.
    done: bool,
}

impl<S> ValvedStream<S> {
    /// The constructor for the valved stream.
    fn new(inner: S) -> Self {
        Self { inner, done: false }
    }
}

impl<S> Stream for ValvedStream<S>
where
    S: Unpin + Stream,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Pending;
        }
        let digest = self.project().inner.poll_next(cx);
        if let Poll::Ready(None) = &digest {
            *self.project().done = true;
        }
        digest
    }
}

impl<S> FusedStream for ValvedStream<S>
where
    S: Unpin + Stream,
{
    fn is_terminated(&self) -> bool {
        self.done
    }
}

impl<A: ActorState> ActorRunner<A> {
    fn new(state: A) -> Self {
        let scheduler = Scheduler::new();
        Self { state, scheduler }
    }

    fn add_broadcaster(&mut self, _broad: broadcast::Sender<A::Output>) {
        todo!()
    }

    fn launch(self) {
        spawn_task(self.run())
    }

    async fn run(mut self) {
        self.state.start_up(&mut self.scheduler).await;
        loop {
            if self.scheduler.is_dead() {
                return;
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
        }
    }

    /// Returns if the actor is dead and should be dropped.
    fn is_dead(&self) -> bool {
        self.count == 0
    }

    /// Yields the next message to be processed by the actor state.
    async fn next(&self) -> A::Message {
        loop {
            tokio::select! {
                msg = self.recv.next() => {
                    match msg {
                        Some(msg) => return msg,
                        None => {
                            self.count -= 1;
                        }
                    }
                },
                msg = self.queue.next(), if !self.queue.is_empty() => {
                    self.count -= 1;
                    return msg;
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
        F: Future<Output = I>,
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
        F: Future<Output = ()>,
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
        S: FusedStream<Item = I>,
        I: Into<A::Message>,
    {
        self.recv
            .push(ActorStream::Secondary(Box::new(stream.map(|m| m.into()))));
    }

    /// This method is vary similar to [`add_stream`] with the key difference that the given stream
    /// does not need to implement [`FusedStream`]. To prevent the actor from potentially running
    /// forever, messages yielded from these streams will be unwrapped, crashing the actor.
    pub fn add_endless_stream<S, I>(&mut self, stream: S)
    where
        S: FusedStream<Item = I>,
        I: Into<A::Message>,
    {
        todo!()
    }

    pub fn schedule<M>(&mut self, deadline: Instant, msg: M)
    where
        M: 'static + Into<A::Message>,
    {
        //self.queue.push(Box::pin(Timer::new(deadline, msg.into())));
        todo!()
    }
}

impl<A: ActorState> From<UnboundedReceiver<A::Message>> for ActorStream<A> {
    fn from(value: UnboundedReceiver<A::Message>) -> Self {
        Self::Main(UnboundedReceiverStream::new(value))
    }
}

enum ActorStream<M> {
    Main(UnboundedReceiverStream<M>),
    Secondary(Box<dyn Unpin + Stream<Item = M>>),
}

impl<A: ActorState> Stream for ActorStream<A> {
    type Item = A::Message;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match *self {
            ActorStream::Main(ref mut stream) => Pin::new(stream).poll_next(cx),
            ActorStream::Secondary(ref mut stream) => Pin::new(stream).poll_next(cx),
        }
    }
}
