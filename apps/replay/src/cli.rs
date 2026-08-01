//! Minimal command-line vocabulary for capture replay.

use std::{ffi::OsString, path::Path};

use thiserror::Error;

/// Serialized report presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Output {
    /// One JSON line suitable for tools.
    Compact,
    /// Indented JSON suitable for an operator.
    Pretty,
}

/// Validated replay command-line configuration.
#[derive(Debug)]
pub struct Config {
    capture: OsString,
    output: Output,
}

impl Config {
    /// Parses command-line arguments after the executable name.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, Error> {
        let mut capture = None;
        let mut output = Output::Compact;

        for argument in arguments {
            if argument == "--pretty" {
                output = Output::Pretty;
            } else if capture.replace(argument).is_some() {
                return Err(Error::MultipleCaptures);
            }
        }

        Ok(Self {
            capture: capture.ok_or(Error::MissingCapture)?,
            output,
        })
    }

    /// Returns the capture path.
    #[must_use]
    pub fn capture(&self) -> &Path {
        Path::new(&self.capture)
    }

    /// Returns the requested JSON presentation.
    #[must_use]
    pub const fn output(&self) -> Output {
        self.output
    }
}

/// An invalid replay command line.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum Error {
    /// No JSONL source was supplied.
    #[error("missing capture path; usage: viperzoo-replay <capture.jsonl> [--pretty]")]
    MissingCapture,
    /// More than one positional source was supplied.
    #[error("only one capture path may be supplied")]
    MultipleCaptures,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_capture_and_optional_output_style() {
        let config = Config::parse(["capture.jsonl".into(), "--pretty".into()])
            .expect("valid replay arguments");

        assert_eq!(config.capture(), Path::new("capture.jsonl"));
        assert_eq!(config.output(), Output::Pretty);
    }

    #[test]
    fn rejects_ambiguous_sources() {
        let error = Config::parse(["one.jsonl".into(), "two.jsonl".into()])
            .expect_err("two capture paths are ambiguous");

        assert_eq!(error, Error::MultipleCaptures);
    }
}
