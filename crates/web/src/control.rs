//! Let a console ask a running application to hold, without knowing what it runs.
//!
//! The event stream is one-directional by construction, so control is a separate
//! and deliberately narrow surface. What crosses it is a [`Signal`] — a request
//! about *running*, not an instruction about the work. This crate has no notion
//! of trees, portals, or dialogs, and adding one would make every future
//! application inherit a woodcutter's vocabulary.
//!
//! ```text
//!  browser ──POST /signal──► Signal ──► application decides what holding means
//!  browser ◄──/events────── State  ◄── application reports what it actually is
//! ```
//!
//! Interpretation belongs entirely to the application. Only it knows where a
//! safe stopping point is: pausing between two swings is harmless, pausing
//! halfway through a directional portal step or an open shop dialog is how you
//! manufacture the states that are hardest to recover from. So a [`Signal`] is a
//! request, never a preemption.
//!
//! [`Controls`] is optional at [`crate::Console::serve`]. Without it the console
//! renders no controls and stays a pure observer, which is what keeps this crate
//! honest: it never assumes something is running, it is *told* whether anything
//! is listening.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

/// A control a console can request of whatever is running.
///
/// Deliberately app-agnostic. The console knows a session can be asked to hold
/// or continue; it does not know what holding means.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    /// Stop starting new work and hold at the next safe point.
    Pause,
    /// Resume from a hold.
    Resume,
    /// Finish at the next safe point and do not resume.
    Stop,
}

impl Signal {
    /// Every [`Signal`], in declaration order.
    pub const VARIANTS: [Self; 3] = [Self::Pause, Self::Resume, Self::Stop];
}

/// What the application currently *is*, as opposed to what was asked of it.
///
/// The distinction matters for honesty. A console that flipped to `Paused` the
/// moment a button was pressed would be lying for as long as the application
/// took to reach a safe point — which can be minutes if it is mid-bank-cycle.
/// [`State::Pausing`] makes that interval visible instead of making a working
/// application look like a hung button.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Working normally.
    #[default]
    Running,
    /// A hold was requested; the application has not reached a safe point yet.
    Pausing,
    /// Held at a safe point, waiting for [`Signal::Resume`].
    Paused,
    /// Finished, by request or otherwise.
    Stopped,
}

impl State {
    /// Every [`State`], in declaration order.
    pub const VARIANTS: [Self; 4] = [Self::Running, Self::Pausing, Self::Paused, Self::Stopped];

    /// Returns whether the application should be doing work right now.
    #[must_use]
    pub const fn is_working(self) -> bool {
        matches!(self, Self::Running | Self::Pausing)
    }
}

/// The console's half of the control surface.
///
/// Signals travel out to the application; state travels back. Both directions
/// are required, because a control that cannot observe its own effect is a
/// control that lies.
#[derive(Clone, Debug)]
pub struct Controls {
    signals: mpsc::Sender<Signal>,
    state: Arc<watch::Sender<State>>,
}

impl Controls {
    /// Returns what the application currently reports itself to be.
    #[must_use]
    pub fn state(&self) -> State {
        *self.state.borrow()
    }

    /// Requests one control, returning whether the application is still listening.
    ///
    /// A pause is acknowledged immediately as [`State::Pausing`], because the
    /// application may be minutes from a point where holding is safe and a
    /// console that showed no change until then would look broken rather than
    /// patient. Only the application may declare [`State::Paused`], and it does
    /// so when it has actually held.
    ///
    /// A closed channel is not an error worth propagating: it means the
    /// application has already stopped, which is exactly what most signals were
    /// asking for.
    pub async fn request(&self, signal: Signal) -> bool {
        // A request the application has not observed yet can still be withdrawn,
        // and the console should say so. Reporting `Pausing` for a resume — the
        // state a pending hold is already in — leaves the button unchanged, so
        // it reads as ignored and gets pressed again.
        match (signal, self.state()) {
            (Signal::Pause, State::Running) => {
                self.state.send_replace(State::Pausing);
            }
            (Signal::Resume, State::Pausing) => {
                self.state.send_replace(State::Running);
            }
            // Only the application may declare `Paused`: a resume from a real
            // hold is not truthful until it has actually released.
            _ => {}
        }

        self.signals.send(signal).await.is_ok()
    }
}

/// The application's half of the control surface.
///
/// Held by whoever is actually doing the work. It receives requests and is the
/// only thing that may declare what state the application is in.
#[derive(Debug)]
#[must_use = "an application that never polls its signals cannot be controlled"]
pub struct Session {
    signals: mpsc::Receiver<Signal>,
    state: Arc<watch::Sender<State>>,
}

impl Session {
    /// Creates a connected [`Controls`] and [`Session`] pair.
    pub fn channel() -> (Controls, Self) {
        let (signals, receiver) = mpsc::channel(8);
        let (state, _) = watch::channel(State::Running);
        let state = Arc::new(state);

        (
            Controls {
                signals,
                state: Arc::clone(&state),
            },
            Self {
                signals: receiver,
                state,
            },
        )
    }

    /// Returns the current state.
    #[must_use]
    pub fn state(&self) -> State {
        *self.state.borrow()
    }

    /// Declares the state the application has actually reached.
    pub fn publish(&self, state: State) {
        self.state.send_replace(state);
    }

    /// Applies every pending signal and returns the resulting state.
    ///
    /// Call this at a point where holding is safe. Signals are folded rather
    /// than handled one at a time, so a console that sent pause-then-resume
    /// while the application was busy resolves to `Running` instead of
    /// stopping to observe an intent that was already withdrawn.
    pub fn poll(&mut self) -> State {
        let mut state = self.resolved(self.state());

        while let Ok(signal) = self.signals.try_recv() {
            state = Self::apply(signal);
        }

        self.publish(state);
        state
    }

