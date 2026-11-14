//! The evaluator's side of the seam: one thread takes every event the
//! render thread sends, keeps the facts among them (what rendered, the
//! adapter, lost images, the native-model renderer, the pointer, key-repeat
//! timing) as state the evaluator reads, and passes the rest on in order.
//!
//! The render thread shares no memory with the evaluator for these facts:
//! it only sends events, so the thread here is where a transport from
//! another process would deliver them instead.
use crate::thread_comm::InputEvent;
use crossbeam_channel::{Receiver, Sender};
use romacs_display_protocol::key_repeat_log::{KeyRepeatCounts, KeyRepeatLog};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

/// A frame's latest presentation the renderer drew, with its atlas sample.
/// Cumulative within that atlas lifetime, not a per-frame delta or
/// compositor confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintDiagnostic {
    pub presentation: u64,
    pub unresolved_face_text_total: u64,
}

/// How a held key repeated, as the render thread saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatTiming {
    /// A repeat came this long after its key's press.
    Repeat(Duration),
    /// A key was held this long before its release.
    Held(Duration),
}

/// The facts as they stand.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The latest drawn presentation of each frame, by frame id.
    rendered: HashMap<u64, PaintDiagnostic>,
    /// The adapter the windows draw with: whether it is a software
    /// rasterizer (wgpu's `DeviceType::Cpu`), once chosen.
    software: Option<bool>,
    /// Images the renderer evicted since the evaluator last took them.
    evicted: Vec<u32>,
    /// A GPU rebuild lost every image since the evaluator last took them.
    reset: bool,
    model_renderer: bool,
    /// Where the pointer is over the primary window, in logical pixels.
    pointer: Option<(i32, i32)>,
    key_repeats: KeyRepeatLog,
}

/// The facts and the wait for a render.
#[derive(Default)]
pub(crate) struct Facts {
    state: Mutex<State>,
    rendered: Condvar,
    /// Called when a newer presentation of the primary frame has drawn, or
    /// the renderer lost its images: the evaluator's wait for input wakes.
    wake: OnceLock<Arc<dyn Fn() + Send + Sync>>,
}

/// What a fact asks of the evaluator's waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notice {
    Nothing,
    /// A newer presentation of this frame drew.
    Rendered(u64),
    /// The renderer lost every image: an idle evaluator wakes to repaint,
    /// and residency sends the images again.
    Reset,
}

/// Apply EVENT to STATE: what goes on to the evaluator, if anything, and
/// what its waits must hear.  Facts stay here; a closed window's render
/// state goes with it.
fn apply(state: &mut State, event: InputEvent) -> (Option<InputEvent>, Notice) {
    match event {
        InputEvent::PresentationRendered { frame, presentation, unresolved_face_text_total } => {
            let latest = state.rendered.get(&frame).map_or(0, |sample| sample.presentation);
            if presentation >= latest {
                state.rendered.insert(frame, PaintDiagnostic { presentation, unresolved_face_text_total });
            }
            (None, if presentation > latest { Notice::Rendered(frame) } else { Notice::Nothing })
        }
        InputEvent::AdapterChosen { software } => { state.software = Some(software); (None, Notice::Nothing) }
        InputEvent::ImageStateChanged { event: romacs_display_protocol::ImageStateEvent::Evicted(image) } => {
            state.evicted.push(image.get());
            (None, Notice::Nothing)
        }
        InputEvent::DisplayReset => { state.reset = true; (None, Notice::Reset) }
        InputEvent::ModelRenderer { live } => { state.model_renderer = live; (None, Notice::Nothing) }
        InputEvent::PointerOver { at } => { state.pointer = at; (None, Notice::Nothing) }
        InputEvent::KeyRepeatTiming(RepeatTiming::Repeat(after)) => { state.key_repeats.repeat(after); (None, Notice::Nothing) }
        InputEvent::KeyRepeatTiming(RepeatTiming::Held(held)) => { state.key_repeats.released(held); (None, Notice::Nothing) }
        InputEvent::WindowClose { emacs_frame_id } => { state.rendered.remove(&emacs_frame_id); (Some(event), Notice::Nothing) }
        event => (Some(event), Notice::Nothing),
    }
}

