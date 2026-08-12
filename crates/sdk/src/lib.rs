//! Provide the stable vocabulary for writing VIPERZOO scripts.
//!
//! This crate re-exports the domain, planning, policy, and engine crates under
//! one dependency. [`session`] composes any typed adapter with a canonical
//! engine while leaving the concrete acquisition implementation in the
//! application dependency graph. Its fluent builder either returns the
//! adapter's typed event stream or owns an exhaustive event handler as part of
//! the session lifecycle. [`action::Actions`] pairs adapter dispatch receipts
//! with canonical post-dispatch world evidence.

pub mod action;
pub mod diagnostics;
mod session;

pub use session::{
    Builder, Error, EventError, Failure, HandledBuilder, HandledSession, Report, Session,
    Termination, session,
};

pub use viperzoo_actions as actions;
pub use viperzoo_adapter_api as adapter;
pub use viperzoo_assets as assets;
pub use viperzoo_engine as engine;
pub use viperzoo_navigation as navigation;
pub use viperzoo_protocol as protocol;
pub use viperzoo_world as world;
