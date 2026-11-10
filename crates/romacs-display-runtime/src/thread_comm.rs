//! Thread communication infrastructure for two-thread architecture.
//!
//! Provides channels between the evaluator and render threads. Input wakeups
//! are owned by the evaluator's cross-platform wait notifier after the input
//! bridge queues a converted event.

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use std::sync::{Arc, Mutex};
use std::time::Instant;

mod command_channel;
pub(crate) use command_channel::{CommandReceiver, CommandSender, command_channel};
mod frame_opacity;
use crate::host_types::GuiFrameGeometryHints;
pub(crate) use frame_opacity::FrameOpacityState;
use romacs_display_protocol::{
    ImageId, ImageLoadToken,
};
pub use romacs_display_protocol::{
    ImageStateEvent, MenuBarItem, PopupMenuItem, TabBarItem, ToolBarImageSource, ToolBarItem,
    ToolBarItemType, VisualConfig,
};

/// Selection addressed by a clipboard request.
///
/// `Clipboard` is the system clipboard; `Primary` is the X11 or Wayland
/// primary selection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ClipboardSelection {
    Clipboard,
}

/// Monitor information transported from the frontend to the evaluator.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
    pub width_mm: i32,
    pub height_mm: i32,
    pub name: Option<String>,
}

/// Frame-local logical coordinates shared by one pointer action and its
/// presentation-qualified target. Keeping the coordinates here prevents the
/// semantic hit and raw action from disagreeing about where the input occurred.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerPosition {
    pub x: f32,
    pub y: f32,
    pub target_frame_id: u64,
}

/// Where the pointer is over the primary window, in logical pixels: what a
/// display answers GNU's `mouse_position_hook' with, read at the moment it
/// is asked.  Nothing while the pointer is elsewhere.
#[derive(Debug)]
pub(crate) struct PointerCell(std::sync::atomic::AtomicU64);

impl Default for PointerCell {
    fn default() -> Self { Self(std::sync::atomic::AtomicU64::new(Self::OUTSIDE)) }
}

impl PointerCell {
    const OUTSIDE: u64 = u64::MAX;
    pub fn set(&self, at: Option<(i32, i32)>) {
        let packed = at.map_or(Self::OUTSIDE, |(x, y)| u64::from(x as u32) << 32 | u64::from(y as u32));
        self.0.store(packed, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn get(&self) -> Option<(i32, i32)> {
        let packed = self.0.load(std::sync::atomic::Ordering::Relaxed);
        (packed != Self::OUTSIDE).then_some(((packed >> 32) as u32 as i32, packed as u32 as i32))
    }
}

/// How a pointer position relates to the immutable presentation on screen.
///
/// `Unpresented` is explicit for exposed/native surface area. A producer cannot
/// accidentally omit presentation state from a pointer action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerTarget {
    Presented {
        presentation: u64,
        hit: Option<romacs_display_protocol::PresentedHit>,
    },
    Unpresented,
}

/// The unit carried by a scroll delta. This replaces the invalid state where a
/// boolean precision flag can disagree with the meaning of the numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollDelta {
    Lines { x: f32, y: f32 },
    Pixels { x: f32, y: f32 },
}

/// Pointer action interpreted at one [`PointerPosition`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerAction {
    Button {
        button: u32,
        pressed: bool,
        modifiers: u32,
    },
    Move {
        modifiers: u32,
    },
    Scroll {
        delta: ScrollDelta,
        modifiers: u32,
    },
}

/// Atomic display-to-evaluator pointer input.
///
/// The transport has one variant for native move, button, and scroll actions,
/// so target qualification cannot be sent, reordered, or forgotten separately.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedPointerInput {
    pub position: PointerPosition,
    pub target: PointerTarget,
    pub action: PointerAction,
}

