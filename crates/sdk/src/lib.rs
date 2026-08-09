//! Provide the stable vocabulary for writing VIPERZOO scripts.
//!
//! This crate re-exports the domain, planning, policy, and engine crates under
//! one dependency. [`Session`] composes any typed adapter with a canonical
//! engine while leaving the concrete acquisition implementation in the
//! application dependency graph.

mod session;

pub use session::{Builder, Error, Owner, Session};

pub use viperzoo_actions as actions;
pub use viperzoo_adapter_api as adapter;
pub use viperzoo_assets as assets;
pub use viperzoo_engine as engine;
pub use viperzoo_navigation as navigation;
pub use viperzoo_protocol as protocol;
pub use viperzoo_world as world;
