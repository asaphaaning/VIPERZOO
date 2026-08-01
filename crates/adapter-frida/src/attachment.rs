//! Own the live Frida runtime on the one thread where it is valid.
//!
//! This is the audited FFI boundary of the workspace. Frida Core values and
//! client process interaction remain here because they are non-`Send` and
//! require narrow unsafe calls. [`Control`] and [`Attachment`] turn that
//! thread-bound runtime into an asynchronous Rust-facing interface without
//! making the engine, scripts, or policy layer depend on Frida types.
//!
//! Commands cross into the owner through a standard channel; callbacks cross
//! out only after becoming typed engine observations or explicit adapter events.

// This module is VIPERZOO's audited FFI boundary: Frida Core initialization
// and Win32 window enumeration both require documented unsafe calls. All
// other workspace code remains under the shared `unsafe_code = "deny"` policy.
#![allow(unsafe_code)]

use std::{
    collections::BTreeSet,
    fs, io,
    path::PathBuf,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use frida::{Device, DeviceManager, Frida, ScriptOption};
use thiserror::Error;
use tokio::sync::oneshot;
use tracing::{debug, instrument};
use viperzoo_adapter_api::{
    action,
    inventory::{self, Item, Snapshot as InventorySnapshot},
    observation::Observation,
    resource::{self, Pool, Resources},
};
use viperzoo_engine::Handle;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
};
use windows_sys::core::BOOL;

use crate::{
    agent::{self, Handler},
    config::{Agent, Config, Target},
    event::{Event, Info},
    recording::{self, Recorder},
};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const OUTGOING_RVA: usize = 0x17_6660;
const INCOMING_RVA: usize = 0x17_8b20;
const MAX_PACKET_SIZE: usize = 1024 * 1024;
const SESSION_READY_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);
const SESSION_READY_ATTEMPTS: u8 = 3;
const SESSION_READY_POLL: Duration = Duration::from_millis(50);
const INVENTORY_SEED_ATTEMPTS: u8 = 5;
const INVENTORY_SEED_RETRY: Duration = Duration::from_millis(200);

/// A live direct Frida attachment.
#[derive(Debug)]
#[must_use = "dropping the attachment requests shutdown; call wait to observe the result"]
pub struct Attachment {
    commands: Sender<Command>,
    events: Receiver<Event>,
    thread: Option<thread::JoinHandle<Result<(), Error>>>,
}

/// Cloneable asynchronous control surface for the dedicated Frida owner.
#[derive(Clone, Debug)]
pub struct Control {
    commands: Sender<Command>,
}

impl Control {
    /// Delegates one action to the normal client and waits for adapter acceptance.
    ///
    /// This receipt only proves that the client action path accepted the intent.
    /// Scripts confirm its game effect through subsequent engine observations.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the attachment stopped or its RPC boundary
    /// rejected the action.
    #[instrument(
        name = "viperzoo::adapter::frida::perform",
        skip(self),
        fields(action = ?action),
        err,
        ret(level = "debug")
    )]
    pub async fn perform(&self, action: action::Action) -> Result<ActionReceipt, ActionError> {
        let (reply, receipt) = oneshot::channel();

        self.commands
            .send(Command::Perform { action, reply })
            .map_err(|_| ActionError::Stopped)?;

        receipt.await.map_err(|_| ActionError::Stopped)?
    }
}

impl action::Client for Control {
    type Error = ActionError;

    async fn perform(&self, action: action::Action) -> Result<(), Self::Error> {
        Control::perform(self, action).await.map(|_| ())
    }
}

/// Adapter acceptance of one client-native action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionReceipt {
    action: action::Action,
}

impl ActionReceipt {
    /// Returns the accepted intent.
    #[must_use]
    pub const fn action(&self) -> &action::Action {
        &self.action
    }
}

impl Attachment {
    /// Returns a cloneable asynchronous client-action handle.
    #[must_use]
    pub fn control(&self) -> Control {
        Control {
            commands: self.commands.clone(),
        }
    }

