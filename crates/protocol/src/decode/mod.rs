//! Dispatch plaintext bodies into typed packets, without partial promotion.
//!
//! Family decoders own byte layout checks for the opcodes they understand. This
//! module owns direction-aware dispatch and the boundary rule: unknown opcodes
//! remain lossless evidence, while malformed known opcodes are rejected before
//! they can become [`crate::packet::Packet`] values. The world can therefore
//! rely on every typed variant satisfying its structural contract.

mod action;
mod bytes;
mod entity;
mod equipment;
mod heartbeat;
mod inventory;
mod map;
mod player;
mod profile;
mod state;
mod travel;

use thiserror::Error;
use tracing::instrument;

use crate::direction::Flow;
use crate::{client, packet, server};

/// Decodes one already-delimited plaintext logical body.
///
/// Unknown opcodes produce [`packet::Unknown`] with the exact original bytes.
/// Malformed layouts of known opcodes return [`enum@Error`] and must not mutate
/// a projected world.
///
/// # Errors
///
/// Returns [`Error::Empty`] for an empty body and [`Error::Malformed`] when a
/// promoted opcode does not satisfy its structural contract.
#[instrument(
    name = "viperzoo::protocol::decode",
    skip(body),
    fields(flow = ?flow, length = body.len()),
    err,
    ret(level = "trace")
)]
pub fn decode(flow: Flow, body: &[u8]) -> Result<packet::Packet, Error> {
    let Some(&opcode) = body.first() else {
        return Err(Error::Empty);
    };

    match flow {
        Flow::Clientbound => decode_clientbound(opcode, body).map(packet::Packet::Clientbound),
        Flow::Serverbound => decode_serverbound(opcode, body).map(packet::Packet::Serverbound),
    }
}

fn decode_clientbound(opcode: u8, body: &[u8]) -> Result<server::Packet, Error> {
    let packet =
        match opcode {
            0x04 | 0x26 => server::Packet::PlayerLocation(player::location(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::Location),
            )?),
            0x06 => server::Packet::MapRegion(map::region(body).ok_or(Error::malformed(
                Flow::Clientbound,
                opcode,
                Problem::MapRegion,
            ))?),
            0x07 => server::Packet::EntityAppearances(entity::appearances(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::Appearances),
            )?),
            0x08 => server::Packet::PlayerStatus(player::status(body).ok_or(Error::malformed(
                Flow::Clientbound,
                opcode,
                Problem::Status,
            ))?),
            0x0A => server::Packet::Message(state::message(body).ok_or(Error::malformed(
                Flow::Clientbound,
                opcode,
                Problem::Message,
            ))?),
            0x2E => server::Packet::TravelMenu(travel::menu(body).ok_or(Error::malformed(
                Flow::Clientbound,
                opcode,
                Problem::TravelMenu,
            ))?),
            0x0F => server::Packet::InventoryItem(inventory::item(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::Inventory),
            )?),
            0x10 => server::Packet::InventoryCleared(inventory::cleared(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::Inventory),
            )?),
            0x13 => server::Packet::ActorVitality(state::actor_vitality(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::ActorVitality),
            )?),
            0x17 => server::Packet::SpellbookEntry(state::spellbook(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::Spellbook),
            )?),
            0x1A => server::Packet::ActorAction(state::actor_action(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::ActorAction),
            )?),
            0x37 => server::Packet::EquipmentItem(equipment::item(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::Equipment),
            )?),
            0x38 => server::Packet::EquipmentCleared(equipment::cleared(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::Equipment),
            )?),
            0x39 => server::Packet::CharacterProfile(profile::character(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::CharacterProfile),
            )?),
            0x0C => server::Packet::EntityMovement(entity::movement(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::Movement),
            )?),
            0x0E => server::Packet::EntityRemoval(entity::removal(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::Removal),
            )?),
            0x15 => server::Packet::MapContext(map::context(body).ok_or(Error::malformed(
                Flow::Clientbound,
                opcode,
                Problem::MapContext,
            ))?),
            0x58 => server::Packet::EntityControl(entity::control(body).ok_or(
                Error::malformed(Flow::Clientbound, opcode, Problem::EntityControl),
            )?),
            0x68 => server::Packet::Heartbeat(heartbeat::ping(body).ok_or(Error::malformed(
                Flow::Clientbound,
                opcode,
                Problem::Heartbeat,
            ))?),
            _ => server::Packet::Unknown(packet::Unknown::new(opcode, body)),
        };

    Ok(packet)
}

