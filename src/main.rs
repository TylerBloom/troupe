use tokio::sync::oneshot::Sender as OneshotSender;
use tokio::time;
use troupe::{
    actor::{ActorBuilder, ActorState},
    client::Client,
};

use std::{collections::HashMap, time::Duration};

#[tokio::main]
async fn main() {
    pub fn cache_action(_cache: &mut Cache, _msg: CacheMessage) {}

    async fn wait(millis: u64) {
        time::sleep(Duration::from_millis(millis)).await;
    }

    async fn then_do() {
        println!("it's been 5 seconds");
    }

    let actor_state = Cache(HashMap::new());
    let actor_builder =
        ActorBuilder::new(actor_state, cache_action).chain_behavior(|_| wait(5000), |_| then_do());
    let client = Client::new(actor_builder);
    client.send_msg((0, "test".to_string()));
    wait(6000).await;
    client.send_msg((0, |cache: &Cache| cache.0.len()));

    wait(10000).await;
    println!("it's been 10 seconds");
}

pub struct Cache(pub HashMap<usize, String>);

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
            let _ = send.send(query(cache));
        });
        CacheMessage::Query(id, query)
    }
}

pub fn cache_action(_cache: &mut Cache, _msg: CacheMessage) {}
