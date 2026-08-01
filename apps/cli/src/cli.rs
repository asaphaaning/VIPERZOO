//! Standalone client-selection vocabulary.

use std::{ffi::OsString, num::NonZeroU32, path::PathBuf};

use thiserror::Error;
use viperzoo_adapter_frida::{Agent, Recording, Target};

/// Validated standalone attachment configuration.
#[derive(Debug)]
pub struct Config {
    target: Target,
    agent: Agent,
    recording: Recording,
}

impl Config {
    /// Parses command-line arguments after the executable name.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, Error> {
        let mut arguments = arguments.into_iter();
        let mut target = None;
        let mut agent = Agent::BuiltIn;
        let mut recording = Recording::Disabled;

        while let Some(argument) = arguments.next() {
            if argument == "--pid" {
                let value = arguments.next().ok_or(Error::MissingPid)?;
                let value = value.to_str().ok_or(Error::NonUnicodePid)?;
                let value = value.parse::<u32>().map_err(|_| Error::Pid(value.into()))?;
                let value = NonZeroU32::new(value).ok_or_else(|| Error::Pid(value.to_string()))?;
                set_target(&mut target, Target::pid(value))?;
            } else if argument == "--process" {
                let value = arguments.next().ok_or(Error::MissingProcess)?;
                let value = value.to_str().ok_or(Error::NonUnicodeProcess)?;

                if value.is_empty() {
                    return Err(Error::EmptyProcess);
                }

                set_target(&mut target, Target::process(value))?;
            } else if argument == "--agent" {
                let value = arguments.next().ok_or(Error::MissingAgent)?;
                agent = Agent::File(PathBuf::from(value));
            } else if argument == "--record" {
                let value = arguments.next().ok_or(Error::MissingRecording)?;
                recording = Recording::Jsonl(PathBuf::from(value));
            } else {
                return Err(Error::Flag(argument));
            }
        }

        Ok(Self {
            target: target.unwrap_or_else(|| Target::process("NexusTK.exe")),
            agent,
            recording,
        })
    }

    /// Returns the unambiguous client target.
    #[must_use]
    pub const fn target(&self) -> &Target {
        &self.target
    }

    /// Returns the selected injected agent source.
    #[must_use]
    pub const fn agent(&self) -> &Agent {
        &self.agent
    }

    /// Returns the optional raw evidence policy.
    #[must_use]
    pub const fn recording(&self) -> &Recording {
        &self.recording
    }
}

fn set_target(target: &mut Option<Target>, value: Target) -> Result<(), Error> {
    if target.replace(value).is_some() {
        Err(Error::MultipleTargets)
    } else {
        Ok(())
    }
}

/// Invalid standalone command-line input.
#[derive(Debug, Error)]
pub enum Error {
    /// `--pid` omitted its process identifier.
    #[error("--pid requires a non-zero process identifier")]
    MissingPid,
    /// `--pid` used platform bytes that were not Unicode.
    #[error("--pid must be valid Unicode")]
    NonUnicodePid,
    /// `--pid` was not a non-zero 32-bit process identifier.
    #[error("invalid process identifier {0:?}")]
    Pid(String),
    /// `--process` omitted its executable name.
    #[error("--process requires an executable name")]
    MissingProcess,
    /// `--process` used platform bytes that were not Unicode.
    #[error("--process must be valid Unicode")]
    NonUnicodeProcess,
    /// `--process` was empty.
    #[error("--process cannot be empty")]
    EmptyProcess,
    /// More than one client target policy was supplied.
    #[error("select exactly one target with --pid or --process")]
    MultipleTargets,
    /// `--agent` omitted its source path.
    #[error("--agent requires a JavaScript source path")]
    MissingAgent,
    /// `--record` omitted its JSONL destination.
    #[error("--record requires a JSONL destination path")]
    MissingRecording,
    /// An unsupported flag was supplied.
    #[error(
        "unknown flag {0:?}; usage: viperzoo [--pid PID | --process NAME] [--agent agent.js] [--record capture.jsonl]"
    )]
    Flag(OsString),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nexustk_name_is_the_standalone_default() {
        let config = Config::parse([]).expect("empty arguments use safe defaults");

        assert_eq!(config.target(), &Target::process("NexusTK.exe"));
        assert_eq!(config.agent(), &Agent::BuiltIn);
        assert_eq!(config.recording(), &Recording::Disabled);
    }

    #[test]
    fn target_selection_is_unambiguous() {
        let error = Config::parse([
            "--pid".into(),
            "42".into(),
            "--process".into(),
            "NexusTK.exe".into(),
        ])
        .expect_err("two target policies are ambiguous");

        assert!(matches!(error, Error::MultipleTargets));
    }
}
