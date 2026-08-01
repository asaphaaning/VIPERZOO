//! Execute client-native intents and confirm their effects in the world.
//!
//! This crate is the policy layer above pure planning and projection. Policies
//! use a transport-neutral [`viperzoo_adapter_api::action::Client`] to ask the
//! normal client to act, then wait for canonical engine snapshots to prove what
//! actually happened. Adapter acceptance alone is never treated as game truth.
//!
//! For example, destination walking repeatedly plans, submits one step,
//! observes its result, and replans. The policy remains reusable because it
//! does not know whether the client is reached through Frida or another adapter.

pub mod combat;
pub mod walk;
