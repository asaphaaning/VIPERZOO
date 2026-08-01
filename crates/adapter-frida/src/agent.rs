//! Translate injected-agent messages at the acquisition boundary.
//!
//! The JavaScript agent reports packet bodies and transport facts in a
//! compatibility envelope. This module treats that payload as untrusted:
//! direction, bytes, and length must cross protocol decoding before the engine
//! sees an observation. Rejections become [`crate::Event::Rejected`] rather
//! than partially applied world state.
//!
//! It also writes optional evidence records beside direct delivery, so recording
//! failure can be reported without interrupting the live projection path.

use std::sync::mpsc::Sender;

use frida::{Message as FridaMessage, ScriptHandler};
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};
use viperzoo_adapter_api::observation::Observation;
use viperzoo_engine::Handle;
use viperzoo_protocol::{decode, direction::Flow};

use crate::{
    event::{Event, Info, Problem, Rejection, SocketOperation, TransportFault},
    recording::Recorder,
};

pub(crate) const SOURCE: &str = include_str!("agent.js");

#[derive(Debug)]
pub(crate) struct Handler {
    engine: Handle,
    events: Sender<Event>,
    info: Info,
    recorder: Recorder,
}

impl Handler {
    pub(crate) fn new(
        engine: Handle,
        events: Sender<Event>,
        info: Info,
        recorder: Recorder,
    ) -> Self {
        Self {
            engine,
            events,
            info,
            recorder,
        }
    }

    fn send(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn handle(&mut self, message: FridaMessage, data: Option<&[u8]>) {
        match message {
            FridaMessage::Send(message) => {
                self.handle_send(&message.payload, data);
            }
            FridaMessage::Error(message) => {
                self.send(Event::ScriptError(Problem::new(
                    message.description,
                    message.stack,
                )));
            }
            FridaMessage::Log(message) => {
                debug!(level = ?message.level, message = %message.payload, "Frida agent log");
            }
            FridaMessage::Other(message) => self.handle_other(&message, data),
        }
    }

    fn handle_other(&mut self, message: &Value, data: Option<&[u8]>) {
        let Some(raw) = message.get("data").and_then(Value::as_str) else {
            debug!(?message, "unrecognized Frida message ignored");
            return;
        };

        match serde_json::from_str::<Envelope>(raw) {
            Ok(envelope) => self.handle_envelope(envelope, data),
            Err(error) => warn!(%error, "Frida compatibility envelope rejected"),
        }
    }

    fn handle_envelope(&mut self, envelope: Envelope, data: Option<&[u8]>) {
        match envelope {
            Envelope::Send { payload } => self.handle_send(&payload, data),
            Envelope::Error { description, stack } => {
                self.send(Event::ScriptError(Problem::new(
                    description,
                    stack.unwrap_or_default(),
                )));
            }
            Envelope::Log { level, payload } => {
                debug!(level, message = %payload, "Frida agent log");
            }
            Envelope::Other => {}
        }
    }

    fn handle_send(&mut self, payload: &Value, data: Option<&[u8]>) {
        match payload.get("type").and_then(Value::as_str) {
            Some("packet") => self.handle_packet(payload, data),
            Some("transport-closed") => self.handle_transport_closed(payload),
            Some("transport-fault") => self.handle_transport_fault(payload),
            Some("ready") => self.send(Event::Ready(self.info.clone())),
            Some("warning") => {
                let message = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Frida agent warning");
                self.send(Event::Warning(message.into()));
            }
            Some("client-action-failed") => {
                let action = payload
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let error = payload
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unspecified client action failure");
                self.send(Event::Warning(
                    format!("client action {action} failed: {error}").into(),
                ));
            }
            Some(kind) => debug!(kind, "Frida agent event ignored"),
            None => warn!(?payload, "Frida send message has no typed payload"),
        }
    }

    fn handle_transport_closed(&mut self, payload: &Value) {
        let source = payload
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        if let Err(error) = self.recorder.transport_closed(source) {
            self.send(Event::Warning(
                format!("raw JSONL recording stopped: {error}").into(),
            ));
        }

        if let Err(error) = self.engine.observe_blocking(Observation::TransportClosed) {
            self.send(Event::Rejected(Rejection::new(None, 0, error.to_string())));
            return;
        }

        self.send(Event::TransportClosed);
    }

    fn handle_transport_fault(&mut self, payload: &Value) {
        let operation = match payload.get("operation").and_then(Value::as_str) {
            Some("receive") => SocketOperation::Receive,
            Some("send") => SocketOperation::Send,
            _ => {
                self.send(Event::Warning(
                    "transport fault has an unsupported operation".into(),
                ));
                return;
            }
        };
        let Some(code) = payload
            .get("code")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok())
        else {
            self.send(Event::Warning(
                "transport fault has no valid Winsock error code".into(),
            ));
            return;
        };
        let operation_name = match operation {
            SocketOperation::Receive => "receive",
            SocketOperation::Send => "send",
        };

        if let Err(error) = self.recorder.transport_fault(operation_name, code) {
            self.send(Event::Warning(
                format!("raw JSONL recording stopped: {error}").into(),
            ));
        }

        self.send(Event::TransportFault(TransportFault::new(operation, code)));
    }

