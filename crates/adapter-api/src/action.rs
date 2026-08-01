//! Express script intent in terms the normal client can perform.
//!
//! These values are semantic requests—move, speak, select a dialog response—
//! rather than raw protocol bytes. An adapter is responsible for turning a
//! valid [`Action`] into the client's own synchronized action path. This keeps
//! session crypto, framing, and client-specific layouts outside scripts and
//! lets policy work with more than one acquisition implementation.

use thiserror::Error;
use viperzoo_protocol::{
    direction::Direction,
    primitive::{EntityId, MapId},
};

/// One intent delegated to the normal game client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// Face and move one tile through the client's own movement builder.
    Step(Direction),
    /// Face one direction through the client's synchronized protocol session.
    Face(Direction),
    /// Ask the client to refresh its current map projection.
    RefreshMap,
    /// Dismiss the current client dialog or overlay with Escape.
    DismissOverlay,
    /// Ask the server for the player's current character/equipment profile.
    RequestProfile,
    /// Cast the spell in a validated spellbook slot.
    Cast(SpellSlot),
    /// Cast a question spell with a validated ASCII answer.
    CastAnswered(AnsweredSpell),
    /// Swing the equipped weapon in a required direction through the client's
    /// synchronized protocol session.
    Attack(Direction),
    /// Attempt to pick up an item through the client's synchronized protocol session.
    Pickup,
    /// Activate an item in a validated inventory slot.
    UseInventory(InventorySlot),
    /// Interact with a projected entity through client opcode `0x43`.
    Interact(EntityId),
    /// Submit one validated NPC dialog selection through client opcode `0x39`.
    Dialog(DialogSelection),
    /// Send one validated public speech line through client opcode `0x0E`.
    Speak(Speech),
    /// Select one map from an already-open native client travel menu.
    ///
    /// The adapter rejects this action when no active menu contains the map.
    Travel(MapId),
    /// Select one map after the next native client travel menu opens.
    ///
    /// The server-authored menu row owns the landing coordinate. Scripts name
    /// only the stable destination identity and cannot accidentally reject a
    /// valid row when the server changes that coordinate.
    TravelOnMenu(MapId),
    /// Select whether map cache checksums may suppress server tile payloads.
    MapData(MapData),
}

/// One NPC dialog response with an optional ASCII argument.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogSelection {
    entity: EntityId,
    command: u8,
    argument: Option<Box<[u8]>>,
    quantity: Option<u8>,
}

impl DialogSelection {
    /// Creates a command-only menu selection.
    #[must_use]
    pub const fn option(entity: EntityId, command: u8) -> Self {
        Self {
            entity,
            command,
            argument: None,
            quantity: None,
        }
    }

    /// Creates a menu selection carrying one bounded ASCII argument.
    ///
    /// # Errors
    ///
    /// Rejects empty, non-ASCII, NUL-containing, or over-255-byte arguments.
    pub fn text(entity: EntityId, command: u8, argument: &str) -> Result<Self, Error> {
        if argument.is_empty()
            || argument.len() > usize::from(u8::MAX)
            || !argument.is_ascii()
            || argument.as_bytes().contains(&0)
        {
            return Err(Error::DialogText);
        }

        Ok(Self {
            entity,
            command,
            argument: Some(argument.as_bytes().into()),
            quantity: None,
        })
    }

    /// Creates a shop confirmation carrying an item name and non-zero quantity.
    ///
    /// # Errors
    ///
    /// Rejects an invalid item-name field or a zero quantity.
    pub fn purchase(
        entity: EntityId,
        command: u8,
        argument: &str,
        quantity: u8,
    ) -> Result<Self, Error> {
        if quantity == 0 {
            return Err(Error::DialogQuantity);
        }

        let mut selection = Self::text(entity, command, argument)?;
        selection.quantity = Some(quantity);
        Ok(selection)
    }

    /// Returns the target NPC identifier.
    #[must_use]
    pub const fn entity(&self) -> EntityId {
        self.entity
    }

    /// Returns the server-authored menu command token.
    #[must_use]
    pub const fn command(&self) -> u8 {
        self.command
    }

    /// Borrows the optional argument bytes.
    #[must_use]
    pub fn argument(&self) -> Option<&[u8]> {
        self.argument.as_deref()
    }

    /// Returns the optional shop quantity.
    #[must_use]
    pub const fn quantity(&self) -> Option<u8> {
        self.quantity
    }
}

/// One bounded ASCII speech line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Speech {
    channel: u8,
    text: Box<[u8]>,
}

impl Speech {
    /// Creates one public-channel speech line.
    ///
    /// # Errors
    ///
    /// Rejects empty, non-ASCII, NUL-containing, or over-255-byte text.
    pub fn say(text: &str) -> Result<Self, Error> {
        Self::new(0, text)
    }

    /// Creates one speech line for an explicit client channel.
    ///
    /// # Errors
    ///
    /// Rejects empty, non-ASCII, NUL-containing, or over-255-byte text.
    pub fn new(channel: u8, text: &str) -> Result<Self, Error> {
        let text = validated_ascii(text, Error::SpeechText)?;
        Ok(Self {
            channel,
            text: text.into(),
        })
    }

    /// Returns the encoded speech channel.
    #[must_use]
    pub const fn channel(&self) -> u8 {
        self.channel
    }

    /// Borrows the encoded text bytes.
    #[must_use]
    pub fn text(&self) -> &[u8] {
        &self.text
    }
}

/// One question spell paired with its protocol answer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnsweredSpell {
    slot: SpellSlot,
    answer: Box<[u8]>,
}

