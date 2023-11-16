use std::{
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{ready, Stream};
use tokio::sync::broadcast::{self, error::RecvError};

use crate::compat::Sendable;

/// A marker type used by the [`ActorBuilder`](crate::ActorBuilder) to know what kind of
/// [`ActorState`](crate::ActorState) it is dealing with. A stream actor is one that receives
/// messages from one or streams and then forwards messages to its clients.
///
/// The client of a [`StreamActor`] is the [`StreamClient`]. This client implements methods for
/// receiving methods that are "forwarded" by the actor. Unlike the
/// [`SinkActor`](crate::sink::SinkActor), stream actors and clients don't directly support
/// request/response style communication. Communication between a stream actor and client(s) can be
/// modelled with a broadcast-style channel (see [`broadcast::channel`]).
#[derive(Debug)]
pub struct StreamActor;

/// A client that receives messages from an actor that broadcasts.
#[derive(Debug)]
pub struct StreamClient<M> {
    recv: BroadcastStream<M>,
}

/// Because of how broadcast streams are implemented in `tokio_streams`, we can not create a
/// broadcast stream from another broadcast stream. Because of this, we must track a second, inner
/// receiver.
struct BroadcastStream<M> {
    /// A copy of the original channel, used for cloning the client.
    copy: broadcast::Receiver<M>,
    /// The stream that is polled.
    fut: from_tokio::ReusableBoxFuture<(Result<M, RecvError>, broadcast::Receiver<M>)>,
}

impl<M> StreamClient<M>
where
    M: Sendable + Clone,
{
    pub(crate) fn new(recv: broadcast::Receiver<M>) -> Self {
        Self {
            recv: BroadcastStream::new(recv),
        }
    }
}

impl<M> BroadcastStream<M>
where
    M: Sendable + Clone,
{
    fn new(stream: broadcast::Receiver<M>) -> Self {
        let copy = stream.resubscribe();
        let fut = from_tokio::ReusableBoxFuture::new(make_future(stream));
        Self { copy, fut }
    }
}

async fn make_future<T: Clone>(
    mut rx: broadcast::Receiver<T>,
) -> (Result<T, RecvError>, broadcast::Receiver<T>) {
    let result = rx.recv().await;
    (result, rx)
}

impl<M> Clone for StreamClient<M>
where
    M: Sendable + Clone,
{
    fn clone(&self) -> Self {
        let recv = BroadcastStream::new(self.recv.copy.resubscribe());
        Self { recv }
    }
}

impl<M> Stream for StreamClient<M>
where
    M: Sendable + Clone,
{
    type Item = Result<M, u64>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().recv).poll_next(cx)
    }
}

impl<M> Stream for BroadcastStream<M>
where
    M: Sendable + Clone,
{
    type Item = Result<M, u64>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let (result, rx) = ready!(self.fut.poll(cx));
        self.fut.set(make_future(rx));
        match result {
            Ok(item) => Poll::Ready(Some(Ok(item))),
            Err(RecvError::Closed) => Poll::Ready(None),
            Err(RecvError::Lagged(n)) => Poll::Ready(Some(Err(n))),
        }
    }
}

impl<M> Debug for BroadcastStream<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BroadcastStream({:?})", self.copy)
    }
}

/// Everything in this module comes tokio_utils. Because of constraints caused by the `Sendable`
/// workaround, the broadcast stream can not use a BroadcastStream from tokio_streams. The source
/// of this issue is that it uses a boxed trait object internally that is marked as Send. This is a
/// direct copy of those ideas but with a `Sendable` bound instead of `'a + Send` on the trait
/// object.
mod from_tokio {
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use crate::compat::SendableFuture;

    pub(super) struct ReusableBoxFuture<M>(Pin<Box<dyn SendableFuture<Output = M>>>);

    impl<T> ReusableBoxFuture<T> {
        /// Create a new `ReusableBoxFuture<T>` containing the provided future.
        pub(super) fn new<F>(future: F) -> Self
        where
            F: SendableFuture<Output = T>,
        {
            Self(Box::pin(future))
        }