/// Input event from render thread to Emacs
#[derive(Debug, Clone)]
pub enum InputEvent {
    Tracked {
        receipt: romacs_display_protocol::input_progress::InputDelivery,
        event: Box<InputEvent>,
    },
    /// Bytes read directly from a Unix TTY.
    ///
    /// This transport fact deliberately carries no terminal-sequence or
    /// modifier interpretation.  The evaluator applies
    /// `keyboard-coding-system` and `input-decode-map` in input order.
    RawTtyBytes {
        bytes: Vec<u8>,
        emacs_frame_id: u64,
    },
    Key {
        key: crate::host_types::FrontendKey,
        modifiers: u32,
        pressed: bool,
        /// Emacs frame_id of the window that produced the key event
        emacs_frame_id: u64,
    },
    PositionedPointer(PositionedPointerInput),
    WindowResize {
        width: u32,
        height: u32,
        /// Physical device pixels per logical Emacs pixel.
        scale_factor: f64,
        /// Emacs frame_id of the window that resized
        emacs_frame_id: u64,
    },
    WindowClose {
        /// Emacs frame_id of the window being closed
        emacs_frame_id: u64,
    },
    WindowFocus {
        focused: bool,
        /// Emacs frame_id of the window that gained/lost focus
        emacs_frame_id: u64,
    },
    /// Monitor configuration changed on the active terminal.
    MonitorsChanged {
        monitors: Vec<MonitorInfo>,
    },
    /// The wgpu device was lost (user shader hang → driver reset) and the
    /// render thread rebuilt its GPU state from scratch. Every renderer-side
    /// media object (decoded images, videos, shader
    /// surfaces, the frame post shader) is gone; the evaluator must
    /// re-resolve them and force a full redisplay.
    DisplayReset,
    /// Image decoding completed or renderer residency was lost.
    ImageStateChanged {
        event: ImageStateEvent,
    },
    /// A shader surface failed to build on the render thread AFTER the Lisp
    /// thread's naga pre-validation accepted it (the naga-accepts /
    /// wgpu-rejects edge, e.g. a device limit). Carries the surface id and the
    /// renderer's error so the evaluator can surface it to Lisp instead of the
    /// failure only living in a log line + a silently-blank quad.
    SurfaceCreateFailed {
        id: u32,
        error: String,
    },
    /// Something the renderer reports about native model `id` (design §3):
    /// load outcome, request settlement, readiness, camera and picks.
    Model {
        id: u32,
        event: romacs_display_protocol::model::ModelEvent,
    },
    /// A current full-frame shader request passed evaluator-side validation
    /// but was rejected later by the active device or a concurrently changed
    /// quality policy. The evaluator reports this through
    /// `romacs-frame-shader-error-functions`.
    FrameShaderFailed {
        error: String,
    },
    /// Popup menu selection made (index into menu items, -1 = cancelled)
    MenuSelection {
        index: i32,
        token: Option<romacs_display_protocol::menu::MenuToken>,
    },
    /// File(s) dropped onto the window.
    ///
    /// Carries no position. GNU builds a drop's position with
    /// `make_lispy_position` (src/keyboard.c, `DRAG_N_DROP_EVENT`) — the same
    /// posn a mouse click gets, naming the window, the part of it, and the
    /// buffer position under the pointer — so the eventual target here is the
    /// presentation-qualified route `PresentedPointer` already takes, not a
    /// pair of surface coordinates. A pair sitting here would be the wrong
    /// answer waiting to be used: it is the pointer's position on the root
    /// surface, while everything a drop has to be resolved against belongs to
    /// the presentation, and the two are different pixels for the length of a
    /// pane morph.
    FileDrop {
        paths: Vec<String>,
    },
    /// Toolbar button clicked (index into toolbar items)
    ToolBarClick {
        index: i32,
        emacs_frame_id: u64,
    },
    /// Pointer observation resolved against an immutable displayed presentation.
    PresentedPointer {
        presentation: u64,
        interaction: u32,
        pressed: bool,
        button: u8,
        x: f32,
        y: f32,
        emacs_frame_id: u64,
    },
    /// Renderer installed this presentation as its drawing and hit-test source.
    /// This editor-snapshot lifetime event is neither GPU submission nor native
    /// display confirmation. Ingestion emits it independently of frame pacing.
    PresentationActivated {
        presentation: u64,
        emacs_frame_id: u64,
    },
    /// Renderer rejected or superseded this presentation before activation.
    PresentationDiscarded {
        presentation: u64,
        emacs_frame_id: u64,
    },
    /// Renderer no longer displays or generates hits for this presentation.
    PresentationRetired {
        presentation: u64,
    },
    /// Menu bar item clicked. `menu_x` is the Emacs menu-bar column used by
    /// legacy Lisp paths; `key` is the exact rendered top-level menu key; and
    /// `anchor` is the frame-local logical-pixel rectangle used by the native
    /// popup renderer.
    MenuBarClick {
        request_id: Option<romacs_display_protocol::menu::MenuBarRequestId>,
        index: i32,
        key: String,
        menu_x: f32,
        anchor: PopupAnchorRect,
        emacs_frame_id: u64,
    },
}

pub(crate) type PopupAnchorRect = romacs_display_protocol::Rect;

