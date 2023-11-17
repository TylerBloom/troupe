use futures::StreamExt;
use instant::{Duration, Instant};
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

struct Counter {
    started: Option<OneshotSender<Started>>,
    completed: OneshotSender<Completed>,
    count: usize,
}

#[async_trait]
impl ActorState for Counter {
    type ActorType = StreamActor;
    type Permanence = Transient;
    type Message = ();
    type Output = usize;

    async fn start_up(&mut self, scheduler: &mut Scheduler<Self>) {
        self.started.take().unwrap().send(Started).unwrap();
        scheduler.schedule(Instant::now() + Duration::from_millis(5), ());
    }

    async fn process(&mut self, scheduler: &mut Scheduler<Self>, (): Self::Message) {
        println!("Processing message!!");
        let next = self.count + 1;
        scheduler.broadcast(std::mem::replace(&mut self.count, next));
        scheduler.schedule(Instant::now() + Duration::from_millis(5), ());
        if self.count > 10 {
            scheduler.shutdown();
        }
    }

    async fn finalize(self, _: &mut Scheduler<Self>) {
        self.completed.send(Completed).unwrap();
    }
}

#[tokio::test]
async fn stop_after_ten_messages() {
    let (started, mut started_recv) = oneshot_channel();
    let (completed, mut comped_recv) = oneshot_channel();
    let state = Counter {
        started: Some(started),
        completed,
        count: 0,
    };
    assert_eq!(Err(TryRecvError::Empty), started_recv.try_recv());
    assert_eq!(Err(TryRecvError::Empty), comped_recv.try_recv());
    let stream = futures::stream::iter(std::iter::empty()).fuse();
    let mut builder = ActorBuilder::new(state);
    let _client = builder.client();
    let mut client = builder.launch(stream);
    /* ----- Successful startup test ----- */
    tokio::select! {
        _ = sleep() => {
            panic!("Actor failed to start!!");
        }
        _ = started_recv => { }
    }
    /* ----- Processes stream test ----- */
    let mut counter = 0;
    while counter < 11 {
        let Some(Ok(num)) = client.next().await else {
            panic!("Actor closed prematurely!!")
        };
        println!("Broadcast received: {num}");
        assert_eq!(num, counter);
        counter += 1;
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
            panic!("Actor failed to close stream!!");
        }
        msg = client.next() => {
            assert_eq!(None, msg)
        }
    }
}