    /// Returns pending lifecycle and diagnostic events.
    #[must_use]
    pub const fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    /// Returns whether the dedicated Frida thread has stopped.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
    }

    /// Requests shutdown and waits for script unload and session detach.
    ///
    /// # Errors
    ///
    /// Returns a typed adapter error when teardown fails or the adapter thread
    /// previously stopped with a fatal acquisition error.
    #[instrument(
        name = "viperzoo::adapter::frida::stop",
        skip(self),
        err,
        ret(level = "debug")
    )]
    pub async fn stop(mut self) -> Result<(), Error> {
        let _ = self.commands.send(Command::Shutdown);
        self.join().await
    }

    /// Waits until the client detaches or all control handles are dropped.
    ///
    /// # Errors
    ///
    /// Returns a typed adapter error when acquisition or teardown fails.
    #[instrument(
        name = "viperzoo::adapter::frida::wait",
        skip(self),
        err,
        ret(level = "debug")
    )]
    pub async fn wait(mut self) -> Result<(), Error> {
        self.join().await
    }

    async fn join(&mut self) -> Result<(), Error> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };

        tokio::task::spawn_blocking(move || thread.join().map_err(|_| Error::ThreadPanicked)?)
            .await
            .map_err(Error::Join)?
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
    }
}

/// Starts direct acquisition on a dedicated operating-system thread.
///
/// # Errors
///
/// Returns [`Error::ThreadStart`] when the dedicated owner thread cannot be
/// created. Attachment lifecycle failures are returned by [`Attachment::wait`]
/// or [`Attachment::stop`].
#[instrument(
    name = "viperzoo::adapter::frida::attach",
    skip(engine),
    fields(target = ?config.target()),
    err,
    ret(level = "debug")
)]
pub fn attach(config: Config, engine: Handle) -> Result<Attachment, Error> {
    let (commands, command_receiver) = mpsc::channel();
    let (events, event_receiver) = mpsc::channel();
    // Frida's session graph is non-Send. One long-lived OS thread is the
    // ownership boundary; Tokio remains responsible for every consumer-facing
    // wait, controller, timer, and application task outside this island.
    let thread = thread::Builder::new()
        .name("viperzoo-frida".into())
        .spawn(move || run(config, engine, command_receiver, events))
        .map_err(Error::ThreadStart)?;

    Ok(Attachment {
        commands,
        events: event_receiver,
        thread: Some(thread),
    })
}

#[instrument(
    name = "viperzoo::adapter::frida::run",
    skip(config, engine, commands, events),
    err
)]
fn run(
    config: Config,
    engine: Handle,
    commands: Receiver<Command>,
    events: Sender<Event>,
) -> Result<(), Error> {
    // SAFETY: This dedicated thread is the sole owner of the Frida runtime,
    // and every borrowed manager, device, session, and script is dropped
    // before the runtime leaves this scope.
    let frida = unsafe { Frida::obtain() };
    let manager = DeviceManager::obtain(&frida);
    let device = manager.get_local_device()?;
    let pid = resolve(&device, config.target())?;
    let source = source(config.agent())?;
    let source = format!("const CONFIG = {};\n{source}", agent_config());
    let info = Info::new(pid, Frida::version());
    let recorder = Recorder::open(config.recording(), pid).map_err(|source| Error::Recording {
        path: recording::path(config.recording())
            .map_or_else(PathBuf::new, std::path::Path::to_owned),
        source,
    })?;
    let session = device.attach(pid)?;
    let mut options = ScriptOption::new();
    let mut script = session.create_script(&source, &mut options)?;

    script.handle_message(Handler::new(
        engine.clone(),
        events.clone(),
        info.clone(),
        recorder,
    ))?;
    engine.observe_blocking(Observation::SessionStarted)?;
    script.load()?;

    match script.exports.call("clientResources", None)? {
        Some(value) => match resource_snapshot(value) {
            Ok(Some(resources)) => {
                engine.observe_blocking(Observation::PlayerResources(resources))?;
                let _ = events.send(Event::ResourcesSeeded(resources));
            }
            Ok(None) => {}
            Err(error) => {
                let _ = events.send(Event::Warning(error.to_string().into()));
            }
        },
        None => {
            let _ = events.send(Event::Warning(
                "client resource RPC returned no result".into(),
            ));
        }
    }

    seed_inventory(&mut script, &engine, &events)?;

    let _ = events.send(Event::Attached(info));

    loop {
        if session.is_detached() {
            debug!(pid, "Frida session detached by client");
            break;
        }

        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Command::Perform { action, reply }) => {
                let result = ensure_action_ready(&mut script, &action)
                    .and_then(|()| perform(&mut script, action));
                let _ = reply.send(result);
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }

    if !session.is_detached() {
        script.unload()?;
        session.detach()?;
    }

    Ok(())
}

