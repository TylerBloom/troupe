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

struct DummyJoint {
    started: Option<OneshotSender<Started>>,
    completed: OneshotSender<Completed>,
}

impl ActorState for DummyJoint {
    type ActorType = JointActor;
    type Permanence = Permanent;
    type Message = Processed;
    type Output = Processed;

    fn start_up(&mut self, _: &mut Scheduler<Self>) -> impl SendableFuture<Output = ()> {
        self.started.take().unwrap().send(Started).unwrap();
        std::future::ready(())
    }

    fn process(
        &mut self,
        scheduler: &mut Scheduler<Self>,
        msg: Self::Message,
    ) -> impl SendableFuture<Output = ()> {
        scheduler.broadcast(msg);
        std::future::ready(())
    }

    fn finalize(self, _: &mut Scheduler<Self>) -> impl SendableFuture<Output = ()> {
        self.completed.send(Completed).unwrap();
        std::future::ready(())
    }
}

async fn sleep() {
    sleep_for(Duration::from_millis(10)).await
}

#[test]
fn are_send() {
    fn is_send<T: Send>() {}

    is_send::<DummyJoint>();
    is_send::<JointActor>();
    is_send::<Permanent>();
    is_send::<Started>();
    is_send::<Processed>();
    is_send::<Completed>();
}

#[tokio::test]
async fn startup_and_teardown() {
    let (started, mut started_recv) = oneshot_channel();
    let (completed, mut comped_recv) = oneshot_channel();
    let state = DummyJoint {
        started: Some(started),
        completed,
    };
    assert_eq!(Err(TryRecvError::Empty), started_recv.try_recv());
    assert_eq!(Err(TryRecvError::Empty), comped_recv.try_recv());
    let stream = futures::stream::iter(std::iter::once(Processed)).fuse();
    let mut client = ActorBuilder::new(state).launch_with_stream(stream);
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
            panic!("Actor failed to process message");
        }
        msg = client.next() => {
            assert_eq!(Some(Ok(Processed)), msg);
        }
    }
    tokio::select! {
        _ = &mut comped_recv => {
            panic!("Actor closed unexpectedly early!!");
        }
        _ = sleep() => { }
    }
    client.send(Processed);
    tokio::select! {
        _ = sleep() => {
            panic!("Actor failed to process message");
        }
        msg = client.next() => {
            assert_eq!(Some(Ok(Processed)), msg);
        }
    }
    /* ----- Successful shutdown test ----- */
    let (sink, mut stream) = client.split();
    drop(sink);
    tokio::select! {
        _ = sleep() => {
            panic!("Actor failed to shutdown!!");
        }
        _ = comped_recv => { }
    }
    tokio::select! {
        _ = sleep() => {
            panic!("Actor stream did not close");
        }
        msg = stream.next() => {
            assert_eq!(None, msg);
        }
    }
}
