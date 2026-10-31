//! The GUI's keyboard for prompts read in the middle of a command
//! (romacs_vm::HostInput): GNU keyboard.c `read_char' redisplays, then
//! waits. The evaluator thread already owns the VM and the input channel, so
//! the wait is a plain blocking receive on that channel; the redisplay uses
//! the same layout engine and presentation sequence as the outer loop.
//!
//! Shared state is per evaluator thread (Rc/RefCell): the outer loop parks
//! the rest of its input batch here before it dispatches a command and takes
//! back whatever the prompt did not consume afterwards, so keys typed ahead of
//! a prompt reach it in order. A RefCell borrow held across a dispatch would
//! panic the moment a prompt waits: a re-entry is loud, never silent.
use romacs_display_runtime::paint_host::PaintSender;
use romacs_display_runtime::thread_comm::InputEvent;
use romacs_vm::{HostEvent, HostFrame, HostInput};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
pub struct Shared {
    /// Events received but not yet dispatched (the outer batch's remainder).
    pub pending: VecDeque<InputEvent>,
    /// Last presentation id submitted, by either loop.
    pub presentation: u64,
    /// The outer loop's in-flight presentation; the prompt settles it before
    /// it submits its own (the renderer retains only the latest sample).
    pub in_flight: Option<u64>,
    /// Set when the prompt settled `in_flight`: it and when it rendered.
    pub settled: Option<(u64, std::time::Instant)>,
    /// Latest frame geometry seen by the prompt.
    pub size: Option<(u32, u32)>,
    pub closed: bool,
    /// Under --trace-latency, when the prompt last read a key.
    pub key_read: Option<std::time::Instant>,
    /// The prompt's last painted frame, its selected window, presentation
    /// and window ends: an unchanged redisplay answers these again.
    pub painted: Option<(romacs_layout_input::FrameSnapshot, u64, u64, Vec<romacs_vm::WindowEnd>)>,
}

pub struct GuiHost {
    pub shared: Rc<RefCell<Shared>>,
    pub sender: Arc<PaintSender>,
    pub engine: Rc<RefCell<romacs_layout2::LayoutEngine>>,
    pub terminal_ready: crossbeam_channel::Receiver<()>,
    /// The process descriptors the next wait also watches.
    pub processes: Vec<(std::os::fd::RawFd, i16)>,
}

/// What one transport event means to a waiting prompt. By reference: a
/// tracked event's receipt must not be cloned or dropped here, only consumed
/// with the event itself.
fn classify(event: &InputEvent) -> Option<HostEvent> {
    match event {
        InputEvent::Tracked { event, .. } | InputEvent::Observed { event, .. } => classify(event),
        InputEvent::WindowClose { .. } => Some(HostEvent::Closed),
        &InputEvent::WindowFocus { focused, .. } => Some(HostEvent::Focus(focused)),
        &InputEvent::WindowResize { width, height, .. } if width > 0 && height > 0 => {
            Some(HostEvent::Resize { width, height })
        }
        &InputEvent::Key { ref key, modifiers, pressed, emacs_frame_id } => crate::native::translate(
            InputEvent::Key { key: key.clone(), modifiers, pressed, emacs_frame_id },
        )
        .map(HostEvent::Key),
        _ => None,
    }
}

/// What SHARED records of an event a prompt read: the frame closing, or its size.
fn note(shared: &mut Shared, event: &HostEvent) {
    match *event {
        HostEvent::Closed => shared.closed = true,
        HostEvent::Resize { width, height } => shared.size = Some((width, height)),
        HostEvent::Key(_) | HostEvent::Background | HostEvent::Process | HostEvent::Timeout | HostEvent::Focus(_) => {}
    }
}

