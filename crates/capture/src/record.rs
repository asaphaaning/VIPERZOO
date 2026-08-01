//! Classify each evidence row before it can enter ordered reduction.
//!
//! [`decode`] validates the external representation in stages: JSON shape,
//! row kind, direction, hexadecimal bytes, declared length, then protocol
//! structure. [`Record::Rejected`] retains the precise boundary failure and
//! line identity, while only [`Record::Packet`] carries a typed packet onward.
//! This makes replay diagnostics useful without allowing damaged evidence to
//! manufacture world facts.

use serde::{Deserialize, Serialize};
use tracing::instrument;
use viperzoo_protocol::{decode as decode_packet, direction::Flow, packet};

use crate::line::Line;

/// One completely classified capture line.
#[derive(Debug)]
pub enum Record {
    /// A new client/Frida attachment session began.
    SessionStarted,
    /// The active client transport closed below the plaintext boundary.
    TransportClosed,
    /// A decoded, direction-specific protocol packet.
    Packet(packet::Packet),
    /// A valid tap row that is not a logical protocol packet.
    Skipped,
    /// A packet-like row that could not cross the typed boundary.
    Rejected(Diagnostic),
}

/// A quarantined boundary problem.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Diagnostic {
    /// The row was not valid JSON.
    Json {
        /// One-based acquisition line.
        line: Line,
        /// Boundary parser message.
        message: String,
    },
    /// A packet row omitted its direction or body.
    IncompletePacket {
        /// One-based acquisition line.
        line: Line,
    },
    /// A packet row used a non-network direction vocabulary.
    Flow {
        /// One-based acquisition line.
        line: Line,
    },
    /// The body was not valid hexadecimal.
    Hex {
        /// One-based acquisition line.
        line: Line,
        /// Boundary decoder message.
        message: String,
    },
    /// Declared and decoded body sizes disagreed.
    Length {
        /// One-based acquisition line.
        line: Line,
        /// JSONL-declared body length.
        declared: usize,
        /// Actual decoded byte length.
        decoded: usize,
    },
    /// A promoted opcode failed structural protocol validation.
    Packet {
        /// One-based acquisition line.
        line: Line,
        /// Typed protocol decoder message.
        message: String,
    },
}

impl Diagnostic {
    /// Returns whether the rejected input had already identified itself as a
    /// logical packet row.
    #[must_use]
    pub const fn is_packet_row(&self) -> bool {
        !matches!(self, Self::Json { .. })
    }
}

/// Classifies one untrusted JSONL line.
#[must_use]
#[instrument(
    name = "viperzoo::capture::decode_record",
    skip(input),
    fields(line = ?line, bytes = input.len()),
    ret(level = "trace")
)]
pub fn decode(line: Line, input: &str) -> Record {
    let row: Row = match serde_json::from_str(input) {
        Ok(row) => row,
        Err(error) => {
            return Record::Rejected(Diagnostic::Json {
                line,
                message: error.to_string(),
            });
        }
    };

    match row.kind {
        Kind::SessionStart => return Record::SessionStarted,
        Kind::TransportClosed => return Record::TransportClosed,
        Kind::Other => return Record::Skipped,
        Kind::Packet => {}
    }

    let (Some(flow), Some(body_hex)) = (row.direction, row.body) else {
        return Record::Rejected(Diagnostic::IncompletePacket { line });
    };
    let Some(flow) = flow.protocol() else {
        return Record::Rejected(Diagnostic::Flow { line });
    };
    let body = match hex::decode(body_hex) {
        Ok(body) => body,
        Err(error) => {
            return Record::Rejected(Diagnostic::Hex {
                line,
                message: error.to_string(),
            });
        }
    };

    if let Some(declared) = row.length
        && declared != body.len()
    {
        return Record::Rejected(Diagnostic::Length {
            line,
            declared,
            decoded: body.len(),
        });
    }

    match decode_packet(flow, &body) {
        Ok(packet) => Record::Packet(packet),
        Err(error) => Record::Rejected(Diagnostic::Packet {
            line,
            message: error.to_string(),
        }),
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Kind {
    Packet,
    #[serde(rename = "capture-session-start")]
    SessionStart,
    TransportClosed,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct Row {
    #[serde(rename = "type")]
    kind: Kind,
    direction: Option<RowFlow>,
    length: Option<usize>,
    #[serde(rename = "hex")]
    body: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RowFlow {
    Incoming,
    Outgoing,
    Clientbound,
    Serverbound,
    #[serde(other)]
    Other,
}

impl RowFlow {
    const fn protocol(self) -> Option<Flow> {
        match self {
            Self::Incoming | Self::Clientbound => Some(Flow::Clientbound),
            Self::Outgoing | Self::Serverbound => Some(Flow::Serverbound),
            Self::Other => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use viperzoo_protocol::{packet, server};

    use super::*;

    #[test]
    fn packet_row_crosses_into_the_typed_protocol() {
        let record = decode(
            Line::FIRST,
            r#"{"type":"packet","direction":"incoming","length":24,"hex":"1512670011001105000757656c636f6d6500e80002020200"}"#,
        );

        assert!(matches!(
            record,
            Record::Packet(packet::Packet::Clientbound(server::Packet::MapContext(_)))
        ));
    }

    #[test]
    fn non_packet_row_is_a_value_not_an_error() {
        let record = decode(Line::FIRST, r#"{"type":"network-send","hex":"aa00"}"#);

        assert!(matches!(record, Record::Skipped));
    }

    #[test]
    fn session_start_is_a_typed_lifecycle_boundary() {
        let record = decode(
            Line::FIRST,
            r#"{"type":"capture-session-start","target_pid":1234}"#,
        );

        assert!(matches!(record, Record::SessionStarted));
    }

    #[test]
    fn transport_close_is_a_typed_lifecycle_boundary() {
        let record = decode(
            Line::FIRST,
            r#"{"type":"transport-closed","captured_unix_ms":3}"#,
        );

        assert!(matches!(record, Record::TransportClosed));
    }

    #[test]
    fn malformed_known_packet_is_quarantined() {
        let record = decode(
            Line::FIRST,
            r#"{"type":"packet","direction":"incoming","length":2,"hex":"1500"}"#,
        );

        assert!(matches!(
            record,
            Record::Rejected(Diagnostic::Packet { .. })
        ));
    }

    #[test]
    fn a_non_network_direction_cannot_become_a_packet() {
        let record = decode(
            Line::FIRST,
            r#"{"type":"packet","direction":"sideways","length":1,"hex":"aa"}"#,
        );

        assert!(matches!(record, Record::Rejected(Diagnostic::Flow { .. })));
    }

    #[test]
    fn only_identified_packet_failures_count_as_packet_rows() {
        let Record::Rejected(json) = decode(Line::FIRST, "not json") else {
            panic!("invalid JSON must be rejected");
        };
        let Record::Rejected(packet) =
            decode(Line::FIRST, r#"{"type":"packet","direction":"incoming"}"#)
        else {
            panic!("incomplete packet must be rejected");
        };

        assert!(!json.is_packet_row());
        assert!(packet.is_packet_row());
    }
}
