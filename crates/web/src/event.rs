//! The closed vocabulary a console socket carries.
//!
//! Everything the browser receives is something the engine already observed.
//! The console is a projection, never a controller, so this enum has no
//! command variants and the socket is one-directional by construction.

use serde::Serialize;

use crate::projection;

/// One message delivered to a connected console.
///
/// ```text
/// engine snapshot ──► Event::World  ──┐
///                                     ├──► one socket ──► browser
/// tracing layer   ──► Event::Trace ──┘
/// ```
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A complete world projection.
    ///
    /// Sent once when a console connects and again whenever the engine
    /// publishes a new revision. Boxed because it is much larger than
    /// [`Event::Trace`] and would otherwise set the size of every message.
    World(Box<projection::World>),
    /// One formatted tracing event, mirroring what reaches the terminal.
    Trace(Trace),
}

impl Event {
    /// Wraps a world projection.
    #[must_use]
    pub fn world(world: projection::World) -> Self {
        Self::World(Box::new(world))
    }

    /// Wraps one tracing line.
    #[must_use]
    pub const fn trace(trace: Trace) -> Self {
        Self::Trace(trace)
    }
}

/// One tracing event rendered for display.
///
/// Fields mirror the terminal `fmt` layer rather than the raw event, so the
/// console shows the same text an operator would read over the shoulder of the
/// process. `target` keeps the namespaced span path that makes VIPERZOO's
/// instrumentation legible.
#[derive(Clone, Debug, Serialize)]
pub struct Trace {
    /// Milliseconds since the console started, which needs no wall clock.
    pub elapsed_ms: u64,
    /// Event level, upper case: `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`.
    pub level: &'static str,
    /// Emitting target, usually the namespaced span name.
    pub target: String,
    /// The rendered message and its recorded fields.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_are_tagged_for_exhaustive_browser_handling() {
        let trace = Event::trace(Trace {
            elapsed_ms: 12,
            level: "INFO",
            target: "viperzoo::app::woodcut::run".into(),
            message: "hello".into(),
        });
        let rendered = serde_json::to_string(&trace).expect("serializes");

        assert!(rendered.contains(r#""kind":"trace""#));
        assert!(rendered.contains(r#""level":"INFO""#));
    }
}
