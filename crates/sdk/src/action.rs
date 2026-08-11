//! Compose adapter dispatch with canonical post-dispatch world evidence.
//!
//! [`Actions::perform`] creates a lazy value. Dispatch begins only when
//! [`Perform::run`] or [`Confirm::run`] is awaited. A confirmation captures its
//! snapshot fence before dispatch, retains the adapter's typed receipt, and
//! resolves the requested world [`Query`] only at a later revision.
//!
//! ```text
//! perform(Action) ──► confirm(Query) ──► within(Duration) ──► run
//!       │                    │                                  │
//!       │                    └── canonical post-dispatch fact  ├─ receipt
//!       └── adapter intent                                      └─ evidence
//! ```

use std::{fmt, time::Duration};

use thiserror::Error as ThisError;
use tracing::instrument;
use viperzoo_adapter_api::action::{Action, Client};
use viperzoo_engine::{WaitError, World};
use viperzoo_world::{
    query::{self, Evidence, Query},
    revision::Revision,
};

/// Binds one adapter client to the canonical world that observes its effects.
#[derive(Clone, Debug)]
pub struct Actions<C> {
    client: C,
    world: World,
}

impl<C> Actions<C> {
    pub(crate) const fn new(client: C, world: World) -> Self {
        Self { client, world }
    }

    /// Borrows the underlying adapter client for reusable policy functions.
    #[must_use]
    pub const fn client(&self) -> &C {
        &self.client
    }

    /// Returns cloneable access to the canonical world paired with this client.
    #[must_use]
    pub fn world(&self) -> World {
        self.world.clone()
    }

    /// Creates a lazy adapter action.
    pub const fn perform(&self, action: Action) -> Perform<'_, C> {
        Perform {
            actions: self,
            action,
        }
    }
}

/// One lazy adapter action without a canonical effect requirement.
#[derive(Debug)]
#[must_use = "an action has no effect until run is awaited"]
pub struct Perform<'a, C> {
    actions: &'a Actions<C>,
    action: Action,
}

impl<'a, C> Perform<'a, C> {
    /// Requires a canonical world query to resolve after dispatch.
    pub fn confirm<Q>(self, query: Q) -> Confirm<'a, C, Q>
    where
        Q: Query,
    {
        Confirm {
            perform: self,
            query,
            deadline: None,
        }
    }
}

impl<C> Perform<'_, C>
where
    C: Client,
    C::Error: fmt::Debug + fmt::Display,
{
    /// Dispatches the action and returns the adapter's typed receipt.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Dispatch`] when the adapter rejects dispatch.
    #[instrument(
        name = "viperzoo::sdk::action::dispatch",
        skip(self),
        fields(action = ?self.action),
        err
    )]
    pub async fn run(self) -> Result<C::Receipt, Error<C::Error>> {
        self.actions
            .client
            .perform(self.action)
            .await
            .map_err(Error::Dispatch)
    }
}

/// A lazy adapter action paired with a post-dispatch world query.
#[derive(Debug)]
#[must_use = "an action confirmation has no effect until run is awaited"]
pub struct Confirm<'a, C, Q> {
    perform: Perform<'a, C>,
    query: Q,
    deadline: Option<Duration>,
}

impl<C, Q> Confirm<'_, C, Q> {
    /// Applies a bounded confirmation deadline.
    pub const fn within(mut self, duration: Duration) -> Self {
        self.deadline = Some(duration);
        self
    }
}