impl Facts {
    /// The facts, through a lock no panic leaves unusable: each fact is a
    /// plain value, whole after any write.
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Keep what EVENT tells; what goes on to the evaluator.
    fn take(&self, event: InputEvent) -> Option<InputEvent> {
        let (pass, notice) = apply(&mut self.state(), event);
        if let Notice::Rendered(_) = notice { self.rendered.notify_all(); }
        if matches!(notice, Notice::Rendered(crate::paint_host::PRIMARY_FRAME) | Notice::Reset)
            && let Some(wake) = self.wake.get()
        {
            wake();
        }
        pass
    }

    pub(crate) fn on_render(&self, wake: Arc<dyn Fn() + Send + Sync>) -> bool {
        self.wake.set(wake).is_ok()
    }

    pub(crate) fn software_adapter(&self) -> Option<bool> { self.state().software }

    pub(crate) fn model_renderer(&self) -> bool { self.state().model_renderer }

    pub(crate) fn pointer(&self) -> Option<(i32, i32)> { self.state().pointer }

    pub(crate) fn take_key_repeats(&self) -> KeyRepeatCounts { self.state().key_repeats.take() }

    /// The images lost since the last call: those evicted, and whether a
    /// GPU rebuild lost them all.
    pub(crate) fn take_lost_images(&self) -> (Vec<u32>, bool) {
        let mut state = self.state();
        (std::mem::take(&mut state.evicted), std::mem::take(&mut state.reset))
    }

    pub(crate) fn images_reset_pending(&self) -> bool { self.state().reset }

    /// FRAME's latest sample once PRESENTATION or a later one has drawn,
    /// waiting at most TIMEOUT: the renderer draws only its newest frame,
    /// so a later presentation drawn settles an earlier one it never drew.
    pub(crate) fn wait_for_render(&self, frame: u64, presentation: u64, timeout: Duration) -> Result<PaintDiagnostic, String> {
        let drawn = |state: &State| state.rendered.get(&frame).copied().filter(|sample| sample.presentation >= presentation);
        let (state, _) = self.rendered
            .wait_timeout_while(self.state(), timeout, |state| drawn(state).is_none())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        drawn(&state).ok_or_else(|| format!("no rendered atlas diagnostic for presentation {presentation}"))
    }
}

/// Start the bridge over INPUT, the render thread's events: the facts it
/// keeps and the channel the rest go on, in the order they came. It ends
/// when the render thread's sender goes, and so does that channel.
pub(crate) fn spawn(input: Receiver<InputEvent>, capacity: usize) -> std::io::Result<(Arc<Facts>, Receiver<InputEvent>)> {
    let facts = Arc::new(Facts::default());
    let (tx, rx) = crossbeam_channel::bounded(capacity);
    let kept = facts.clone();
    std::thread::Builder::new().name("romacs-input".into()).spawn(move || run(&kept, &input, &tx))?;
    Ok((facts, rx))
}

