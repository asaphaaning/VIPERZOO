//! Configure the optional browser diagnostics owned by an SDK session.
//!
//! A [`Console`] is both configuration and a tracing source. Applications may
//! install [`Console::layer`] before starting the session; once selected on the
//! session builder, the SDK binds and owns its HTTP lifecycle beside the
//! adapter and engine.

use std::net::SocketAddr;

use viperzoo_assets::Catalog;
use viperzoo_engine::World;
use viperzoo_web::{self as web, control};

/// A diagnostic console selected for one session.
#[derive(Clone, Debug)]
#[must_use = "a diagnostic console has no effect until added to a session builder"]
pub struct Console {
    console: web::Console,
    address: SocketAddr,
    controls: Option<control::Controls>,
}

impl Console {
    /// Creates a read-only console bound to `address` when the session starts.
    pub fn new(address: SocketAddr) -> Self {
        Self {
            console: web::Console::new(),
            address,
            controls: None,
        }
    }

    /// Adds an application-owned control surface to the console.
    pub fn controls(self, controls: control::Controls) -> Self {
        Self {
            controls: Some(controls),
            ..self
        }
    }

    /// Returns the tracing layer that mirrors events into this console.
    #[must_use]
    pub fn layer(&self) -> web::trace::Layer {
        self.console.layer()
    }

    /// Returns the requested listen address.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    pub(crate) async fn start(
        self,
        world: World,
        assets: Option<Catalog>,
    ) -> Result<web::Server, web::Error> {
        self.console
            .start(world, assets, self.address, self.controls)
            .await
    }
}

pub use viperzoo_web::control::{Controls, Session, Signal, State};
