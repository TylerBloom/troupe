use async_trait::async_trait;
use futures::FutureExt;

use crate::{ActorState, Scheduler};

/// This actor wraps another actor and provides panic catching.
/// Should the actor state panic during any of its methods, the panic is caught, and the actor does
/// not die.
///
/// Since the future returned by the inner state's actor method might not be [`UnwindSafe`], any
/// panic will lead to the old state being dropped and a new one being created.
/// As such, a way to create a new state is also required.
///
/// Note: Panics are not caught for the `finalize` actor method.
pub struct PanicCatcher<A, F> {
    // The only time this is ever `None` is when it is processing a message.
    inner: Option<A>,
    create: F,
}

impl<A: Default, F> PanicCatcher<A, F> {
    pub fn new(state: A) -> PanicCatcher<A, F> {
        #[allow(unreachable_code)]
        Self {
            inner: Some(state),
            // Use the default method of A
            create: todo!(),
        }
    }
}

#[async_trait]
impl<A, F> ActorState for PanicCatcher<A, F>
where
    A: ActorState,
    F: 'static + Send + FnMut() -> A,
{
    type ActorType = A::ActorType;
    type Permanence = A::Permanence;
    type Message = A::Message;
    type Output = A::Output;

    async fn start_up(&mut self, scheduler: &mut Scheduler<Self::Message, Self::Output>) {
        let state = self.inner.take().unwrap();
        let inner = async move {
            state.start_up(scheduler).await;
            state
        }
        .catch_unwind()
        .await
        .unwrap_or_else(|_| (self.create)());
        self.inner.insert(inner);
    }

    async fn process(&mut self, scheduler: &mut Scheduler<Self::Message, Self::Output>, msg: Self::Message) {
        todo!()
    }
}
