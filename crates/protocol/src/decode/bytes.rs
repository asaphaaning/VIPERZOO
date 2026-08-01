//! Bounds-checked byte reading shared by packet-family decoders.

pub(super) fn be_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

pub(super) fn be_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

pub(super) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    pub(super) const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(super) fn peek(&self, length: usize) -> Option<&'a [u8]> {
        self.bytes
            .get(self.position..self.position.checked_add(length)?)
    }

    pub(super) fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let bytes = self.peek(length)?;
        self.position += length;

        Some(bytes)
    }

    pub(super) fn take_u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    pub(super) fn take_u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    pub(super) fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.position..]
    }
}
