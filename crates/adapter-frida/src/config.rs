//! Describe how a direct attachment is selected and instrumented.
//!
//! [`Target`] avoids an ambiguous process choice, [`Agent`] controls the
//! injected hook source, and [`Recording`] is an independent evidence policy.
//! Keeping all three as closed values lets [`Config`] describe an attachment
//! without exposing Frida construction details to callers.

use std::{num::NonZeroU32, path::PathBuf};

/// One unambiguous client selection policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Target {
    /// Attach to one exact process identifier.
    Pid(NonZeroU32),
    /// Attach only when exactly one process has this name.
    Process(Box<str>),
}

impl Target {
    /// Selects one exact process identifier.
    #[must_use]
    pub const fn pid(pid: NonZeroU32) -> Self {
        Self::Pid(pid)
    }

    /// Selects one process by case-insensitive executable name.
    #[must_use]
    pub fn process(name: impl Into<Box<str>>) -> Self {
        Self::Process(name.into())
    }
}

/// JavaScript agent source used inside the attached process.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Agent {
    /// Use the passive, versioned tap bundled with this crate.
    #[default]
    BuiltIn,
    /// Load a development agent from a local file.
    File(PathBuf),
}

/// Optional raw evidence persistence independent of live engine delivery.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Recording {
    /// Do not persist callback evidence.
    #[default]
    Disabled,
    /// Append compatible packet JSONL to this path.
    Jsonl(PathBuf),
}

/// Complete direct attachment configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    target: Target,
    agent: Agent,
    recording: Recording,
}

impl Config {
    /// Creates a direct attachment using built-in packet instrumentation and
    /// transport-owned session liveness.
    #[must_use]
    pub fn new(target: Target) -> Self {
        Self {
            target,
            agent: Agent::BuiltIn,
            recording: Recording::Disabled,
        }
    }

    /// Uses the selected [`Agent`] source.
    #[must_use]
    pub fn with_agent(self, agent: Agent) -> Self {
        Self { agent, ..self }
    }

    /// Persists raw callback evidence using the selected [`Recording`] policy.
    #[must_use]
    pub fn with_recording(self, recording: Recording) -> Self {
        Self { recording, ..self }
    }

    /// Returns the client selection policy.
    #[must_use]
    pub const fn target(&self) -> &Target {
        &self.target
    }

    /// Returns the JavaScript source policy.
    #[must_use]
    pub const fn agent(&self) -> &Agent {
        &self.agent
    }

    /// Returns the optional evidence recording policy.
    #[must_use]
    pub const fn recording(&self) -> &Recording {
        &self.recording
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_attachments_are_passive_outside_explicit_actions() {
        let config = Config::new(Target::process("NexusTK.exe"));

        assert_eq!(config.agent(), &Agent::BuiltIn);
        assert_eq!(config.recording(), &Recording::Disabled);
    }
}
