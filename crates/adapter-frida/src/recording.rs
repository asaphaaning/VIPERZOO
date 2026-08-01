//! Persist optional evidence without making recording part of live delivery.
//!
//! The direct adapter sends observations to the engine first as its operational
//! responsibility. This module independently appends compatible evidence rows
//! for later analysis. A write failure changes the recorder to [`Recorder::Failed`]
//! so repeated callback handling does not repeatedly attempt a broken sink.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use viperzoo_protocol::direction::Flow;

use crate::config::Recording;

#[derive(Debug)]
pub(crate) enum Recorder {
    Disabled,
    Active(BufWriter<File>),
    Failed,
}

impl Recorder {
    pub(crate) fn open(recording: &Recording, pid: u32) -> Result<Self, io::Error> {
        let Recording::Jsonl(path) = recording else {
            return Ok(Self::Disabled);
        };

        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }

        let mut writer = BufWriter::new(OpenOptions::new().create(true).append(true).open(path)?);

        write_row(
            &mut writer,
            &Row::CaptureSessionStart {
                target_pid: pid,
                captured_unix_ms: captured_unix_ms(),
            },
        )?;

        Ok(Self::Active(writer))
    }

    pub(crate) fn packet(
        &mut self,
        flow: Flow,
        body: &[u8],
        thread_id: Option<u32>,
    ) -> Result<(), io::Error> {
        let Self::Active(writer) = self else {
            return Ok(());
        };
        let direction = match flow {
            Flow::Clientbound => "incoming",
            Flow::Serverbound => "outgoing",
        };
        let row = Row::Packet {
            direction,
            length: body.len(),
            hex: hex::encode(body),
            thread_id,
            captured_unix_ms: captured_unix_ms(),
        };

        if let Err(error) = write_row(writer, &row) {
            *self = Self::Failed;
            return Err(error);
        }

        Ok(())
    }

    pub(crate) fn transport_closed(&mut self, source: &str) -> Result<(), io::Error> {
        let Self::Active(writer) = self else {
            return Ok(());
        };
        let row = Row::TransportClosed {
            source,
            captured_unix_ms: captured_unix_ms(),
        };

        if let Err(error) = write_row(writer, &row) {
            *self = Self::Failed;
            return Err(error);
        }

        Ok(())
    }

    pub(crate) fn transport_fault(&mut self, operation: &str, code: i32) -> Result<(), io::Error> {
        let Self::Active(writer) = self else {
            return Ok(());
        };
        let row = Row::TransportFault {
            operation,
            code,
            captured_unix_ms: captured_unix_ms(),
        };

        if let Err(error) = write_row(writer, &row) {
            *self = Self::Failed;
            return Err(error);
        }

        Ok(())
    }
}

pub(crate) fn path(recording: &Recording) -> Option<&Path> {
    match recording {
        Recording::Disabled => None,
        Recording::Jsonl(path) => Some(path),
    }
}

fn write_row(writer: &mut impl Write, row: &Row<'_>) -> Result<(), io::Error> {
    serde_json::to_writer(&mut *writer, row).map_err(io::Error::other)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn captured_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum Row<'a> {
    CaptureSessionStart {
        target_pid: u32,
        captured_unix_ms: u128,
    },
    Packet {
        direction: &'a str,
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
        operation: &'a str,
        code: i32,
        captured_unix_ms: u128,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_match_the_existing_capture_boundary() {
        let mut output = Vec::new();

        write_row(
            &mut output,
            &Row::CaptureSessionStart {
                target_pid: 1234,
                captured_unix_ms: 1,
            },
        )
        .expect("memory writer accepts session row");
        write_row(
            &mut output,
            &Row::Packet {
                direction: "incoming",
                length: 3,
                hex: "130000".into(),
                thread_id: Some(44),
                captured_unix_ms: 2,
            },
        )
        .expect("memory writer accepts packet row");
        write_row(
            &mut output,
            &Row::TransportClosed {
                source: "recv-zero",
                captured_unix_ms: 3,
            },
        )
        .expect("memory writer accepts transport close row");

        let rows = String::from_utf8(output).expect("JSONL is UTF-8");
        let mut rows = rows.lines();
        let session: serde_json::Value =
            serde_json::from_str(rows.next().expect("session row exists")).expect("valid JSON");
        let packet: serde_json::Value =
            serde_json::from_str(rows.next().expect("packet row exists")).expect("valid JSON");
        let closed: serde_json::Value =
            serde_json::from_str(rows.next().expect("close row exists")).expect("valid JSON");

        assert_eq!(session["type"], "capture-session-start");
        assert_eq!(session["target_pid"], 1234);
        assert_eq!(session["captured_unix_ms"], 1);
        assert_eq!(packet["type"], "packet");
        assert_eq!(packet["direction"], "incoming");
        assert_eq!(packet["hex"], "130000");
        assert_eq!(packet["thread_id"], 44);
        assert_eq!(packet["captured_unix_ms"], 2);
        assert_eq!(closed["type"], "transport-closed");
        assert_eq!(closed["source"], "recv-zero");
        assert_eq!(closed["captured_unix_ms"], 3);
    }
}