fn seed_inventory(
    script: &mut frida::Script<'_>,
    engine: &Handle,
    events: &Sender<Event>,
) -> Result<(), Error> {
    for attempt in 1..=INVENTORY_SEED_ATTEMPTS {
        let result = script.exports.call("clientInventory", None)?;

        match result.map(inventory_snapshot) {
            Some(Ok(InventorySeed::Known(inventory))) => {
                let capacity = inventory.capacity();
                let occupied = inventory.items().len();

                engine.observe_blocking(Observation::PlayerInventory(inventory))?;
                let _ = events.send(Event::InventorySeeded { capacity, occupied });
                return Ok(());
            }
            Some(Err(error)) => {
                let _ = events.send(Event::Warning(error.to_string().into()));
                return Ok(());
            }
            Some(Ok(InventorySeed::Unknown(_))) | None if attempt < INVENTORY_SEED_ATTEMPTS => {
                thread::sleep(INVENTORY_SEED_RETRY);
            }
            Some(Ok(InventorySeed::Unknown(reason))) => {
                let _ = events.send(Event::Warning(
                    format!(
                        "client inventory model remained unavailable after warm-attachment retries: {reason}"
                    )
                    .into(),
                ));
            }
            None => {
                let _ = events.send(Event::Warning(
                    "client inventory RPC returned no result".into(),
                ));
            }
        }
    }

    Ok(())
}

fn ensure_action_ready(
    script: &mut frida::Script<'_>,
    action: &action::Action,
) -> Result<(), ActionError> {
    if !requires_outgoing_session(action) || client_session_ready(script)? {
        return Ok(());
    }

    // Incoming traffic now establishes the shared build-752 session object.
    // Refresh retries cover the narrower case where a just-attached client is
    // temporarily between dispatch cycles and neither hook has fired yet.
    for attempt in 1..=SESSION_READY_ATTEMPTS {
        let _ = perform(script, action::Action::RefreshMap)?;
        let deadline = Instant::now() + SESSION_READY_ATTEMPT_TIMEOUT;

        while Instant::now() < deadline {
            thread::sleep(SESSION_READY_POLL);

            if client_session_ready(script)? {
                return Ok(());
            }
        }

        debug!(
            attempt,
            "client session priming attempt produced no traffic"
        );
    }

    Err(ActionError::SessionUnavailable)
}

fn client_session_ready(script: &mut frida::Script<'_>) -> Result<bool, ActionError> {
    script
        .exports
        .call("clientSessionReady", None)
        .map_err(|error| ActionError::Rpc(error.to_string().into()))
        .map(|value| value.and_then(|value| value.as_bool()).unwrap_or(false))
}

const fn requires_outgoing_session(action: &action::Action) -> bool {
    matches!(
        action,
        action::Action::Face(_)
            | action::Action::Attack(_)
            | action::Action::Pickup
            | action::Action::RequestProfile
            | action::Action::Interact(_)
            | action::Action::Dialog(_)
            | action::Action::Speak(_)
            | action::Action::CastAnswered(_)
            | action::Action::Travel(_)
            | action::Action::TravelOnMenu(_)
    )
}

