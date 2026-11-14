use crate::thread_comm::{ClipboardCommand, ClipboardSelection};
use arboard::Clipboard;
use crossbeam_channel::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;
use winit::event_loop::OwnedDisplayHandle;

#[cfg(target_os = "linux")]
use arboard::{ClearExtLinux, GetExtLinux, LinuxClipboardKind, SetExtLinux};
#[cfg(target_os = "linux")]
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

mod text_policy;

#[cfg(target_os = "linux")]
mod wayland_data_control;

const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);
const REQUEST_QUEUE_CAPACITY: usize = 32;

trait ClipboardBackend: Send {
    fn set_text(&mut self, selection: ClipboardSelection, text: Option<&str>)
    -> Result<(), String>;

    fn text(&mut self, selection: ClipboardSelection) -> Result<text_policy::TextRead, String>;
}

enum ServiceCommand {
    Request(ClipboardCommand),
    Shutdown { acknowledged: Sender<()> },
}

/// A non-blocking handle to the serialized, display-owned clipboard worker.
///
/// Native clipboard calls never run on Winit's event-loop thread.  In
/// particular, reading from a Wayland selection may wait on another client;
/// isolating that wait keeps rendering and input responsive.
pub(crate) struct ClipboardService {
    commands: Sender<ServiceCommand>,
    /// Where requests are answered (`Fact::Reply`).
    answers: Answers,
    worker: Option<JoinHandle<()>>,
}

impl ClipboardService {
    pub(crate) fn for_display(display: OwnedDisplayHandle, answers: Sender<crate::thread_comm::Fact>) -> Result<Self, String> {
        std::cfg_select! {
            target_os = "linux" => {
                let raw_display = display
                    .display_handle()
                    .map_err(|err| format!("failed to access the native display: {err}"))?
                    .as_raw();
                if let RawDisplayHandle::Wayland(raw_display) = raw_display {
                    tracing::info!(
                        "Clipboard service using the native Wayland data-device backend"
                    );
                    // SAFETY: WaylandClipboard owns `display`, which keeps this
                    // wl_display alive until after `clipboard` is dropped.
                    let clipboard = unsafe {
                        smithay_clipboard::Clipboard::new(raw_display.display.as_ptr())
                    };
                    return Self::start(WaylandClipboard::new(clipboard, display), answers);
                }
            }
            _ => {
                let _ = display;
            }
        }

        tracing::info!("Clipboard service using the arboard platform backend");
        Self::start(ArboardClipboard::new()?, answers)
    }

    fn start(backend: impl ClipboardBackend + 'static, answers: Sender<crate::thread_comm::Fact>) -> Result<Self, String> {
        let (commands, receiver) = crossbeam_channel::bounded(REQUEST_QUEUE_CAPACITY);
        let answers = Answers(std::sync::Arc::new(std::sync::Mutex::new(Some(answers))));
        let worker_answers = answers.clone();
        let worker = std::thread::Builder::new()
            .name("romacs-clipboard".to_owned())
            .spawn(move || run_worker(Box::new(backend), receiver, &worker_answers))
            .map_err(|err| format!("failed to start clipboard worker: {err}"))?;
        Ok(Self {
            commands,
            answers,
            worker: Some(worker),
        })
    }

    #[cfg(test)]
    fn with_backend(backend: impl ClipboardBackend + 'static, answers: Sender<crate::thread_comm::Fact>) -> Self {
        Self::start(backend, answers).expect("clipboard worker should start")
    }

    pub(crate) fn submit(&self, command: ClipboardCommand) {
        match self.commands.try_send(ServiceCommand::Request(command)) {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(ServiceCommand::Request(command))) => {
                self.answers.send(reject_command(command, "clipboard worker queue is full".to_owned()));
            }
            Err(crossbeam_channel::TrySendError::Disconnected(ServiceCommand::Request(
                command,
            ))) => {
                self.answers.send(reject_command(command, "clipboard worker is unavailable".to_owned()));
            }
            Err(_) => unreachable!("submit only sends clipboard requests"),
        }
    }
}

impl Drop for ClipboardService {
    fn drop(&mut self) {
        // A worker detached below must not keep the render side's input
        // channel open: its waiters are answered by that channel's close.
        self.answers.close();
        let Some(worker) = self.worker.take() else {
            return;
        };
        let (acknowledged, acknowledgement) = crossbeam_channel::bounded(1);
        match self
            .commands
            .try_send(ServiceCommand::Shutdown { acknowledged })
        {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                // Dropping `commands` disconnects the worker after queued
                // requests finish.  It owns the display lifetime until then.
                tracing::warn!("clipboard worker queue is full during shutdown; detaching it");
                return;
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                if worker.join().is_err() {
                    tracing::warn!("clipboard worker panicked");
                }
                return;
            }
        }

