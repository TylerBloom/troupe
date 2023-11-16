use instant::Duration;
use tokio::sync::oneshot::error::TryRecvError;
use troupe::{compat::sleep_for, prelude::*};

#[derive(Debug, PartialEq, Eq)]
struct Started;

#[derive(Debug, PartialEq, Eq)]
struct Completed;

struct DummySink {
    started: Option<OneshotSender<Started>>,
    completed: OneshotSender<Completed>,
}

#[async_trait]
impl ActorState for DummySink {
    type ActorType = SinkActor;
    type Permanence = Permanent;
    type Message = ();
    type Output = ();

    async fn start_up(&mut self, _: &mut Scheduler<Self>) {
        self.started.take().unwrap().send(Started).unwrap();
    }

    async fn process(&mut self, _: &mut Scheduler<Self>, _: Self::Message) {}

    async fn finalize(self, _: &mut Scheduler<Self>) {
        self.completed.send(Completed).unwrap();
    }
}

#[tokio::test]
async fn startup_and_teardown() {
    let (started, mut started_recv) = oneshot_channel();
    let (completed, mut comped_recv) = oneshot_channel();
    let state = DummySink {
        started: Some(started),
        completed,
    };
    assert_eq!(Err(TryRecvError::Empty), started_recv.try_recv());
    assert_eq!(Err(TryRecvError::Empty), comped_recv.try_recv());
    let client = ActorBuilder::new(state).launch();
    /* ----- Successful startup test ----- */
    tokio::select! {
        _ = sleep_for(Duration::from_millis(10)) => {
            panic!("Actor failed to start!!");
        }
        _ = started_recv => { }
    }
    /* ----- Stablized actor test ----- */
    tokio::select! {
        _ = &mut comped_recv => {
            panic!("Actor closed unexpectedly early!!");
        }
        _ = sleep_for(Duration::from_millis(10)) => { }
    }
    client.send(());
    /* ----- Successful shutdown test ----- */
    drop(client);
    tokio::select! {
        _ = sleep_for(Duration::from_millis(10)) => {
            panic!("Actor failed to shutdown!!");
        }
        _ = comped_recv => { }
    }
}