fn perform(
    script: &mut frida::Script<'_>,
    action: action::Action,
) -> Result<ActionReceipt, ActionError> {
    let (function, arguments) = match &action {
        action::Action::Step(direction) => (
            "clientStep",
            serde_json::json!([match direction {
                viperzoo_protocol::direction::Direction::Up => "up",
                viperzoo_protocol::direction::Direction::Right => "right",
                viperzoo_protocol::direction::Direction::Down => "down",
                viperzoo_protocol::direction::Direction::Left => "left",
            }]),
        ),
        action::Action::Face(direction) => ("clientFace", serde_json::json!([direction.to_wire()])),
        action::Action::RefreshMap => ("clientRefresh", serde_json::json!([])),
        action::Action::DismissOverlay => ("clientDismissOverlay", serde_json::json!([])),
        action::Action::RequestProfile => ("clientRequestProfile", serde_json::json!([])),
        action::Action::Cast(slot) => ("clientCastSpell", serde_json::json!([slot.value()])),
        action::Action::CastAnswered(spell) => (
            "clientAnsweredSpell",
            serde_json::json!([spell.slot().value(), spell.answer()]),
        ),
        action::Action::Attack(direction) => {
            ("clientAttack", serde_json::json!([direction.to_wire()]))
        }
        action::Action::Pickup => ("clientPickup", serde_json::json!([])),
        action::Action::UseInventory(slot) => {
            ("clientUseInventory", serde_json::json!([slot.value()]))
        }
        action::Action::Interact(entity) => ("clientInteract", serde_json::json!([entity.value()])),
        action::Action::Dialog(selection) => (
            "clientDialog",
            serde_json::json!([
                selection.entity().value(),
                selection.command(),
                selection.argument(),
                selection.quantity()
            ]),
        ),
        action::Action::Speak(speech) => (
            "clientSpeak",
            serde_json::json!([speech.channel(), speech.text()]),
        ),
        action::Action::Travel(map) => ("clientTravel", serde_json::json!([map.value()])),
        action::Action::TravelOnMenu(map) => {
            ("clientTravelOnMenu", serde_json::json!([map.value()]))
        }
        action::Action::MapData(policy) => (
            "setForceMapData",
            serde_json::json!([matches!(policy, action::MapData::ForceResponse)]),
        ),
    };
    let accepted = script
        .exports
        .call(function, Some(arguments))
        .map_err(|error| ActionError::Rpc(error.to_string().into()))?
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    if !accepted {
        return Err(ActionError::Rejected(action));
    }

    Ok(ActionReceipt { action })
}

fn resolve(device: &Device<'_>, target: &Target) -> Result<u32, Error> {
    match target {
        Target::Pid(pid) => Ok(pid.get()),
        Target::Process(name) => {
            let matches = device
                .enumerate_processes()
                .into_iter()
                .filter(|process| process.get_name().eq_ignore_ascii_case(name))
                .map(|process| process.get_pid())
                .collect::<Vec<_>>();

            let visible = visible_processes();
            let visible_matches = matches
                .iter()
                .copied()
                .filter(|pid| visible.contains(pid))
                .collect::<Vec<_>>();

            match (matches.as_slice(), visible_matches.as_slice()) {
                ([], _) => Err(Error::ProcessNotFound(name.clone())),
                ([pid], _) | (_, [pid]) => Ok(*pid),
                _ => Err(Error::AmbiguousProcess {
                    name: name.clone(),
                    pids: matches,
                }),
            }
        }
    }
}

fn visible_processes() -> BTreeSet<u32> {
    let mut pids = BTreeSet::new();

    // SAFETY: `pids` lives for the synchronous duration of `EnumWindows`.
    // The callback casts the exact pointer back, never retains it, and only
    // calls Win32 query functions with HWND values supplied by the enumerator.
    unsafe {
        let pointer = std::ptr::from_mut(&mut pids).cast::<std::ffi::c_void>();
        let _ = EnumWindows(Some(record_visible_process), pointer as LPARAM);
    }

    pids
}

unsafe extern "system" fn record_visible_process(window: HWND, state: LPARAM) -> BOOL {
    // SAFETY: The callback is invoked only by `visible_processes`, which passes
    // a live `BTreeSet<u32>` pointer for the entire synchronous enumeration.
    let pids = unsafe { &mut *(state as *mut BTreeSet<u32>) };

    // SAFETY: `window` is supplied by EnumWindows and `pid` is a valid output.
    if unsafe { IsWindowVisible(window) } != 0 {
        let mut pid = 0_u32;
        unsafe { GetWindowThreadProcessId(window, &raw mut pid) };

        if pid != 0 {
            pids.insert(pid);
        }
    }

    1
}

fn source(agent: &Agent) -> Result<String, Error> {
    match agent {
        Agent::BuiltIn => Ok(agent::SOURCE.into()),
        Agent::File(path) => fs::read_to_string(path).map_err(|source| Error::ReadAgent {
            path: path.clone(),
            source,
        }),
    }
}