/// Frame reference in commands flowing from Emacs to the render thread.
///
/// Replaces raw `u64` `emacs_frame_id` — no sentinel values.
/// Matches GNU Emacs convention: 0 is never a valid frame ID
/// (`frame_next_id = 1` in GNU Emacs `frame.c:343`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum FrameRef {
    /// Route to the primary frame (resolved at render-time).
    Primary,
    /// Route to a specific frame by its Emacs-assigned ID.
    Frame(u64),
}

impl FrameRef {
    pub(crate) fn raw_id(&self) -> u64 {
        match self {
            Self::Primary => 0,
            Self::Frame(id) => *id,
        }
    }
}

impl From<FrameRef> for u64 {
    fn from(f: FrameRef) -> u64 {
        f.raw_id()
    }
}



/// Lifecycle commands for the render thread.
#[derive(Debug)]
pub(crate) enum LifecycleCommand {
    /// Shutdown the render thread
    Shutdown,
}

/// Window and chrome management commands.
#[derive(Debug)]
pub(crate) enum WindowCommand {
    /// Wake presentation after a synchronous CPU opacity control operation.
    RefreshFrameOpacity,
    /// Set the window title
    SetWindowTitle { title: String },
    /// Update geometry hints for a specific GUI frame window.
    /// `frame.raw_id() == 0` also targets the adopted primary window.
    SetFrameGeometryHints {
        frame: FrameRef,
        geometry_hints: GuiFrameGeometryHints,
    },
    /// Create a new OS window for a top-level Emacs frame
    CreateWindow {
        frame: FrameRef,
        width: u32,
        height: u32,
        title: String,
        geometry_hints: GuiFrameGeometryHints,
    },
    /// One bounded admission owns creation, readiness and independent rollback.
    RealizeFrame {
        frame: FrameRef,
        width: u32,
        height: u32,
        title: String,
        geometry_hints: GuiFrameGeometryHints,
        visual: Option<VisualConfig>,
        adopt_primary: bool,
        // Deferred realization owns readiness; legacy ownership is admission-only.
        reply: Option<Sender<Result<(), String>>>,
        live: std::sync::Arc<std::sync::atomic::AtomicBool>,
        deadline: std::time::Instant,
    },
    /// Associate the already-created primary OS window with its real Emacs frame ID.
    AdoptPrimaryFrame { frame: FrameRef },
    /// Destroy an OS window for a top-level Emacs frame
    DestroyWindow { frame: FrameRef },
    /// Request window attention (urgency hint / taskbar flash)
    RequestAttention { urgent: bool },
}



/// Asset and embedded-content commands.
#[derive(Debug)]
pub(crate) enum AssetCommand {
    /// Load image from encoded data bytes (PNG, JPEG, SVG, etc.)
    /// Already-completed CPU semantics; the renderer only establishes residency.
    ImageLoadDecoded {
        load: ImageLoadToken,
        decoded: std::sync::Arc<romacs_renderer_wgpu::image_cache::SemanticImageDecoded>,
    },
    /// Retire an image after its last render presentation releases it.
    ImageRetire { image: ImageId },
    /// Debug-only: latch the device-lost flag so the full device-loss
    /// recovery path (GPU rebuild + `InputEvent::DisplayReset`) runs against
    /// a healthy device. Sent by the hidden `romacs--debug-lose-device`
    /// builtin; never used in production paths.
    DebugSimulateDeviceLoss,
}


/// Config and styling commands.
#[derive(Debug)]
// This public command enum preserves its established by-value payload API.
// Boxing `VisualConfig` only to narrow the enum would break downstream
// constructors and destructuring code.
#[allow(clippy::large_enum_variant)]
pub(crate) enum ConfigCommand {
    /// Replace the complete, already validated visual configuration snapshot.
    SetVisualConfig(VisualConfig),
}

/// Clipboard requests routed through the display owner to its serialized worker.
///
/// The evaluator may await `reply`, but the Winit event loop only forwards the
/// request and never performs native clipboard I/O itself.
#[derive(Debug)]
pub(crate) enum ClipboardCommand {
    SetText {
        selection: ClipboardSelection,
        text: Option<String>,
        expires_at: Instant,
        reply: Sender<Result<(), String>>,
    },
    GetText {
        selection: ClipboardSelection,
        expires_at: Instant,
        reply: Sender<Result<Option<String>, String>>,
    },
}

