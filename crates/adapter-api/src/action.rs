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
    /// Submit an interaction and its ordered selections as one response-gated
    /// dialog transaction.
    ///
    /// The selections are all addressed to the transaction's entity. Adapters
    /// preserve their order across client-owned transport boundaries and do
    /// not advance response-dependent selections before the server dialog.
    DialogTransaction(DialogTransaction),
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

/// One ordered NPC interaction and its follow-up menu selections.
///
/// A dialog transaction begins with client opcode `0x43` and continues with
/// one or more `0x39` selections. [`DialogTransaction::new`] requires every
/// selection to target the same entity, making it impossible to enqueue an
/// interleaved conversation through this action. Adapters pace the interaction
/// and gate later selections on the matching server dialog response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogTransaction {
    entity: EntityId,
    selections: Box<[DialogSelection]>,
}

impl DialogTransaction {
    /// Creates one non-empty, entity-consistent dialog transaction.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DialogTransactionEmpty`] when no selection is supplied
    /// or [`Error::DialogTransactionEntity`] when a selection targets another
    /// entity.
    pub fn new(
        entity: EntityId,
        selections: impl IntoIterator<Item = DialogSelection>,
    ) -> Result<Self, Error> {
        let selections = selections.into_iter().collect::<Box<[_]>>();

        if selections.is_empty() {
            return Err(Error::DialogTransactionEmpty);
        }

        if selections
            .iter()
            .any(|selection| selection.entity() != entity)
        {
            return Err(Error::DialogTransactionEntity);
        }

        Ok(Self { entity, selections })
    }

    /// Returns the NPC that owns this interaction and every selection.
    #[must_use]
    pub const fn entity(&self) -> EntityId {
        self.entity
    }

    /// Borrows the selections in their required transport order.
    #[must_use]
    pub fn selections(&self) -> &[DialogSelection] {
        &self.selections
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

/// How many units a shop or bank [`Command`] applies to.
///
/// The three forms are distinct grammar, not a count with special values, so
/// they are variants rather than a `u16` where `0` would have to mean "all".
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Quantity {
    /// A single unit, rendered as the bare command form.
    #[default]
    One,
    /// An explicit count, rendered as a trailing `number (count)`.
    Exactly(u16),
    /// Every unit the shop or bank will accept, rendered as a leading `all`.
    All,
}

impl Quantity {
    /// Every [`Quantity`], in declaration order.
    pub const VARIANTS: [Self; 3] = [Self::One, Self::Exactly(1), Self::All];
}

/// A spoken shop or bank command the server parses from ordinary speech.
///
/// These lines are public speech that the server interprets as transactions.
/// All four operations share one grammar, which is why they are one type:
///
/// ```text
/// <prefix> [all] <item> [number <count>] [suffix]
/// ```
///
/// Rendering them from a closed vocabulary keeps the exact wording in one
/// place and makes the quantity forms reachable without re-deriving their
/// syntax at each call site.
///
/// # Evidence
///
/// The wording follows community documentation of the live command set, not
/// packets captured from this server. A rendered command is therefore a
/// *candidate*: callers must confirm the effect through authoritative
/// inventory or bank projection, because dispatching speech is never evidence
/// that a transaction completed.
///
/// ```
/// use viperzoo_adapter_api::action::{Command, Quantity};
///
/// assert_eq!(
///     Command::Buy { item: "Axe", quantity: Quantity::Exactly(3) }.line(),
///     "I buy Axe number 3"
/// );
/// assert_eq!(
///     Command::Buy { item: "Axe", quantity: Quantity::One }.line(),
///     "I buy Axe"
/// );
/// assert_eq!(
///     Command::Deposit { item: "Ginko wood", quantity: Quantity::All }.line(),
///     "I will deposit all Ginko wood"
/// );
/// assert_eq!(
///     Command::Withdraw { item: "Yellow scroll", quantity: Quantity::Exactly(2) }.line(),
///     "Give my Yellow scroll number 2 back"
/// );
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command<'a> {
    /// Purchase from a shop keeper.
    Buy {
        /// Item name as the server spells it.
        item: &'a str,
        /// How many units to request.
        quantity: Quantity,
    },
    /// Sell to a shop keeper, whose command reads from their perspective.
    Sell {
        /// Item name as the server spells it.
        item: &'a str,
        /// How many units to offer.
        quantity: Quantity,
    },
    /// Store items with a bank attendant.
    Deposit {
        /// Item name as the server spells it.
        item: &'a str,
        /// How many units to store.
        quantity: Quantity,
    },
    /// Reclaim stored items from a bank attendant.
    Withdraw {
        /// Item name as the server spells it.
        item: &'a str,
        /// How many units to reclaim.
        quantity: Quantity,
    },
}

impl Command<'_> {
    /// Returns the command's leading words and any trailing word.
    const fn frame(&self) -> (&'static str, &'static str) {
        match self {
            Self::Buy { .. } => ("I buy", ""),
            Self::Sell { .. } => ("Buy my", ""),
            Self::Deposit { .. } => ("I will deposit", ""),
            Self::Withdraw { .. } => ("Give my", "back"),
        }
    }

    /// Returns the item and quantity this command applies to.
    const fn subject(&self) -> (&str, Quantity) {
        match self {
            Self::Buy { item, quantity }
            | Self::Sell { item, quantity }
            | Self::Deposit { item, quantity }
            | Self::Withdraw { item, quantity } => (item, *quantity),
        }
    }

    /// Renders the command as the line a player would speak.
    #[must_use]
    pub fn line(&self) -> String {
        let (prefix, suffix) = self.frame();
        let (item, quantity) = self.subject();
        let mut line = String::from(prefix);

        if matches!(quantity, Quantity::All) {
            line.push_str(" all");
        }

        line.push(' ');
        line.push_str(item);

        if let Quantity::Exactly(count) = quantity {
            line.push_str(" number ");
            line.push_str(&count.to_string());
        }

        if !suffix.is_empty() {
            line.push(' ');
            line.push_str(suffix);
        }

        line
    }

    /// Renders the command as validated public speech.
    ///
    /// # Errors
    ///
    /// Rejects an item name that would produce empty, non-ASCII,
    /// NUL-containing, or over-255-byte speech.
    pub fn speak(&self) -> Result<Speech, Error> {
        Speech::say(&self.line())
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
    /// A dialog transaction must include at least one selection.
    #[error("dialog transaction must include at least one selection")]
    DialogTransactionEmpty,
    /// Every dialog transaction selection must target its interaction entity.
    #[error("dialog transaction selections must target the interaction entity")]
    DialogTransactionEntity,
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
    fn dialog_transaction_is_non_empty_and_entity_consistent() {
        let attendant = EntityId::new(0x2e78);
        let stranger = EntityId::new(0x2c08);

        assert_eq!(
            DialogTransaction::new(attendant, []),
            Err(Error::DialogTransactionEmpty)
        );
        assert_eq!(
            DialogTransaction::new(attendant, [DialogSelection::option(stranger, 0x40)]),
            Err(Error::DialogTransactionEntity)
        );

        let transaction = DialogTransaction::new(
            attendant,
            [
                DialogSelection::option(attendant, 0x40),
                DialogSelection::text(attendant, 0x4a, "Yellow scroll")
                    .expect("captured item name is valid"),
            ],
        )
        .expect("all selections target the attendant");

        assert_eq!(transaction.entity(), attendant);
        assert_eq!(transaction.selections().len(), 2);
        assert_eq!(transaction.selections()[1].command(), 0x4a);
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