    fn handle_packet(&mut self, payload: &Value, data: Option<&[u8]>) {
        let flow = match payload.get("direction").and_then(Value::as_str) {
            Some("incoming") => Some(Flow::Clientbound),
            Some("outgoing") => Some(Flow::Serverbound),
            _ => None,
        };
        let Some(flow) = flow else {
            self.send(Event::Rejected(Rejection::new(
                None,
                data.map_or(0, <[u8]>::len),
                "packet direction is missing or unsupported",
            )));
            return;
        };
        let Some(data) = data else {
            self.send(Event::Rejected(Rejection::new(
                Some(flow),
                0,
                "packet callback has no binary body",
            )));
            return;
        };

        let thread_id = payload
            .get("threadId")
            .and_then(Value::as_u64)
            .and_then(|thread_id| u32::try_from(thread_id).ok());

        if let Err(error) = self.recorder.packet(flow, data, thread_id) {
            self.send(Event::Warning(
                format!("raw JSONL recording stopped: {error}").into(),
            ));
        }

        let packet = match decode(flow, data) {
            Ok(packet) => packet,
            Err(error) => {
                self.send(Event::Rejected(Rejection::new(
                    Some(flow),
                    data.len(),
                    error.to_string(),
                )));
                return;
            }
        };

        if let Err(error) = self.engine.observe_blocking(packet.into()) {
            self.send(Event::Rejected(Rejection::new(
                Some(flow),
                data.len(),
                error.to_string(),
            )));
        }
    }
}

