use derive_more::{Display, From};
use troupe::prelude::*;

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

impl ActorState for Ping {
    /// This actor is a [`SinkActor`] as it does not broadcast anything. That kind gives us a
    /// [`SinkClient`] when the actor is launched.
    type ActorKind = SinkActor;

    /// A message of type `T` must be `T: Into<PingCommand>` or `(T, OneshotSender<usize>):
    /// Into<PingCommand` in order to be sent to this actor.
    type Message = PingCommand;

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
