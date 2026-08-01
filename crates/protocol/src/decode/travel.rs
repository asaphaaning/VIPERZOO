//! Server travel-menu boundary validation.

use crate::travel;

/// Promotes the captured `0x2E` family to an explicit menu-open event.
///
/// The destination-list body is retained untouched by [`travel::Menu`] until
/// additional samples establish its variable record grammar.
pub(super) fn menu(body: &[u8]) -> Option<travel::Menu> {
    (body.first() == Some(&0x2e)).then(|| travel::Menu::from_body(body))
}
