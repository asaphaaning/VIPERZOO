//! Command-line vocabulary for differential projection verification.

use std::{ffi::OsString, path::Path};

use thiserror::Error;

/// Validated verification sources.
#[derive(Debug)]
pub struct Config {
    capture: OsString,
    reference: OsString,
}

impl Config {
    /// Parses `<capture.jsonl> <engine-world-state.json>`.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, Error> {
        let mut arguments = arguments.into_iter();
        let capture = arguments.next().ok_or(Error::MissingCapture)?;
        let reference = arguments.next().ok_or(Error::MissingReference)?;

        if arguments.next().is_some() {
            return Err(Error::ExtraArguments);
        }

        Ok(Self { capture, reference })
    }

    /// Returns the plaintext capture source.
    #[must_use]
    pub fn capture(&self) -> &Path {
        Path::new(&self.capture)
    }

    /// Returns the Python reference snapshot.
    #[must_use]
    pub fn reference(&self) -> &Path {
        Path::new(&self.reference)
    }
}

/// Invalid differential-verification command line.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum Error {
    /// No capture source was supplied.
    #[error("missing capture; usage: viperzoo-verify <capture.jsonl> <engine-world-state.json>")]
    MissingCapture,
    /// No Python reference state was supplied.
    #[error("missing Python reference state")]
    MissingReference,
    /// More than two positional sources were supplied.
    #[error("verification accepts exactly one capture and one reference state")]
    ExtraArguments,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_exactly_two_sources() {
        let config = Config::parse(["tap.jsonl".into(), "world.json".into()])
            .expect("two sources are unambiguous");

        assert_eq!(config.capture(), Path::new("tap.jsonl"));
        assert_eq!(config.reference(), Path::new("world.json"));
        assert_eq!(
            Config::parse(["tap.jsonl".into()]).expect_err("reference is required"),
            Error::MissingReference
        );
    }
}
