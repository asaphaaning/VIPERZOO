//! Mirror the terminal's tracing output onto the console socket.
//!
//! [`Layer`] composes *alongside* `fmt::layer()` rather than replacing it, so
//! attaching a console never changes what an operator sees in the terminal.
//! It formats each event once and broadcasts it; subscribers that lag are
//! dropped rather than allowed to stall instrumentation, because tracing must
//! never be able to block the engine it observes.

use std::{
    collections::VecDeque,
    fmt::Write as _,
    sync::{Arc, Mutex},
    time::Instant,
};

use tokio::sync::broadcast;
use tracing::{Event as TracingEvent, Subscriber, field::Visit};
use tracing_subscriber::{layer::Context, registry::LookupSpan};

use crate::event::{Event, Trace};

/// Tracing lines retained for consoles that connect later.
const HISTORY: usize = 300;

/// A `tracing` layer that publishes formatted events to connected consoles.
#[derive(Clone, Debug)]
pub struct Layer {
    events: broadcast::Sender<Event>,
    history: Arc<Mutex<VecDeque<Trace>>>,
    started: Instant,
}

impl Layer {
    /// Creates a layer publishing into `events` and retaining recent lines.
    #[must_use]
    pub fn new(events: broadcast::Sender<Event>, history: Arc<Mutex<VecDeque<Trace>>>) -> Self {
        Self {
            events,
            history,
            started: Instant::now(),
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for Layer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &TracingEvent<'_>, context: Context<'_, S>) {
        let metadata = event.metadata();
        let mut fields = Fields::default();

        event.record(&mut fields);

        // Span names carry VIPERZOO's namespaced instrumentation, which is
        // most of what makes a line readable. Prefer them over the module
        // target when the event happened inside one.
        let target = context.event_span(event).map_or_else(
            || metadata.target().to_owned(),
            |span| span.name().to_owned(),
        );
        let trace = Trace {
            elapsed_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            level: metadata.level().as_str(),
            target,
            message: fields.into_message(),
        };

        if let Ok(mut history) = self.history.lock() {
            while history.len() >= HISTORY {
                history.pop_front();
            }
            history.push_back(trace.clone());
        }

        // A console that cannot keep up is not a reason to slow the engine.
        let _ = self.events.send(Event::trace(trace));
    }
}

/// Collects an event's message and remaining fields into one line.
#[derive(Default)]
struct Fields {
    message: String,
    rest: String,
}

impl Fields {
    fn into_message(self) -> String {
        match (self.message.is_empty(), self.rest.is_empty()) {
            (true, true) => String::new(),
            (true, false) => self.rest,
            (false, true) => self.message,
            (false, false) => format!("{} {}", self.message, self.rest),
        }
    }
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
            return;
        }

        if !self.rest.is_empty() {
            self.rest.push(' ');
        }
        let _ = write!(self.rest, "{}={value:?}", field.name());
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
            return;
        }

        if !self.rest.is_empty() {
            self.rest.push(' ');
        }
        let _ = write!(self.rest, "{}={value}", field.name());
    }
}
