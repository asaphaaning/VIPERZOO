//! Validated destination and client selection.

use std::{ffi::OsString, num::NonZeroU32, path::PathBuf};

use thiserror::Error;
use viperzoo_adapter_frida::{Recording, Target};

/// Command-line configuration for the walk example.
#[derive(Debug)]
pub struct Config {
    x: u16,
    y: u16,
    client: Target,
    recording: Recording,
}

impl Config {
    /// Parses `X Y [--pid PID | --process NAME]`.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, Error> {
        let mut arguments = arguments.into_iter();
        let x = coordinate(arguments.next(), "X")?;
        let y = coordinate(arguments.next(), "Y")?;
        let mut client = None;
        let mut recording = None;

        while let Some(argument) = arguments.next() {
            if argument == "--record" {
                let path = arguments.next().ok_or(Error::Missing("recording path"))?;

                if recording
                    .replace(Recording::Jsonl(PathBuf::from(path)))
                    .is_some()
                {
                    return Err(Error::MultipleRecordings);
                }

                continue;
            }

            let target = if argument == "--pid" {
                let value = arguments.next().ok_or(Error::Missing("PID"))?;
                let value = value.to_str().ok_or(Error::Unicode("PID"))?;
                let value = value.parse().map_err(|_| Error::Value(value.into()))?;
                let pid = NonZeroU32::new(value).ok_or_else(|| Error::Value(value.to_string()))?;

                Target::pid(pid)
            } else if argument == "--process" {
                let value = arguments.next().ok_or(Error::Missing("NAME"))?;
                let value = value.to_str().ok_or(Error::Unicode("NAME"))?;

                if value.is_empty() {
                    return Err(Error::Value(value.into()));
                }

                Target::process(value)
            } else {
                return Err(Error::Flag(argument));
            };

            if client.replace(target).is_some() {
                return Err(Error::MultipleClients);
            }
        }

        Ok(Self {
            x,
            y,
            client: client.unwrap_or_else(|| Target::process("NexusTK.exe")),
            recording: recording.unwrap_or_default(),
        })
    }

    /// Returns the target X coordinate.
    #[must_use]
    pub const fn x(&self) -> u16 {
        self.x
    }

    /// Returns the target Y coordinate.
    #[must_use]
    pub const fn y(&self) -> u16 {
        self.y
    }

    /// Returns the selected client.
    #[must_use]
    pub const fn client(&self) -> &Target {
        &self.client
    }

    /// Returns optional raw packet evidence persistence.
    #[must_use]
    pub const fn recording(&self) -> &Recording {
        &self.recording
    }
}

fn coordinate(value: Option<OsString>, name: &'static str) -> Result<u16, Error> {
    let value = value.ok_or(Error::Missing(name))?;
    let value = value.to_str().ok_or(Error::Unicode(name))?;

    value.parse().map_err(|_| Error::Value(value.into()))
}

/// Invalid walk command line.
#[derive(Debug, Error)]
pub enum Error {
    #[error("missing {0}; usage: viperzoo-walk X Y [--pid PID | --process NAME]")]
    Missing(&'static str),
    #[error("{0} must be valid Unicode")]
    Unicode(&'static str),
    #[error("invalid value {0:?}")]
    Value(String),
    #[error("unknown flag {0:?}")]
    Flag(OsString),
    #[error("select at most one client with --pid or --process")]
    MultipleClients,
    #[error("specify --record at most once")]
    MultipleRecordings,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_is_required_and_typed() {
        let config = Config::parse(["10".into(), "1".into()]).expect("valid destination");

        assert_eq!((config.x(), config.y()), (10, 1));
        assert_eq!(config.client(), &Target::process("NexusTK.exe"));
        assert_eq!(config.recording(), &Recording::Disabled);
    }

    #[test]
    fn recording_is_explicit_and_independent_of_client_selection() {
        let config = Config::parse([
            "10".into(),
            "1".into(),
            "--record".into(),
            "walk.jsonl".into(),
            "--pid".into(),
            "42".into(),
        ])
        .expect("recording and an exact client can be selected together");

        assert_eq!(
            config.recording(),
            &Recording::Jsonl(PathBuf::from("walk.jsonl"))
        );
    }
}