fn agent_config() -> serde_json::Value {
    serde_json::json!({
        "outgoingRva": OUTGOING_RVA,
        "incomingRva": INCOMING_RVA,
        "maxPacketSize": MAX_PACKET_SIZE,
    })
}

fn resource_snapshot(value: serde_json::Value) -> Result<Option<Resources>, ResourceError> {
    match serde_json::from_value(value)? {
        ResourceReply::Known {
            vita,
            max_vita,
            mana,
            max_mana,
            source,
        } => {
            if source.as_ref() != "client-memory-build-752" {
                return Err(ResourceError::Source(source));
            }

            let vita = Pool::new(vita, max_vita)?;
            let mana = Pool::new(mana, max_mana)?;

            Ok(Some(Resources::new(
                vita,
                mana,
                resource::Source::ClientMemoryBuild752,
            )))
        }
        ResourceReply::Unknown { reason } => {
            debug!(%reason, "client memory resource seed unavailable");
            Ok(None)
        }
    }
}

fn inventory_snapshot(value: serde_json::Value) -> Result<InventorySeed, InventoryError> {
    match serde_json::from_value(value)? {
        InventoryReply::Known {
            capacity,
            items,
            source,
        } => {
            if source.as_ref() != "client-memory-build-752" {
                return Err(InventoryError::Source(source));
            }

            let items = items
                .into_iter()
                .map(ItemReply::parse)
                .collect::<Result<Vec<_>, _>>()?;

            Ok(InventorySeed::Known(InventorySnapshot::new(
                capacity,
                items,
                inventory::Source::ClientMemoryBuild752,
            )?))
        }
        InventoryReply::Unknown { reason } => {
            debug!(%reason, "client memory inventory seed unavailable");
            Ok(InventorySeed::Unknown(reason))
        }
    }
}