impl ClipboardCommand {
    pub(crate) fn is_expired(&self) -> bool {
        let expires_at = match self {
            Self::SetText { expires_at, .. } | Self::GetText { expires_at, .. } => expires_at,
        };
        Instant::now() >= *expires_at
    }
}

/// One paint: the primary frame, the child frames shown over it, lowest
/// first, and the text animations drawn over the primary frame, each by
/// its key. A child or an animation absent from the next paint is removed.
#[derive(Debug)]
pub(crate) struct PaintScene {
    pub root: romacs_display_protocol::FrameGlyphBuffer,
    pub children: Vec<romacs_display_protocol::FrameGlyphBuffer>,
    pub text_animations: Vec<(u64, romacs_display_protocol::text_animation::TextAnimation)>,
}

impl From<romacs_display_protocol::FrameGlyphBuffer> for PaintScene {
    fn from(root: romacs_display_protocol::FrameGlyphBuffer) -> Self {
        Self { root, children: Vec::new(), text_animations: Vec::new() }
    }
}

/// Command from Emacs to render thread
#[derive(Debug)]
// This public transport enum preserves its established by-value command API.
// Boxing `Config` would move the same downstream break one level outward.
#[allow(clippy::large_enum_variant)]
pub(crate) enum RenderCommand {
    Paint(Box<PaintScene>),
    Lifecycle(LifecycleCommand),
    Window(WindowCommand),
    Asset(AssetCommand),
    Config(ConfigCommand),
    Clipboard(ClipboardCommand),
}

/// Channel capacities
// GNU Emacs' `kbd_buffer` holds 4096 input events and `tty_read_avail_input`
// stops reading terminal bytes when the buffer is under pressure rather than
// silently dropping command input.  Keep Romacs' render-to-evaluator input
// queue at the same scale and use backpressure for durable user input below.
const INPUT_CHANNEL_CAPACITY: usize = 4096;
const COMMAND_CHANNEL_CAPACITY: usize = 64;

/// Communication channels between threads
pub(crate) struct ThreadComms {
    /// The pointer over the primary window: Render → Emacs.
    pub pointer: Arc<PointerCell>,

    /// Commands: Emacs → Render
    pub cmd_tx: CommandSender,
    pub cmd_rx: CommandReceiver,

    /// Input events: Render → Emacs
    pub input_tx: Sender<InputEvent>,
    pub input_rx: Receiver<InputEvent>,

    pub frame_opacity: Arc<Mutex<FrameOpacityState>>,
    pub lost_images: Arc<Mutex<LostImages>>,
}

/// Images the renderer no longer has though nothing retired them: those
/// it evicted, by id, and whether a GPU rebuild lost every one.  The
/// render thread records them; the evaluator's residency takes them.
#[derive(Debug, Default)]
pub(crate) struct LostImages {
    pub evicted: Vec<u32>,
    pub reset: bool,
}

impl ThreadComms {
    /// Create new thread communication channels
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = command_channel(COMMAND_CHANNEL_CAPACITY);
        let (input_tx, input_rx) = bounded(INPUT_CHANNEL_CAPACITY);
        Self {
            pointer: Arc::default(),
            cmd_tx,
            cmd_rx,
            input_tx,
            input_rx,
            frame_opacity: Arc::default(),
            lost_images: Arc::default(),
        }
    }

    /// Split into Emacs-side and Render-side handles
    pub fn split(self) -> (EmacsComms, RenderComms) {
        let emacs = EmacsComms {
            pointer: self.pointer.clone(),
            frame_opacity: Arc::clone(&self.frame_opacity),
            lost_images: Arc::clone(&self.lost_images),
            cmd_tx: self.cmd_tx,
            input_rx: self.input_rx,
        };

        let render = RenderComms {
            pointer: self.pointer,
            input_interrupt: None,
            keep_alive_without_frames: false,
            native_window_waits: None,
            paint_diagnostics: None,
            input_stream: Default::default(),
            frame_opacity: self.frame_opacity,
            lost_images: self.lost_images,
            cmd_rx: self.cmd_rx,
            input_tx: self.input_tx,
        };

        (emacs, render)
    }
}

impl Default for ThreadComms {
    fn default() -> Self {
        Self::new()
    }
}

/// Emacs thread communication handle
pub(crate) struct EmacsComms {
    pub pointer: Arc<PointerCell>,
    pub cmd_tx: CommandSender,
    pub input_rx: Receiver<InputEvent>,
    pub frame_opacity: Arc<Mutex<FrameOpacityState>>,
    pub lost_images: Arc<Mutex<LostImages>>,
}

