//! Run the engine’s deterministic form without asynchronous ownership.
//!
//! [`Reducer`] is the replay-friendly counterpart to [`crate::Task`]. Each
//! [`viperzoo_adapter_api::observation::Observation`] is reduced immediately in
//! caller order, making the resulting [`viperzoo_world::snapshot::Snapshot`]
//! reproducible in tests and finite input analysis. It deliberately does not
//! buffer, schedule, or share state between tasks.

use tracing::instrument;
use viperzoo_adapter_api::observation::Observation;
use viperzoo_world::{
    snapshot::Snapshot,
    world::{Change, World},
};

/// Deterministic projection of an ordered observation sequence.
#[derive(Debug, Default)]
pub struct Reducer {
    world: World,
}

impl Reducer {
    /// Creates an empty, warm-attachment-capable projection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one ordered [`Observation`].
    #[instrument(
        name = "viperzoo::engine::reduce",
        skip(self, observation),
        fields(observation = observation_name(&observation)),
        ret(level = "debug")
    )]
    pub fn observe(&mut self, observation: Observation) -> Change {
        match observation {
            Observation::SessionStarted => self.world.begin_session(),
            Observation::TransportClosed => self.world.observe_transport_close(),
            Observation::Packet(packet) => self.world.apply(&packet),
            Observation::PlayerResources(resources) => self.world.seed_resources(resources),
            Observation::PlayerInventory(inventory) => self.world.seed_inventory(&inventory),
        }
    }

    /// Returns the current immutable projection.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.world.snapshot()
    }
}

const fn observation_name(observation: &Observation) -> &'static str {
    match observation {
        Observation::SessionStarted => "session-started",
        Observation::TransportClosed => "transport-closed",
        Observation::Packet(_) => "packet",
        Observation::PlayerResources(_) => "player-resources",
        Observation::PlayerInventory(_) => "player-inventory",
    }
}

#[cfg(test)]
mod tests {
    use viperzoo_adapter_api::resource::{Pool, Resources, Source};
    use viperzoo_protocol::{decode, direction::Flow};
    use viperzoo_world::knowledge::Source as WorldSource;

    use super::*;

    #[test]
    fn session_boundary_resets_session_scoped_state() {
        let bytes = hex::decode("04000300010003000100410000").expect("fixture hex is valid");
        let packet = decode(Flow::Clientbound, &bytes).expect("fixture packet is valid");
        let mut reducer = Reducer::new();

        let _ = reducer.observe(packet.into());
        let _ = reducer.observe(Observation::SessionStarted);

        assert_eq!(reducer.snapshot().processed_packet_count(), 0);
        assert!(reducer.snapshot().player().location().position().is_none());
    }

    #[test]
    fn transport_close_crosses_the_adapter_boundary() {
        let mut reducer = Reducer::new();

        let _ = reducer.observe(Observation::TransportClosed);

        assert_eq!(
            reducer.snapshot().connection(),
            viperzoo_world::session::Connection::TransportClosed
        );
    }

    #[test]
    fn memory_resources_seed_late_attachment_without_overwriting_packets() {
        let memory = Resources::new(
            Pool::new(17, 49).expect("valid VITA pool"),
            Pool::new(32, 32).expect("valid mana pool"),
            Source::ClientMemoryBuild752,
        );
        let mut reducer = Reducer::new();

        let _ = reducer.observe(Observation::PlayerResources(memory));
        let seeded = reducer.snapshot();

        assert_eq!(
            seeded.player().resources().vita().current().value(),
            Some(&17)
        );
        assert_eq!(
            seeded.player().resources().vita().current().source(),
            Some(WorldSource::ClientMemoryBuild752)
        );

        let bytes = hex::decode("08280000001000000008000000000000000000000000")
            .expect("fixture hex is valid");
        let packet = decode(Flow::Clientbound, &bytes).expect("fixture packet is valid");
        let _ = reducer.observe(packet.into());
        let packet_backed = reducer.snapshot();

        assert_eq!(
            packet_backed.player().resources().vita().current().value(),
            Some(&16)
        );
        assert_eq!(
            packet_backed.player().resources().vita().current().source(),
            Some(WorldSource::PlayerStatus)
        );

        let _ = reducer.observe(Observation::PlayerResources(memory));
        let reseeded = reducer.snapshot();

        assert_eq!(
            reseeded.player().resources().vita().current().value(),
            Some(&16)
        );
        assert_eq!(
            reseeded.player().resources().vita().current().source(),
            Some(WorldSource::PlayerStatus)
        );
    }
}
