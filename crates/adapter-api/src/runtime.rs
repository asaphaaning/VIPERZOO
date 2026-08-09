//! Compose acquisition without prescribing a transport or executor.
//!
//! An [`Adapter`] starts with one observation [`Sink`] and yields three
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

    /// Starts acquisition against the given observation sink.
    fn start<S>(self, sink: S) -> impl Future<Output = Result<Started<Self>, Self::Error>> + Send
    where
        S: observation::Sink;
}

/// The capabilities produced by a particular [`Adapter`] implementation.
pub type Started<A> =
    Running<<A as Adapter>::Client, <A as Adapter>::Events, <A as Adapter>::Driver>;

/// Single-owner lifecycle capability of a running [`Adapter`].
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
        pin::Pin,
        sync::{Arc, Mutex},
        task::{Context, Poll},
    };

    use super::*;
    use crate::observation::Observation;

    #[derive(Clone, Debug, Default)]
    struct Sink {
        observations: Arc<Mutex<Vec<Observation>>>,
    }

    impl observation::Sink for Sink {
        async fn observe(&self, observation: Observation) -> Result<(), observation::Error> {
            self.observe_blocking(observation)
        }

        fn observe_blocking(&self, observation: Observation) -> Result<(), observation::Error> {
            self.observations
                .lock()
                .expect("test observation lock is not poisoned")
                .push(observation);
            Ok(())
        }
    }

    #[derive(Clone, Debug)]
    struct Client;

    impl action::Client for Client {
        type Error = Infallible;

        async fn perform(&self, _action: action::Action) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct Events;

    impl Stream for Events {
        type Item = Infallible;

        fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(None)
        }
    }

    #[derive(Debug)]
    struct Owner;

    impl Driver for Owner {
        type Error = Infallible;

        fn is_finished(&self) -> bool {
            false
        }

        async fn wait(self) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn shutdown(self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct Fake;

    impl Adapter for Fake {
        type Client = Client;
        type Driver = Owner;
        type Error = Infallible;
        type Event = Infallible;
        type Events = Events;

        async fn start<S>(
            self,
            sink: S,
        ) -> Result<Running<Self::Client, Self::Events, Self::Driver>, Self::Error>
        where
            S: observation::Sink,
        {
            sink.observe(Observation::SessionStarted)
                .await
                .expect("fake sink accepts a session boundary");

            Ok(Running::new(Client, Events, Owner))
        }
    }

    #[tokio::test]
    async fn fake_adapter_proves_the_contract_without_an_engine() {
        let sink = Sink::default();
        let observations = Arc::clone(&sink.observations);
        let running = Fake.start(sink).await.expect("fake adapter starts");

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