fn decode_serverbound(opcode: u8, body: &[u8]) -> Result<client::Packet, Error> {
    let packet =
        match opcode {
            0x0E => client::Packet::Speech(action::speech(body).ok_or(Error::malformed(
                Flow::Serverbound,
                opcode,
                Problem::SpeechAction,
            ))?),
            0x07 => client::Packet::Pickup(action::pickup(body).ok_or(Error::malformed(
                Flow::Serverbound,
                opcode,
                Problem::PickupAction,
            ))?),
            0x0B => client::Packet::Disconnect(action::disconnect(body).ok_or(
                Error::malformed(Flow::Serverbound, opcode, Problem::DisconnectAction),
            )?),
            0x06 | 0x32 => client::Packet::Movement(action::movement(body).ok_or(
                Error::malformed(Flow::Serverbound, opcode, Problem::MovementAction),
            )?),
            0x0F => client::Packet::Cast(action::cast(body).ok_or(Error::malformed(
                Flow::Serverbound,
                opcode,
                Problem::CastAction,
            ))?),
            0x39 => client::Packet::Dialog(action::dialog(body).ok_or(Error::malformed(
                Flow::Serverbound,
                opcode,
                Problem::DialogAction,
            ))?),
            0x3F => client::Packet::TravelSelection(action::travel_selection(body).ok_or(
                Error::malformed(Flow::Serverbound, opcode, Problem::TravelSelectionAction),
            )?),
            0x43 => client::Packet::Interact(action::interact(body).ok_or(Error::malformed(
                Flow::Serverbound,
                opcode,
                Problem::InteractAction,
            ))?),
            0x11 => client::Packet::Facing(action::facing(body).ok_or(Error::malformed(
                Flow::Serverbound,
                opcode,
                Problem::FacingAction,
            ))?),
            0x13 => client::Packet::Attack(action::attack(body).ok_or(Error::malformed(
                Flow::Serverbound,
                opcode,
                Problem::AttackAction,
            ))?),
            0x1C => client::Packet::UseInventory(action::use_inventory(body).ok_or(
                Error::malformed(Flow::Serverbound, opcode, Problem::UseInventoryAction),
            )?),
            0x38 => client::Packet::Refresh(action::refresh(body).ok_or(Error::malformed(
                Flow::Serverbound,
                opcode,
                Problem::RefreshAction,
            ))?),
            0x69 => client::Packet::Obstruction(action::obstruction(body).ok_or(
                Error::malformed(Flow::Serverbound, opcode, Problem::ObstructionAction),
            )?),
            0x75 => client::Packet::Heartbeat(heartbeat::pong(body).ok_or(Error::malformed(
                Flow::Serverbound,
                opcode,
                Problem::Heartbeat,
            ))?),
            _ => client::Packet::Unknown(packet::Unknown::new(opcode, body)),
        };

    Ok(packet)
}

