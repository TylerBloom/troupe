[![Crates.io](https://img.shields.io/crates/v/troupe.svg)](https://crates.io/crates/troupe)
[![Documentation](https://docs.rs/troupe/badge.svg)](https://docs.rs/troupe/)
![GitHub Workflows](https://github.com/TylerBloom/troupe/actions/workflows/ci.yml/badge.svg)
[![Coverage Status](https://codecov.io/gh/TylerBloom/troupe/branch/main/graph/badge.svg)](https://codecov.io/gh/TylerBloom/troupe)
![Maintenance](https://img.shields.io/badge/Maintenance-Actively%20Developed-brightgreen.svg)

# About
Troupe is an experimental, high-level library for modeling your application using the [actor model](https://en.wikipedia.org/wiki/Actor_model).
Actors model concurrent mutable state via isolated components and message passing.
This approach enables you to both have clear seperation of concerns between different components of your program as well as plainly model how a component can change.
An actor approach is strongest when:
  - there are multiple independent sources of mutation in your application
  - you want to model the data flow through part or all of your system
  - you want to want to build your system from a series of distinct components that can evolve independently

Troupe was born while building full-stack Rust apps.
There, the frontend client authenticates, opens a websocket connection, pulls data from the server, and presents a UI to the user.
In the single-threaded envinorment of WASM, it is difficult to manage all of the sources of mutation: UI interactions, updates from the server, etc.
There, actors can be used to manage the websocket connection, process data from the server, and propagate it to the rest of the app (including non-actor parts like the UI).
The UI can then communicate with the actors to present the UI and process interactions.
On the server side, actors can be used to manage websockets, buffer communicate between the database and cached data, and track changes in a user's session info.

To achieve isolation, actors in `troupe` are contained within an async processes of existing async runtimes.
Because of this, `troupe` is able to support several async runtimes, including `tokio`, `async-std`, and `wasm-bindgen-futures` (for WASM targets), which allows you to slowly incorporate it into your project's architecture.
Do note that actor communication uses `tokio`'s channels regardless of the async runtime.

# Model
The heart of a `troupe` actor is a state type and a message type.
Let's take a simple cache as an example:
```rust
pub struct Cache {
    inner: HashMap<Uuid, YourData>,
}

pub enum CacheCommand {
    Insert(Uuid, YourData),
    Get(Uuid, OneshotSender<Option<YourData>>),
    Delete(Uuid),
}
```

The message type encapsulates how the state can change and what data the actor has to process. These messages are sent from various source to be processed by the state. To finish the actor, `Cache` just has to implement the `ActorState` trait.
```rust
impl ActorState for Cache {
    type ActorKind = SinkActor;
    type Message = CacheCommand;

    async fn process(&mut self, _: &mut Scheduler<Self>, msg: CacheCommand) {
        match msg {
            CacheCommand::Insert(key, val) => {
                self.inner.insert(key, val);
            }
            CacheCommand::Get(key, send) => {
                let _ = send.send(self.inner.get(&key).cloned());
            }
            CacheCommand::Delete(key) => {
                self.inner.remove(&key);
            }
        }
    }
}
```

Beyond the message type, `ActorState` has one other associated type: its `ActorKind`.
Where the state describes how the actor reacts to messages, the kind describes how messages get into and out of the actor.
`troupe` ships three kinds: `SinkActor` for actors that only receive messages, `StreamActor` for actors that only broadcast them, and `JointActor` for actors that do both.

The kind is not just a label.
When the actor is launched, its kind is constructed, and that construction does three things.
It builds the client that is handed back to the caller, it attaches whatever inbound streams that client needs (such as the MPSC channel behind a `SinkClient`), and it holds whatever state the actor needs in order to send messages back out (such as the broadcast channel behind a `StreamClient`).
That last piece is stored in the scheduler, which derefs to the kind, so a stream or joint actor broadcasts by simply calling `scheduler.broadcast(msg)`.

A kind can also require configuration via its associated `Config` type, which is handed to the builder with `ActorBuilder::config` before launching.
All three of the kinds above use `()` here, so they can be launched directly.

Because `ActorKind` is a public trait, other message-passing patterns can be modelled without changing the scheduler or the builder.
Clients with built-in back pressure or an SPSC-style client, for example, are each just a new kind and its client type.

Once running, `troupe` pairs every actor state with a scheduler.
The scheduler is responsible for managing futures that the state queues and attached streams of message.
The queued futures will be polled at the same time that the scheduler waits for inbound messages.
Sink and joint actors start out with one attached stream, the channel used to communicate between the client and the actor, which their `ActorKind` attaches at launch.
Client message streams use tokio MPSC-style channels, but actors can add any stream that yield messages that can be converted into the actor's message type (such as socket connections), either up front with `ActorBuilder::attach_stream` or at any point afterwards with `Scheduler::attach_stream`.
A stream actor has no client-backed channel, so all of its messages come from streams and futures added this way.
The scheduler also holds the actor's kind and derefs to it, which is how a state reaches methods like `broadcast`.
Conceptually, the scheduler-actor relationship can be model as:
```
 _____________________________________________
|                    Actor                    |
|  ___________                   ___________  |
| | Scheduler | --- message --> |   State   | |
| | [streams] | <-- streams --- |           | |
| | [futures] | <-- futures --- |           | |
| |___________|                 |___________| |
|_____________________________________________|
```

As stated before, every actor returns a client.
Each actor defines how its clients behave.
There are three types of clients: `SinkClient`, `StreamClient`, and `JointClient`.

A `SinkClient` is the type of client you are most likely to use.
It enables you to send messages directly to the actor.
These messages can be either "fire-and-forget" or "request-response" style messages.
Note that a `SinkClient` does not actually implement the [`Sink`](https://docs.rs/futures/latest/futures/sink/trait.Sink.html) trait, but, rather, serves the same conceptual purpose.
A `StreamClient` listens for messages that are broadcast from an actor.
Unlike a `SinkClient`, a `StreamClient` does not directly support any type of message passing into the actor.
It does, however, implement the [`Stream`](https://docs.rs/futures/latest/futures/stream/trait.Stream.html) trait.
Lastly, a `JointClient` is both a `SinkClient` and a `StreamClient` put together.
A `JointClient` can be decomposed into one or both of the other clients.
Note, the actor's kind defines what kind of client is constructed, so you can not, for example, get a `StreamClient` for an actor that will never broadcast a message.

Actors do not run forever.
An actor can retire itself at any point by calling `Scheduler::shutdown` (or `Scheduler::shutdown_and_finish`, which lets the futures the scheduler is managing for it run to completion first).
Because of this, the response to a request-response style message is not guaranteed to arrive, so awaiting a `Tracker` yields an `Option`.

Actors might also exhaust their source of messages.
This happens when all streams have ran dry and no message-yeilding futures are queued.
When this happens, there is built-in "garbage collection" for troupe actors.
The scheduler will mark an actor as "dead" and then shutdown the actor process if it ever reaches this state.
For `SinkActors`, this can only occur if all message-sending clients have been dropped.

# Backwards Compatibility
Troupe is currently experimental and subject to potential breaking changes (with due consideration).
Breaking changes might occur to improve API ergonomics, to tweak the actor model, or to use new Rust language features.
In particular, there are still language features that would improve this crate upon stabilization:
 - [async fn in traits with `Send` bounds](https://rust-lang.github.io/rfcs/3654-return-type-notation.html)
 - [associated-type defaults](https://rust-lang.github.io/rfcs/2532-associated-type-defaults.html)

`ActorState`'s methods are written as `-> impl MaybeSendFuture<Output = _>` rather than as plain `async fn` so that the `Send` bound can be dropped on WASM targets.
A stable way to write a maybe-`Send` `async fn` in a trait would let that workaround, and the `Sendable` bound it carries, be removed.
Associated-type defaults would let `ActorKind::Config` default to `()`, so kinds that need no configuration would not have to spell it out.

# Future Work
Currently, `troupe` only provides a framework for building actors.
If there are certain patterns that are commonplace, a general version of that pattern might find its way into this crate or a similar crate.
One such example is something like a local-first buffered state.
Here, you have some data that exists in two locations, say client and server, and you want update the state in one location then propagate those changes elsewhere.

Another possible actor pattern in a "poll" actor.
This would build upon a broadcast actor, but the broadcast message would contain a oneshot channel for communicating a "vote" with the actor.


# Usage and Licensing
This project is currently licensed under a LGPL-2.1 license.
This allows you to use this crate to build other crates (library or application) under any license (open-source or otherwise).
The primary intent of using this license is to require crates that modify to source of this crate to also be open-source licensed (LGPL or stronger).

# Contributing
If there is an aspect of the project that you wish to see changed, improved, or even removed, open a ticket or PR.
If you have an interesting design pattern that uses actors and would like to see if in this crate, open a ticket!!
