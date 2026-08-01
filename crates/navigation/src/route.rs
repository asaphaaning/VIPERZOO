//! Represent a useful planning result without encoding execution policy.
//!
//! A [`Route`] is non-empty when constructed, so [`Plan::Route`] and
//! [`Plan::Frontier`] always tell a controller that one movement is available.
//! Consuming a route is only a convenience for the controller; the planner does
//! not assume every step will succeed or that the snapshot stays current.

use std::collections::VecDeque;

use viperzoo_protocol::direction::Direction;

/// The result of planning to a target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Plan {
    /// The player is already at the target.
    Arrived,
    /// At least one movement remains and the target is inside known bounds.
    Route(Route),
    /// A non-empty partial route reaches the edge of currently observed map
    /// data toward a target that remains outside it.
    ///
    /// Executing one step and replanning allows ordinary viewport packets to
    /// expand the projected map without treating the unbounded coordinate
    /// domain as traversable.
    Frontier(Route),
}

/// A non-empty sequence of cardinal steps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Route {
    steps: VecDeque<Direction>,
}

impl Route {
    pub(crate) fn from_steps(steps: Vec<Direction>) -> Option<Self> {
        (!steps.is_empty()).then(|| Self {
            steps: steps.into(),
        })
    }

    /// Returns the first remaining planned direction without consuming it.
    ///
    /// [`Iterator`] consumers can exhaust a [`Route`], so this accurately
    /// represents the absence of a remaining step instead of panicking.
    #[must_use]
    pub fn first(&self) -> Option<Direction> {
        self.steps.front().copied()
    }

    /// Returns the number of remaining steps.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Returns whether no steps remain.
    ///
    /// A constructed [`Route`] is never empty; this becomes useful only after
    /// consuming it through [`Iterator`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

impl Iterator for Route {
    type Item = Direction;

    fn next(&mut self) -> Option<Self::Item> {
        self.steps.pop_front()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.steps.len(), Some(self.steps.len()))
    }
}

impl ExactSizeIterator for Route {}