        /// Replace the future currently stored in this box.
        ///
        /// This reallocates if and only if the layout of the provided future is
        /// different from the layout of the currently stored future.
        pub(super) fn set<F>(&mut self, future: F)
        where
            F: SendableFuture<Output = T>,
        {
            if let Err(future) = self.try_set(future) {
                *self = Self::new(future);
            }
        }

        /// Replace the future currently stored in this box.
        ///
        /// This function never reallocates, but returns an error if the provided
        /// future has a different size or alignment from the currently stored
        /// future.
        fn try_set<F>(&mut self, future: F) -> Result<(), F>
        where
            F: SendableFuture<Output = T>,
        {
            // If we try to inline the contents of this function, the type checker complains because
            // the bound `T: 'a` is not satisfied in the call to `pending()`. But by putting it in an
            // inner function that doesn't have `T` as a generic parameter, we implicitly get the bound
            // `F::Output: 'a` transitively through `F: 'a`, allowing us to call `pending()`.
            #[inline(always)]
            fn real_try_set<F>(this: &mut ReusableBoxFuture<F::Output>, future: F) -> Result<(), F>
            where
                F: SendableFuture,
            {
                // future::Pending<T> is a ZST so this never allocates.
                let boxed = std::mem::replace(&mut this.0, Box::pin(std::future::pending()));
                reuse_pin_box(boxed, future, |boxed| this.0 = Pin::from(boxed))
            }

            real_try_set(self, future)
        }

        /// Get a pinned reference to the underlying future.
        fn get_pin(&mut self) -> Pin<&mut (dyn SendableFuture<Output = T>)> {
            self.0.as_mut()
        }

        /// Poll the future stored inside this box.
        pub(super) fn poll(&mut self, cx: &mut Context<'_>) -> Poll<T> {
            self.get_pin().poll(cx)
        }
    }

    #[allow(unsafe_code)]
    fn reuse_pin_box<T: ?Sized, U, O, F>(
        boxed: Pin<Box<T>>,
        new_value: U,
        callback: F,
    ) -> Result<O, U>
    where
        F: FnOnce(Box<U>) -> O,
    {
        use std::alloc::Layout;

        let layout = Layout::for_value::<T>(&*boxed);
        if layout != Layout::new::<U>() {
            return Err(new_value);
        }

        // SAFETY: We don't ever construct a non-pinned reference to the old `T` from now on, and we
        // always drop the `T`.
        let raw: *mut T = Box::into_raw(unsafe { Pin::into_inner_unchecked(boxed) });

        // When dropping the old value panics, we still want to call `callback` — so move the rest of
        // the code into a guard type.
        let guard = CallOnDrop::new(|| {
            let raw: *mut U = raw.cast::<U>();
            unsafe { raw.write(new_value) };

            // SAFETY:
            // - `T` and `U` have the same layout.
            // - `raw` comes from a `Box` that uses the same allocator as this one.
            // - `raw` points to a valid instance of `U` (we just wrote it in).
            let boxed = unsafe { Box::from_raw(raw) };

            callback(boxed)
        });

        // Drop the old value.
        unsafe { std::ptr::drop_in_place(raw) };

        // Run the rest of the code.
        Ok(guard.call())
    }

    struct CallOnDrop<O, F: FnOnce() -> O> {
        f: std::mem::ManuallyDrop<F>,
    }

    impl<O, F: FnOnce() -> O> CallOnDrop<O, F> {
        fn new(f: F) -> Self {
            let f = std::mem::ManuallyDrop::new(f);
            Self { f }
        }

        #[allow(unsafe_code)]
        fn call(self) -> O {
            let mut this = std::mem::ManuallyDrop::new(self);
            let f = unsafe { std::mem::ManuallyDrop::take(&mut this.f) };
            f()
        }
    }

    impl<O, F: FnOnce() -> O> Drop for CallOnDrop<O, F> {
        #[allow(unsafe_code, unused_results)]
        fn drop(&mut self) {
            let f = unsafe { std::mem::ManuallyDrop::take(&mut self.f) };
            f();
        }
    }
}