        if acknowledgement
            .recv_timeout(WORKER_SHUTDOWN_TIMEOUT)
            .is_err()
        {
            // A foreign Wayland selection owner can stall a transfer forever.
            // Detaching is safe: the worker owns its OwnedDisplayHandle, so the
            // native display outlives every clipboard object it may still use.
            tracing::warn!("clipboard worker did not stop promptly; detaching it");
            return;
        }
        if worker.join().is_err() {
            tracing::warn!("clipboard worker panicked");
        }
    }
}

fn run_worker(mut backend: Box<dyn ClipboardBackend>, commands: Receiver<ServiceCommand>, answers: &Answers) {
    while let Ok(command) = commands.recv() {
        match command {
            ServiceCommand::Request(command) if command.is_expired() => {
                answers.send(reject_command(
                    command,
                    "clipboard request expired before execution".to_owned(),
                ));
            }
            ServiceCommand::Request(command) => answers.send(execute_command(backend.as_mut(), command)),
            ServiceCommand::Shutdown { acknowledged } => {
                // Acknowledgement means the native backend (including
                // smithay-clipboard's internal worker) has fully stopped.
                // If that drop stalls, ClipboardService's bounded wait will
                // detach this thread while its OwnedDisplayHandle stays live.
                drop(backend);
                let _ = acknowledged.send(());
                return;
            }
        }
    }
}

/// Run COMMAND on BACKEND: its request and its answer.
fn execute_command(backend: &mut dyn ClipboardBackend, command: ClipboardCommand) -> (u64, crate::thread_comm::Reply) {
    use crate::thread_comm::Reply;
    match command {
        ClipboardCommand::SetText { selection, text, request, .. } => {
            let result = backend.set_text(selection, text.as_deref());
            if let Err(err) = &result {
                tracing::warn!(?selection, "clipboard set failed: {err}");
            }
            (request, Reply::Done(result))
        }
        ClipboardCommand::GetText { selection, request, .. } => {
            // The evaluator-facing wire carries text or absence; the two
            // absence reasons stay inside the runtime (see `text_policy`).
            let result = backend
                .text(selection)
                .map(text_policy::TextRead::into_option);
            if let Err(err) = &result {
                tracing::warn!(?selection, "clipboard read failed: {err}");
            }
            (request, Reply::Text(result))
        }
    }
}

/// COMMAND refused for ERROR: its request and its answer.
pub(crate) fn reject_command(command: ClipboardCommand, error: String) -> (u64, crate::thread_comm::Reply) {
    use crate::thread_comm::Reply;
    match command {
        ClipboardCommand::SetText { request, .. } => (request, Reply::Done(Err(error))),
        ClipboardCommand::GetText { request, .. } => (request, Reply::Text(Err(error))),
    }
}

/// Where the service's answers go, until the service is dropped.
#[derive(Clone)]
struct Answers(std::sync::Arc<std::sync::Mutex<Option<Sender<crate::thread_comm::Fact>>>>);

impl Answers {
    fn send(&self, (request, reply): (u64, crate::thread_comm::Reply)) {
        let answers = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let sent = answers.as_ref().is_some_and(|answers| answers.send(crate::thread_comm::Fact::Reply { request, reply }).is_ok());
        if !sent { tracing::debug!("clipboard answer: nobody waits for it now"); }
    }

