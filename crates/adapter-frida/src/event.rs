//! Operational events emitted by the direct adapter boundary.

use viperzoo_adapter_api::resource;
use viperzoo_protocol::direction::Flow;

/// One direct-adapter lifecycle or diagnostic event.
#[derive(Debug)]
pub enum Event {
    /// Frida attached and loaded the tap into the selected client.
    Attached(Info),
    /// The injected tap resolved both plaintext hook addresses.
    Ready(Info),
    /// A validated warm-attachment resource snapshot entered the engine.
    ResourcesSeeded(resource::Resources),
    /// A complete warm-attachment inventory snapshot entered the engine.
    InventorySeeded {
        /// Number of slots covered by the client scan.
        capacity: u8,
        /// Number of occupied slots in the client scan.
        occupied: usize,
    },
    /// The active game socket closed below the plaintext protocol hooks.
    TransportClosed,
    /// A Winsock operation failed outside the plaintext packet boundary.
    TransportFault(TransportFault),
    /// One callback could not become a typed protocol observation.
    Rejected(Rejection),
    /// The injected JavaScript agent reported a boundary warning.
    Warning(Box<str>),
    /// The injected JavaScript runtime reported an uncaught error.
    ScriptError(Problem),
}

/// A failed Winsock operation observed on the active game socket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportFault {
    operation: SocketOperation,
    code: i32,
}

impl TransportFault {
    pub(crate) const fn new(operation: SocketOperation, code: i32) -> Self {
        Self { operation, code }
    }

    /// Returns the failed socket operation.
    #[must_use]
    pub const fn operation(self) -> SocketOperation {
        self.operation
    }

    /// Returns the immediate `WSAGetLastError` result.
    #[must_use]
    pub const fn code(self) -> i32 {
        self.code
    }
}

/// Closed vocabulary of instrumented game-socket operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketOperation {
    /// A receive attempt failed.
    Receive,
    /// A send attempt failed.
    Send,
}

/// Stable facts about the active Frida attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Info {
    pid: u32,
    frida_version: Box<str>,
}

impl Info {
    pub(crate) fn new(pid: u32, frida_version: impl Into<Box<str>>) -> Self {
        Self {
            pid,
            frida_version: frida_version.into(),
        }
    }

    /// Returns the attached process identifier.
    #[must_use]
    pub const fn pid(&self) -> u32 {
        self.pid
    }

    /// Returns the native Frida core version.
    #[must_use]
    pub const fn frida_version(&self) -> &str {
        &self.frida_version
    }
}

/// A packet callback rejected at the acquisition boundary.
#[derive(Debug)]
pub struct Rejection {
    flow: Option<Flow>,
    length: usize,
    reason: Box<str>,
}

impl Rejection {
    pub(crate) fn new(flow: Option<Flow>, length: usize, reason: impl Into<Box<str>>) -> Self {
        Self {
            flow,
            length,
            reason: reason.into(),
        }
    }

    /// Returns the known packet flow, when the boundary direction was valid.
    #[must_use]
    pub const fn flow(&self) -> Option<Flow> {
        self.flow
    }

    /// Returns the callback data length.
    #[must_use]
    pub const fn length(&self) -> usize {
        self.length
    }

    /// Returns the boundary rejection reason.
    #[must_use]
    pub const fn reason(&self) -> &str {
        &self.reason
    }
}

/// Uncaught JavaScript runtime failure.
#[derive(Debug)]
pub struct Problem {
    description: Box<str>,
    stack: Box<str>,
}

impl Problem {
    pub(crate) fn new(description: impl Into<Box<str>>, stack: impl Into<Box<str>>) -> Self {
        Self {
            description: description.into(),
            stack: stack.into(),
        }
    }

    /// Returns the JavaScript error description.
    #[must_use]
    pub const fn description(&self) -> &str {
        &self.description
    }

    /// Returns the JavaScript stack trace.
    #[must_use]
    pub const fn stack(&self) -> &str {
        &self.stack
    }
}
