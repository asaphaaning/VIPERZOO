//! Command-line vocabulary for live capture following.

use std::{ffi::OsString, path::Path};

use thiserror::Error;

/// Where acquisition begins within the current capture file.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Start {
    /// Reconstruct all retained history before publishing live changes.
    #[default]
    Beginning,
    /// Attach at the current end with an explicitly incomplete warm state.
    End,
}

/// Projection detail written for ready and changed events.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Output {
    /// Small operational state suitable for a terminal or lightweight reader.
    #[default]
    Summary,
    /// Complete immutable world snapshots for stateful machine consumers.
    Snapshot,
}

impl Output {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "summary" => Some(Self::Summary),
            "snapshot" => Some(Self::Snapshot),
            _ => None,
        }
    }
}

impl Start {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "beginning" => Some(Self::Beginning),
            "end" => Some(Self::End),
            _ => None,
        }
    }
}

/// Validated live-follow configuration.
#[derive(Debug)]
pub struct Config {
    capture: OsString,
    start: Start,
    output: Output,
}

impl Config {
    /// Parses command-line arguments after the executable name.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, Error> {
        let mut arguments = arguments.into_iter();
        let mut capture = None;
        let mut start = Start::default();
        let mut output = Output::default();

        while let Some(argument) = arguments.next() {
            if argument == "--from" {
                let value = arguments.next().ok_or(Error::MissingStart)?;
                let value = value.to_str().ok_or(Error::NonUnicodeStart)?;
                start = Start::parse(value).ok_or_else(|| Error::Start(value.to_owned()))?;
            } else if argument == "--output" {
                let value = arguments.next().ok_or(Error::MissingOutput)?;
                let value = value.to_str().ok_or(Error::NonUnicodeOutput)?;
                output = Output::parse(value).ok_or_else(|| Error::Output(value.to_owned()))?;
            } else if argument.to_string_lossy().starts_with('-') {
                return Err(Error::Flag(argument));
            } else if capture.replace(argument).is_some() {
                return Err(Error::MultipleCaptures);
            }
        }

        Ok(Self {
            capture: capture.ok_or(Error::MissingCapture)?,
            start,
            output,
        })
    }

    /// Returns the actively appended capture path.
    #[must_use]
    pub fn capture(&self) -> &Path {
        Path::new(&self.capture)
    }

    /// Returns the requested attachment position.
    #[must_use]
    pub const fn start(&self) -> Start {
        self.start
    }

    /// Returns the requested projection detail.
    #[must_use]
    pub const fn output(&self) -> Output {
        self.output
    }
}

/// Invalid live-follow command-line input.
#[derive(Debug, Error)]
pub enum Error {
    /// No JSONL source was supplied.
    #[error(
        "missing capture path; usage: viperzoo-live <capture.jsonl> [--from beginning|end] [--output summary|snapshot]"
    )]
    MissingCapture,
    /// More than one positional source was supplied.
    #[error("only one capture path may be supplied")]
    MultipleCaptures,
    /// `--from` omitted its value.
    #[error("--from requires one of: beginning, end")]
    MissingStart,
    /// `--from` contained platform bytes that were not Unicode.
    #[error("--from must be valid Unicode")]
    NonUnicodeStart,
    /// `--from` contained an unknown state.
    #[error("unknown attachment position {0:?}; expected beginning or end")]
    Start(String),
    /// `--output` omitted its value.
    #[error("--output requires one of: summary, snapshot")]
    MissingOutput,
    /// `--output` contained platform bytes that were not Unicode.
    #[error("--output must be valid Unicode")]
    NonUnicodeOutput,
    /// `--output` contained an unknown detail level.
    #[error("unknown output detail {0:?}; expected summary or snapshot")]
    Output(String),
    /// An unsupported flag was supplied.
    #[error("unknown live projection flag {0:?}")]
    Flag(OsString),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beginning_is_the_safe_reconstructing_default() {
        let config = Config::parse(["capture.jsonl".into()]).expect("one source is valid");

        assert_eq!(config.capture(), Path::new("capture.jsonl"));
        assert_eq!(config.start(), Start::Beginning);
        assert_eq!(config.output(), Output::Summary);
    }

    #[test]
    fn end_attachment_is_explicit() {
        let config = Config::parse(["capture.jsonl".into(), "--from".into(), "end".into()])
            .expect("end is a canonical attachment position");

        assert_eq!(config.start(), Start::End);
    }

    #[test]
    fn unknown_start_state_is_rejected() {
        let error = Config::parse(["capture.jsonl".into(), "--from".into(), "middle".into()])
            .expect_err("middle has no defined acquisition semantics");

        assert!(matches!(error, Error::Start(_)));
    }

    #[test]
    fn complete_snapshots_require_explicit_selection() {
        let config = Config::parse(["capture.jsonl".into(), "--output".into(), "snapshot".into()])
            .expect("snapshot is a canonical output detail");

        assert_eq!(config.output(), Output::Snapshot);
    }
}
