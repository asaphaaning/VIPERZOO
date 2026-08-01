//! Monotonic projected-world revisions.

use serde::Serialize;

/// A monotonic revision of canonical projected state.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Revision(u64);

impl Revision {
    /// The state before any meaningful observation.
    pub const INITIAL: Self = Self(0);

    /// Returns the following revision.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Returns the numeric revision.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}
