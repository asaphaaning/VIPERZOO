//! Explain and wait for application prerequisites as one typed policy.

use crate::{
    query::{Query, State},
    revision::Revision,
    session::{Connection, Epoch},
    snapshot::Snapshot,
};

/// Requires every canonical application prerequisite.
pub fn application() -> Readiness {
    Readiness::application()
}

/// Creates readiness from the selected prerequisites.
pub fn requirements(requirements: impl IntoIterator<Item = Requirement>) -> Readiness {
    Readiness::new(requirements)
}

/// One canonical prerequisite an application may require before acting.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Requirement {
    /// A session observation established an active connection.
    Session,
    /// Active map identity is known.
    Map,
    /// Player localization is known.
    Position,
    /// Current and maximum VITA and mana are known.
    Resources,
    /// An authoritative carried-inventory snapshot is complete.
    Inventory,
    /// An authoritative equipment snapshot is complete.
    Equipment,
}

impl Requirement {
    /// Every canonical prerequisite in declaration order.
    pub const VARIANTS: [Self; 6] = [
        Self::Session,
        Self::Map,
        Self::Position,
        Self::Resources,
        Self::Inventory,
        Self::Equipment,
    ];

    fn is_met(self, snapshot: &Snapshot) -> bool {
        match self {
            Self::Session => {
                snapshot.session_epoch() > Epoch::INITIAL
                    && snapshot.connection() == Connection::Active
            }
            Self::Map => snapshot.map().context().is_some(),
            Self::Position => snapshot.player().location().position().is_some(),
            Self::Resources => {
                let resources = snapshot.player().resources();

                resources.vita().current().value().is_some()
                    && resources.vita().maximum().value().is_some()
                    && resources.mana().current().value().is_some()
                    && resources.mana().maximum().value().is_some()
            }
            Self::Inventory => snapshot.inventory_complete(),
            Self::Equipment => snapshot.equipment_complete(),
        }
    }
}

/// A composable set of prerequisites that is also a world [`Query`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[must_use = "readiness has no effect until it is evaluated or awaited"]
pub struct Readiness {
    requirements: Vec<Requirement>,
}

impl Readiness {
    /// Creates readiness from a deduplicated set of requirements.
    pub fn new(requirements: impl IntoIterator<Item = Requirement>) -> Self {
        let mut requirements = requirements.into_iter().collect::<Vec<_>>();
        requirements.sort_unstable();
        requirements.dedup();

        Self { requirements }
    }

    /// Requires every canonical application prerequisite.
    pub fn application() -> Self {
        Self::new(Requirement::VARIANTS)
    }

    /// Adds one prerequisite while preserving canonical ordering.
    pub fn require(mut self, requirement: Requirement) -> Self {
        if !self.requirements.contains(&requirement) {
            self.requirements.push(requirement);
            self.requirements.sort_unstable();
        }

        self
    }

    /// Explains readiness at one coherent snapshot, whether ready or pending.
    #[must_use]
    pub fn report(&self, snapshot: &Snapshot) -> Report {
        let missing = self
            .requirements
            .iter()
            .copied()
            .filter(|requirement| !requirement.is_met(snapshot))
            .collect();

        Report {
            revision: snapshot.revision(),
            required: self.requirements.clone(),
            missing,
        }
    }
}

impl Query for Readiness {
    type Output = Report;

    fn evaluate(&self, snapshot: &Snapshot) -> State<Self::Output> {
        let report = self.report(snapshot);

        if report.is_ready() {
            State::Ready(report)
        } else {
            State::Pending
        }
    }
}

/// Evidence explaining which prerequisites are satisfied or missing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    revision: Revision,
    required: Vec<Requirement>,
    missing: Vec<Requirement>,
}

impl Report {
    /// Returns the coherent revision this report evaluated.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Borrows every prerequisite included in the policy.
    #[must_use]
    pub fn required(&self) -> &[Requirement] {
        &self.required
    }

    /// Borrows the unmet prerequisites in canonical order.
    #[must_use]
    pub fn missing(&self) -> &[Requirement] {
        &self.missing
    }

    /// Returns whether every required fact is established.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.missing.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn report_names_every_missing_requirement_in_canonical_order() {
        let snapshot = World::new().snapshot();
        let readiness = Readiness::application();
        let report = readiness.report(&snapshot);

        assert_eq!(report.missing(), Requirement::VARIANTS);
        assert!(!report.is_ready());
        assert_eq!(readiness.evaluate(&snapshot), State::Pending);
    }

    #[test]
    fn readiness_deduplicates_composed_requirements() {
        let readiness = Readiness::new([Requirement::Map, Requirement::Map])
            .require(Requirement::Position)
            .require(Requirement::Map);
        let report = readiness.report(&World::new().snapshot());

        assert_eq!(
            report.required(),
            &[Requirement::Map, Requirement::Position]
        );
    }

    #[test]
    fn active_connection_requires_an_established_session_epoch() {
        let mut world = World::new();
        let readiness = Readiness::new([Requirement::Session]);

        assert_eq!(readiness.evaluate(&world.snapshot()), State::Pending);

        let _ = world.begin_session();
        assert!(matches!(
            readiness.evaluate(&world.snapshot()),
            State::Ready(report) if report.is_ready()
        ));
    }
}