impl<C, Q> Confirm<'_, C, Q>
where
    C: Client,
    C::Error: fmt::Debug + fmt::Display,
    Q: Query,
{
    /// Dispatches the action, then resolves `query` at a later world revision.
    ///
    /// The returned [`Outcome`] preserves both proofs: the adapter receipt says
    /// dispatch crossed its owned boundary, while [`Outcome::evidence`] dates
    /// the canonical world fact that followed it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Dispatch`] when adapter dispatch fails, or
    /// [`Error::Confirmation`] when the world closes or the configured deadline
    /// elapses before the query resolves.
    #[instrument(
        name = "viperzoo::sdk::action::confirm",
        skip(self),
        fields(action = ?self.perform.action),
        err
    )]
    pub async fn run(self) -> ConfirmationResult<C, Q> {
        let mut subscription = self.perform.actions.world.subscribe();
        let anchor = subscription.latest().revision();
        let receipt = self
            .perform
            .actions
            .client
            .perform(self.perform.action)
            .await
            .map_err(Error::Dispatch)?;
        let query = query::after(anchor)
            .and(self.query)
            .map(|(_, output)| output);
        let output = match self.deadline {
            Some(duration) => subscription.wait(query).within(duration).await,
            None => subscription.wait(query).run().await,
        }
        .map_err(Error::Confirmation)?;
        let evidence = Evidence::new(output, subscription.latest().revision());

        Ok(Outcome {
            anchor,
            receipt,
            evidence,
        })
    }
}

/// Result of canonically confirming one adapter action.
pub type ConfirmationResult<C, Q> =
    Result<Outcome<<C as Client>::Receipt, <Q as Query>::Output>, Error<<C as Client>::Error>>;

/// Adapter dispatch and canonical world evidence for one semantic action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Outcome<R, T> {
    anchor: Revision,
    receipt: R,
    evidence: Evidence<T>,
}

impl<R, T> Outcome<R, T> {
    /// Returns the pre-dispatch canonical revision fence.
    #[must_use]
    pub const fn anchor(&self) -> Revision {
        self.anchor
    }

    /// Borrows the adapter-specific dispatch receipt.
    #[must_use]
    pub const fn receipt(&self) -> &R {
        &self.receipt
    }

    /// Borrows the canonical post-dispatch query evidence.
    #[must_use]
    pub const fn evidence(&self) -> &Evidence<T> {
        &self.evidence
    }

    /// Consumes the outcome into its adapter receipt and world evidence.
    #[must_use]
    pub fn into_parts(self) -> Parts<R, T> {
        Parts {
            receipt: self.receipt,
            evidence: self.evidence,
        }
    }
}

/// Named owned parts of one confirmed action outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parts<R, T> {
    /// Adapter-specific dispatch receipt.
    pub receipt: R,
    /// Canonical post-dispatch world evidence.
    pub evidence: Evidence<T>,
}

/// Failure to dispatch or canonically confirm a semantic action.
#[derive(Debug, ThisError)]
pub enum Error<E>
where
    E: fmt::Debug + fmt::Display,
{
    /// The adapter did not dispatch the requested action.
    #[error("adapter action dispatch failed: {0}")]
    Dispatch(E),
    /// The canonical world did not establish the requested effect.
    #[error("action effect was not confirmed: {0}")]
    Confirmation(WaitError),
}

#[cfg(test)]
mod tests {
    use viperzoo_adapter_api::{action::MockClient, observation::Observation};
    use viperzoo_world::query;

    use super::*;

    #[tokio::test]
    async fn confirmation_preserves_dispatch_and_post_dispatch_evidence() {
        let running = viperzoo_engine::channel(viperzoo_engine::Config::default())
            .spawn()
            .expect("test runtime starts the engine");
        let ingress = running.ingress();
        let world = running.world();
        let owner = running.owner();
        let observation = ingress.clone();
        let mut client = MockClient::new();
        client.expect_perform().once().returning(move |_| {
            let observation = observation.clone();

            Box::pin(async move {
                observation
                    .observe(Observation::SessionStarted)
                    .await
                    .expect("engine accepts confirmation evidence");

                Ok(())
            })
        });
        let actions = Actions::new(client, world);

        let outcome = actions
            .perform(Action::RefreshMap)
            .confirm(query::session::active())
            .within(Duration::from_millis(50))
            .run()
            .await
            .expect("dispatch is canonically confirmed");

        assert_eq!(outcome.anchor(), Revision::INITIAL);
        assert_eq!(outcome.evidence().revision(), Revision::INITIAL.next());

        drop(actions);
        drop(ingress);
        owner.wait().await.expect("engine owner stops");
    }
}
