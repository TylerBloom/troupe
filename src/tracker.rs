use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::sync::oneshot;

pub struct Tracker<T> {
    receiver: oneshot::Receiver<T>,
}

impl<T> Tracker<T> {
    pub fn new(receiver: oneshot::Receiver<T>) -> Self {
        Self { receiver }
    }
}

impl<T> Future for Tracker<T> {
    type Output = Result<T, oneshot::error::RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Pin::new(&mut this.receiver).poll(cx)
    }
}
