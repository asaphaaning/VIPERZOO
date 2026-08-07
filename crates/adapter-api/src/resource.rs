//! Carry coherent VITA and mana facts across the adapter boundary.
//!
//! A validated client-memory read can seed resource state during a warm
//! attachment, when no suitable packet has arrived yet. The types here prove
//! basic pool invariants before crossing into the world; the reducer then keeps
//! packet-derived values authoritative when the two sources overlap.

use thiserror::Error;

/// A current/maximum resource pair that cannot be constructed incoherently.
///
/// Two readings of the same pool are only comparable if both are whole, so the
/// pair is validated once at the boundary rather than re-checked by everything
/// downstream. A client-memory read is the motivating case: it can catch the
/// process mid-update and report a current value above the maximum, or a
/// maximum of zero for a pool the character does not have. Either would project
/// as a real observation if the pair were a plain struct.
///
/// ```
/// use viperzoo_adapter_api::resource::Pool;
///
/// let vita = Pool::new(783, 783).expect("full pool is coherent");
///
/// assert_eq!(vita.current(), 783);
/// assert_eq!(vita.maximum(), 783);
///
/// // A torn read cannot become a projected fact.
/// assert!(Pool::new(900, 783).is_err());
/// assert!(Pool::new(0, 0).is_err());
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pool {
    current: u32,
    maximum: u32,
}

impl Pool {
    /// Creates a resource pool whose current value is within a non-zero maximum.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroMaximum`] or [`Error::CurrentExceedsMaximum`] when
    /// the pair cannot describe a valid client resource pool.
    pub const fn new(current: u32, maximum: u32) -> Result<Self, Error> {
        if maximum == 0 {
            return Err(Error::ZeroMaximum);
        }

        if current > maximum {
            return Err(Error::CurrentExceedsMaximum { current, maximum });
        }

        Ok(Self { current, maximum })
    }

    /// Returns the current value.
    #[must_use]
    pub const fn current(self) -> u32 {
        self.current
    }

    /// Returns the maximum value.
    #[must_use]
    pub const fn maximum(self) -> u32 {
        self.maximum
    }
}

/// A coherent VITA and mana snapshot read from the attached client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resources {
    vita: Pool,
    mana: Pool,
    source: Source,
}

impl Resources {
    /// Creates one validated resource snapshot.
    #[must_use]
    pub const fn new(vita: Pool, mana: Pool, source: Source) -> Self {
        Self { vita, mana, source }
    }

    /// Returns the VITA pool.
    #[must_use]
    pub const fn vita(self) -> Pool {
        self.vita
    }

    /// Returns the mana pool.
    #[must_use]
    pub const fn mana(self) -> Pool {
        self.mana
    }

    /// Returns the acquisition source.
    #[must_use]
    pub const fn source(self) -> Source {
        self.source
    }
}

/// The closed vocabulary of validated non-protocol resource sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// `NexusTK` build 752's persistent resource model.
    ClientMemoryBuild752,
}

/// Invalid client resource data.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum Error {
    /// A resource maximum must be non-zero.
    #[error("resource maximum is zero")]
    ZeroMaximum,
    /// The current value exceeds its maximum.
    #[error("resource current value {current} exceeds maximum {maximum}")]
    CurrentExceedsMaximum {
        /// Rejected current value.
        current: u32,
        /// Rejected maximum value.
        maximum: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_rejects_impossible_values() {
        assert_eq!(Pool::new(0, 0), Err(Error::ZeroMaximum));
        assert_eq!(
            Pool::new(18, 17),
            Err(Error::CurrentExceedsMaximum {
                current: 18,
                maximum: 17,
            })
        );
    }
}
