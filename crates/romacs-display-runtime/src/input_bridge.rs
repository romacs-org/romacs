//! The evaluator's side of the seam: one thread takes the render thread's
//! two streams.  Facts (what rendered, the adapter, lost images, the
//! native-model renderer, the pointer, key-repeat timing) and replies to
//! the evaluator's requests come on their own stream and are taken at once:
//! facts kept as state the evaluator reads, replies handed to their
//! waiters.  Input events come in order, with backpressure, and are passed
//! on to the evaluator's queue.
//!
//! The render thread shares no memory with the evaluator for these: it only
//! sends, so the thread here is where a transport from another process
//! would deliver them instead.
use crate::thread_comm::{Fact, InputEvent};
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

/// What answers a request when its reply comes: called once, on the bridge.
pub(crate) type Answer = Box<dyn FnOnce(crate::thread_comm::Reply) + Send>;

/// The facts and the wait for a render.
#[derive(Default)]
pub(crate) struct Facts {
    state: Mutex<State>,
    /// The requests waiting for their replies, by id.
    awaiting: Mutex<HashMap<u64, Answer>>,
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

/// Apply FACT to STATE: what the evaluator's waits must hear of it.
fn apply(state: &mut State, fact: Fact) -> Notice {
    match fact {
        Fact::PresentationRendered { frame, presentation, unresolved_face_text_total } => {
            let latest = state.rendered.get(&frame).map_or(0, |sample| sample.presentation);
            if presentation >= latest {
                state.rendered.insert(frame, PaintDiagnostic { presentation, unresolved_face_text_total });
            }
            if presentation > latest { Notice::Rendered(frame) } else { Notice::Nothing }
        }
        Fact::AdapterChosen { software } => { state.software = Some(software); Notice::Nothing }
        Fact::ImageEvicted(image) => { state.evicted.push(image); Notice::Nothing }
        Fact::DisplayReset => { state.reset = true; Notice::Reset }
        Fact::ModelRenderer { live } => { state.model_renderer = live; Notice::Nothing }
        Fact::PointerOver { at } => { state.pointer = at; Notice::Nothing }
        Fact::KeyRepeatTiming(RepeatTiming::Repeat(after)) => { state.key_repeats.repeat(after); Notice::Nothing }
        Fact::KeyRepeatTiming(RepeatTiming::Held(held)) => { state.key_repeats.released(held); Notice::Nothing }
        Fact::FrameClosed(frame) => { state.rendered.remove(&frame); Notice::Nothing }
        Fact::Reply { .. } => Notice::Nothing,
    }
}

impl Facts {
    /// The facts, through a lock no panic leaves unusable: each fact is a
    /// plain value, whole after any write.
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn awaiting(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Answer>> {
        self.awaiting.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Answer REQUEST's reply with ANSWER when it comes.
    pub(crate) fn expect(&self, request: u64, answer: Answer) {
        self.awaiting().insert(request, answer);
    }

    /// REQUEST was never sent: no reply will come.
    pub(crate) fn forget(&self, request: u64) {
        self.awaiting().remove(&request);
    }

    /// The render side is gone: every waiting request's answer is dropped,
    /// so its waiter sees its channel close at once, not at its deadline.
    fn abandon(&self) {
        self.awaiting().clear();
    }

    /// Keep FACT, or hand a reply to its waiter.
    fn take(&self, fact: Fact) {
        if let Fact::Reply { request, reply } = fact {
            let answer = self.awaiting().remove(&request);
            if let Some(answer) = answer { answer(reply); }
            return;
        }
        let notice = apply(&mut self.state(), fact);
        if let Notice::Rendered(_) = notice { self.rendered.notify_all(); }
        if matches!(notice, Notice::Rendered(crate::paint_host::PRIMARY_FRAME) | Notice::Reset)
            && let Some(wake) = self.wake.get()
        {
            wake();
        }
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

/// Start the bridge over the render thread's INPUT and FACTS: the facts it
/// keeps and the channel input goes on, in the order it came.  It ends when
/// the render thread's input ends and what it held has gone on, and so does
/// that channel.
pub(crate) fn spawn(input: Receiver<InputEvent>, facts: Receiver<Fact>, capacity: usize)
    -> std::io::Result<(Arc<Facts>, Receiver<InputEvent>)> {
    let kept = Arc::new(Facts::default());
    let (tx, rx) = crossbeam_channel::bounded(capacity);
    let bridge = kept.clone();
    std::thread::Builder::new().name("romacs-input".into()).spawn(move || run(&bridge, &input, &facts, &tx))?;
    Ok((kept, rx))
}

/// How many input events the bridge holds for a busy evaluator beyond its
/// own queue.  Past that it stops reading input, the render thread's queue
/// fills and holds it back; facts and replies are read throughout.
const HELD: usize = 4096;

/// Pass input on in order, as the render thread would have queued it:
/// pointer motion dropped while the evaluator is behind, every other event
/// held for it.  Facts and replies are taken whatever the evaluator holds.
fn run(kept: &Facts, input: &Receiver<InputEvent>, facts: &Receiver<Fact>, out: &Sender<InputEvent>) {
    let mut held: std::collections::VecDeque<InputEvent> = std::collections::VecDeque::new();
    let (mut input_open, mut facts_open) = (true, true);
    while input_open || !held.is_empty() {
        let mut select = crossbeam_channel::Select::new();
        let tell = facts_open.then(|| select.recv(facts));
        let read = (input_open && held.len() < HELD).then(|| select.recv(input));
        let room = (!held.is_empty()).then(|| select.send(out));
        let ready = select.select();
        let index = Some(ready.index());
        if index == tell {
            match ready.recv(facts) {
                Ok(fact) => kept.take(fact),
                Err(_) => {
                    // The render side is gone, though input may still wait.
                    facts_open = false;
                    kept.abandon();
                }
            }
        } else if index == read {
            match ready.recv(input) {
                Ok(event) => {
                    if !crate::thread_comm::is_lossy_input_event(&event) {
                        held.push_back(event);
                    } else if held.is_empty() && let Err(crossbeam_channel::TrySendError::Disconnected(_)) = out.try_send(event) {
                        return kept.abandon();
                    }
                }
                Err(_) => {
                    // The render side is gone: no reply will come, though
                    // what it sent still reaches the evaluator.
                    input_open = false;
                    kept.abandon();
                }
            }
        } else if index == room {
            let Some(event) = held.pop_front() else { continue };
            if ready.send(out, event).is_err() { return kept.abandon(); }
        }
    }
    // What the render side told before it went still counts.
    for fact in facts.try_iter() { kept.take(fact); }
    kept.abandon();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_comm::Reply;

    fn key() -> InputEvent {
        InputEvent::Key { key: crate::host_types::FrontendKey::Character('k'), modifiers: 0, pressed: true, emacs_frame_id: 1 }
    }

    fn rendered(frame: u64, presentation: u64) -> Fact {
        Fact::PresentationRendered { frame, presentation, unresolved_face_text_total: presentation * 10 }
    }

    fn bridge(capacity: usize) -> (Sender<InputEvent>, Sender<Fact>, Arc<Facts>, Receiver<InputEvent>) {
        let (input, input_rx) = crossbeam_channel::unbounded();
        let (facts, facts_rx) = crossbeam_channel::unbounded();
        let (kept, out) = spawn(input_rx, facts_rx, capacity).unwrap();
        (input, facts, kept, out)
    }

    /// Facts stay with the bridge; input goes on, in order.
    #[test]
    fn facts_are_kept_and_input_passes_in_order() {
        let (input, facts, kept, out) = bridge(16);
        for fact in [Fact::AdapterChosen { software: true }, Fact::PointerOver { at: Some((3, 4)) },
                     Fact::ModelRenderer { live: true }, Fact::DisplayReset, Fact::ImageEvicted(7),
                     Fact::KeyRepeatTiming(RepeatTiming::Repeat(Duration::from_millis(40)))] {
            facts.send(fact).unwrap();
        }
        input.send(key()).unwrap();
        input.send(key()).unwrap();
        drop(input);
        assert_eq!(out.iter().count(), 2);
        assert_eq!(kept.software_adapter(), Some(true));
        assert_eq!(kept.pointer(), Some((3, 4)));
        assert!(kept.model_renderer());
        assert!(kept.images_reset_pending());
        assert_eq!(kept.take_lost_images(), (vec![7], true));
        assert_eq!(kept.take_lost_images(), (vec![], false), "taken once");
        assert_eq!(kept.take_key_repeats().repeats, 1);
    }

    /// A wait for a render is answered per frame, by the frame's latest
    /// presentation; the primary frame's newer ones wake the evaluator once
    /// each, and so does a device reset.
    #[test]
    fn renders_and_resets_wake_the_waiter() {
        let kept = Facts::default();
        let wakes = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counted = wakes.clone();
        assert!(kept.on_render(Arc::new(move || { counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed); })));
        assert!(kept.wait_for_render(1, 1, Duration::ZERO).is_err(), "nothing drawn yet");
        for (frame, presentation) in [(1, 1), (1, 1), (2, 5), (1, 3)] {
            kept.take(rendered(frame, presentation));
        }
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 2, "no wake for a frame drawn again or another frame");
        assert_eq!(kept.wait_for_render(1, 2, Duration::ZERO).unwrap().presentation, 3, "a later presentation settles an earlier one");
        assert!(kept.wait_for_render(1, 4, Duration::ZERO).is_err(), "stale samples are no evidence");
        assert_eq!(kept.wait_for_render(2, 5, Duration::ZERO).unwrap().unresolved_face_text_total, 50);
        kept.take(rendered(2, 4));
        assert_eq!(kept.wait_for_render(2, 0, Duration::ZERO).unwrap().presentation, 5, "samples only move forward");
        kept.take(Fact::DisplayReset);
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 3);
        kept.take(Fact::FrameClosed(2));
        assert!(kept.wait_for_render(2, 0, Duration::ZERO).is_err(), "a closed window's sample goes");
    }

    #[test]
    fn a_wait_sees_a_render_from_another_thread() {
        let (_input, facts, kept, _out) = bridge(16);
        let sender = std::thread::spawn(move || facts.send(rendered(1, 7)).unwrap());
        assert_eq!(kept.wait_for_render(1, 7, Duration::from_secs(5)).unwrap().unresolved_face_text_total, 70);
        sender.join().unwrap();
    }

    /// Input beyond what the evaluator and the bridge hold waits; facts and
    /// replies never do, and motion is dropped while anything is held.
    #[test]
    fn a_full_queue_never_stops_facts_or_replies() {
        let (input, facts, kept, out) = bridge(1);
        let motion = || InputEvent::PositionedPointer(crate::thread_comm::PositionedPointerInput {
            position: crate::thread_comm::PointerPosition { x: 1.0, y: 1.0, target_frame_id: 1 },
            target: crate::thread_comm::PointerTarget::Unpresented,
            action: crate::thread_comm::PointerAction::Move { modifiers: 0 },
        });
        for _ in 0..(HELD + 100) { input.send(key()).unwrap(); }
        for _ in 0..100 { input.send(motion()).unwrap(); }
        let (answered, answer) = crossbeam_channel::bounded(1);
        kept.expect(5, Box::new(move |reply| { let _ = answered.send(reply); }));
        facts.send(Fact::Reply { request: 5, reply: Reply::Done(Ok(())) }).unwrap();
        facts.send(rendered(1, 9)).unwrap();
        assert_eq!(answer.recv_timeout(Duration::from_secs(5)).unwrap(), Reply::Done(Ok(())));
        assert_eq!(kept.wait_for_render(1, 9, Duration::from_secs(5)).unwrap().presentation, 9);
        drop(input);
        assert_eq!(out.iter().filter(|event| matches!(event, InputEvent::Key { .. })).count(), HELD + 100, "every key arrives, in order");
    }

    /// A reply goes to its own waiter, once; one nobody awaits is dropped;
    /// the render side's end closes every waiter's channel at once, even
    /// while held input still waits for the evaluator.
    #[test]
    fn replies_reach_their_waiters_and_the_end_closes_the_rest() {
        let (input, facts, kept, out) = bridge(1);
        let waiter = |kept: &Facts, request| {
            let (answered, answer) = crossbeam_channel::bounded(1);
            kept.expect(request, Box::new(move |reply| { let _ = answered.send(reply); }));
            answer
        };
        let (one, two) = (waiter(&kept, 1), waiter(&kept, 2));
        facts.send(Fact::Reply { request: 1, reply: Reply::Done(Ok(())) }).unwrap();
        facts.send(Fact::Reply { request: 9, reply: Reply::Done(Ok(())) }).unwrap();
        assert_eq!(one.recv_timeout(Duration::from_secs(5)).unwrap(), Reply::Done(Ok(())));
        for _ in 0..3 { input.send(key()).unwrap(); }
        drop(input);
        assert!(two.recv_timeout(Duration::from_secs(5)).is_err(), "closed once the render side went, with input still held");
        assert_eq!(out.iter().count(), 3);
    }

    /// The fact stream's end releases every waiter even while input is held
    /// at the bridge's limit and the evaluator reads nothing.
    #[test]
    fn the_fact_streams_end_releases_waiters_behind_held_input() {
        let (input, facts, kept, _out) = bridge(1);
        for _ in 0..(HELD + 10) { input.send(key()).unwrap(); }
        let (answered, answer) = crossbeam_channel::bounded(1);
        kept.expect(3, Box::new(move |reply| { let _ = answered.send(reply); }));
        drop(facts);
        assert!(matches!(answer.recv_timeout(Duration::from_secs(5)), Err(crossbeam_channel::RecvTimeoutError::Disconnected)));
        drop(input);
    }
}
