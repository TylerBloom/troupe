//! This module contains various actors and actor wrappers to provide support for common
//! operations. Actor wrappers work in a similar fashion to the iterator wrapper types returned by
//! many of the methods on [`Iterator`]. This module also contains types that implement
//! [`ActorState`] provided you implement a seperate trait. This is done because current patterns,
//! such as the `Manager`/`Worker` pattern, can not be easily generalized via the [`ActorState`]
//! trait.
//!
//! This collection is meant to reflect common usage patterns. If there is a pattern that you use
//! or would like to see supported, open an issue or PR!


// TODO: Implement cast members
//  - Panic catcher
//  - Manager/Worker
//  - Self-sharding actor

#[cfg(feature = "panic_catcher")]
mod panic_catcher;
#[cfg(feature = "panic_catcher")]
pub use panic_catcher::*;

#[cfg(feature = "manager")]
mod manager;
#[cfg(feature = "manager")]
pub use manager::*;