/// The evaluator's out-of-band quit, as the render thread sees it. GNU
/// keyboard.c never queues the quit character typed while Lisp runs: it
/// sets `quit-flag' at once (handle_interrupt). The embedder decides which
/// key event is that character and counts it for a busy evaluator.
pub trait InputInterrupt: Send + Sync {
    /// Whether the key EVENT was taken as an interrupt instead of being
    /// queued: true only while the evaluator is busy.
    fn interrupt(&self, event: &InputEvent) -> bool;
    /// The evaluator's busy span, while it is busy: a new one each time it
    /// leaves a wait for input.
    fn busy_span(&self) -> Option<u32>;
}

/// Render thread communication handle
pub(crate) struct RenderComms {
    pub pointer: Arc<PointerCell>,
    /// The embedder's quit for a busy evaluator, when it has one.
    pub input_interrupt: Option<Arc<dyn InputInterrupt>>,
    pub(crate) paint_diagnostics: Option<Arc<crate::paint_host::PaintDiagnostics>>,
    /// Daemon root, rather than the primary native frame, owns display lifetime.
    pub keep_alive_without_frames: bool,
    pub native_window_waits: Option<Arc<crate::native_window_wait::NativeWindowWaits>>,
    input_stream: romacs_display_protocol::input_progress::InputStream,
    pub cmd_rx: CommandReceiver,
    pub input_tx: Sender<InputEvent>,
    pub frame_opacity: Arc<Mutex<FrameOpacityState>>,
    pub lost_images: Arc<Mutex<LostImages>>,
}