/// A prompt's next event within TIMEOUT: a queued one, else what INPUT
/// brings, a terminal's or a process's readiness, or the timeout.  Events a
/// prompt ignores (releases, lone modifiers) go on waiting, on PROCESSES too.
fn next_event(shared: &std::cell::RefCell<Shared>, input: &crossbeam_channel::Receiver<InputEvent>,
              terminal: &crossbeam_channel::Receiver<()>, timeout: Option<Duration>,
              processes: &[(std::os::fd::RawFd, i16)]) -> Result<HostEvent, String> {
    let deadline = timeout.map(|timeout| std::time::Instant::now() + timeout);
    loop {
        let queued = next_queued(&mut shared.borrow_mut());
        let event = match queued {
            Next::Closed => return Ok(HostEvent::Closed),
            Next::Event(event) => event,
            // A termination signal whose wake other work took still ends the prompt.
            Next::Wait if romacs_vm::host::termination_signal(true).is_some() => {
                shared.borrow_mut().closed = true;
                return Ok(HostEvent::Closed);
            }
            Next::Wait => {
                let left = deadline.map(|end| end.saturating_duration_since(std::time::Instant::now()));
                match crate::terminal_wait::wait(input, terminal, left, processes) {
                    crate::terminal_wait::Wake::Input(event) => event,
                    // A termination signal ends the prompt as a closed frame
                    // does; the loop then runs `kill-emacs' (main.rs terminate).
                    crate::terminal_wait::Wake::Terminal if romacs_vm::host::termination_signal(true).is_some() => {
                        shared.borrow_mut().closed = true;
                        return Ok(HostEvent::Closed);
                    }
                    crate::terminal_wait::Wake::Terminal => return Ok(HostEvent::Background),
                    crate::terminal_wait::Wake::Process => return Ok(HostEvent::Process),
                    crate::terminal_wait::Wake::Closed => return Err("input closed while reading".into()),
                    crate::terminal_wait::Wake::Timeout => return Ok(HostEvent::Timeout),
                }
            },
        };
        if let Some(classified) = classify(&event) {
            note(&mut shared.borrow_mut(), &classified);
            return Ok(classified);
        }
    }
}

/// What a read takes next from the shared queue, without touching the channel.
#[derive(Debug)]
enum Next { Closed, Event(InputEvent), Wait }

/// Closing is sticky: once the frame is gone every later read answers Closed
/// at once (Lisp may catch the first `quit' and prompt again; that prompt must
/// not wait on a channel no window feeds any more).
fn next_queued(shared: &mut Shared) -> Next {
    if shared.closed {
        return Next::Closed;
    }
    match shared.pending.pop_front() {
        Some(event) => Next::Event(event),
        None => Next::Wait,
    }
}

impl GuiHost {
    /// Move every queued channel event into `pending`, dropping the ones a
    /// prompt ignores (releases, activations), so "pending" means a real event.
    fn drain(&mut self) {
        let mut shared = self.shared.borrow_mut();
        let available = self.sender.input().len();
        shared.pending.extend(self.sender.input().try_iter().take(available));
        let kept = std::mem::take(&mut shared.pending)
            .into_iter()
            .filter(|event| classify(event).is_some())
            .collect();
        shared.pending = kept;
    }


}

impl HostInput for GuiHost {
    fn clipboard_text(&mut self) -> Result<Option<String>, String> {
        self.sender.clipboard_text()
    }

    fn set_clipboard_text(&mut self, text: &str) -> Result<(), String> {
        self.sender.set_clipboard_text(text)
    }

    fn input_pending(&mut self) -> bool {
        self.drain();
        let shared = self.shared.borrow();
        // A closed frame is not repainted.
        shared.closed || !shared.pending.is_empty()
    }

