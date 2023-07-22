//! This module contains an example implementation of an `Actor` that holds a `HashMap`.

use tokio::sync::oneshot::{
    channel as oneshot_channel, Receiver as OneshotReceiver, Sender as OneshotSender,
};

use std::collections::HashMap;

use crate::ActorState;

pub struct Cache(HashMap<usize, String>);

pub enum CacheMessage {
    Insert(OneshotSender<Option<String>>, usize, String),
    Query(usize, Box<dyn Send + FnOnce(&Cache)>),
}

impl ActorState for Cache {
    type Message = CacheMessage;
}

impl From<(OneshotSender<Option<String>>, (usize, String))> for CacheMessage {
    fn from((send, (id, data)): (OneshotSender<Option<String>>, (usize, String))) -> Self {
        CacheMessage::Insert(send, id, data)
    }
}

impl<F, T> From<(OneshotSender<T>, (usize, F))> for CacheMessage
where
    F: 'static + Send + FnOnce(&Cache) -> T,
    T: 'static + Send,
{
    fn from((send, (id, query)): (OneshotSender<T>, (usize, F))) -> Self {
        let query = Box::new(move |cache: &Cache| {
            send.send(query(cache));
        });
        CacheMessage::Query(id, query)
    }
}

pub fn cache_action(_cache: &mut Cache, _msg: CacheMessage) {
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::Client;

    use super::{Cache, cache_action};

    #[test]
    fn cache_usability() {
        let client = Client::new(Cache(HashMap::new()), cache_action);
        client.send_msg((0, "test".to_string()));
        client.send_msg((0, |cache: &Cache| cache.0.len()));
    }
}
