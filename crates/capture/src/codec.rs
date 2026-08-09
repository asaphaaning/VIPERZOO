//! Frame bidirectional newline-delimited capture evidence.
//!
//! [`Codec`] is deliberately the only owner of JSONL record boundaries. The
//! encoder writes typed [`Frame`] values, while the decoder classifies each
//! complete row into a [`Record`]. Malformed individual rows remain recoverable
//! [`Record::Rejected`] values instead of terminating the evidence stream.

use std::io;

use bytes::BytesMut;
use serde::Serialize;
use thiserror::Error;
use tokio_util::codec::{Decoder, Encoder};
use tracing::instrument;

use crate::{
    frame::{Frame, Operation},
    line::Line,
    record::{self, Record},
};

/// Bidirectional framing for the capture JSONL vocabulary.
#[derive(Clone, Debug)]
pub struct Codec {
    line: Option<Line>,
}

impl Codec {
    /// Starts a new capture stream at [`Line::FIRST`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            line: Some(Line::FIRST),
        }
    }

    /// Returns the identity of the next row, if the line space remains.
    #[must_use]
    pub const fn next_line(&self) -> Option<Line> {
        self.line
    }

    fn take_line(&mut self) -> Result<Line, Error> {
        let line = self.line.ok_or(Error::LineCapacity)?;
        self.line = line.next();

        Ok(line)
    }

    fn classify(&mut self, mut row: BytesMut) -> Result<Record, Error> {
        if row.last() == Some(&b'\n') {
            row.truncate(row.len() - 1);
        }
        if row.last() == Some(&b'\r') {
            row.truncate(row.len() - 1);
        }

        Ok(record::classify(self.take_line()?, &row))
    }
}

impl Default for Codec {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for Codec {
    type Item = Record;
    type Error = Error;

    #[instrument(
        name = "viperzoo::capture::codec::decode",
        skip(self, source),
        fields(line = ?self.line, buffered = source.len()),
        err,
        ret(level = "trace")
    )]
    fn decode(&mut self, source: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let Some(boundary) = source.iter().position(|byte| *byte == b'\n') else {
            return Ok(None);
        };
        let row = source.split_to(boundary + 1);

        self.classify(row).map(Some)
    }

    fn decode_eof(&mut self, source: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(source)? {
            Some(record) => Ok(Some(record)),
            None if source.is_empty() => Ok(None),
            None => {
                let row = source.split();
                self.classify(row).map(Some)
            }
        }
    }
}

impl Encoder<Frame<'_>> for Codec {
    type Error = Error;

    #[instrument(
        name = "viperzoo::capture::codec::encode",
        skip(self, frame, destination),
        err
    )]
    fn encode(&mut self, frame: Frame<'_>, destination: &mut BytesMut) -> Result<(), Self::Error> {
        let row = Row::from(frame);
        let encoded = serde_json::to_vec(&row).map_err(Error::Encode)?;

        destination.reserve(encoded.len() + 1);
        destination.extend_from_slice(&encoded);
        destination.extend_from_slice(b"\n");

        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum Row<'a> {
    CaptureSessionStart {
        target_pid: u32,
        captured_unix_ms: u128,
    },
    Packet {
        direction: Direction,
        length: usize,
        hex: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<u32>,
        captured_unix_ms: u128,
    },
    TransportClosed {
        source: &'a str,
        captured_unix_ms: u128,
    },
    TransportFault {
        operation: OperationRow,
        code: i32,
        captured_unix_ms: u128,
    },
}

impl<'a> From<Frame<'a>> for Row<'a> {
    fn from(frame: Frame<'a>) -> Self {
        match frame {
            Frame::SessionStarted {
                target_pid,
                captured_unix_ms,
            } => Self::CaptureSessionStart {
                target_pid,
                captured_unix_ms,
            },
            Frame::Packet {
                flow,
                body,
                thread_id,
                captured_unix_ms,
            } => Self::Packet {
                direction: Direction::from(flow),
                length: body.len(),
                hex: hex::encode(body),
                thread_id,
                captured_unix_ms,
            },
            Frame::TransportClosed {
                source,
                captured_unix_ms,
            } => Self::TransportClosed {
                source,
                captured_unix_ms,
            },
            Frame::TransportFault {
                operation,
                code,
                captured_unix_ms,
            } => Self::TransportFault {
                operation: OperationRow::from(operation),
                code,
                captured_unix_ms,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum Direction {
    Incoming,
    Outgoing,
}

impl From<viperzoo_protocol::direction::Flow> for Direction {
    fn from(flow: viperzoo_protocol::direction::Flow) -> Self {
        match flow {
            viperzoo_protocol::direction::Flow::Clientbound => Self::Incoming,
            viperzoo_protocol::direction::Flow::Serverbound => Self::Outgoing,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum OperationRow {
    Receive,
    Send,
}

impl From<Operation> for OperationRow {
    fn from(operation: Operation) -> Self {
        match operation {
            Operation::Receive => Self::Receive,
            Operation::Send => Self::Send,
        }
    }
}

/// A fatal capture framing failure.
#[derive(Debug, Error)]
pub enum Error {
    /// The underlying byte stream failed.
    #[error("capture stream I/O failed: {0}")]
    Io(#[from] io::Error),
    /// A typed evidence frame could not be represented as JSON.
    #[error("unable to encode capture frame: {0}")]
    Encode(#[source] serde_json::Error),
    /// The stream exceeded the complete `u64` line identity space.
    #[error("capture exceeds the supported line identity space")]
    LineCapacity,
}

#[cfg(test)]
mod tests {
    use viperzoo_protocol::{direction::Flow, packet};

    use super::*;

    #[test]
    fn fragmented_rows_wait_for_the_newline_boundary() {
        let mut codec = Codec::new();
        let mut source = BytesMut::from(
            &br#"{"type":"packet","direction":"outgoing","length":3,"hex":"130000"}"#[..],
        );

        assert!(
            codec
                .decode(&mut source)
                .expect("prefix is valid")
                .is_none()
        );

        source.extend_from_slice(b"\n");
        let record = codec
            .decode(&mut source)
            .expect("row is valid")
            .expect("newline completes the row");

        assert!(matches!(
            record,
            Record::Packet(packet::Packet::Serverbound(_))
        ));
    }

    #[test]
    fn finite_streams_accept_a_final_row_without_a_newline() {
        let mut codec = Codec::new();
        let mut source = BytesMut::from(&br#"{"type":"transport-closed"}"#[..]);

        assert!(matches!(
            codec.decode_eof(&mut source).expect("final row is valid"),
            Some(Record::TransportClosed)
        ));
    }

    #[test]
    fn packet_frames_round_trip_through_the_shared_vocabulary() {
        let mut codec = Codec::new();
        let mut source = BytesMut::new();

        codec
            .encode(
                Frame::Packet {
                    flow: Flow::Serverbound,
                    body: &[0x13, 0x00, 0x00],
                    thread_id: Some(44),
                    captured_unix_ms: 2,
                },
                &mut source,
            )
            .expect("packet frame encodes");

        let record = codec
            .decode(&mut source)
            .expect("encoded row decodes")
            .expect("encoded row is complete");

        assert!(matches!(
            record,
            Record::Packet(packet::Packet::Serverbound(_))
        ));
    }
}
