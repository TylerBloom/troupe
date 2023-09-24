use futures::future::Ready;
use futures::StreamExt;
use futures::{stream::FuturesUnordered, FutureExt};
pub use std::task::{Context, Poll};
use tokio::sync::mpsc;

pub trait ActorState {
    type Message: Send;
}

pub struct Actor<T, A, F, G>
where
    T: ActorState,
{
    state: T,
    receiver: mpsc::UnboundedReceiver<T::Message>,
    action: A,
    futures: Vec<(F, G)>,
}

impl<T, A, F, G, Fut, Gut> Actor<T, A, F, G>
where
    T: Send + ActorState,
    A: Send + FnMut(&mut T, T::Message),
    F: Send + Fn(&T) -> Fut,
    G: Send + Fn(&T) -> Gut,
    Fut: std::future::Future<Output = ()> + Send,
    Gut: std::future::Future<Output = ()> + Send,
{
    pub fn builder(state: T, action: A) -> ActorBuilder<T, A, F, G, Fut, Gut> {
        ActorBuilder::new(state, action)
    }

    pub async fn perform(mut self) -> ! {
        fn mapped_index(x: usize) -> impl FnOnce(()) -> Ready<usize> {
            move |()| futures::future::ready(x)
        }
        // `futures` is a vector of user-defined objects that will generate a `Future`
        // that we can await. This makes the actor "alive" in some sense.

        // call each future that was provided
        let mut futs_unordered: FuturesUnordered<_> = self
            .futures
            .iter()
            .enumerate()
            .map(|(index, (fut, _))| (fut)(&self.state).map(mapped_index(index)))
            .collect();
        loop {
            tokio::select! {
                Some(msg) = self.receiver.recv() => {
                    println!("received!");
                    (self.action)(&mut self.state, msg);
                },
                Some(x) = futs_unordered.next() =>
                    {
                        let index = x.into_inner();
                        let (old_fut, then) = &self.futures[index];
                        (then)(&self.state).await;
                        let new_fut = (old_fut)(&self.state).map(mapped_index(index));
                        futs_unordered.push(new_fut);
                    },
            }
        }
    }
}

pub struct ActorBuilder<T, A, F, G, Fut, Gut>
where
    T: ActorState,
    F: Fn(&T) -> Fut,
    G: Fn(&T) -> Gut,
    Fut: std::future::Future + Send,
    Gut: std::future::Future + Send,
{
    state: T,
    action: A,
    futures: Vec<(F, G)>,
    sender: mpsc::UnboundedSender<T::Message>,
    receiver: mpsc::UnboundedReceiver<T::Message>,
}

impl<T, A, F, G, Fut, Gut> ActorBuilder<T, A, F, G, Fut, Gut>
where
    T: Send + ActorState,
    A: Send + FnMut(&mut T, T::Message),
    F: Send + Fn(&T) -> Fut,
    G: Send + Fn(&T) -> Gut,
    Fut: std::future::Future + Send,
    Gut: std::future::Future + Send,
{
    pub fn new(state: T, action: A) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            state,
            sender,
            receiver,
            action,
            futures: vec![],
        }
    }

    pub fn chain_behavior(mut self, when: F, then: G) -> Self {
        self.futures.push((when, then));
        self
    }

    pub fn build(self) -> (mpsc::UnboundedSender<T::Message>, Actor<T, A, F, G>) {
        let actor = Actor {
            state: self.state,
            receiver: self.receiver,
            action: self.action,
            futures: self.futures,
        };
        (self.sender, actor)
    }
}
