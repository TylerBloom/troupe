#![allow(dead_code, unused)]
/*
    - The spawned task will process messages using a dev-defined function
    - The task will receive exactly one message type
    - The dev can define any number of tracker-message pairs
      - A tracker needs to be a Future and implement a Tracker trait
      - We need to be able to convert messages into task-bound message
      - Messages will have a trait (this is how we can create tracker-message pairs easily)
*/

use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot::{channel as oneshot_channel, Receiver as OneshotReceiver, Sender as OneshotSender},
};

pub mod map;

pub struct Client<T: ActorState> {
    send: UnboundedSender<T::Message>,
}

impl<T> Client<T>
where
    T: 'static + Send + ActorState,
{
    pub fn new<F>(state: T, action: F) -> Self
    where
        F: 'static + Send + FnMut(&mut T, <T as ActorState>::Message),
    {
        let (send, recv) = unbounded_channel();
        let actor = Actor {
            recv,
            state,
            action,
        };
        _ = tokio::spawn(actor.perform());
        Self { send }
    }

    pub fn send_msg<M, O>(&self, msg: M) -> Tracker<O>
    where
        (OneshotSender<O>, M): Into<T::Message>,
    {
        let (send, recv) = oneshot_channel();
        _ = self.send.send((send, msg).into());
        Tracker { recv }
    }
}

pub struct Tracker<T> {
    recv: OneshotReceiver<T>,
}

pub struct Actor<T, F>
where
    T: ActorState,
{
    recv: UnboundedReceiver<T::Message>,
    state: T,
    action: F,
}

impl<T, F> Actor<T, F>
where
    T: 'static + Send + ActorState,
    F: 'static + Send + FnMut(&mut T, T::Message),
{
    pub async fn perform(mut self) -> ! {
        loop {
            let msg = self.recv.recv().await.unwrap();
            (self.action)(&mut self.state, msg);
        }
    }
}

pub trait ActorState {
    type Message: Send;
}
