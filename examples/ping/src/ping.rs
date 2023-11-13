use derive_more::{Display, From};
use troupe::{async_trait, scheduler::Scheduler, sink::SinkActor, ActorState, OneshotSender};

/// The wrapper for the hashmap that will be our actor.
#[derive(Default)]
pub struct Ping {
    /// The number of requests that the actor as received
    count: usize,
}

/// The type that the cache receives and processes. `derive_more`'s `From` macro is very handy
/// here.
#[derive(From, Display)]
pub enum PingCommand {
    /// Because of `derive_more`, `()` can be used to create this variant.
    Ping,
    #[display(fmt = "RequestAndRespond")]
    RequestAndRespond(OneshotSender<usize>),
}

#[async_trait]
impl ActorState for Ping {
    /// A message of type `T` must be `T: Into<PingCommand>` or `(T, OneshotSender<usize>):
    /// Into<PingCommand` in order to be sent to this actor.
    type Message = PingCommand;

    /// This actor is a [`SinkActor`] as it does not broadcast anything.
    type ActorType = SinkActor;

    /// Sink actors don't output anything, so we can use a unit here. Ideally, [`ActorState`] would
    /// have a type default for this.
    type Output = ();

    async fn process(&mut self, _: &mut Scheduler<Self>, msg: PingCommand) {
        println!("Ping message receive: {msg}");
        match msg {
            // Simply increment the counter
            PingCommand::Ping => self.count += 1,
            // Get the current count, increment, and send back the orignal count
            PingCommand::RequestAndRespond(send) => {
                let resp = self.count;
                self.count += 1;
                // We ignore the result returned by `send` because it means that the calls as
                // stopped listening and we can't do anything about it. You can also explicitly
                // drop the result.
                let _ = send.send(resp);
            }
        }
    }
}

impl From<((), OneshotSender<usize>)> for PingCommand {
    fn from(((), send): ((), OneshotSender<usize>)) -> Self {
        send.into()
    }
}
