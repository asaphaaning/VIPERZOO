//! Monotonic projected-world revisions.

use serde::Serialize;

/// A monotonic revision of canonical projected state.
///
/// Every accepted observation advances the revision, whether or not it changed
/// anything a script can see. That is what lets a consumer distinguish *nothing
/// happened* from *nothing changed*: a stalled revision means the engine stopped
/// accepting evidence, while a rising revision with unchanged values means the
/// world is being observed and is genuinely steady.
///
/// It also dates every fact. [`crate::knowledge::Knowledge`] stores the revision
/// that established a value, so precedence questions — is this memory seed older
/// than the packet that superseded it? — are answered by comparison rather than
/// by bookkeeping at each call site.
///
/// ```
/// use viperzoo_world::revision::Revision;
///
/// let first = Revision::INITIAL;
/// let second = first.next();
///
/// assert!(second > first);
/// assert_eq!(second.value(), 1);
/// ```
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