    fn redisplay(&mut self, frame: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> {
        // As the loop's changed_frame: a timer run in a prompt that left the
        // frame as this prompt last painted it, with nothing presented
        // since, submits no frame and answers that paint's window ends.
        if let Some((painted, selected, presentation, ends)) = &self.shared.borrow().painted {
            let shared = self.shared.borrow();
            if *presentation == shared.presentation && shared.in_flight.is_none()
                && *selected == frame.selected && crate::frames_alike(painted, &frame.frame) {
                return Ok(ends.clone());
            }
        }
        let started = std::time::Instant::now();
        let wait = Duration::from_secs(30);
        let presentation = {
            let mut shared = self.shared.borrow_mut();
            if let Some(in_flight) = shared.in_flight.take() {
                self.sender.wait_for_render(in_flight, wait)?;
                shared.settled = Some((in_flight, std::time::Instant::now()));
            }
            shared.presentation = shared.presentation.checked_add(1).ok_or("presentation exhausted")?;
            shared.presentation
        };
        let laying = std::time::Instant::now();
        let (glyphs, ends) = crate::paint_prompt(&mut self.engine.borrow_mut(), &self.sender, presentation, &frame)?;
        let submitted = std::time::Instant::now();
        // Synchronous, like the oracle path: the prompt is on screen before
        // the evaluator blocks for its key.
        self.sender.wait_for_render(presentation, wait)?;
        self.shared.borrow_mut().painted = Some((frame.frame.clone(), frame.selected, presentation, ends.clone()));
        eprintln!(
            "{}",
            serde_json::json!({"diagnostic":"prompt-paint","presentation":presentation,
                "selected":frame.selected,
                "minibuffer":frame.frame.windows.last().map(|w| w.snapshot.text.utf8.clone()),
                "glyphs":glyphs})
        );
        // The key's command and hooks ran before this redisplay began.
        if let Some(read) = self.shared.borrow_mut().key_read.take() {
            let now = std::time::Instant::now();
            eprintln!("{}", serde_json::json!({"diagnostic":"prompt-key-latency","presentation":presentation,"units":"ns",
                "command_ns":started.duration_since(read).as_nanos(),"redisplay_ns":now.duration_since(started).as_nanos(),
                "key_to_render_ns":now.duration_since(read).as_nanos(),
                // command_ns includes snapshot_ns; redisplay_ns is layout_ns, the
                // wait for the previous frame, and render_ns.
                "snapshot_ns":frame.snapshot.as_nanos(),"layout_ns":submitted.duration_since(laying).as_nanos(),
                "render_ns":now.duration_since(submitted).as_nanos()}));
        }
        Ok(ends)
    }

    fn watch_processes(&mut self, fds: &[(std::os::fd::RawFd, i16)]) {
        self.processes = fds.to_vec();
    }

    fn next_event(&mut self, timeout: Option<Duration>) -> Result<HostEvent, String> {
        // The descriptors this read watches, through every wait it makes.
        let processes = std::mem::take(&mut self.processes);
        let event = next_event(&self.shared, self.sender.input(), &self.terminal_ready, timeout, &processes)?;
        if crate::tracing() && matches!(event, HostEvent::Key(_)) {
            self.shared.borrow_mut().key_read = Some(std::time::Instant::now());
        }
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_frame_answers_every_later_read_at_once() {
        let mut shared = Shared::default();
        shared.pending.push_back(InputEvent::WindowClose { emacs_frame_id: 0 });
        shared.pending.push_back(InputEvent::WindowClose { emacs_frame_id: 0 });
        let Next::Event(event) = next_queued(&mut shared) else { panic!("queued event first") };
        assert_eq!(classify(&event), Some(HostEvent::Closed));
        shared.closed = true; // what GuiHost::note records for that event
        for _ in 0..3 {
            assert!(matches!(next_queued(&mut shared), Next::Closed), "a later read must not wait");
        }
        assert_eq!(shared.pending.len(), 1, "nothing more is consumed once closed");
        assert!(matches!(next_queued(&mut Shared::default()), Next::Wait));
    }

    /// A read that first gets an event a prompt ignores (a key release)
    /// goes on watching its processes: output that arrives next ends the
    /// read at once, not at its time limit.
    #[test]
    fn an_ignored_event_keeps_the_processes_watched() {
        use romacs_display_runtime::host_types::FrontendKey;
        use std::os::fd::AsRawFd;
        // poll.h's POLLIN, as `Vm::process_wait_fds' gives it for output.
        const POLLIN: i16 = 0x1;
        let (keys, input) = crossbeam_channel::unbounded::<InputEvent>();
        let (_terminals, terminal) = crossbeam_channel::bounded::<()>(1);
        keys.send(InputEvent::Key { key: FrontendKey::Character('a'), modifiers: 0, pressed: false, emacs_frame_id: 0 }).unwrap();
        let (mut writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
        let wrote = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            std::io::Write::write_all(&mut writer, b"output").unwrap();
            writer
        });
        let shared = RefCell::new(Shared::default());
        let start = std::time::Instant::now();
        let event = next_event(&shared, &input, &terminal, Some(Duration::from_secs(3)), &[(reader.as_raw_fd(), POLLIN)]);
        assert!(matches!(event, Ok(HostEvent::Process)), "{event:?}");
        assert!(start.elapsed() < Duration::from_secs(1), "the read waited {:?}", start.elapsed());
        drop(wrote.join());
    }

    /// A focus change during a Lisp read reaches the read as GNU's special
    /// event instead of being dropped (keyboard.el runs its binding).
    #[test]
    fn focus_changes_reach_a_lisp_read() {
        for focused in [true, false] {
            let event = InputEvent::WindowFocus { focused, emacs_frame_id: 0 };
            assert_eq!(classify(&event), Some(HostEvent::Focus(focused)));
        }
    }
}