#[derive(Debug)]
enum InventorySeed {
    Known(InventorySnapshot),
    Unknown(Box<str>),
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum ResourceReply {
    Known {
        vita: u32,
        max_vita: u32,
        mana: u32,
        max_mana: u32,
        source: Box<str>,
    },
    Unknown {
        reason: Box<str>,
    },
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum InventoryReply {
    Known {
        capacity: u8,
        items: Vec<ItemReply>,
        source: Box<str>,
    },
    Unknown {
        reason: Box<str>,
    },
}

#[derive(Debug, serde::Deserialize)]
struct ItemReply {
    slot: u8,
    icon_id: u16,
    icon_color: u8,
    name: Box<str>,
    amount: u32,
}

impl ItemReply {
    fn parse(self) -> Result<Item, inventory::Error> {
        Item::new(
            self.slot,
            self.icon_id,
            self.icon_color,
            self.name,
            self.amount,
        )
    }
}

#[derive(Debug)]
enum Command {
    Shutdown,
    Perform {
        action: action::Action,
        reply: oneshot::Sender<Result<ActionReceipt, ActionError>>,
    },
}

/// Failure to submit a client-native action.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ActionError {
    /// The dedicated attachment owner is no longer running.
    #[error("Frida attachment has stopped")]
    Stopped,
    /// No outgoing session was observed after bounded native refresh priming.
    #[error("client outgoing session was not observed after native refresh priming")]
    SessionUnavailable,
    /// The Frida RPC transport failed.
    #[error("Frida action RPC failed: {0}")]
    Rpc(Box<str>),
    /// The client-native path declined the action.
    #[error("client rejected action {0:?}")]
    Rejected(action::Action),
}

#[derive(Debug, Error)]
enum ResourceError {
    #[error("client resource RPC returned an invalid shape: {0}")]
    Shape(#[from] serde_json::Error),
    #[error("client resource RPC returned unsupported source {0}")]
    Source(Box<str>),
    #[error("client resource RPC returned invalid values: {0}")]
    Values(#[from] resource::Error),
}

#[derive(Debug, Error)]
enum InventoryError {
    #[error("client inventory RPC returned an invalid shape: {0}")]
    Shape(#[from] serde_json::Error),
    #[error("client inventory RPC returned unsupported source {0}")]
    Source(Box<str>),
    #[error("client inventory RPC returned invalid values: {0}")]
    Values(#[from] inventory::Error),
}

/// Fatal direct attachment failure.
#[derive(Debug, Error)]
pub enum Error {
    /// The dedicated Frida thread could not be started.
    #[error("unable to start dedicated Frida thread: {0}")]
    ThreadStart(io::Error),
    /// The dedicated Frida thread panicked.
    #[error("dedicated Frida thread panicked")]
    ThreadPanicked,
    /// Tokio could not complete the non-blocking join bridge.
    #[error("unable to join dedicated Frida thread: {0}")]
    Join(tokio::task::JoinError),
    /// The requested process name has no running match.
    #[error("no running process named {0}")]
    ProcessNotFound(Box<str>),
    /// The requested process name has multiple running matches.
    #[error("multiple processes named {name}: {pids:?}")]
    AmbiguousProcess {
        /// Ambiguous executable name.
        name: Box<str>,
        /// Matching process identifiers.
        pids: Vec<u32>,
    },
    /// A development agent file could not be read.
    #[error("unable to read Frida agent {path}: {source}")]
    ReadAgent {
        /// Agent source path.
        path: PathBuf,
        /// Filesystem failure.
        source: io::Error,
    },
    /// The optional append-only evidence file could not be initialized.
    #[error("unable to initialize Frida recording {path}: {source}")]
    Recording {
        /// JSONL evidence path.
        path: PathBuf,
        /// Filesystem failure.
        source: io::Error,
    },
    /// The official Frida binding rejected a lifecycle operation.
    #[error(transparent)]
    Frida(#[from] frida::Error),
    /// The canonical engine stopped while the adapter was attached.
    #[error(transparent)]
    Engine(#[from] viperzoo_engine::Error),
}

#[cfg(test)]
mod tests {
    use viperzoo_protocol::{
        direction::Direction,
        primitive::{EntityId, MapId},
    };

    use super::*;

    #[test]
    fn only_direct_plaintext_actions_require_captured_sender() {
        let entity = EntityId::new(7);
        let spell_slot = action::SpellSlot::new(2).expect("spell slot");

        for action in [
            action::Action::Face(Direction::Up),
            action::Action::Attack(Direction::Up),
            action::Action::Pickup,
            action::Action::RequestProfile,
            action::Action::Interact(entity),
            action::Action::Dialog(action::DialogSelection::option(entity, 0x40)),
            action::Action::Speak(action::Speech::say("hello").expect("speech")),
            action::Action::CastAnswered(
                action::AnsweredSpell::new(spell_slot, "n").expect("spell answer"),
            ),
            action::Action::Travel(MapId::new(0x03f3)),
            action::Action::TravelOnMenu(MapId::new(0x03f3)),
        ] {
            assert!(requires_outgoing_session(&action), "{action:?}");
        }

        for action in [
            action::Action::Step(Direction::Up),
            action::Action::RefreshMap,
            action::Action::DismissOverlay,
            action::Action::Cast(spell_slot),
            action::Action::UseInventory(action::InventorySlot::new(1).expect("inventory slot")),
            action::Action::MapData(action::MapData::ForceResponse),
        ] {
            assert!(!requires_outgoing_session(&action), "{action:?}");
        }
    }

    #[test]
    fn complete_client_inventory_reply_crosses_typed_boundary() {
        let value = serde_json::json!({
            "state": "known",
            "capacity": 27,
            "items": [
                {
                    "slot": 10,
                    "icon_id": 0xc285,
                    "icon_color": 0,
                    "name": "Axe",
                    "amount": 1
                },
                {
                    "slot": 11,
                    "icon_id": 0xce99,
                    "icon_color": 0,
                    "name": "Ginko wood",
                    "amount": 30
                }
            ],
            "source": "client-memory-build-752"
        });

        let inventory = inventory_snapshot(value).expect("valid inventory boundary");
        let InventorySeed::Known(inventory) = inventory else {
            panic!("expected known inventory");
        };

        assert_eq!(inventory.capacity(), 27);
        assert_eq!(inventory.items().len(), 2);
        assert_eq!(inventory.items()[1].name(), "Ginko wood");
        assert_eq!(inventory.items()[1].amount(), 30);
    }
}
