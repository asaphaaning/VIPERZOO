//! Compose acquisition without prescribing a transport or executor.
//!
//! An [`Adapter`] starts with one observation [`observation::Sink`] and yields three
//! independent capabilities:
//!
//! ```text
//!                  ┌─ client ──► typed actions
//! Adapter + Sink ──┼─ events ──► lifecycle and diagnostics
//!                  └─ driver ──► wait or shut down
//! ```
//!
//! [`Running`] is deliberately a named product rather than a tuple. Consumers
//! can move each capability into the task that owns it without remembering a
//! positional convention.

use std::future::Future;

use futures_core::Stream;

use crate::{action, observation};

/// One runtime facility an [`Adapter`] makes available to an application.
///
/// The vocabulary describes capabilities, not configuration. For example,
/// [`Capability::Recording`] says the selected adapter is actively retaining
/// evidence for this run; the adapter still owns the transport-specific policy
/// that selected where and how that evidence is written.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum Capability {
    /// Typed application actions can cross the adapter boundary.
    Actions,
    /// A closed adapter-specific diagnostic event stream is available.
    Events,
    /// Late attachment seeds canonical state from validated client memory.
    WarmAttachment,
    /// Raw boundary evidence is being persisted for replay or analysis.
    Recording,
}

impl Capability {
    /// Every canonical adapter capability in declaration order.
    pub const VARIANTS: [Self; 4] = [
        Self::Actions,
        Self::Events,
        Self::WarmAttachment,
        Self::Recording,
    ];

    const fn mask(self) -> u8 {
        1 << self as u8
    }
}

/// The closed set of facilities active for one adapter session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities {
    bits: u8,
}

impl Capabilities {
    /// No optional adapter facilities.
    pub const NONE: Self = Self { bits: 0 };

    /// Creates a capability set from the selected vocabulary.
    #[must_use]
    pub const fn new(capabilities: &[Capability]) -> Self {
        let mut result = Self::NONE;
        let mut index = 0;

        while index < capabilities.len() {
            result = result.with(capabilities[index]);
            index += 1;
        }

        result
    }

    /// Returns a set that also contains `capability`.
    #[must_use]
    pub const fn with(self, capability: Capability) -> Self {
        Self {
            bits: self.bits | capability.mask(),
        }
    }

    /// Returns a set that contains `capability` exactly when `enabled`.
    #[must_use]
    pub const fn with_if(self, capability: Capability, enabled: bool) -> Self {
        if enabled { self.with(capability) } else { self }
    }

    /// Returns whether `capability` is active.
    #[must_use]
    pub const fn contains(self, capability: Capability) -> bool {
        self.bits & capability.mask() != 0
    }

    /// Iterates active capabilities in canonical order.
    pub fn iter(self) -> impl Iterator<Item = Capability> {
        Capability::VARIANTS
            .into_iter()
            .filter(move |capability| self.contains(*capability))
    }
}

/// The independent capabilities produced by a running [`Adapter`].
#[derive(Debug)]
#[must_use = "a running adapter must be driven or shut down"]
pub struct Running<Client, Events, Driver> {
    /// Cloneable client action capability.
    pub client: Client,
    /// Typed lifecycle and diagnostic event stream.
    pub events: Events,
    /// Single owner of the adapter lifecycle.
    pub driver: Driver,
}

impl<Client, Events, Driver> Running<Client, Events, Driver> {
    /// Creates a running adapter from its independent capabilities.
    pub const fn new(client: Client, events: Events, driver: Driver) -> Self {
        Self {
            client,
            events,
            driver,
        }
    }
}

/// A transport-specific source of observations, actions, and typed events.
pub trait Adapter: Sized {
    /// Client action capability exposed to application policy.
    type Client: action::Client;
    /// One adapter-specific lifecycle or diagnostic event.
    type Event: Send + 'static;
    /// Asynchronous stream of [`Adapter::Event`] values.
    type Events: Stream<Item = Self::Event> + Send + Unpin + 'static;
    /// Single lifecycle owner for the running adapter.
    type Driver: Driver<Error = Self::Error>;
    /// Adapter startup and lifecycle failure.
    type Error;

    /// Returns the facilities active for this configured adapter.
    fn capabilities(&self) -> Capabilities;

    /// Starts acquisition against the given observation sink.
    fn start<S>(self, sink: S) -> impl Future<Output = Result<Started<Self>, Self::Error>> + Send
    where
        S: observation::Sink + Clone;
}

/// The capabilities produced by a particular [`Adapter`] implementation.
pub type Started<A> =
    Running<<A as Adapter>::Client, <A as Adapter>::Events, <A as Adapter>::Driver>;

/// Single-owner lifecycle capability of a running [`Adapter`].
#[cfg_attr(
    any(test, feature = "test-util"),
    mockall::automock(type Error = std::convert::Infallible;)
)]
pub trait Driver: Sized + Send + 'static {
    /// Fatal acquisition or teardown failure.
    type Error;

    /// Returns whether acquisition has already stopped.
    fn is_finished(&self) -> bool;

    /// Waits for natural adapter termination.
    fn wait(self) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Requests adapter termination and waits for teardown.
    fn shutdown(self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::{
        action::MockClient,
        observation::{MockSink, Observation},
    };

    #[derive(Debug)]
    struct Fake {
        client: MockClient,
        driver: MockDriver,
    }

    impl Adapter for Fake {
        type Client = MockClient;
        type Driver = MockDriver;
        type Error = Infallible;
        type Event = Infallible;
        type Events = tokio_stream::Empty<Infallible>;

        fn capabilities(&self) -> Capabilities {
            Capabilities::new(&[Capability::Actions, Capability::Events])
        }

        async fn start<S>(
            self,
            sink: S,
        ) -> Result<Running<Self::Client, Self::Events, Self::Driver>, Self::Error>
        where
            S: observation::Sink + Clone,
        {
            sink.observe(Observation::SessionStarted)
                .await
                .expect("fake sink accepts a session boundary");

            Ok(Running::new(
                self.client,
                tokio_stream::empty(),
                self.driver,
            ))
        }
    }

    #[test]
    fn capabilities_are_closed_and_canonically_ordered() {
        let capabilities = Capabilities::new(&[
            Capability::Recording,
            Capability::Actions,
            Capability::Recording,
        ]);

        assert_eq!(
            capabilities.iter().collect::<Vec<_>>(),
            vec![Capability::Actions, Capability::Recording]
        );
        assert!(!capabilities.contains(Capability::WarmAttachment));
    }

    #[tokio::test]
    async fn fake_adapter_proves_the_contract_without_an_engine() {
        let observations = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&observations);
        let mut sink = MockSink::new();
        sink.expect_observe()
            .times(1)
            .returning(move |observation| {
                observed
                    .lock()
                    .expect("test observation lock is not poisoned")
                    .push(observation);
                Box::pin(std::future::ready(Ok(())))
            });
        let mut driver = MockDriver::new();
        driver
            .expect_shutdown()
            .times(1)
            .returning(|| Box::pin(std::future::ready(Ok(()))));
        let adapter = Fake {
            client: MockClient::new(),
            driver,
        };
        let running = adapter
            .start(Arc::new(sink))
            .await
            .expect("fake adapter starts");

        running.driver.shutdown().await.expect("fake owner stops");

        let observations = observations
            .lock()
            .expect("test observation lock is not poisoned");
        assert!(matches!(
            observations.as_slice(),
            [Observation::SessionStarted]
        ));
    }
}