    /// Maps one signal onto the state it produces at a safe point.
    const fn apply(signal: Signal) -> State {
        match signal {
            Signal::Pause => State::Paused,
            Signal::Resume => State::Running,
            Signal::Stop => State::Stopped,
        }
    }

    /// Settles a requested hold now that a safe point has been reached.
    ///
    /// [`State::Pausing`] is what a console publishes to acknowledge a request
    /// it cannot yet fulfil. Reaching here *is* the fulfilment, so it resolves
    /// rather than lingering as a state nothing would ever clear.
    const fn resolved(&self, state: State) -> State {
        match state {
            State::Pausing => State::Paused,
            other => other,
        }
    }

    /// Waits until the application should work again, or should stop for good.
    ///
    /// Returns `false` once [`State::Stopped`] is reached, which callers treat
    /// as "leave the loop".
    pub async fn wait_while_paused(&mut self) -> bool {
        loop {
            match self.poll() {
                State::Stopped => return false,
                State::Running | State::Pausing => return true,
                State::Paused => {}
            }

            // The signal must be applied here, not merely awaited. Dropping it
            // and looping back to `poll` would leave nothing for `try_recv` to
            // find, so a resume would be read, discarded, and waited on again —
            // a hold that no console could ever release.
            match self.signals.recv().await {
                Some(signal) => self.publish(Self::apply(signal)),
                // Every console has gone away; holding forever helps nobody.
                None => {
                    self.publish(State::Running);
                    return true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A requested hold is acknowledged at once but must not claim to have
    /// happened: the application is still working until it reaches a safe point.
    #[tokio::test]
    async fn a_pause_is_only_observed_at_a_safe_point() {
        let (controls, mut session) = Session::channel();

        assert!(controls.request(Signal::Pause).await);
        assert_eq!(controls.state(), State::Pausing);
        assert!(
            controls.state().is_working(),
            "a pause that has not landed must not read as held"
        );

        assert_eq!(session.poll(), State::Paused);
        assert!(!controls.state().is_working());
    }

    /// A console that changed its mind while the application was busy must not
    /// leave it holding for an intent that was already withdrawn.
    #[tokio::test]
    async fn folded_signals_resolve_to_the_last_intent() {
        let (controls, mut session) = Session::channel();

        assert!(controls.request(Signal::Pause).await);
        assert!(controls.request(Signal::Resume).await);

        assert_eq!(session.poll(), State::Running);
    }

    #[tokio::test]
    async fn stopping_leaves_the_loop() {
        let (controls, mut session) = Session::channel();

        assert!(controls.request(Signal::Stop).await);

        assert!(!session.wait_while_paused().await);
        assert_eq!(controls.state(), State::Stopped);
    }

    /// A resume arriving while the application is already holding must release
    /// it. Awaiting a signal and then re-reading the queue loses it entirely.
    #[tokio::test]
    async fn a_resume_arriving_during_a_hold_is_not_swallowed() {
        let (controls, mut session) = Session::channel();

        assert!(controls.request(Signal::Pause).await);
        assert_eq!(session.poll(), State::Paused);

        let waiting = tokio::spawn(async move {
            let released = session.wait_while_paused().await;
            (released, session.state())
        });

        // Sent only after the application is already parked inside the wait.
        tokio::task::yield_now().await;
        assert!(controls.request(Signal::Resume).await);

        let (released, state) = waiting.await.expect("the hold releases");
        assert!(released);
        assert_eq!(state, State::Running);
    }

    /// A hold that has not landed can be withdrawn, and the console must show
    /// the withdrawal — otherwise the button appears to ignore the click.
    #[tokio::test]
    async fn a_pending_hold_can_be_withdrawn() {
        let (controls, mut session) = Session::channel();

        assert!(controls.request(Signal::Pause).await);
        assert_eq!(controls.state(), State::Pausing);

        assert!(controls.request(Signal::Resume).await);
        assert_eq!(controls.state(), State::Running);

        // The application, reaching a safe point, agrees: the fold resolves to
        // the last intent rather than the first.
        assert_eq!(session.poll(), State::Running);
    }

    /// Toggling faster than the application can observe must still settle on
    /// whatever was asked last.
    #[tokio::test]
    async fn rapid_toggling_settles_on_the_last_intent() {
        let (controls, mut session) = Session::channel();

        for signal in [Signal::Pause, Signal::Resume, Signal::Pause] {
            assert!(controls.request(signal).await);
        }

        assert_eq!(session.poll(), State::Paused);
    }

    /// A request the application has not yet reached is visible immediately, so
    /// a console shows patience rather than appearing dead.
    #[tokio::test]
    async fn a_requested_hold_is_acknowledged_before_it_lands() {
        let (controls, mut session) = Session::channel();

        assert!(controls.request(Signal::Pause).await);
        assert_eq!(controls.state(), State::Pausing);

        assert_eq!(session.poll(), State::Paused);
        assert_eq!(controls.state(), State::Paused);
    }

    #[tokio::test]
    async fn a_resume_releases_a_hold() {
        let (controls, mut session) = Session::channel();

        assert!(controls.request(Signal::Pause).await);
        assert_eq!(session.poll(), State::Paused);

        assert!(controls.request(Signal::Resume).await);
        assert!(session.wait_while_paused().await);
        assert_eq!(controls.state(), State::Running);
    }
}