/// Why a known packet body failed structural validation.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum Problem {
    /// Invalid map-context dimensions or variable text layout.
    #[error("invalid map context layout")]
    MapContext,
    /// Invalid map rectangle dimensions, records, or opaque tail.
    #[error("invalid map region layout")]
    MapRegion,
    /// Invalid player position seed or authoritative result.
    #[error("invalid player location layout")]
    Location,
    /// Invalid flag-controlled player status blocks.
    #[error("invalid player status layout")]
    Status,
    /// Invalid visible-entity batch.
    #[error("invalid entity appearance layout")]
    Appearances,
    /// Invalid one-tile entity movement.
    #[error("invalid entity movement layout")]
    Movement,
    /// Invalid stable entity removal.
    #[error("invalid entity removal layout")]
    Removal,
    /// Invalid unresolved visibility-control observation.
    #[error("invalid entity control layout")]
    EntityControl,
    /// Invalid heartbeat challenge or response.
    #[error("invalid heartbeat layout")]
    Heartbeat,
    /// Invalid client movement request.
    #[error("invalid client movement action layout")]
    MovementAction,
    /// Invalid client speech action.
    #[error("invalid client speech action")]
    SpeechAction,
    /// Invalid direction-only facing request.
    #[error("invalid client facing action layout")]
    FacingAction,
    /// Invalid ordinary attack request.
    #[error("invalid client attack action layout")]
    AttackAction,
    /// Invalid spell invocation request.
    #[error("invalid client cast action layout")]
    CastAction,
    /// Invalid client dialog action.
    #[error("invalid client dialog action")]
    DialogAction,
    /// Invalid client travel-menu selection.
    #[error("invalid client travel selection action")]
    TravelSelectionAction,
    /// Invalid client entity interaction request.
    #[error("invalid client interaction action")]
    InteractAction,
    /// Invalid client movement-obstruction report.
    #[error("invalid client movement obstruction layout")]
    ObstructionAction,
    /// Invalid spellbook slot update.
    #[error("invalid spellbook entry layout")]
    Spellbook,
    /// Invalid inventory slot update.
    #[error("invalid inventory item layout")]
    Inventory,
    /// Invalid equipment item or slot-clear layout.
    #[error("invalid equipment layout")]
    Equipment,
    /// Invalid detailed character/equipment profile.
    #[error("invalid character profile layout")]
    CharacterProfile,
    /// Invalid pickup action layout.
    #[error("invalid pickup action layout")]
    PickupAction,
    /// Invalid clean-disconnect action layout.
    #[error("invalid client disconnect action layout")]
    DisconnectAction,
    /// Invalid inventory-use action layout.
    #[error("invalid inventory-use action layout")]
    UseInventoryAction,
    /// Invalid visible-map refresh action layout.
    #[error("invalid client refresh action layout")]
    RefreshAction,
    /// Invalid actor animation/action state.
    #[error("invalid actor action layout")]
    ActorAction,
    /// Invalid actor health-bar effect.
    #[error("invalid actor vitality layout")]
    ActorVitality,
    /// Invalid server message.
    #[error("invalid server message layout")]
    Message,
    /// Invalid server travel menu.
    #[error("invalid server travel-menu layout")]
    TravelMenu,
}

/// A plaintext body decoding failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum Error {
    /// No opcode was available.
    #[error("cannot decode an empty plaintext body")]
    Empty,
    /// A known opcode did not satisfy its promoted structure.
    #[error("malformed {flow:?} opcode 0x{opcode:02X}: {problem}")]
    Malformed {
        /// Direction of the rejected body.
        flow: Flow,
        /// Known opcode that was rejected.
        opcode: u8,
        /// Structural family that failed.
        problem: Problem,
    },
}

