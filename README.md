[![Crates.io](https://img.shields.io/crates/v/troupe.svg)](https://crates.io/crates/troupe)
[![Documentation](https://docs.rs/troupe/badge.svg)](https://docs.rs/troupe/)
![GitHub Workflows](https://github.com/TylerBloom/troupe/actions/workflows/ci.yml/badge.svg)
[![Coverage Status](https://codecov.io/gh/TylerBloom/troupe/branch/main/graph/badge.svg)](https://codecov.io/gh/TylerBloom/troupe)
![Maintenance](https://img.shields.io/badge/Maintenance-Actively%20Developed-brightgreen.svg)

# About
Troupe is an experimental, high-level library for modeling your application using the [actor model](https://en.wikipedia.org/wiki/Actor_model). Actors in the `troupe` model are some state that processes messages from various sources and can (optionally) broadcast messages. An "actor" in `troupe` is contained within an async process, such as those generated by `tokio::spawn`. Because of this, `troupe` is able to support several async runtimes, including `tokio`, `async-std`, and `wasm-bindgen-futures` (for WASM targets). However, actor communication is built upon `tokio`'s channels.

# Model
The `troupe` model pairs every actor state with a scheduler. The scheduler is responsible for managing inbound streams to the actor as well as managing any futures that the actor has queued. By default, every actor has at least one inbound stream. Usually, this stream is a tokio MPSC-style channel to enable other parts of our application to communicate with the actor, but some actors can define their own inbound stream. Conceptually, the scheduler-actor relationship can be model as:
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

Once spawned, communication with an actor and everything else is generally done through a client. Every actor defines how its clients behavior. There are three types of clients: `SinkClient`, `StreamClient`, and `JointClient`.

A `SinkClient` is the type of client you are most likely to use. It enables you to send messages directly to the actor. These messages can be either "fire-and-forget" or "request-response" style messages. Note that a `SinkClient` does not actually implement the [`Sink`](https://docs.rs/futures/latest/futures/sink/trait.Sink.html) trait, but, rather, serves the same conceptual purpose. A `StreamClient` listens for messages that are broadcast from an actor. Unlike a `SinkClient`, a `StreamClient` does not directly support any type of message passing into the actor. It does, however, implement the [`Stream`](https://docs.rs/futures/latest/futures/stream/trait.Stream.html) trait. Lastly, a `JointClient` is both a `SinkClient` and a `StreamClient` put together. A `JointClient` can be decomposed into one or both of the other clients. Note, the actor type defines what kind of clients can be constructed, so you can not, for example, construct a `StreamClient` for an actor that will never broadcast a message.

The last major component of the `troupe` actor model is permanence. Some actors are designed to run forever. These are called `Permanent` actors. Other actors are designed to run for some time (perhaps a long time) and then close. These are called `Transient` actors. This distinction largely serves to help with the ergonomics of interacting with actors.

Lastly, there is built-in "garbage collection" for troupe actors. The scheduler will mark an actor as "dead" and then close the actor process down if all of its held streams close and it is holding no futures that can yield future messages. This check occurs regardless of the user-defined permanence of the actor.

# Usage and Licensing
This project is currently licensed under a LGPL license. The intent of this is for modifications to this library. For forks or other modifications to this library, those versions are copyleft. For any crate (application or library) that uses `troupe`, there is no copyleft clause.

# Contributing
Troupe is currently experimental and subject to potential breaking changes (with due consideration). If there is an aspect of the project that you wish to see changed, improved, or even removed, open a ticket or PR.
