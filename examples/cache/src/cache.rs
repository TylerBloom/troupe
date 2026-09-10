use std::{collections::HashMap, hash::Hash};

use derive_more::From;
use troupe::prelude::*;

/// The wrapper for the hashmap that will be our actor.
#[derive(Default)]
pub struct Cache<K, T> {
    inner: HashMap<K, T>,
}

/// The type that the cache receives and processes. `derive_more`'s `From` macro is very handy
/// here.
#[derive(From)]
pub enum CacheCommand<K, T> {
    Insert(K, T),
    Get(K, OneshotSender<Option<T>>),
    Delete(K),
    #[allow(clippy::type_complexity)]
    #[from(ignore)]
    Query(K, Box<dyn 'static + Send + FnOnce(Option<&T>)>),
}

impl<K, T> ActorState for Cache<K, T>
where
    K: 'static + Send + Hash + Eq,
    T: 'static + Send + Clone,
{
    /// This actor is a [`SinkActor`] as it does not broadcast anything. That kind gives us a
    /// [`SinkClient`] when the actor is launched.
    type ActorKind = SinkActor;

    type Message = CacheCommand<K, T>;

    async fn process(&mut self, _: &mut Scheduler<Self>, msg: CacheCommand<K, T>) {
        println!("Message received!!");
        match msg {
            CacheCommand::Insert(key, val) => {
                self.inner.insert(key, val);
            }
            // The OneshotSender returns a returns a Result since the Receiver might have been
            // dropped. If this happens, then the call no longer wants the result of the query
            CacheCommand::Get(key, send) => drop(send.send(self.inner.get(&key).cloned())),
            CacheCommand::Delete(key) => {
                self.inner.remove(&key);
            }
            CacheCommand::Query(key, query) => {
                query(self.inner.get(&key));
            }
        }
    }
}

/// In order to allow users to return arbitary types from inside the `process` method, we must box
/// the query function and send the data back to the user from within the boxed function.
impl<F, K, T, M> From<((K, F), OneshotSender<Option<M>>)> for CacheCommand<K, T>
where
    F: 'static + Send + FnOnce(&T) -> M,
    M: 'static + Send,
    K: 'static + Send + Hash + Eq,
    T: 'static + Send + Clone,
{
    fn from(((key, query), send): ((K, F), OneshotSender<Option<M>>)) -> Self {
        let query = Box::new(move |item: Option<&T>| {
            let _ = send.send(item.map(query));
        });
        Self::Query(key, query)
    }
}
