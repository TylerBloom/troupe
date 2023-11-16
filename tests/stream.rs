use futures::StreamExt;
use instant::Duration;
use tokio::sync::oneshot::error::TryRecvError;
use troupe::{compat::sleep_for, prelude::*};

#[derive(Debug, PartialEq, Eq)]
struct Started;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Processed;

#[derive(Debug, PartialEq, Eq)]
struct Completed;

struct DummyStream {
    started: Option<OneshotSender<Started>>,
    completed: OneshotSender<Completed>,
}

#[async_trait]
impl ActorState for DummyStream {
    type ActorType = StreamActor;
    type Permanence = Permanent;
    type Message = Processed;
    type Output = Processed;

    async fn start_up(&mut self, _: &mut Scheduler<Self>) {
        self.started.take().unwrap().send(Started).unwrap();
    }

    async fn process(&mut self, scheduler: &mut Scheduler<Self>, msg: Self::Message) {
        scheduler.broadcast(msg);
    }

    async fn finalize(self, _: &mut Scheduler<Self>) {
        self.completed.send(Completed).unwrap();
    }
}

async fn sleep() {
    sleep_for(Duration::from_millis(10)).await
}

#[tokio::test]
async fn startup_and_teardown() {
    let (started, mut started_recv) = oneshot_channel();
    let (completed, mut comped_recv) = oneshot_channel();
    let state = DummyStream {
        started: Some(started),
        completed,
    };
    assert_eq!(Err(TryRecvError::Empty), started_recv.try_recv());
    assert_eq!(Err(TryRecvError::Empty), comped_recv.try_recv());
    let stream = futures::stream::iter(std::iter::once(Processed)).fuse();
    let mut client = ActorBuilder::new(state).launch(stream);
    /* ----- Successful startup test ----- */
    tokio::select! {
        _ = sleep() => {
            panic!("Actor failed to start!!");
        }
        _ = started_recv => { }
    }
    /* ----- Processes stream test ----- */
    tokio::select! {
        _ = sleep() => {
            panic!("Actor failed to process message!!");
        }
        msg = client.next() => {
            assert_eq!(Some(Ok(Processed)), msg)
        }
    }
    /* ----- Successful shutdown test ----- */
    tokio::select! {
        _ = sleep() => {
            panic!("Actor failed to shutdown!!");
        }
        _ = comped_recv => { }
    }
    tokio::select! {
        _ = sleep() => {
            panic!("Actor failed to process message!!");
        }
        msg = client.next() => {
            assert_eq!(None, msg)
        }
    }
}