impl AnsweredSpell {
    /// Creates a spell answer accepted by the client protocol.
    ///
    /// # Errors
    ///
    /// Rejects empty, non-ASCII, NUL-containing, or over-255-byte answers.
    pub fn new(slot: SpellSlot, answer: &str) -> Result<Self, Error> {
        let answer = validated_ascii(answer, Error::SpellAnswer)?;
        Ok(Self {
            slot,
            answer: answer.into(),
        })
    }

    /// Returns the selected spellbook slot.
    #[must_use]
    pub const fn slot(&self) -> SpellSlot {
        self.slot
    }

    /// Borrows the answer bytes without the wire terminator.
    #[must_use]
    pub fn answer(&self) -> &[u8] {
        &self.answer
    }
}

fn validated_ascii(value: &str, error: Error) -> Result<&[u8], Error> {
    if value.is_empty()
        || value.len() > usize::from(u8::MAX)
        || !value.is_ascii()
        || value.as_bytes().contains(&0)
    {
        return Err(error);
    }

    Ok(value.as_bytes())
}

/// A one-based inventory slot accepted by the `NexusTK` letter-key path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InventorySlot(u8);

impl InventorySlot {
    /// First valid inventory slot.
    pub const MIN: u8 = 1;
    /// Last slot addressable through the client's 26 letter keys.
    pub const MAX: u8 = 26;

    /// Creates a validated one-based inventory slot.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InventorySlot`] outside `1..=26`.
    pub const fn new(value: u8) -> Result<Self, Error> {
        if value < Self::MIN || value > Self::MAX {
            return Err(Error::InventorySlot(value));
        }

        Ok(Self(value))
    }

    /// Returns the one-based slot value.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

/// Map-region cache-hint policy at the client plaintext boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapData {
    /// Preserve the normal client's cached-region checksums.
    PreserveCache,
    /// Zero cache hints so observed scrolling strips are returned and projected.
    ForceResponse,
}

/// Asynchronous client action boundary implemented by acquisition adapters.
pub trait Client: Send + Sync {
    /// Adapter-specific submission failure.
    type Error;

    /// Delegates an action to an attached client-owned transport session.
    fn perform(&self, action: Action) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// A one-based spellbook slot accepted by the `NexusTK` client hotkey path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpellSlot(u8);

impl SpellSlot {
    /// First valid spellbook slot.
    pub const MIN: u8 = 1;
    /// Last valid spellbook slot.
    pub const MAX: u8 = 26;

    /// Creates a validated one-based slot.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SpellSlot`] outside `1..=26`.
    pub const fn new(value: u8) -> Result<Self, Error> {
        if value < Self::MIN || value > Self::MAX {
            return Err(Error::SpellSlot(value));
        }

        Ok(Self(value))
    }

    /// Returns the one-based slot value.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

/// Invalid action input.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum Error {
    /// Spell slots are one-based and limited to the client's 26 letter keys.
    #[error("spell slot {0} is outside 1..=26")]
    SpellSlot(u8),
    /// Inventory slots are one-based and limited to the client's letter keys.
    #[error("inventory slot {0} is outside 1..=26")]
    InventorySlot(u8),
    /// Dialog text is encoded as one non-empty ASCII u8-length field.
    #[error("dialog text must be 1..=255 ASCII bytes without NUL")]
    DialogText,
    /// Shop quantities are non-zero.
    #[error("dialog quantity must be non-zero")]
    DialogQuantity,
    /// Speech uses one non-empty ASCII u8-length field.
    #[error("speech text must be 1..=255 ASCII bytes without NUL")]
    SpeechText,
    /// Question-spell answers are NUL-terminated ASCII.
    #[error("spell answer must be 1..=255 ASCII bytes without NUL")]
    SpellAnswer,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spell_slot_has_no_invalid_public_value() {
        assert!(SpellSlot::new(0).is_err());
        assert_eq!(SpellSlot::new(1).expect("first slot").value(), 1);
        assert_eq!(SpellSlot::new(26).expect("last slot").value(), 26);
        assert!(SpellSlot::new(27).is_err());
    }

    #[test]
    fn inventory_slot_has_no_invalid_public_value() {
        assert!(InventorySlot::new(0).is_err());
        assert_eq!(InventorySlot::new(1).expect("first slot").value(), 1);
        assert_eq!(InventorySlot::new(26).expect("last slot").value(), 26);
        assert!(InventorySlot::new(27).is_err());
    }

    #[test]
    fn dialog_text_rejects_unencodable_values() {
        let entity = EntityId::new(0x2c0b);
        assert!(DialogSelection::text(entity, 0x4a, "").is_err());
        assert!(DialogSelection::text(entity, 0x4a, "økse").is_err());
        assert_eq!(
            DialogSelection::text(entity, 0x4a, "Axe")
                .expect("captured item name is valid")
                .argument(),
            Some(&b"Axe"[..])
        );
        assert!(DialogSelection::purchase(entity, 0x4c, "Axe", 0).is_err());
        assert_eq!(
            DialogSelection::purchase(entity, 0x4c, "Axe", 2)
                .expect("captured purchase is valid")
                .quantity(),
            Some(2)
        );
    }

    #[test]
    fn captured_bank_actions_have_typed_inputs() {
        let speech = Speech::say("I will deposit all Ginko wood").expect("captured speech");
        assert_eq!(speech.channel(), 0);
        assert_eq!(speech.text(), b"I will deposit all Ginko wood");

        let gateway = AnsweredSpell::new(SpellSlot::new(2).expect("slot"), "n").expect("answer");
        assert_eq!(gateway.slot().value(), 2);
        assert_eq!(gateway.answer(), b"n");

        let Action::Travel(map) = Action::Travel(MapId::new(0x03f3)) else {
            panic!("expected map-only travel action");
        };
        assert_eq!(map.value(), 0x03f3);
    }
}
