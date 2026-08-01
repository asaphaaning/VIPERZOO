//! Finite JSONL replay into the deterministic world reducer.

use std::{
    io::BufRead,
    path::{Path, PathBuf},
};

use serde::Serialize;
use thiserror::Error;
use tracing::{debug, instrument};
use viperzoo_adapter_api::observation::Observation;
use viperzoo_capture::{
    line::Line,
    record::{self, Record},
};
use viperzoo_engine::Reducer;
use viperzoo_world::snapshot::Snapshot;

/// The outcome of replaying one finite capture source.
#[derive(Debug, Serialize)]
pub struct Report {
    source: PathBuf,
    input_line_count: u64,
    packet_row_count: u64,
    skipped_row_count: u64,
    session_start_count: u64,
    diagnostics: Vec<record::Diagnostic>,
    snapshot: Snapshot,
}

impl Report {
    /// Returns the number of explicit client attachment boundaries.
    #[must_use]
    pub const fn session_start_count(&self) -> u64 {
        self.session_start_count
    }

    /// Returns rows that could not become ordered packet observations.
    #[must_use]
    pub fn diagnostics(&self) -> &[record::Diagnostic] {
        &self.diagnostics
    }

    /// Returns the final immutable projected world.
    #[must_use]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

/// Replays packet rows in source order and returns one immutable [`Snapshot`].
///
/// # Errors
///
/// Returns [`Error::Read`] when the source fails before yielding its next
/// complete line, or [`Error::LineCapacity`] if it exceeds the supported line
/// identity space.
#[instrument(
    name = "viperzoo::replay::capture",
    skip(reader),
    fields(source = %source.display()),
    err,
    ret(level = "trace")
)]
pub fn replay(reader: impl BufRead, source: &Path) -> Result<Report, Error> {
    let mut reducer = Reducer::new();
    let mut input_line_count = 0;
    let mut packet_row_count = 0;
    let mut skipped_row_count = 0;
    let mut session_start_count = 0;
    let mut diagnostics = Vec::new();
    let mut line_number = Line::FIRST;

    for input in reader.lines() {
        input_line_count += 1;
        let input = input.map_err(|source_error| Error::Read {
            path: source.to_owned(),
            line: line_number,
            source: source_error,
        })?;

        match record::decode(line_number, &input) {
            Record::SessionStarted => {
                session_start_count += 1;
                let _ = reducer.observe(Observation::SessionStarted);
            }
            Record::TransportClosed => {
                let _ = reducer.observe(Observation::TransportClosed);
            }
            Record::Packet(packet) => {
                packet_row_count += 1;
                let _ = reducer.observe(packet.into());
            }
            Record::Skipped => skipped_row_count += 1,
            Record::Rejected(diagnostic) => {
                packet_row_count += u64::from(diagnostic.is_packet_row());
                debug!(diagnostic = ?diagnostic, "capture row quarantined");
                diagnostics.push(diagnostic);
            }
        }

        line_number = line_number.next().ok_or(Error::LineCapacity)?;
    }

    Ok(Report {
        source: source.to_owned(),
        input_line_count,
        packet_row_count,
        skipped_row_count,
        session_start_count,
        diagnostics,
        snapshot: reducer.snapshot(),
    })
}

/// A fatal finite replay failure.
#[derive(Debug, Error)]
pub enum Error {
    /// The source stopped yielding complete lines.
    #[error("unable to read {path} at line {line:?}: {source}")]
    Read {
        /// Capture source path.
        path: PathBuf,
        /// One-based capture line.
        line: Line,
        /// Underlying stream error.
        source: std::io::Error,
    },
    /// The stream exceeded the complete `u64` line identity space.
    #[error("capture exceeds the supported line identity space")]
    LineCapacity,
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use viperzoo_protocol::primitive::{MapId, Position};

    use super::*;

    #[test]
    fn replay_skips_non_packets_and_quarantines_malformed_known_packets() {
        let input = concat!(
            "{\"type\":\"network-send\",\"hex\":\"aa00\"}\n",
            "{\"type\":\"packet\",\"direction\":\"incoming\",\"length\":24,\"hex\":\"1512670011001105000757656c636f6d6500e80002020200\"}\n",
            "{\"type\":\"packet\",\"direction\":\"incoming\",\"length\":2,\"hex\":\"1500\"}\n",
        );
        let report = replay(Cursor::new(input), Path::new("memory.jsonl"))
            .expect("in-memory capture is readable");

        assert_eq!(report.skipped_row_count, 1);
        assert_eq!(report.packet_row_count, 2);
        assert_eq!(report.diagnostics().len(), 1);
        assert_eq!(report.snapshot().map().epoch().value(), 1);
        assert_eq!(report.snapshot().processed_packet_count(), 1);
    }

    #[test]
    fn captured_foundation_fixture_has_stable_projection() {
        let fixture = include_str!("../../../fixtures/foundation.jsonl");
        let report = replay(Cursor::new(fixture), Path::new("foundation.jsonl"))
            .expect("fixture is readable");
        let snapshot = report.snapshot();
        let context = snapshot
            .map()
            .context()
            .expect("fixture identifies the map");

        assert!(report.diagnostics().is_empty());
        assert_eq!(snapshot.map().epoch().value(), 1);
        assert_eq!(context.id(), MapId::new(0x1267));
        assert_eq!(snapshot.map().tiles().len(), 2);
        assert_eq!(snapshot.entities().len(), 1);
        assert_eq!(
            snapshot.player().resources().vita().current().value(),
            Some(&151)
        );
        assert_eq!(
            snapshot.player().resources().mana().maximum().value(),
            Some(&98)
        );
        assert_eq!(snapshot.unknown_packet_count(), 0);
        assert_eq!(
            snapshot.player().location().position(),
            Some(Position::new(3, 1))
        );
    }

    #[test]
    fn report_json_is_deterministic() {
        let fixture = include_str!("../../../fixtures/foundation.jsonl");
        let first = replay(Cursor::new(fixture), Path::new("foundation.jsonl"))
            .expect("first fixture read succeeds");
        let second = replay(Cursor::new(fixture), Path::new("foundation.jsonl"))
            .expect("second fixture read succeeds");

        assert_eq!(
            serde_json::to_string(&first).expect("report serializes"),
            serde_json::to_string(&second).expect("report serializes")
        );
    }

    #[test]
    fn foundation_snapshot_matches_the_checked_in_golden_value() {
        let fixture = include_str!("../../../fixtures/foundation.jsonl");
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/foundation-snapshot.json"))
                .expect("golden snapshot is valid JSON");
        let report = replay(Cursor::new(fixture), Path::new("foundation.jsonl"))
            .expect("fixture is readable");
        let actual = serde_json::to_value(report.snapshot()).expect("snapshot serializes");

        assert_eq!(actual, expected);
    }

    #[test]
    fn empty_capture_preserves_unknown_facets() {
        let report =
            replay(Cursor::new(""), Path::new("empty.jsonl")).expect("empty source is readable");

        assert!(report.snapshot().map().context().is_none());
        assert!(report.snapshot().player().location().position().is_none());
        assert!(
            report
                .snapshot()
                .player()
                .resources()
                .vita()
                .current()
                .is_unknown()
        );
        assert_eq!(report.snapshot().map().epoch().value(), 0);
    }
}
