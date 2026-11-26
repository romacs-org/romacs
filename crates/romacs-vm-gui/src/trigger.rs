//! The live loop's side of the redisplay trigger (romacs-vm
//! display_trigger.rs): after Lisp it ran between keys (a timer, process
//! output, a background callback), whether the frame on screen went stale,
//! asked before a redisplay cycle is entered. The loop marks its
//! presentation changed only for a stale frame, so Lisp that changed
//! nothing shown runs no cycle.
use crate::{Shown, presentation::Presentation, prompt, title, tracing};
use romacs_display_runtime::paint_host::PaintSender;
use romacs_vm::{DisplayAcceptance, Staleness, Vm};
use serde_json::json;
use std::time::Instant;

/// What the frame on screen stood for (`Vm::display_acceptance`), none
/// until one is painted; and, while a frame found current is being built
/// anyway to verify it (`romacs-display-check`), the request for it
/// (`Presentation::changes`): a frame asked for again since, or Lisp run
/// since, shows more than the check saw, and verifies nothing.
pub struct Trigger {
    accepted: Option<DisplayAcceptance>,
    verifying: Option<u64>,
}

impl Trigger {
    /// The trigger of a loop whose window SHOWN shows its first paint, or
    /// shows nothing yet.
    pub fn new(vm: &mut Vm, shown: bool) -> Result<Self, String> {
        Ok(Self { accepted: shown.then(|| vm.display_acceptance(None)).transpose()?, verifying: None })
    }

    /// The frame on screen is the one just built, painted or found equal
    /// to the one painted: what it stands for is taken.
    /// SUBMITTED, the frame painted, when it was: the pictures the windows
    /// it carries were built with are the ones it names.
    pub fn accept(&mut self, vm: &mut Vm, submitted: Option<&romacs_layout_input::FrameSnapshot>) -> Result<(), String> {
        self.accepted = Some(vm.display_acceptance(submitted)?);
        self.verifying = None;
        Ok(())
    }

    /// The frame built for LIVE differs from the one on screen: when it was
    /// built only to verify a check that found it current, an input the
    /// check does not read.
    pub fn changed(&mut self, vm: &mut Vm, live: &Presentation) {
        if self.verifying.take() == Some(live.changes()) { vm.trigger_missed("the frame built after a check found it current differs"); }
    }

    /// A command runs: its own changes are no trigger's miss.
    pub fn command(&mut self) {
        self.verifying = None;
    }

    /// Lisp the loop ran between keys has returned; EXPLICIT when it asked
    /// for a frame itself (a terminal's output, a callback's non-nil). LIVE
    /// is marked changed for that, or when the frame on screen, PREVIOUS's,
    /// is stale. A current frame still gets its title considered, as a
    /// redisplay would, and settles `romacs--redisplay-since-mark`.
    pub fn after_lisp(&mut self, vm: &mut Vm, explicit: bool, live: &mut Presentation, previous: &Shown,
                      sender: &PaintSender, host: &std::cell::RefCell<prompt::Shared>) -> Result<(), String> {
        // Lisp ran since a verification was asked for: no longer one.
        self.verifying = None;
        if explicit || self.stale(vm, live, previous, sender, host)? { live.changed(); }
        if self.verifying.is_some() { self.verifying = Some(live.changes()); }
        Ok(())
    }

    /// Whether to build a frame: not while one is already due, always
    /// before any was accepted, else when the frame on screen is stale
    /// (`Vm::display_stale`), or, under `romacs-display-check`, to verify
    /// one found current. The whole decision is counted
    /// (`Vm::trigger_open`): the check, the text animations, the title.
    fn stale(&mut self, vm: &mut Vm, live: &Presentation, previous: &Shown, sender: &PaintSender,
             host: &std::cell::RefCell<prompt::Shared>) -> Result<bool, String> {
        if live.pending() { return Ok(false); }
        let Some(accepted) = &self.accepted else { return Ok(true) };
        let (bracket, start) = (vm.trigger_open(), Instant::now());
        let stale = current(vm, previous, accepted).map(|current| !current);
        if let Ok(false) = stale {
            title::consider(vm, sender, host);
            vm.redisplay_found_current(start.elapsed());
        }
        vm.trigger_close(bracket, !matches!(stale, Ok(false)));
        let verify = matches!(stale, Ok(false)) && vm.trigger_verified();
        self.verifying = verify.then_some(0);
        Ok(stale? || verify)
    }
}

/// Whether PREVIOUS, the frame on screen accepted as ACCEPTED, is current:
/// its windows and frame (`Vm::display_stale`) and its text animations. A
/// check that fails reads as stale, so the cycle meets the failure as before.
fn current(vm: &mut Vm, previous: &Shown, accepted: &DisplayAcceptance) -> Result<bool, String> {
    let staleness = vm.display_stale(&previous.frame.snapshot, accepted).unwrap_or(Staleness::Outdated("check failed"));
    if tracing() {
        let why = match staleness { Staleness::Outdated(why) => Some(why), Staleness::Current => None };
        crate::diagnostic_line(&json!({"diagnostic":"display-trigger","stale":staleness.is_stale(),"why":why}));
    }
    Ok(!staleness.is_stale() && vm.text_animations()? == previous.frame.animations)
}
