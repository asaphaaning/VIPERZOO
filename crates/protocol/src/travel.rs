//! Server-provided travel-menu availability.

use serde::Serialize;

use crate::primitive::Body;

/// One opened server travel menu.
///
/// The `0x2E` family is now promoted far enough to establish the semantic
/// action boundary: a client may only submit a destination after this menu is
/// open. Its variable destination-list grammar remains intentionally retained
/// as raw bytes until more than the captured Buya selector variants are
/// available for a total decoder. Independent Buya menus advertised Kugnae
/// Gathering `0x03F3` with entry rows `(18,13)` and `(18,14)`. The map is the
/// stable script intent; entry coordinates belong to the current server row
/// and must be copied by the native selection path rather than hard-coded.
/// NexusTK 752 materializes these as `0x94`-byte rows in selector constructor
/// RVA `0x001C2AC0`; the unrelated RVA `0x0005C1E0` container uses `0x4C`
/// records and is not part of this packet family.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Menu {
    body: Body,
}

impl Menu {
    pub(crate) fn from_body(body: &[u8]) -> Self {
        Self {
            body: Body::from_slice(body),
        }
    }

    /// Returns the original validated plaintext menu body.
    #[must_use]
    pub const fn body(&self) -> &Body {
        &self.body
    }
}
