//! Represent facts that may be absent, stale, or established by different evidence.
//!
//! `Option<T>` alone cannot answer why a projected value exists or whether a
//! later packet should supersede it. [`Knowledge`] carries the value together
//! with its establishing revision and [`Source`]. Reducers use that evidence to
//! preserve protocol precedence over supplementary late-attachment memory.

use serde::Serialize;

use crate::revision::Revision;

/// The observation that established a projected value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Incoming `0x04` login/map-entry/refresh localization.
    PositionSeed,
    /// Incoming `0x26` authoritative movement result.
    PositionResult,
    /// Incoming `0x08` player status block.
    PlayerStatus,
    /// A decoded client-authored action at the plaintext protocol boundary.
    ClientAction,
    /// `NexusTK` build 752's validated persistent resource model.
    ClientMemoryBuild752,
}

/// A value that is either unobserved or accompanied by coherent evidence.
///
/// `Option<T>` says whether a value exists; it cannot say why, or whether the
/// next observation should replace it. That matters here because two sources
/// disagree in kind: the server's packets are authoritative, while a warm
/// attachment's client-memory read is a stopgap that must never overwrite them.
/// Carrying the [`Revision`] and [`Source`] alongside the value keeps that rule
/// in one place instead of repeating it at every assignment.
///
/// ```text
/// Unknown                          nothing observed this epoch
///   └─ observe(memory)  ──►  Observed { value, revision, ClientMemoryBuild752 }
///        └─ observe(packet) ──►  Observed { value, revision, PlayerStatus }
///             └─ observe(memory) ──►  unchanged: packets outrank memory
/// ```
///
/// ```
/// use viperzoo_world::knowledge::{Knowledge, Source};
/// use viperzoo_world::revision::Revision;
///
/// let unknown: Knowledge<u32> = Knowledge::Unknown;
/// assert_eq!(unknown.value(), None);
///
/// let seeded = Knowledge::observed(783, Revision::INITIAL.next(), Source::ClientMemoryBuild752);
/// assert_eq!(seeded.value(), Some(&783));
/// assert_eq!(seeded.source(), Some(Source::ClientMemoryBuild752));
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Knowledge<T> {
    /// This attachment epoch has not established a value.
    #[default]
    Unknown,
    /// A typed observation established the value.
    Observed {
        /// The projected value.
        value: T,
        /// Revision at which it was established.
        revision: Revision,
        /// Protocol or adapter source of the observation.
        source: Source,
    },
}

impl<T> Knowledge<T> {
    /// Creates an observed value with inseparable evidence.
    #[must_use]
    pub const fn observed(value: T, revision: Revision, source: Source) -> Self {
        Self::Observed {
            value,
            revision,
            source,
        }
    }

    /// Borrows the value while preserving whether it is known.
    #[must_use]
    pub const fn value(&self) -> Option<&T> {
        match self {
            Self::Unknown => None,
            Self::Observed { value, .. } => Some(value),
        }
    }

    /// Returns the evidence source when the value is known.
    #[must_use]
    pub const fn source(&self) -> Option<Source> {
        match self {
            Self::Unknown => None,
            Self::Observed { source, .. } => Some(*source),
        }
    }

    /// Maps an observed value while preserving its evidence.
    #[must_use]
    pub fn map<U>(self, transform: impl FnOnce(T) -> U) -> Knowledge<U> {
        match self {
            Self::Unknown => Knowledge::Unknown,
            Self::Observed {
                value,
                revision,
                source,
            } => Knowledge::Observed {
                value: transform(value),
                revision,
                source,
            },
        }
    }

    /// Returns whether no observation has established a value.
    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}
