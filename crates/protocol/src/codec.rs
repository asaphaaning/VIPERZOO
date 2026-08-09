//! Decode one bounded plaintext body into a typed protocol packet.
//!
//! The plaintext tap supplies a direction and a body length before it supplies
//! bytes. [`Codec`] preserves that boundary explicitly:
//!
//! ```text
//! tap metadata             tap bytes
//! (flow, length)              │
//!       │                     │
//!       └──────► Codec ◄──────┘
//!                    │
//!                    ▼
//!          direction-specific Packet
//! ```
//!
//! This is not the encrypted socket framing protocol. The client has already
//! delimited and decrypted the body by the time this codec runs. A future
//! socket codec can therefore compose below this layer without changing the
//! packet model.

use std::io;

use bytes::BytesMut;
use thiserror::Error;
use tokio_util::codec::Decoder;
use tracing::instrument;

use crate::{decode, direction::Flow, packet};

pub use crate::decode::{Error as BodyError, Problem};

/// One complete plaintext body already bounded in memory.
///
/// This is the value-shaped counterpart to streaming [`Codec`] use. Converting
/// it into [`packet::Packet`] runs the same length-aware decoder as a tap or
/// asynchronous reader.
///
/// ```
/// use viperzoo_protocol::{
///     codec::Body,
///     direction::Flow,
///     packet,
/// };
///
/// let body = Body::new(Flow::Serverbound, [0x13, 0x00, 0x00]);
/// let packet = packet::Packet::try_from(body)?;
///
/// assert!(matches!(packet, packet::Packet::Serverbound(_)));
/// # Ok::<(), viperzoo_protocol::codec::Error>(())
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Body<B> {
    flow: Flow,
    bytes: B,
}

impl<B> Body<B> {
    /// Associates complete body bytes with their observed direction.
    #[must_use]
    pub const fn new(flow: Flow, bytes: B) -> Self {
        Self { flow, bytes }
    }

    /// Returns the observed body direction.
    #[must_use]
    pub const fn flow(&self) -> Flow {
        self.flow
    }

    /// Returns the stored body representation.
    #[must_use]
    pub const fn bytes(&self) -> &B {
        &self.bytes
    }
}

impl<B> TryFrom<Body<B>> for packet::Packet
where
    B: AsRef<[u8]>,
{
    type Error = Error;

    fn try_from(body: Body<B>) -> Result<Self, Self::Error> {
        let bytes = body.bytes.as_ref();
        let mut source = BytesMut::from(bytes);
        let mut codec = Codec::new(body.flow, bytes.len());

        codec.decode_eof(&mut source)?.ok_or(Error::Incomplete)
    }
}

/// A decoder for exactly one length-bounded plaintext body.
///
/// Construct a fresh codec from the metadata accompanying each tap callback.
/// [`Decoder::decode`] then waits for the complete body, which makes fragmented
/// in-memory and asynchronous reads equivalent to the callback path.
#[derive(Clone, Debug)]
pub struct Codec {
    flow: Flow,
    length: usize,
    state: State,
}

impl Codec {
    /// Creates a decoder for one body with the declared `flow` and `length`.
    #[must_use]
    pub const fn new(flow: Flow, length: usize) -> Self {
        Self {
            flow,
            length,
            state: State::Awaiting,
        }
    }

    /// Returns the direction attached to the bounded body.
    #[must_use]
    pub const fn flow(&self) -> Flow {
        self.flow
    }

    /// Returns the exact number of body bytes this codec accepts.
    #[must_use]
    pub const fn length(&self) -> usize {
        self.length
    }
}

impl Decoder for Codec {
    type Item = packet::Packet;
    type Error = Error;

    #[instrument(
        name = "viperzoo::protocol::codec::decode",
        skip(self, source),
        fields(flow = ?self.flow, declared = self.length, buffered = source.len()),
        err,
        ret(level = "trace")
    )]
    fn decode(&mut self, source: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.state == State::Complete {
            return if source.is_empty() {
                Ok(None)
            } else {
                Err(Error::Additional {
                    length: source.len(),
                })
            };
        }

        if source.len() > self.length {
            return Err(Error::Length {
                declared: self.length,
                available: source.len(),
            });
        }

        if source.len() < self.length {
            source.reserve(self.length - source.len());
            return Ok(None);
        }

        let body = source.split_to(self.length);
        let packet = decode::body(self.flow, &body)?;
        self.state = State::Complete;

        Ok(Some(packet))
    }

    fn decode_eof(&mut self, source: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.state == State::Awaiting && source.len() != self.length {
            return Err(Error::Length {
                declared: self.length,
                available: source.len(),
            });
        }

        self.decode(source)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Awaiting,
    Complete,
}

/// A bounded plaintext body decoding failure.
#[derive(Debug, Error)]
pub enum Error {
    /// The underlying byte source failed.
    #[error("unable to read plaintext body: {0}")]
    Io(#[from] io::Error),
    /// The tap metadata and available body bytes disagreed.
    #[error("plaintext body declared {declared} bytes but supplied {available}")]
    Length {
        /// Length declared by the acquisition boundary.
        declared: usize,
        /// Bytes available for this body.
        available: usize,
    },
    /// Bytes appeared after the single bounded body completed.
    #[error("{length} additional bytes followed the bounded plaintext body")]
    Additional {
        /// Number of unexpected bytes.
        length: usize,
    },
    /// A complete in-memory body did not yield its packet.
    #[error("complete plaintext body produced no protocol packet")]
    Incomplete,
    /// The complete body failed structural protocol decoding.
    #[error(transparent)]
    Body(#[from] BodyError),
}

#[cfg(test)]
mod tests {
    use crate::{packet, server};

    use super::*;

    #[test]
    fn waits_for_the_declared_body_before_decoding() {
        let body = hex::decode("1512670011001105000757656c636f6d6500e80002020200")
            .expect("fixture is valid hex");
        let mut codec = Codec::new(Flow::Clientbound, body.len());
        let mut source = BytesMut::from(&body[..8]);

        assert!(
            codec
                .decode(&mut source)
                .expect("prefix is valid")
                .is_none()
        );

        source.extend_from_slice(&body[8..]);
        let packet = codec
            .decode(&mut source)
            .expect("complete body is valid")
            .expect("complete body produces a packet");

        assert!(matches!(
            packet,
            packet::Packet::Clientbound(server::Packet::MapContext(_))
        ));
        assert!(source.is_empty());
    }

    #[test]
    fn eof_rejects_a_body_shorter_than_its_tap_metadata() {
        let mut codec = Codec::new(Flow::Clientbound, 3);
        let mut source = BytesMut::from(&[0x13, 0x00][..]);

        assert!(matches!(
            codec.decode_eof(&mut source),
            Err(Error::Length {
                declared: 3,
                available: 2
            })
        ));
    }

    #[test]
    fn a_codec_yields_only_its_single_bounded_body() {
        let mut codec = Codec::new(Flow::Serverbound, 3);
        let mut source = BytesMut::from(&[0x13, 0x00, 0x00][..]);

        assert!(codec.decode(&mut source).expect("body is valid").is_some());
        source.extend_from_slice(&[0x13, 0x00, 0x00]);

        assert!(matches!(
            codec.decode(&mut source),
            Err(Error::Additional { length: 3 })
        ));
    }
}