impl ScriptHandler for Handler {
    fn on_message(&mut self, message: FridaMessage, data: Option<Vec<u8>>) {
        self.handle(message, data.as_deref());
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Envelope {
    Send {
        payload: Value,
    },
    Log {
        level: String,
        payload: String,
    },
    Error {
        description: String,
        stack: Option<String>,
    },
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_frida_send_reaches_the_typed_payload_boundary() {
        let raw =
            r#"{"type":"send","payload":{"type":"packet","direction":"incoming","length":3}}"#;
        let binding_message: FridaMessage =
            serde_json::from_str(raw).expect("upstream binding accepts ordinary send payloads");
        let FridaMessage::Send(message) = binding_message else {
            panic!("ordinary send envelope must reach the typed send variant");
        };

        assert_eq!(
            message.payload.get("direction").and_then(Value::as_str),
            Some("incoming")
        );
    }

    #[test]
    fn built_in_agent_seeds_session_and_inventory_from_incoming_client_state() {
        assert!(SOURCE.contains("outgoingSession = session;"));
        assert!(SOURCE.contains("clientInventory()"));
        assert!(SOURCE.contains("mainModule.base.add(0x001a3870)"));
        assert!(SOURCE.contains("displayName.replace(/ \\(\\d+\\)$/, '')"));
        assert!(SOURCE.contains("client inventory accessor signature mismatch"));
        assert!(SOURCE.contains("transport-closed"));
        assert!(SOURCE.contains("WSOCK32_CLOSESOCKET_IAT_RVA"));
        assert!(SOURCE.contains("WSOCK32_RECV_IAT_RVA"));
        assert!(SOURCE.contains("WSOCK32_SEND_IAT_RVA"));
        assert!(SOURCE.contains("this.lastError"));
        assert!(SOURCE.contains("CLIENT_IDLE_WATCHER_SLOT_RVA"));
        assert!(SOURCE.contains("clientActivity()"));
        assert!(SOURCE.contains("networkPollAddress"));
        assert!(SOURCE.contains("directPlaintextSendDepth"));
        assert!(SOURCE.contains("flushPlaintextBody(this.session, this.threadId)"));
        assert!(SOURCE.contains("emitPacket('outgoing', body, pending.bytes.length, threadId)"));
        assert!(SOURCE.contains("client-thread action queue"));
        assert!(SOURCE.contains("requestNetworkPoll()"));
        assert!(SOURCE.contains("PostMessageW(Control wake down) failed"));
        assert!(SOURCE.contains("clientSpeak(channel, text)"));
        assert!(SOURCE.contains("clientAnsweredSpell(slot, answer)"));
        assert!(SOURCE.contains("clientTravel(map)"));
        assert!(SOURCE.contains("clientTravelOnMenu(map)"));
        assert!(SOURCE.contains("const body = [...prefix, bytes.length, ...bytes];"));
        assert!(SOURCE.contains("body.push(quantityBytes.length, ...quantityBytes, 0x00)"));
    }

    #[test]
    fn built_in_agent_retains_guarded_idle_state_as_research_only() {
        assert!(SOURCE.contains("CLIENT_IDLE_RESET_RVA"));
        assert!(SOURCE.contains("client IdleWatcher reset signature mismatch"));
        assert!(SOURCE.contains("resetIdle(watcher)"));
        assert!(SOURCE.contains("is not session maintenance"));
    }

    #[test]
    fn incoming_network_boundary_is_observation_only() {
        let incoming = SOURCE
            .split_once("Interceptor.attach(incomingAddress")
            .map(|(_, body)| body)
            .expect("built-in agent retains the incoming hook");

        assert!(incoming.contains("this.session = session;"));
        assert!(incoming.contains("observePendingTravelMenu(this.output, this.length);"));
        assert!(!incoming.contains("flushPlaintextBody(this.session, this.threadId);"));
    }

    #[test]
    fn travel_selection_uses_the_native_client_menu_model() {
        assert!(SOURCE.contains("input.readU8() !== 0x2e"));
        assert!(SOURCE.contains("TRAVEL_SELECTOR_SUBMIT_RVA"));
        assert!(SOURCE.contains("TRAVEL_SELECTOR_CONSTRUCT_RVA"));
        assert!(SOURCE.contains("TRAVEL_SELECTOR_SLOT_RVA"));
        assert!(SOURCE.contains("TRAVEL_SELECTOR_VTABLE_RVA"));
        assert!(SOURCE.contains("matchingTravelSelectorRow(selector, selection)"));
        assert!(SOURCE.contains("selection.x === null"));
        assert!(SOURCE.contains("constructor has populated the complete 0x94-byte row vector"));
        assert!(SOURCE.contains("bindPendingTravelSelection(this.selector)"));
        assert!(SOURCE.contains("submitTravelSelectorRow(selection.selector, selection.row)"));
        assert!(!SOURCE.contains("pendingPlaintextBodies.push(pendingTravelSelection)"));
        assert!(SOURCE.contains("pendingTravelSelection = null"));
        assert!(SOURCE.contains("An immediate selection is a guarded warm-attachment recovery"));
    }

    #[test]
    fn bounded_wake_exhaustion_reports_deferred_not_failed() {
        assert!(SOURCE.contains("remains queued after ${pending.wakeAttempts} network wakes"));
        assert!(!SOURCE.contains(
            "pending action did not reach the client network thread after ${pending.wakeAttempts} wakes"
        ));
    }

    #[test]
    fn explicit_attack_uses_a_bounded_directional_network_wake() {
        let attack = SOURCE
            .split_once("function invokeClientAttack(direction)")
            .and_then(|(_, tail)| tail.split_once("function invokeClientPickup()"))
            .map(|(body, _)| body)
            .expect("built-in agent retains the attack function");

        assert!(attack.contains("invokeCombatPlaintextBody"));
        assert!(SOURCE.contains("MAX_PENDING_PLAINTEXT_WAKE_ATTEMPTS"));
        assert!(SOURCE.contains("satisfyPendingNativeBody(opcode)"));
        assert!(SOURCE.contains("pending.action === 'attack'"));
    }

    #[test]
    fn client_step_retries_until_transport_evidence_arrives() {
        let step = SOURCE
            .split_once("function beginClientStep(direction)")
            .and_then(|(_, tail)| tail.split_once("function invokeClientRefresh()"))
            .map(|(body, _)| body)
            .expect("built-in agent retains the client-step state machine");

        assert!(SOURCE.contains("MAX_CLIENT_STEP_TAP_ATTEMPTS"));
        assert!(step.contains("submitClientStepTap(state)"));
        assert!(step.contains("compactMovement || fullMovement || obstruction"));
        assert!(step.contains("state.phase = 'complete'"));
    }
}
