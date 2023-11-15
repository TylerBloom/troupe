use std::marker::PhantomData;

use tokio::sync::mpsc::UnboundedSender;

use crate::{oneshot_channel, ActorBuilder, ActorState, OneshotSender, Transient, Permanent};

/// A marker type used by the [`ActorBuilder`] to know what kind of [`ActorState`] it is dealing
/// with. A sink actor is one that receives messages from other parts of the application. By adding
/// a [`OneshotChannel`] into the message, the actor can respond with a particular piece of data.
/// This allows for type-safe communication between different parts of your application.
///
/// The client of a [`SinkActor`] is the [`SinkClient`]. This client implements methods that allow
/// for the sending of messages to this client. Communication between a sink client and
/// sink actor can be roughly modelled with an MPSC-style channel (see [`mpsc::channel`]).
pub struct SinkActor;

pub struct SinkClient<T, M> {
    ty: PhantomData<T>,
    send: UnboundedSender<M>,
}

impl<T, M> SinkClient<T, M> {
    pub(crate) fn new(send: UnboundedSender<M>) -> Self {
        Self {
            send,
            ty: PhantomData,
        }
    }

    pub fn builder<A>(state: A) -> ActorBuilder<A>
    where
        A: ActorState<Message = M, ActorType = SinkActor>,
    {
        ActorBuilder::new(state)
    }

    pub fn send(&self, msg: impl Into<M>) -> bool {
        // This returns a result. It only errors when the connected actor panics. Should we "bubble
        // up" that panic?
        self.send.send(msg.into()).is_ok()
    }
}

impl<M> SinkClient<Permanent, M> {
    pub fn track<I, O>(&self, msg: I) -> permanent::Tracker<O>
    where
        M: From<(I, OneshotSender<O>)>,
    {
        let (send, recv) = oneshot_channel();
        let msg = M::from((msg, send));
        self.send(msg);
        permanent::Tracker::new(recv)
    }
}

impl<M> SinkClient<Transient, M> {
    pub fn track<I, O>(&self, msg: I) -> transient::Tracker<O>
    where
        M: From<(I, OneshotSender<O>)>,
    {
        let (send, recv) = oneshot_channel();
        let msg = M::from((msg, send));
        self.send(msg);
        transient::Tracker::new(recv)
    }
}

impl<T, M> Clone for SinkClient<T, M> {
    fn clone(&self) -> Self {
        Self::new(self.send.clone())
    }
}

pub mod permanent {
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    use crate::OneshotReceiver;

    pub struct Tracker<T> {
        recv: OneshotReceiver<T>,
    }

    impl<T> Tracker<T> {
        pub fn new(recv: OneshotReceiver<T>) -> Self {
            Self { recv }
        }
    }

    impl<T> Future for Tracker<T> {
        type Output = T;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            Pin::new(&mut self.recv).poll(cx).map(Result::unwrap)
        }
    }
}

pub mod transient {
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    use crate::OneshotReceiver;

    pub struct Tracker<T> {
        recv: OneshotReceiver<T>,
    }

    impl<T> Tracker<T> {
        pub fn new(recv: OneshotReceiver<T>) -> Self {
            Self { recv }
        }
    }

    impl<T> Future for Tracker<T> {
        type Output = Option<T>;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            Pin::new(&mut self.recv).poll(cx).map(Result::ok)
        }
    }
}