/// Pass the rest on as the render thread would have queued it: pointer
/// motion dropped while the evaluator's queue is full, everything else
/// waiting for room.  Facts are kept whatever the queue holds.
fn run(facts: &Facts, input: &Receiver<InputEvent>, out: &Sender<InputEvent>) {
    for event in input {
        let Some(event) = facts.take(event) else { continue };
        let sent = if crate::thread_comm::is_lossy_input_event(&event) {
            !matches!(out.try_send(event), Err(crossbeam_channel::TrySendError::Disconnected(_)))
        } else {
            out.send(event).is_ok()
        };
        if !sent { return; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> InputEvent {
        InputEvent::Key { key: crate::host_types::FrontendKey::Character('k'), modifiers: 0, pressed: true, emacs_frame_id: 1 }
    }

    fn rendered(frame: u64, presentation: u64) -> InputEvent {
        InputEvent::PresentationRendered { frame, presentation, unresolved_face_text_total: presentation * 10 }
    }

    /// Facts stay with the bridge; everything else goes on, in order.
    #[test]
    fn facts_are_kept_and_the_rest_pass_in_order() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (facts, out) = spawn(rx, 16).unwrap();
        for event in [key(), InputEvent::AdapterChosen { software: true }, InputEvent::PointerOver { at: Some((3, 4)) },
                      InputEvent::ModelRenderer { live: true }, InputEvent::DisplayReset, key(),
                      InputEvent::ImageStateChanged { event: romacs_display_protocol::ImageStateEvent::Evicted(romacs_display_protocol::ImageId::new(7)) },
                      InputEvent::KeyRepeatTiming(RepeatTiming::Repeat(Duration::from_millis(40)))] {
            tx.send(event).unwrap();
        }
        drop(tx);
        let passed: Vec<_> = out.iter().collect();
        assert_eq!(passed.len(), 2);
        assert!(passed.iter().all(|event| matches!(event, InputEvent::Key { .. })));
        assert_eq!(facts.software_adapter(), Some(true));
        assert_eq!(facts.pointer(), Some((3, 4)));
        assert!(facts.model_renderer());
        assert!(facts.images_reset_pending());
        assert_eq!(facts.take_lost_images(), (vec![7], true));
        assert_eq!(facts.take_lost_images(), (vec![], false), "taken once");
        assert_eq!(facts.take_key_repeats().repeats, 1);
    }

    /// A wait for a render is answered per frame, by the frame's latest
    /// presentation; the primary frame's newer ones wake the evaluator once
    /// each.
    #[test]
    fn renders_settle_their_frames_waits_and_wake_once_per_newer_primary() {
        let facts = Facts::default();
        let wakes = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counted = wakes.clone();
        assert!(facts.on_render(Arc::new(move || { counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed); })));
        assert!(facts.wait_for_render(1, 1, Duration::ZERO).is_err(), "nothing drawn yet");
        for (frame, presentation) in [(1, 1), (1, 1), (2, 5), (1, 3)] {
            assert!(facts.take(rendered(frame, presentation)).is_none());
        }
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 2, "no wake for a frame drawn again or another frame");
        assert_eq!(facts.wait_for_render(1, 2, Duration::ZERO).unwrap().presentation, 3, "a later presentation settles an earlier one");
        assert!(facts.wait_for_render(1, 4, Duration::ZERO).is_err(), "stale samples are no evidence");
        assert_eq!(facts.wait_for_render(2, 5, Duration::ZERO).unwrap().unresolved_face_text_total, 50);
    }

    #[test]
    fn a_wait_sees_a_render_from_another_thread() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (facts, _out) = spawn(rx, 16).unwrap();
        let sender = std::thread::spawn(move || tx.send(rendered(1, 7)).unwrap());
        assert_eq!(facts.wait_for_render(1, 7, Duration::from_secs(5)).unwrap().unresolved_face_text_total, 70);
        sender.join().unwrap();
    }

    /// A busy evaluator's full queue drops pointer motion, as the render
    /// thread's own queue does, and never stops the bridge keeping facts.
    #[test]
    fn a_full_queue_drops_motion_and_keeps_taking_facts() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (facts, out) = spawn(rx, 1).unwrap();
        let motion = || InputEvent::PositionedPointer(crate::thread_comm::PositionedPointerInput {
            position: crate::thread_comm::PointerPosition { x: 1.0, y: 1.0, target_frame_id: 1 },
            target: crate::thread_comm::PointerTarget::Unpresented,
            action: crate::thread_comm::PointerAction::Move { modifiers: 0 },
        });
        for _ in 0..100 { tx.send(motion()).unwrap(); }
        tx.send(rendered(1, 9)).unwrap();
        assert_eq!(facts.wait_for_render(1, 9, Duration::from_secs(5)).unwrap().presentation, 9);
        assert_eq!(out.len(), 1, "one motion queued, the rest dropped");
    }

    /// A closed window's render state goes; an older sample never replaces
    /// a newer one.
    #[test]
    fn samples_only_move_forward_and_go_with_their_window() {
        let facts = Facts::default();
        facts.take(rendered(2, 5));
        facts.take(rendered(2, 3));
        assert_eq!(facts.wait_for_render(2, 0, Duration::ZERO).unwrap().presentation, 5);
        assert!(facts.take(InputEvent::WindowClose { emacs_frame_id: 2 }).is_some(), "the close goes on");
        assert!(facts.wait_for_render(2, 0, Duration::ZERO).is_err());
    }

    /// A device reset wakes the evaluator's wait, as a newer render does,
    /// so an idle session repaints its images at once.
    #[test]
    fn a_reset_wakes_the_waiter() {
        let facts = Facts::default();
        let wakes = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counted = wakes.clone();
        assert!(facts.on_render(Arc::new(move || { counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed); })));
        assert!(facts.take(InputEvent::DisplayReset).is_none());
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(facts.images_reset_pending());
    }
}