impl RenderComms {
    pub(crate) fn create_window(
        &self,
        event_loop: &dyn winit::event_loop::ActiveEventLoop,
        attrs: winit::window::WindowAttributes,
        frame: u64,
    ) -> Result<Box<dyn winit::window::Window>, String> {
        let create = || {
            event_loop
                .create_window(attrs)
                .map_err(|error| error.to_string())
        };
        match &self.native_window_waits {
            Some(waits) => waits.run(frame, create),
            None => create(),
        }
    }
    /// Record a pressed key's or a scroll's platform receive time in the
    /// presentation trace, when one is open.
    fn trace_input_received(event: &InputEvent) {
        #[cfg(target_os = "linux")]
        if romacs_display_protocol::present_trace::enabled() {
            let frame = match event {
                InputEvent::Key {
                    pressed: true,
                    emacs_frame_id,
                    ..
                } => *emacs_frame_id,
                InputEvent::PositionedPointer(PositionedPointerInput {
                    position,
                    action: PointerAction::Scroll { .. },
                    ..
                }) => position.target_frame_id,
                _ => return,
            };
            let mut time = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            // SAFETY: valid out-pointer, POSIX monotonic clock. The
            // compositor's clock ID must match before subtraction.
            if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } == 0 {
                romacs_display_protocol::present_trace::input_received(
                    frame,
                    libc::CLOCK_MONOTONIC as u32,
                    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64,
                );
            }
        }
    }

    fn is_lossy_input_event(event: &InputEvent) -> bool {
        matches!(
            event,
            InputEvent::PositionedPointer(PositionedPointerInput {
                action: PointerAction::Move { .. },
                ..
            })
        )
    }

    fn should_log_delivery(event: &InputEvent) -> bool {
        matches!(
            event,
            InputEvent::WindowResize { .. }
                | InputEvent::WindowClose { .. }
                | InputEvent::WindowFocus { .. }
                | InputEvent::MonitorsChanged { .. }
                | InputEvent::DisplayReset
        )
    }

    fn event_name(event: &InputEvent) -> &'static str {
        match event {
            InputEvent::Tracked { event, .. } => Self::event_name(event),
            InputEvent::RawTtyBytes { .. } => "raw-tty-bytes",
            InputEvent::Key { .. } => "key",
            InputEvent::PositionedPointer(PositionedPointerInput { action, .. }) => match action {
                PointerAction::Button { .. } => "positioned-pointer-button",
                PointerAction::Move { .. } => "positioned-pointer-move",
                PointerAction::Scroll { .. } => "positioned-pointer-scroll",
            },
            InputEvent::WindowResize { .. } => "window-resize",
            InputEvent::WindowClose { .. } => "window-close",
            InputEvent::WindowFocus { .. } => "window-focus",
            InputEvent::MonitorsChanged { .. } => "monitors-changed",
            InputEvent::DisplayReset => "display-reset",
            InputEvent::ImageStateChanged { .. } => "image-state-changed",
            InputEvent::SurfaceCreateFailed { .. } => "surface-create-failed",
            InputEvent::Model { .. } => "model",
            InputEvent::FrameShaderFailed { .. } => "frame-shader-failed",
            InputEvent::MenuSelection { .. } => "menu-selection",
            InputEvent::FileDrop { .. } => "file-drop",
            InputEvent::ToolBarClick { .. } => "toolbar-click",
            InputEvent::PresentedPointer { .. } => "presented-pointer",
            InputEvent::PresentationActivated { .. } => "presentation-activated",
            InputEvent::PresentationDiscarded { .. } => "presentation-discarded",
            InputEvent::PresentationRetired { .. } => "presentation-retired",
            InputEvent::MenuBarClick { .. } => "menubar-click",
        }
    }

    /// The evaluator's busy span, while it is busy (`InputInterrupt').
    pub(crate) fn evaluator_busy_span(&self) -> Option<u32> {
        self.input_interrupt.as_ref().and_then(|hook| hook.busy_span())
    }

    /// Queue a display input event for the input bridge.
    ///
    /// After converting the display event, the bridge owns notifying the
    /// evaluator's wait backend.
    pub(crate) fn send_input(&self, event: InputEvent) {
        let _ = self.send_input_with_receipt(event);
    }

    pub(crate) fn send_input_with_receipt(
        &self,
        event: InputEvent,
    ) -> Option<romacs_display_protocol::input_progress::InputReceipt> {
        self.send_input_with_tracking(event, None)
    }

    /// Native key repeats use evaluator consumption, not bridge dequeueing,
    /// to bound their backlog. Physical presses still use lossless delivery.
    pub(super) fn send_key_input_with_receipt(
        &self,
        event: InputEvent,
    ) -> Option<romacs_display_protocol::input_progress::InputReceipt> {
        if self.input_interrupt.as_ref().is_some_and(|hook| hook.interrupt(&event)) {
            return None;
        }
        let delivery = matches!(&event, InputEvent::Key { pressed: true, .. })
            .then(romacs_display_protocol::input_progress::InputDelivery::for_read);
        let read = delivery.as_ref().map(|delivery| delivery.receipt());
        self.send_input_with_tracking(event, delivery);
        read
    }

    fn send_input_with_tracking(
        &self,
        event: InputEvent,
        read: Option<romacs_display_protocol::input_progress::InputDelivery>,
    ) -> Option<romacs_display_protocol::input_progress::InputReceipt> {
        crate::paint_host::trace_key_enqueue(&event, read.as_ref());
        let receipt = if matches!(
            &event,
            InputEvent::Key {
                key: crate::host_types::FrontendKey::Keysym(0xff55 | 0xff56),
                pressed: true,
                ..
            } | InputEvent::PositionedPointer(PositionedPointerInput {
                action: PointerAction::Scroll { .. },
                ..
            })
        ) {
            self.input_stream.issue()
        } else {
            None
        };
        let observer = receipt.as_ref().map(|delivery| delivery.receipt());
        Self::trace_input_received(&event);
        let event = if let Some(receipt) = receipt {
            InputEvent::Tracked {
                receipt,
                event: Box::new(event),
            }
        } else {
            event
        };
        let event = if let Some(receipt) = read {
            InputEvent::Tracked {
                receipt,
                event: Box::new(event),
            }
        } else {
            event
        };
        let log_delivery = Self::should_log_delivery(&event);
        let event_name = Self::event_name(&event);
        if Self::is_lossy_input_event(&event) {
            match self.input_tx.try_send(event) {
                Ok(()) => {
                    if log_delivery {
                        tracing::debug!("send_input: queued {}", event_name);
                    }
                }
                Err(TrySendError::Full(event)) => {
                    tracing::debug!(
                        "send_input: dropped lossy {} because the input queue is full",
                        Self::event_name(&event)
                    );
                }
                Err(TrySendError::Disconnected(event)) => {
                    tracing::warn!(
                        "send_input: dropped {} because the input queue is disconnected",
                        Self::event_name(&event)
                    );
                }
            }
            return observer;
        }

        match self.input_tx.send(event) {
            Ok(()) => {
                if log_delivery {
                    tracing::debug!("send_input: queued {}", event_name);
                }
            }
            Err(err) => {
                tracing::warn!(
                    "send_input: dropped {} because the input queue is disconnected",
                    Self::event_name(&err.0)
                );
            }
        }
        observer
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "tests/positioned_input_test.rs"]
mod positioned_input_test;
