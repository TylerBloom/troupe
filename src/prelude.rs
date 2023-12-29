//! Re-exports of commonly used items.

pub use crate::{
    compat::SendableFuture,
    joint::{JointActor, JointClient},
    oneshot_channel,
    scheduler::Scheduler,
    sink::{SinkActor, SinkClient},
    stream::{StreamActor, StreamClient},
    ActorBuilder, ActorState, OneshotReceiver, OneshotSender, Permanent, Transient,
};
