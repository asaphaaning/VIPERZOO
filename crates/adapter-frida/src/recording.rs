//! Persist optional evidence without making recording part of live delivery.
//!
//! The direct adapter records raw callback evidence before attempting protocol
//! promotion, so rejected bodies remain available for later analysis. Recording
//! stays independent of live delivery: a write failure changes the recorder to
//! [`Recorder::Failed`] without stopping the engine observation path.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use bytes::BytesMut;
use tokio_util::codec::Encoder;
use viperzoo_capture::{
    codec::Codec,
    frame::{Frame, Operation},
};
use viperzoo_protocol::direction::Flow;

use crate::config::Recording;

#[derive(Debug)]
pub(crate) enum Recorder {
    Disabled,
    Active(Box<Active>),
    Failed,
}

#[derive(Debug)]
pub(crate) struct Active {
    writer: BufWriter<File>,
    codec: Codec,
    buffer: BytesMut,
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

        let writer = BufWriter::new(OpenOptions::new().create(true).append(true).open(path)?);
        let mut active = Active {
            writer,
            codec: Codec::new(),
            buffer: BytesMut::new(),
        };

        active.write(Frame::SessionStarted {
            target_pid: pid,
            captured_unix_ms: captured_unix_ms(),
        })?;

        Ok(Self::Active(Box::new(active)))
    }

    pub(crate) fn packet(
        &mut self,
        flow: Flow,
        body: &[u8],
        thread_id: Option<u32>,
    ) -> Result<(), io::Error> {
        let Self::Active(active) = self else {
            return Ok(());
        };
        let frame = Frame::Packet {
            flow,
            body,
            thread_id,
            captured_unix_ms: captured_unix_ms(),
        };

        if let Err(error) = active.write(frame) {
            *self = Self::Failed;
            return Err(error);
        }

        Ok(())
    }

    pub(crate) fn transport_closed(&mut self, source: &str) -> Result<(), io::Error> {
        let Self::Active(active) = self else {
            return Ok(());
        };
        let frame = Frame::TransportClosed {
            source,
            captured_unix_ms: captured_unix_ms(),
        };

        if let Err(error) = active.write(frame) {
            *self = Self::Failed;
            return Err(error);
        }

        Ok(())
    }

    pub(crate) fn transport_fault(
        &mut self,
        operation: Operation,
        code: i32,
    ) -> Result<(), io::Error> {
        let Self::Active(active) = self else {
            return Ok(());
        };
        let frame = Frame::TransportFault {
            operation,
            code,
            captured_unix_ms: captured_unix_ms(),
        };

        if let Err(error) = active.write(frame) {
            *self = Self::Failed;
            return Err(error);
        }

        Ok(())
    }
}

impl Active {
    fn write(&mut self, frame: Frame<'_>) -> Result<(), io::Error> {
        self.codec
            .encode(frame, &mut self.buffer)
            .map_err(io::Error::other)?;
        self.writer.write_all(&self.buffer)?;
        self.writer.flush()?;
        self.buffer.clear();

        Ok(())
    }
}

pub(crate) fn path(recording: &Recording) -> Option<&Path> {
    match recording {
        Recording::Disabled => None,
        Recording::Jsonl(path) => Some(path),
    }
}

fn captured_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_match_the_existing_capture_boundary() {
        let mut output = BytesMut::new();

        let mut codec = Codec::new();

        codec
            .encode(
                Frame::SessionStarted {
                    target_pid: 1234,
                    captured_unix_ms: 1,
                },
                &mut output,
            )
            .expect("memory writer accepts session row");
        codec
            .encode(
                Frame::Packet {
                    flow: Flow::Clientbound,
                    body: &[0x13, 0x00, 0x00],
                    thread_id: Some(44),
                    captured_unix_ms: 2,
                },
                &mut output,
            )
            .expect("memory writer accepts packet row");
        codec
            .encode(
                Frame::TransportClosed {
                    source: "recv-zero",
                    captured_unix_ms: 3,
                },
                &mut output,
            )
            .expect("memory writer accepts transport close row");

        let rows = String::from_utf8(output.to_vec()).expect("JSONL is UTF-8");
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
