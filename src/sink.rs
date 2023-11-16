use std::marker::PhantomData;

use tokio::sync::mpsc::UnboundedSender;

use crate::{oneshot_channel, OneshotSender, Permanent, Transient};

/// A marker type used by the [`ActorBuilder`](crate::ActorBuilder) to know what kind of
/// [`ActorState`](crate::ActorState) it is dealing with. A sink actor is one that receives
/// messages from other parts of the application. By adding a oneshot channel to the message,
/// the actor can respond with a particular piece of data. This allows for type-safe communication
/// between different parts of your application.
///
/// The client of a [`SinkActor`] is the [`SinkClient`]. This client implements methods that allow
/// for the sending of messages to this client. Communication between a sink client and sink actor
/// can be roughly modelled with an MPSC-style channel (see [`mpsc::channel`](tokio::sync::mpsc)).
#[derive(Debug)]
pub struct SinkActor;

/// A client to an actor. This client sends messages to the actor and supports two styles of
/// messaging. The first is fire-and-forget messages. These messages are sent to the client
/// immediately (no `.await` needed). The actor will process them eventually. The second kind is
/// request-response or "trackable" messages. These messages are identical to the last kind except
/// they contain a one-time use channel that the actor will use to send a message back.
#[derive(Debug)]
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

    /// Returns if the actor that the client is connected to is dead or not.
    pub fn is_closed(&self) -> bool {
        self.send.is_closed()
    }

    /// Sends a fire-and-forget style message to the actor and returns if the message was sent
    /// successfully.
    pub fn send(&self, msg: impl Into<M>) -> bool {
        self.send.send(msg.into()).is_ok()
    }
}

impl<M> SinkClient<Permanent, M> {
    /// Sends a request-response style message to a [`Permanent`] actor. The given data is paired
    /// with a one-time use channel and sent to the actor. A [`Tracker`](permanent::Tracker) that
    /// will receive a response from the actor is returned.
    ///
    /// Note: Since this client is one for a permanent actor, there is an implicit unwrap once the
    /// tracker receives a message from the actor. If the actor drops the other half of the channel
    /// or has died somehow (likely from a panic), the returned tracker will panic too.
    pub fn track<I, O>(&self, msg: I) -> permanent::Tracker<O>
    where
        M: From<(I, OneshotSender<O>)>,
    {
        let (send, recv) = oneshot_channel();
        let msg = M::from((msg, send));
        let _ = self.send(msg);
        permanent::Tracker::new(recv)
    }
}

impl<M> SinkClient<Transient, M> {
    /// Sends a request-response style message to a [`Transient`] actor. The given data is paired
    /// with a one-time use channel and sent to the actor. A [`Tracker`](transient::Tracker) that
    /// will receive a response from the actor is returned.
    pub fn track<I, O>(&self, msg: I) -> transient::Tracker<O>
    where
        M: From<(I, OneshotSender<O>)>,
    {
        let (send, recv) = oneshot_channel();
        let msg = M::from((msg, send));
        let _ = self.send(msg);
        transient::Tracker::new(recv)
    }
}

impl<T, M> Clone for SinkClient<T, M> {
    fn clone(&self) -> Self {
        Self::new(self.send.clone())
    }
}

/// A module for things used to interact with the [`Permanent`] actors.
pub mod permanent {
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    use crate::OneshotReceiver;

    /// A tracker for a request-response style message sent to a [`Permanent`](crate::Permanent) actor.
    ///
    /// Note: This tracker implicitly unwraps the message produced by its channel receiver. If the
    /// actor drops the other half of the channel or has died somehow (likely from a panic), this
    /// tracker will panic when polled.
    #[derive(Debug)]
    pub struct Tracker<T> {
        recv: OneshotReceiver<T>,
    }

    impl<T> Tracker<T> {
        /// A constructor for the tracker.
        pub(crate) fn new(recv: OneshotReceiver<T>) -> Self {
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

/// A module for things used to interact with the [`Transient`] actors.
pub mod transient {
    use std::{
        fmt::Debug,
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    use crate::OneshotReceiver;

    /// A tracker for a request-response style message sent to a [`Transient`](crate::Transient) actor.
    ///
    /// Note: This tracker might be created after a failed attempt to send a message to a dead
    /// actor. This means that the tracker will return `None` when polled; however, that does not
    /// mean that the message was successfully received by the actor.
    #[derive(Debug)]
    pub struct Tracker<T> {
        recv: OneshotReceiver<T>,
    }

    impl<T> Tracker<T> {
        /// A constuctor for the tracker.
        pub(crate) fn new(recv: OneshotReceiver<T>) -> Self {
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
