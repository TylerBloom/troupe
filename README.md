[![Crates.io](https://img.shields.io/crates/v/troupe.svg)](https://crates.io/crates/troupe)
[![Documentation](https://docs.rs/troupe/badge.svg)](https://docs.rs/troupe/)
![GitHub Workflows](https://github.com/TylerBloom/troupe/actions/workflows/ci.yml/badge.svg)
[![Coverage Status](https://codecov.io/gh/TylerBloom/troupe/branch/main/graph/badge.svg)](https://codecov.io/gh/TylerBloom/troupe)
![Maintenance](https://img.shields.io/badge/Maintenance-Actively%20Developed-brightgreen.svg)

# About
Troupe is an experimental, high-level library for modeling your application using the [actor model](https://en.wikipedia.org/wiki/Actor_model).
Actors model concurrent mutable state via isolated components and message passing.
This approach enables you to have clear seperation of concerns between different components of your program as well as plainly model how a component can change.
An actor approach is strongest when:
  - there are multiple independent sources of mutation in your application
  - you want to model the data flow through part or all of your system
  - you want to want to build your system from a series of distinct components that can evolve independently

Troupe was born while building full-stack Rust apps. There, the frontend client authenticates, opens a websocket connection, pulls data from the server, and presents a UI to the user. In the single-threaded envinorment of WASM, it is difficult to manage all of the sources of mutation: UI interactions, updates from the server, etc. There, actors can be used to manage the websocket connection, process data from the server, and propagate it to the rest of the app (including non-actor parts like the UI). The UI can then communicate with the actors to present the UI and process interactions. On the server side, actors can be used to manage websockets, buffer communicate between the database and cached data, and track changes in a user's session info.

To achieve isolation, actors in `troupe` are contained within an async processes of existing async runtimes. Because of this, `troupe` is able to support several async runtimes, including `tokio`, `async-std`, and `wasm-bindgen-futures` (for WASM targets), which allows you to slowly incorporate it into your project's architecture. However, actor communication uses `tokio`'s channels regardless of the async runtime.

# Model
At their heart, actors in the `troupe` model are a state type and a message type.
Let's take a simple cache as an example:
```rust
pub struct Cache {
    inner: HashMap<Uuid, YourData>,
}

pub enum CacheCommand {
    Insert(Uuid, YourData),
    Get(Uuid, OneshotSender<Option<YourData>>),
    Delete(Uuid),
    Query(Uuid, Box<dyn 'static + Send + FnOnce(Option<&YourData>)>),
}
```

The message type encapsulates how the state can change and what data the actor has to process. These messages are sent from various source to be processed by the state. To finish the actor, `Cache` just has to implement the `ActorState` trait.
```rust
impl ActorState for Cache {
    type Message = CacheCommand;
    type ActorType = SinkActor;
    type Output = ();

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
            CacheCommand::Query(key, query) => {
                query(self.inner.get(&key));
            }
        }
    }
}
```

There are several other components at work here, such as `SinkActor`. Continue reading this or read the docs to get a complete understanding of how to use the full `troupe` toolkit to customize your actors. When the cache actor is launched, it returns a client, which allows you to send messages to it.

Once launched, `troupe` pairs every actor state with a scheduler. The scheduler is responsible for managing inbound streams of message, like those coming from the actor's clients. The scheduler also manages any futures that the actor queues for itself to process at a later time. Client message streams use tokio MPSC-style channels, but actors can add other inbound stream (such as socket connections). Conceptually, the scheduler-actor relationship can be model as:
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

As stated before, every actor returns a client. Each actor defines how its clients behave. There are three types of clients: `SinkClient`, `StreamClient`, and `JointClient`.

A `SinkClient` is the type of client you are most likely to use. It enables you to send messages directly to the actor. These messages can be either "fire-and-forget" or "request-response" style messages. Note that a `SinkClient` does not actually implement the [`Sink`](https://docs.rs/futures/latest/futures/sink/trait.Sink.html) trait, but, rather, serves the same conceptual purpose. A `StreamClient` listens for messages that are broadcast from an actor. Unlike a `SinkClient`, a `StreamClient` does not directly support any type of message passing into the actor. It does, however, implement the [`Stream`](https://docs.rs/futures/latest/futures/stream/trait.Stream.html) trait. Lastly, a `JointClient` is both a `SinkClient` and a `StreamClient` put together. A `JointClient` can be decomposed into one or both of the other clients. Note, the actor type defines what kind of clients can be constructed, so you can not, for example, construct a `StreamClient` for an actor that will never broadcast a message.

The last major component of the `troupe` actor model is permanence. Some actors are designed to run forever. These are called `Permanent` actors. Other actors are designed to run for some time (perhaps a long time) and then close. These are called `Transient` actors. This distinction largely serves to help with the ergonomics of interacting with actors.

Lastly, there is built-in "garbage collection" for troupe actors. The scheduler will mark an actor as "dead" and then close the actor process down if all of its held streams close and it is holding no futures that can yield future messages. This check occurs regardless of the user-defined permanence of the actor.

# Backwards Compatibility
Troupe is currently experimental and subject to potential breaking changes (with due consideration). Breaking changes might occur to improve API ergonomics, tweak the actor model, or use new Rust language features. In particular, there are several language features that will be used improve this crate upon stabilization:
 - [`async fn` in traits]
 - [specialization]
 - [associate-type defaults]

The stabilization of the first two present garunteed breaking changes for this crate but will drastically improve usability and ergonomics. Specialization will enable the `ActorBuilder` to present identically-named methods for launching the actor while returning the appropiate client type. The stabilization of `async fn` in traits will allow for the loosening of constraints on `ActorState`s in WASM contexts, allowing them to just be `'static` instead of `'static + Send`.

# Usage and Licensing
This project is currently licensed under a LGPL license. The intent of this is for modifications to this library. For forks or other modifications to this library, those versions are copyleft. For any crate (application or library) that uses `troupe`, there is no copyleft clause.

# Contributing
If there is an aspect of the project that you wish to see changed, improved, or even removed, open a ticket or PR.
