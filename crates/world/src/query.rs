//! Describe pure questions over one coherent world snapshot.
//!
//! A [`Query`] returns either [`State::Pending`] or a typed
//! [`State::Ready`] value. Queries contain no waiting, clocks, channels, or
//! retry policy; the asynchronous engine decides when to evaluate them again.
//! This keeps domain questions deterministic and directly testable.

use crate::{revision::Revision, snapshot::Snapshot};

/// The result of evaluating a [`Query`] against one snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State<T> {
    /// The snapshot does not yet establish the requested fact.
    Pending,
    /// The snapshot establishes the requested value.
    Ready(T),
}

impl<T> State<T> {
    /// Transforms a ready value while preserving pending state.
    #[must_use]
    pub fn map<U>(self, transform: impl FnOnce(T) -> U) -> State<U> {
        match self {
            Self::Pending => State::Pending,
            Self::Ready(value) => State::Ready(transform(value)),
        }
    }

    /// Returns the ready value, if the query has resolved.
    #[must_use]
    pub fn ready(self) -> Option<T> {
        match self {
            Self::Pending => None,
            Self::Ready(value) => Some(value),
        }
    }
}

/// A deterministic question evaluated against a coherent [`Snapshot`].
pub trait Query {
    /// Value established once the query resolves.
    type Output;

    /// Evaluates the query against one snapshot revision.
    fn evaluate(&self, snapshot: &Snapshot) -> State<Self::Output>;

    /// Transforms the ready output of this query.
    fn map<F, T>(self, transform: F) -> Map<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Output) -> T,
    {
        Map {
            query: self,
            transform,
        }
    }

    /// Resolves once this query and `other` are both ready in one snapshot.
    fn and<Q>(self, other: Q) -> And<Self, Q>
    where
        Self: Sized,
        Q: Query,
    {
        And {
            left: self,
            right: other,
        }
    }

    /// Resolves with the first ready value, preferring this query when both
    /// resolve in the same snapshot.
    fn or<Q>(self, other: Q) -> Or<Self, Q>
    where
        Self: Sized,
        Q: Query<Output = Self::Output>,
    {
        Or {
            preferred: self,
            fallback: other,
        }
    }
}

/// Creates a query by selecting an optional owned value from each snapshot.
pub fn select<T, F>(select: F) -> Select<F>
where
    F: Fn(&Snapshot) -> Option<T>,
{
    Select { select }
}

/// Creates a query that resolves with `()` when a predicate becomes true.
pub fn when<F>(predicate: F) -> When<F>
where
    F: Fn(&Snapshot) -> bool,
{
    When { predicate }
}

/// Creates a query that resolves after the given revision.
#[must_use]
pub const fn after(revision: Revision) -> After {
    After { revision }
}

/// Query created by [`select`].
pub struct Select<F> {
    select: F,
}

impl<T, F> Query for Select<F>
where
    F: Fn(&Snapshot) -> Option<T>,
{
    type Output = T;

    fn evaluate(&self, snapshot: &Snapshot) -> State<Self::Output> {
        (self.select)(snapshot).map_or(State::Pending, State::Ready)
    }
}

/// Query created by [`when`].
pub struct When<F> {
    predicate: F,
}

impl<F> Query for When<F>
where
    F: Fn(&Snapshot) -> bool,
{
    type Output = ();

    fn evaluate(&self, snapshot: &Snapshot) -> State<Self::Output> {
        if (self.predicate)(snapshot) {
            State::Ready(())
        } else {
            State::Pending
        }
    }
}

/// Query produced by [`Query::map`].
pub struct Map<Q, F> {
    query: Q,
    transform: F,
}

impl<Q, F, T> Query for Map<Q, F>
where
    Q: Query,
    F: Fn(Q::Output) -> T,
{
    type Output = T;

    fn evaluate(&self, snapshot: &Snapshot) -> State<Self::Output> {
        self.query
            .evaluate(snapshot)
            .map(|value| (self.transform)(value))
    }
}

/// Query produced by [`Query::and`].
pub struct And<L, R> {
    left: L,
    right: R,
}

impl<L, R> Query for And<L, R>
where
    L: Query,
    R: Query,
{
    type Output = (L::Output, R::Output);

    fn evaluate(&self, snapshot: &Snapshot) -> State<Self::Output> {
        match (self.left.evaluate(snapshot), self.right.evaluate(snapshot)) {
            (State::Ready(left), State::Ready(right)) => State::Ready((left, right)),
            (State::Pending | State::Ready(_), State::Pending)
            | (State::Pending, State::Ready(_)) => State::Pending,
        }
    }
}

/// Query produced by [`Query::or`].
pub struct Or<P, F> {
    preferred: P,
    fallback: F,
}

impl<P, F> Query for Or<P, F>
where
    P: Query,
    F: Query<Output = P::Output>,
{
    type Output = P::Output;

    fn evaluate(&self, snapshot: &Snapshot) -> State<Self::Output> {
        match self.preferred.evaluate(snapshot) {
            State::Ready(value) => State::Ready(value),
            State::Pending => self.fallback.evaluate(snapshot),
        }
    }
}

/// Query produced by [`after`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct After {
    revision: Revision,
}

impl Query for After {
    type Output = Revision;

    fn evaluate(&self, snapshot: &Snapshot) -> State<Self::Output> {
        let revision = snapshot.revision();

        if revision > self.revision {
            State::Ready(revision)
        } else {
            State::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::world::World;

    use super::*;

    #[test]
    fn composition_preserves_pending_until_one_snapshot_proves_every_fact() {
        let snapshot = World::new().snapshot();
        let query = when(|snapshot| snapshot.revision() == Revision::INITIAL)
            .and(after(Revision::INITIAL))
            .map(|_| true);

        assert_eq!(query.evaluate(&snapshot), State::Pending);
    }

    #[test]
    fn preferred_query_wins_when_both_are_ready() {
        let snapshot = World::new().snapshot();
        let query = select(|_| Some("preferred")).or(select(|_| Some("fallback")));

        assert_eq!(query.evaluate(&snapshot), State::Ready("preferred"));
    }
}