impl Error {
    const fn malformed(flow: Flow, opcode: u8, problem: Problem) -> Self {
        Self::Malformed {
            flow,
            opcode,
            problem,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::packet::HasOpaqueTail;
    use crate::player as protocol_player;
    use crate::primitive::Position;

    use super::*;

    fn body(value: &str) -> Vec<u8> {
        hex::decode(value).expect("test fixture contains valid hex")
    }

    #[test]
    fn decodes_captured_map_context_and_position_seed() {
        let packet = decode(
            Flow::Clientbound,
            &body("1512670011001105000757656c636f6d6500e80002020200"),
        )
        .expect("captured map context is structurally valid");
        let packet::Packet::Clientbound(server::Packet::MapContext(context)) = packet else {
            panic!("decoder returned a different promoted family");
        };

        assert_eq!(context.id().value(), 0x1267);
        assert_eq!(
            (context.dimensions().width(), context.dimensions().height()),
            (17, 17)
        );
        assert_eq!(context.title(), "Welcome");
        assert_eq!(context.light(), 0x00e8);

        let packet = decode(Flow::Clientbound, &body("04000300010003000100410000"))
            .expect("captured location seed is structurally valid");
        let packet::Packet::Clientbound(server::Packet::PlayerLocation(location)) = packet else {
            panic!("decoder returned a different promoted family");
        };

        assert!(matches!(location, protocol_player::Location::Seed { .. }));
        assert_eq!(location.position(), Position::new(3, 1));
        assert_eq!(location.viewport(), Position::new(3, 1));
    }

    #[test]
    fn promotes_captured_travel_menu_opening() {
        let body = body("2e05776d6b7275080402d60236064b75676e6165000003f30012000d00");
        let packet = decode(Flow::Clientbound, &body)
            .expect("captured travel menu boundary is structurally valid");

        let packet::Packet::Clientbound(server::Packet::TravelMenu(menu)) = packet else {
            panic!("expected typed server travel-menu event");
        };
        assert_eq!(menu.body().as_slice(), body);
    }

    #[test]
    fn status_blocks_are_coherent_with_their_flags() {
        let packet = decode(
            Flow::Clientbound,
            &body("087900000200030000009700000062040403030400006208a016000000001b000000970000006200000259000000000000000000000001000063bd0000012b62000066276f00"),
        )
        .expect("captured full status is structurally valid");
        let packet::Packet::Clientbound(server::Packet::PlayerStatus(status)) = packet else {
            panic!("decoder returned a different promoted family");
        };

        let resources = status
            .resources()
            .expect("resource flag establishes this block");
        let full = status.full().expect("full flag establishes this block");

        assert_eq!((resources.vita(), resources.mana()), (151, 98));
        assert_eq!((full.max_vita(), full.max_mana()), (151, 98));
        assert_eq!(full.level(), 3);
    }

    #[test]
    fn region_is_row_major_and_retains_only_the_tail() {
        let packet = decode(
            Flow::Clientbound,
            &body("0600000b000e0102003b0000038a002e000100000b030b00"),
        )
        .expect("captured-style strip is structurally valid");
        let packet::Packet::Clientbound(server::Packet::MapRegion(region)) = packet else {
            panic!("decoder returned a different promoted family");
        };

        assert_eq!(region.origin(), Position::new(11, 14));
        assert_eq!((region.size().width(), region.size().height()), (1, 2));
        assert_eq!(region.tiles()[0].ground_id(), 0x003b);
        assert_eq!(region.tiles()[0].object_id(), 0x038a);
        assert_eq!(region.tiles()[1].pass_value(), 1);
        assert_eq!(region.opaque_tail().as_slice(), [0x0b, 0x03, 0x0b, 0x00]);
    }

    #[test]
    fn unknown_packet_retains_the_exact_body() {
        let original = body("aa01020304");
        let packet = decode(Flow::Clientbound, &original).expect("unknown bodies are values");
        let packet::Packet::Clientbound(server::Packet::Unknown(unknown)) = packet else {
            panic!("decoder returned a different promoted family");
        };

        assert_eq!(unknown.opcode(), 0xaa);
        assert_eq!(unknown.body().as_slice(), original);
    }

    #[test]
    fn malformed_known_packet_is_not_downgraded_to_unknown() {
        let error = decode(Flow::Clientbound, &[0x15, 0x00]).expect_err("layout must fail");

        assert_eq!(
            error,
            Error::Malformed {
                flow: Flow::Clientbound,
                opcode: 0x15,
                problem: Problem::MapContext,
            }
        );
    }

    #[test]
    fn decodes_high_confidence_client_actions() {
        let speech = decode(
            Flow::Serverbound,
            &body("0e001d492077696c6c206465706f73697420616c6c2047696e6b6f20776f6f6400"),
        )
        .expect("captured deposit speech is structurally valid");
        let packet::Packet::Serverbound(client::Packet::Speech(speech)) = speech else {
            panic!("expected client speech");
        };
        assert_eq!(speech.channel(), 0);
        assert_eq!(speech.text(), "I will deposit all Ginko wood");

        let interact = decode(Flow::Serverbound, &body("430100002e7800"))
            .expect("captured bank NPC interaction is structurally valid");
        let packet::Packet::Serverbound(client::Packet::Interact(interact)) = interact else {
            panic!("expected entity interaction");
        };
        assert_eq!(interact.entity().value(), 0x2e78);

        let dialog = decode(
            Flow::Serverbound,
            &body("390100002e78004c0d59656c6c6f77207363726f6c6c013100"),
        )
        .expect("captured scroll-purchase confirmation is structurally valid");
        let packet::Packet::Serverbound(client::Packet::Dialog(dialog)) = dialog else {
            panic!("expected NPC dialog");
        };
        assert_eq!(dialog.entity().value(), 0x2e78);
        assert_eq!(dialog.command(), 0x4c);
        assert_eq!(dialog.argument(), Some("Yellow scroll"));
        assert_eq!(dialog.tail().as_slice(), [1, b'1', 0]);

        let selection = decode(Flow::Serverbound, &body("3f03f30012000d00"))
            .expect("captured travel-menu selection is structurally valid");
        let packet::Packet::Serverbound(client::Packet::TravelSelection(selection)) = selection
        else {
            panic!("expected travel-menu selection");
        };
        assert_eq!(selection.map().value(), 0x03f3);
        assert_eq!(selection.position(), Position::new(18, 13));

        let movement = decode(
            Flow::Serverbound,
            &body("06019750000b0070001500680111691b00"),
        )
        .expect("captured movement request is structurally valid");
        let packet::Packet::Serverbound(client::Packet::Movement(movement)) = movement else {
            panic!("decoder returned a different client family");
        };

        assert_eq!(movement.direction(), crate::direction::Direction::Right);
        assert_eq!(movement.origin(), Position::new(11, 112));
        assert_eq!(movement.last_walk(), 0x97);

        let attack = decode(Flow::Serverbound, &body("130000"))
            .expect("captured attack request is structurally valid");
        assert!(matches!(
            attack,
            packet::Packet::Serverbound(client::Packet::Attack(_))
        ));

        let facing = decode(Flow::Serverbound, &body("110100"))
            .expect("captured facing request is structurally valid");
        assert!(matches!(
            facing,
            packet::Packet::Serverbound(client::Packet::Facing(_))
        ));

        let refresh = decode(Flow::Serverbound, &body("3800"))
            .expect("captured refresh request is structurally valid");
        assert!(matches!(
            refresh,
            packet::Packet::Serverbound(client::Packet::Refresh(_))
        ));

        let disconnect = decode(Flow::Serverbound, &body("0b00"))
            .expect("captured clean disconnect is structurally valid");
        assert!(matches!(
            disconnect,
            packet::Packet::Serverbound(client::Packet::Disconnect(_))
        ));

        let obstruction = decode(Flow::Serverbound, &body("69000200170000"))
            .expect("captured obstruction report is structurally valid");
        let packet::Packet::Serverbound(client::Packet::Obstruction(obstruction)) = obstruction
        else {
            panic!("decoder returned a different client family");
        };
        assert_eq!(obstruction.origin(), Position::new(2, 23));
        assert_eq!(obstruction.direction(), crate::direction::Direction::Up);
    }

    #[test]
    fn decodes_spellbook_combat_and_message_transaction() {
        let spell = decode(
            Flow::Clientbound,
            &body("17010506536f6f746865024e4f21647900"),
        )
        .expect("captured spellbook entry is valid");
        let packet::Packet::Clientbound(server::Packet::SpellbookEntry(spell)) = spell else {
            panic!("expected spellbook entry");
        };
        assert_eq!((spell.slot(), spell.kind(), spell.name()), (1, 5, "Soothe"));
        assert_eq!(spell.question(), "NO");

        let vitality = decode(Flow::Clientbound, &body("130001f55f0064ffffffce01031900"))
            .expect("captured healing effect is valid");
        let packet::Packet::Clientbound(server::Packet::ActorVitality(vitality)) = vitality else {
            panic!("expected actor vitality");
        };
        assert_eq!(vitality.actor().value(), 0x0001_f55f);
        assert_eq!((vitality.percent(), vitality.amount()), (100, -50));

        let action = decode(Flow::Clientbound, &body("1a0001f55f06001e0061737400"))
            .expect("captured magic action is valid");
        let packet::Packet::Clientbound(server::Packet::ActorAction(action)) = action else {
            panic!("expected actor action");
        };
        assert_eq!(
            (action.actor().value(), action.kind(), action.duration()),
            (0x0001_f55f, 6, 30)
        );

        let message = decode(
            Flow::Clientbound,
            &body("0a030010596f75206361737420536f6f7468652e02020200"),
        )
        .expect("captured cast message is valid");
        let packet::Packet::Clientbound(server::Packet::Message(message)) = message else {
            panic!("expected server message");
        };
        assert_eq!(message.text(), "You cast Soothe.");
    }

    #[test]
    fn decodes_inventory_slot() {
        let packet = decode(
            Flow::Clientbound,
            &body(
                "0f01c0d9000f526162626974206d656174202833290b526162626974206d65617400000003010000000000000096863600",
            ),
        )
        .expect("captured inventory item is valid");
        let packet::Packet::Clientbound(server::Packet::InventoryItem(item)) = packet else {
            panic!("expected inventory item");
        };

        assert_eq!(
            (item.slot(), item.canonical_name(), item.amount()),
            (1, "Rabbit meat", 3)
        );
    }

    #[test]
    fn decodes_woodcut_inventory_and_equipment_lifecycle() {
        let equipped = decode(
            Flow::Clientbound,
            &body("3701c285000341786503417865000f42400000746f2000"),
        )
        .expect("captured equipped axe is valid");
        let packet::Packet::Clientbound(server::Packet::EquipmentItem(axe)) = equipped else {
            panic!("expected equipped item");
        };
        assert_eq!((axe.slot(), axe.canonical_name()), (1, "Axe"));
        assert_eq!(axe.durability(), 1_000_000);

        let equipment_clear = decode(Flow::Clientbound, &body("380100003700"))
            .expect("captured broken-axe clear is valid");
        let packet::Packet::Clientbound(server::Packet::EquipmentCleared(clear)) = equipment_clear
        else {
            panic!("expected equipment clear");
        };
        assert_eq!(clear.slot(), 1);

        let inventory_clear = decode(Flow::Clientbound, &body("100a0c000b06000000"))
            .expect("captured equipped inventory clear is valid");
        let packet::Packet::Clientbound(server::Packet::InventoryCleared(clear)) = inventory_clear
        else {
            panic!("expected inventory clear");
        };
        assert_eq!(clear.slot(), 10);
    }

    #[test]
    fn decodes_captured_character_profile_shape() {
        let mut hex = String::from("396300000000000000000000680750656173616e74");
        hex.push_str(&"00".repeat(140));
        hex.push_str("01000001001018426f726e20696e204879756c203136312c2053756d6d6572567d7400");
        let body = hex::decode(hex).expect("valid profile fixture hex");
        let packet = decode(Flow::Clientbound, &body).expect("profile is structurally valid");
        let packet::Packet::Clientbound(server::Packet::CharacterProfile(profile)) = packet else {
            panic!("expected character profile");
        };

        assert!(profile.equipment().is_empty());
    }

    #[test]
    fn decodes_pickup_and_inventory_use_actions() {
        let pickup =
            decode(Flow::Serverbound, &body("070100")).expect("captured pickup request is valid");
        let packet::Packet::Serverbound(client::Packet::Pickup(pickup)) = pickup else {
            panic!("expected pickup request");
        };
        assert_eq!(pickup.mode(), 1);

        let use_item = decode(Flow::Serverbound, &body("1c0a000702be00"))
            .expect("captured axe activation is valid");
        let packet::Packet::Serverbound(client::Packet::UseInventory(use_item)) = use_item else {
            panic!("expected inventory-use request");
        };
        assert_eq!(use_item.slot(), 10);
        assert_eq!(use_item.actor().value(), 0x0007_02be);
    }
}