    fn close(&self) {
        *self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

struct ArboardClipboard {
    clipboard: Clipboard,
}

impl ArboardClipboard {
    fn new() -> Result<Self, String> {
        Clipboard::new()
            .map(|clipboard| Self { clipboard })
            .map_err(|err| format!("failed to initialize the system clipboard: {err}"))
    }

    /// Classify an arboard read into the clipboard's text vocabulary; this is
    /// the adapter between arboard's error type and `text_policy`.
    fn text_result(
        result: Result<String, arboard::Error>,
    ) -> Result<text_policy::TextRead, String> {
        match result {
            Ok(text) => Ok(text_policy::TextRead::Text(text)),
            Err(err) => match text_policy::classify_arboard(&err) {
                Some(failure) => Ok(text_policy::TextRead::from(failure)),
                None => Err(err.to_string()),
            },
        }
    }
}

impl ClipboardBackend for ArboardClipboard {
    fn set_text(
        &mut self,
        selection: ClipboardSelection,
        text: Option<&str>,
    ) -> Result<(), String> {
        std::cfg_select! {
            target_os = "linux" => {
                let selection = match selection {
                    ClipboardSelection::Clipboard => LinuxClipboardKind::Clipboard,
                };
                match text {
                    Some(text) => self
                        .clipboard
                        .set()
                        .clipboard(selection)
                        .text(text.to_owned()),
                    None => self.clipboard.clear_with().clipboard(selection),
                }
                .map_err(|err| err.to_string())
            }
            _ => {
                match selection {
                    ClipboardSelection::Clipboard => match text {
                        Some(text) => self.clipboard.set_text(text.to_owned()),
                        None => self.clipboard.clear(),
                    }
                    .map_err(|err| err.to_string()),
                    ClipboardSelection::Primary => {
                        self.primary.store(text);
                        Ok(())
                    }
                }
            }
        }
    }

    fn text(&mut self, selection: ClipboardSelection) -> Result<text_policy::TextRead, String> {
        std::cfg_select! {
            target_os = "linux" => {
                let selection = match selection {
                    ClipboardSelection::Clipboard => LinuxClipboardKind::Clipboard,
                };
                Self::text_result(self.clipboard.get().clipboard(selection).text())
            }
            _ => {
                match selection {
                    ClipboardSelection::Clipboard => Self::text_result(self.clipboard.get_text()),
                    ClipboardSelection::Primary => Ok(self.primary.load()),
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
struct WaylandClipboard {
    // Field order is significant: drop the protocol objects before their display.
    clipboard: smithay_clipboard::Clipboard,
    data_control: DataControl,
    _display_owner: OwnedDisplayHandle,
}

/// Lazily bound fallback for CLIPBOARD reads; see `wayland_data_control`.
#[cfg(target_os = "linux")]
enum DataControl {
    Unbound,
    Unavailable,
    Bound(wayland_data_control::DataControlReader),
}

#[cfg(target_os = "linux")]
impl WaylandClipboard {
    fn new(clipboard: smithay_clipboard::Clipboard, display: OwnedDisplayHandle) -> Self {
        Self {
            clipboard,
            data_control: DataControl::Unbound,
            _display_owner: display,
        }
    }

    /// The fallback's answer, or `None` when this compositor has no
    /// data-control (or the reader could not be bound); the caller keeps
    /// smithay's classification in that case.
    fn data_control_text(&mut self) -> Result<Option<text_policy::TextRead>, String> {
        if let DataControl::Unbound = self.data_control {
            self.data_control = match self.bind_data_control() {
                Ok(Some(reader)) => DataControl::Bound(reader),
                Ok(None) => DataControl::Unavailable,
                Err(err) => {
                    tracing::warn!("clipboard data-control fallback unavailable: {err}");
                    DataControl::Unavailable
                }
            };
        }
        match &mut self.data_control {
            DataControl::Bound(reader) => Ok(Some(reader.read_text()?)),
            DataControl::Unbound | DataControl::Unavailable => Ok(None),
        }
    }

    fn bind_data_control(&self) -> Result<Option<wayland_data_control::DataControlReader>, String> {
        let RawDisplayHandle::Wayland(raw_display) = self
            ._display_owner
            .display_handle()
            .map_err(|err| format!("failed to access the native display: {err}"))?
            .as_raw()
        else {
            return Ok(None);
        };
        // SAFETY: `_display_owner` keeps the display alive and is dropped
        // after `data_control` (field order above).
        unsafe { wayland_data_control::DataControlReader::connect(raw_display.display) }
    }
}

#[cfg(target_os = "linux")]
impl ClipboardBackend for WaylandClipboard {
    fn set_text(
        &mut self,
        selection: ClipboardSelection,
        text: Option<&str>,
    ) -> Result<(), String> {
        let Some(text) = text else {
            return Err(
                "disowning a selection is not supported by the native Wayland clipboard backend"
                    .to_owned(),
            );
        };
        match selection {
            ClipboardSelection::Clipboard => self.clipboard.store(text.to_owned()),
        }
        // smithay-clipboard queues ownership requests to its protocol worker;
        // success here means accepted by that API, not compositor confirmation.
        Ok(())
    }

    fn text(&mut self, selection: ClipboardSelection) -> Result<text_policy::TextRead, String> {
        // Hyprland delivers the selection only to winit's data device, so
        // smithay's device can report an absence while the compositor holds a
        // selection.  Which selections may then consult the data-control
        // fallback is `text_policy::wayland_read`'s business.
        let native = match selection {
            ClipboardSelection::Clipboard => self.clipboard.load(),
        };
        text_policy::wayland_read(selection, native, || self.data_control_text())
    }
}

#[cfg(test)]
#[path = "clipboard/tests/clipboard_test.rs"]
mod tests;

#[cfg(test)]
#[path = "clipboard/tests/contract_test.rs"]
mod contract_tests;
