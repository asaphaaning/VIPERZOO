//! Provide the stable vocabulary for writing VIPERZOO scripts.
//!
//! This crate re-exports the domain, planning, policy, and engine crates under
//! one dependency. It intentionally does not re-export a live acquisition
//! adapter: scripts may depend on [`engine`] and [`actions`] without becoming
//! coupled to Frida, replay, or a future transport implementation.

pub use viperzoo_actions as actions;
pub use viperzoo_adapter_api as adapter;
pub use viperzoo_assets as assets;
pub use viperzoo_engine as engine;
pub use viperzoo_navigation as navigation;
pub use viperzoo_protocol as protocol;
pub use viperzoo_world as world;
