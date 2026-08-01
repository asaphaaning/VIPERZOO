//! Capture-stream line identity.

use std::num::NonZeroU64;

use serde::Serialize;

/// A one-based line within one acquisition stream.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Line(NonZeroU64);

impl Line {
    /// First line in an acquisition stream.
    pub const FIRST: Self = Self(NonZeroU64::MIN);

    /// Returns the one-based numeric representation.
    #[must_use]
    pub const fn number(self) -> u64 {
        self.0.get()
    }

    /// Advances to the next line, or returns `None` if the stream exhausted
    /// the complete `u64` line space.
    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self.0.get().checked_add(1) {
            Some(number) => match NonZeroU64::new(number) {
                Some(number) => Some(Self(number)),
                None => None,
            },
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_identity_is_one_based_and_monotonic() {
        let first = Line::FIRST;
        let second = first.next().expect("the first line has a successor");

        assert_eq!(first.number(), 1);
        assert_eq!(second.number(), 2);
    }
}
