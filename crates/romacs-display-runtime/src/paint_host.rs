//! Core-free Seam P host using the existing native event loop and compositor.
//! `FrameGlyphBuffer` owns both font tables; no evaluator or font handle crosses
//! the native-thread boundary. Construct and run on the process main thread.

use crate::render_thread::{self, InitialWindowReply, InitialWindowSize};
use crate::thread_comm::{
    ClipboardCommand, ClipboardSelection, EmacsComms, FrameRef, LifecycleCommand, RenderCommand,
    RenderComms, ThreadComms, WindowCommand,
};
use romacs_display_protocol::FrameGlyphBuffer;

pub const PRIMARY_FRAME: u64 = 1;

/// A sample from the live window atlas after queue presentation. Cumulative
/// within that atlas lifetime, not a per-frame delta or compositor confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintDiagnostic {
    pub presentation: u64,
    pub unresolved_face_text_total: u64,
}

#[derive(Default)]
pub(crate) struct PaintDiagnostics {
    latest: std::sync::Mutex<Option<PaintDiagnostic>>,
    changed: std::sync::Condvar,
    /// The adapter the window draws with: 0 not chosen yet, 1 a GPU, 2 a
    /// software rasterizer (wgpu's `DeviceType::Cpu`).
    adapter: std::sync::atomic::AtomicU8,
    /// Called when a presentation newer than the latest sample is published: the
    /// evaluator's wait wakes for it, where it would otherwise have to
    /// poll for the sample.
    wake: std::sync::OnceLock<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

impl PaintDiagnostics {
    pub(crate) fn publish_adapter(&self, software: bool) {
        self.adapter.store(if software { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
    }

    fn software_adapter(&self) -> Option<bool> {
        match self.adapter.load(std::sync::atomic::Ordering::Relaxed) {
            0 => None,
            kind => Some(kind == 2),
        }
    }

    pub(crate) fn publish(&self, diagnostic: PaintDiagnostic) {
        let newer = {
            let mut latest = self.latest.lock().unwrap();
            let newer = latest.is_none_or(|sample| sample.presentation < diagnostic.presentation);
            *latest = Some(diagnostic);
            newer
        };
        self.changed.notify_all();
        // Once per presentation: the frames the render thread draws again
        // on its own (effects, video) retire nothing and wake no one.
        if newer && let Some(wake) = self.wake.get() { wake(); }
    }

    /// The latest sample once PRESENTATION or a later one has rendered: the
    /// renderer draws only its newest frame, so a later presentation drawn
    /// supersedes an earlier one it never drew.
    fn wait(
        &self,
        presentation: u64,
        timeout: std::time::Duration,
    ) -> Result<PaintDiagnostic, String> {
        let (latest, _) = self
            .changed
            .wait_timeout_while(
                self.latest
                    .lock()
                    .map_err(|_| "paint diagnostic lock poisoned")?,
                timeout,
                |latest| latest.is_none_or(|sample| sample.presentation < presentation),
            )
            .map_err(|_| "paint diagnostic lock poisoned")?;
        latest
            .filter(|sample| sample.presentation >= presentation)
            .ok_or_else(|| format!("no rendered atlas diagnostic for presentation {presentation}"))
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use std::time::Duration;

    /// The wake a waiter registered runs once for each newer presentation,
    /// after its sample is stored, so a waiter woken by it finds the render
    /// without polling; a frame the render thread draws again wakes no one.
    #[test]
    fn a_newer_presentation_rendered_wakes_its_waiter_once_with_the_sample_readable() {
        let state = std::sync::Arc::new(PaintDiagnostics::default());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (reader, log) = (state.clone(), seen.clone());
        let wake: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(move || {
            log.lock().unwrap().push(reader.wait(0, Duration::ZERO).map(|sample| sample.presentation));
        });
        assert!(state.wake.set(wake).is_ok());
        for presentation in [1, 1, 2, 2, 2, 3] {
            state.publish(PaintDiagnostic { presentation, unresolved_face_text_total: 0 });
        }
        assert_eq!(*seen.lock().unwrap(), vec![Ok(1), Ok(2), Ok(3)], "no wake for a presentation drawn again");
    }

    #[test]
    fn diagnostic_preserves_nonzero_renderer_sample_and_identity() {
        let state = PaintDiagnostics::default();
        assert!(state.wait(1, Duration::ZERO).is_err());
        state.publish(PaintDiagnostic {
            presentation: 1,
            unresolved_face_text_total: 19,
        });
        assert_eq!(
            state
                .wait(1, Duration::ZERO)
                .unwrap()
                .unresolved_face_text_total,
            19
        );
        assert!(
            state.wait(2, Duration::ZERO).is_err(),
            "stale samples are not evidence"
        );
        state.publish(PaintDiagnostic {
            presentation: 2,
            unresolved_face_text_total: 27,
        });
        assert_eq!(
            state.wait(2, Duration::ZERO).unwrap(),
            PaintDiagnostic {
                presentation: 2,
                unresolved_face_text_total: 27,
            }
        );
        assert_eq!(
            state.wait(1, Duration::ZERO).unwrap().presentation,
            2,
            "a later presentation drawn settles an earlier one never drawn"
        );
    }

    #[test]
    fn diagnostic_wait_observes_native_thread_publication() {
        let state = std::sync::Arc::new(PaintDiagnostics::default());
        let producer = state.clone();
        let worker = std::thread::spawn(move || {
            producer.publish(PaintDiagnostic {
                presentation: 7,
                unresolved_face_text_total: 3,
            })
        });
        assert_eq!(
            state
                .wait(7, Duration::from_secs(5))
                .unwrap()
                .unresolved_face_text_total,
            3
        );
        worker.join().unwrap();
    }
}

pub struct PaintHost {
    event_loop: render_thread::RenderEventLoop,
    render: RenderComms,
    initial: render_thread::InitialWindowReceiver,
    _lifetime: render_thread::InitialWindowLifetime,
    title: String,
}

pub struct PaintSender {
    comms: EmacsComms,
    proxy: render_thread::RenderEventLoopProxy,
    diagnostics: std::sync::Arc<PaintDiagnostics>,
}

impl PaintHost {
    /// Connect to this exact compositor, never the user's ambient X11 display.
    #[cfg(target_os = "linux")]
    pub fn wayland(
        socket: &std::path::Path,
        width: u32,
        height: u32,
        title: String,
    ) -> Result<(Self, PaintSender), String> {
        Self::wayland_with(socket, width, height, title, false)
    }

    /// A server's host: no window until [`PaintSender::show_primary`] asks
    /// for one, and the session outlives each window it shows. The GPU
    /// starts with the first window and stays for the next.
    #[cfg(target_os = "linux")]
    pub fn wayland_server(
        socket: &std::path::Path,
        width: u32,
        height: u32,
        title: String,
    ) -> Result<(Self, PaintSender), String> {
        Self::wayland_with(socket, width, height, title, true)
    }

    #[cfg(target_os = "linux")]
    fn wayland_with(
        socket: &std::path::Path,
        width: u32,
        height: u32,
        title: String,
        server: bool,
    ) -> Result<(Self, PaintSender), String> {
        if width == 0 || height == 0 {
            return Err("paint host size must be nonzero".into());
        }
        let event_loop = render_thread::build_render_event_loop_wayland(socket)?;
        let proxy = event_loop.create_proxy();
        let (reply, initial) = InitialWindowReply::channel(proxy.clone());
        let lifetime = reply
            .ready(InitialWindowSize { width, height })
            .ok_or("initial window receiver closed")?;
        let (comms, mut render) = ThreadComms::new().split();
        let diagnostics = std::sync::Arc::new(PaintDiagnostics::default());
        render.paint_diagnostics = Some(diagnostics.clone());
        render.keep_alive_without_frames = server;
        if !server {
            comms
                .cmd_tx
                .try_send(RenderCommand::Window(WindowCommand::AdoptPrimaryFrame {
                    frame: FrameRef::Frame(PRIMARY_FRAME),
                }))
                .map_err(|error| error.to_string())?;
        }
        Ok((
            Self {
                event_loop,
                render,
                initial,
                _lifetime: lifetime,
                title,
            },
            PaintSender {
                comms,
                proxy,
                diagnostics,
            },
        ))
    }

    /// Let the render thread take the quit character out of band while the
    /// evaluator is busy, and show that it is.
    pub fn set_input_interrupt(&mut self, hook: std::sync::Arc<dyn crate::thread_comm::InputInterrupt>) {
        self.render.input_interrupt = Some(hook);
    }

    pub fn run(self) -> Result<render_thread::RenderLoopExit, render_thread::RenderLoopError> {
        render_thread::run_render_loop_current_thread(
            self.event_loop,
            self.render,
            self.initial,
            self.title,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        )
    }
}

// Enabled only by the standalone paint-host latency probe.
// Pair by the actual read receipt, never by FIFO position: native admission
// may discard events before delivery, so a positional side queue can invent lag.
static TRACE_KEYS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static KEY_TIMES: std::sync::Mutex<Vec<(romacs_display_protocol::input_progress::InputReceipt, std::time::Instant)>> =
    std::sync::Mutex::new(Vec::new());

pub fn enable_key_latency_trace() {
    let _ = TRACE_START.set(std::time::Instant::now());
    TRACE_KEYS.store(true, std::sync::atomic::Ordering::Relaxed);
}
static TRACE_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
/// Under the latency trace, one line as a GPU startup PHASE ends:
/// `romacs-gpu: {"phase": PHASE, "ms": since the trace was enabled}`.
pub(crate) fn trace_gpu(phase: &str) {
    if let Some(start) = TRACE_START.get().filter(|_| TRACE_KEYS.load(std::sync::atomic::Ordering::Relaxed)) {
        eprintln!("romacs-gpu: {{\"phase\":\"{phase}\",\"ms\":{:.1}}}", start.elapsed().as_secs_f64() * 1000.0);
    }
}
pub(crate) fn trace_key_enqueue(event: &crate::thread_comm::InputEvent, delivery: Option<&romacs_display_protocol::input_progress::InputDelivery>) {
    if TRACE_KEYS.load(std::sync::atomic::Ordering::Relaxed)
        && matches!(event, crate::thread_comm::InputEvent::Key { pressed: true, .. })
        && let Some(delivery) = delivery {
        let mut times = KEY_TIMES.lock().unwrap();
        times.retain(|(receipt, _)| !receipt.cancelled());
        times.push((delivery.receipt(), std::time::Instant::now()));
    }
}
pub fn take_key_enqueue_time(event: &crate::thread_comm::InputEvent) -> Option<std::time::Instant> {
    match event {
        crate::thread_comm::InputEvent::Tracked { receipt, .. } => {
            let actual = receipt.receipt();
            let mut times = KEY_TIMES.lock().unwrap();
            let index = times.iter().position(|(queued, _)| queued.same_input(&actual))?;
            Some(times.remove(index).1)
        }
        _ => None,
    }
}

#[cfg(test)]
#[test]
fn latency_receipt_does_not_pair_a_discarded_native_event() {
    use crate::thread_comm::InputEvent;
    use romacs_display_protocol::input_progress::InputDelivery;
    let discarded = InputDelivery::for_read();
    let delivered = InputDelivery::for_read();
    let first = std::time::Instant::now();
    let second = first + std::time::Duration::from_millis(1);
    KEY_TIMES.lock().unwrap().extend([(discarded.receipt(), first), (delivered.receipt(), second)]);
    drop(discarded);
    let event = InputEvent::Tracked { receipt: delivered, event: Box::new(InputEvent::Key {
        key: crate::host_types::FrontendKey::Character('k'), modifiers: 0,
        pressed: true, emacs_frame_id: 1,
    }) };
    assert_eq!(take_key_enqueue_time(&event), Some(second));
    assert_eq!(take_key_enqueue_time(&event), None, "one event has one timestamp");
    KEY_TIMES.lock().unwrap().retain(|(receipt, _)| !receipt.cancelled());
}

impl PaintSender {
    /// Call WAKE, from the render thread, when a presentation newer than
    /// any before it has rendered: a paint's completion then ends a wait
    /// for input itself. One waiter per sender: a second registration
    /// would leave its wait unwoken, and is refused.
    ///
    /// # Panics
    ///
    /// When a waiter is already registered on this sender.
    pub fn on_render(&self, wake: std::sync::Arc<dyn Fn() + Send + Sync>) {
        romacs_display_protocol::call_count::count("sender.on_render");
        let registered = self.diagnostics.wake.set(wake).is_ok();
        assert!(registered, "one render waiter per paint sender");
    }

    /// Whether the window draws on a software rasterizer: None until the
    /// render thread has chosen its adapter.
    pub fn software_adapter(&self) -> Option<bool> {
        romacs_display_protocol::call_count::count("sender.software_adapter");
        self.diagnostics.software_adapter()
    }

    /// Read CLIPBOARD text through the existing display-owned worker. Only the
    /// evaluator waits, for at most one second; native I/O never runs here or
    /// on the render thread. Absence of text is distinct from transport errors.
    pub fn clipboard_text(&self) -> Result<Option<String>, String> {
        romacs_display_protocol::call_count::count("sender.clipboard_text");
        self.clipboard_request(|expires_at, reply| ClipboardCommand::GetText {
            selection: ClipboardSelection::Clipboard,
            expires_at,
            reply,
        })
    }

    /// Publish owned CLIPBOARD text through the same worker. A timeout is not
    /// a background-command result or proof of cancellation: native work that
    /// already started may finish later; queued expired requests are rejected.
    pub fn set_clipboard_text(&self, text: &str) -> Result<(), String> {
        romacs_display_protocol::call_count::count("sender.set_clipboard_text");
        self.clipboard_request(|expires_at, reply| ClipboardCommand::SetText {
            selection: ClipboardSelection::Clipboard,
            text: Some(text.to_owned()),
            expires_at,
            reply,
        })
    }

    fn clipboard_request<T>(
        &self,
        command: impl FnOnce(
            std::time::Instant,
            crossbeam_channel::Sender<Result<T, String>>,
        ) -> ClipboardCommand,
    ) -> Result<T, String> {
        let expires_at = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let (reply, response) = crossbeam_channel::bounded(1);
        self.comms
            .cmd_tx
            .try_send(RenderCommand::Clipboard(command(expires_at, reply)))
            .map_err(|error| format!("clipboard request could not be queued: {error}"))?;
        self.proxy.wake_up();
        response.recv_deadline(expires_at).map_err(|error| match error {
            crossbeam_channel::RecvTimeoutError::Timeout => "clipboard request timed out".to_owned(),
            crossbeam_channel::RecvTimeoutError::Disconnected => "clipboard reply channel closed".to_owned(),
        })?
    }

    /// Wait for a successfully rendered primary presentation, not merely ingress
    /// activation: PRESENTATION or a later one, since this bounded snapshot
    /// retains only the latest. No sample exists before the first successful render.
    pub fn wait_for_render(
        &self,
        presentation: u64,
        timeout: std::time::Duration,
    ) -> Result<PaintDiagnostic, String> {
        romacs_display_protocol::call_count::count(if timeout.is_zero() { "sender.render_poll" } else { "sender.wait_for_render" });
        self.diagnostics.wait(presentation, timeout)
    }

    /// Queue one complete frame, including `fonts` and `char_fonts`. Returns
    /// backpressure rather than silently dropping a repaint. The producer must
    /// use PRIMARY_FRAME and a fresh nonzero presentation ID for every revision.
    pub fn submit(&self, frame: FrameGlyphBuffer) -> Result<(), String> {
        self.submit_scene(frame, Vec::new(), Vec::new())
    }

    /// Queue a frame, the child frames shown over it, lowest first, and the
    /// text animations drawn over it, as one paint: each child names
    /// PRIMARY_FRAME as its parent and carries the frame's presentation. A
    /// child or an animation the last scene showed and this one does not is
    /// removed.
    pub fn submit_scene(
        &self,
        frame: FrameGlyphBuffer,
        children: Vec<FrameGlyphBuffer>,
        text_animations: Vec<(u64, romacs_display_protocol::text_animation::TextAnimation)>,
    ) -> Result<(), String> {
        romacs_display_protocol::call_count::count("sender.submit_scene");
        validate_frame(&frame)?;
        validate_children(&frame, &children)?;
        let scene = crate::thread_comm::PaintScene { root: frame, children, text_animations };
        let result = self
            .comms
            .cmd_tx
            .try_send(RenderCommand::Paint(Box::new(scene)))
            .map_err(|error| error.to_string());
        self.proxy.wake_up();
        result
    }

    /// Make DECODED, pixels already decoded, resident in the renderer's
    /// image cache as IMAGE (load ATTEMPT, from 1): the renderer only
    /// uploads them, before any later-sent frame draws IMAGE.  False, and
    /// nothing sent, while the command channel is full.
    pub fn load_image(&self, image: u32, attempt: u64,
                      decoded: std::sync::Arc<romacs_renderer_wgpu::image_cache::SemanticImageDecoded>) -> Result<bool, String> {
        use romacs_display_protocol::{ImageId, ImageLoadAttempt, ImageLoadToken};
        romacs_display_protocol::call_count::count("sender.load_image");
        let attempt = ImageLoadAttempt::new(attempt).ok_or("image load attempts start at 1")?;
        let load = ImageLoadToken::new(ImageId::new(image), attempt);
        self.send_asset(crate::thread_comm::AssetCommand::ImageLoadDecoded { load, decoded })
    }

    /// Release IMAGE's texture once no presented frame draws it.  False,
    /// and nothing sent, while the command channel is full.
    pub fn retire_image(&self, image: u32) -> Result<bool, String> {
        romacs_display_protocol::call_count::count("sender.retire_image");
        self.send_asset(crate::thread_comm::AssetCommand::ImageRetire { image: romacs_display_protocol::ImageId::new(image) })
    }

    /// The images the renderer lost since the last call: those it evicted,
    /// and whether a GPU rebuild lost them all.
    pub fn take_lost_images(&self) -> (Vec<u32>, bool) {
        romacs_display_protocol::call_count::count("sender.take_lost_images");
        let mut lost = self.comms.lost_images.lock().unwrap_or_else(|e| e.into_inner());
        (std::mem::take(&mut lost.evicted), std::mem::take(&mut lost.reset))
    }

    /// Whether a GPU rebuild lost every image since the last take.
    pub fn images_reset_pending(&self) -> bool {
        romacs_display_protocol::call_count::count("sender.images_reset_pending");
        self.comms.lost_images.lock().unwrap_or_else(|e| e.into_inner()).reset
    }

    /// Debug: rebuild the GPU state as after a device loss
    /// (`romacs--debug-lose-device`). Whether the request went.
    pub fn debug_lose_device(&self) -> Result<bool, String> {
        romacs_display_protocol::call_count::count("sender.debug_lose_device");
        self.send_asset(crate::thread_comm::AssetCommand::DebugSimulateDeviceLoss)
    }

    /// Whether a GPU renderer is live now, so a native model can be drawn
    /// (`romacs-model-available-p`).
    #[must_use]
    pub fn model_renderer_live(&self) -> bool {
        romacs_display_protocol::call_count::count("sender.model_renderer_live");
        self.comms.model_renderer.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Create native model surface ID from MESHES at VIEW on BACKGROUND
    /// (sRGB RGBA), LOGICAL pixels in size. It loads on the renderer's own
    /// thread; its outcome arrives as an `InputEvent::Model`. False, and
    /// nothing sent, while the command channel is full.
    ///
    /// # Errors
    /// The render thread is gone.
    pub fn create_model(
        &self,
        id: u32,
        meshes: Vec<romacs_display_protocol::model::ModelMeshSpec>,
        view: romacs_display_protocol::model::ModelView,
        background: [f32; 4],
        logical: (u32, u32),
    ) -> Result<bool, String> {
        let (width, height) = logical;
        romacs_display_protocol::call_count::count("sender.create_model");
        self.send_model_request(crate::thread_comm::AssetCommand::ModelCreate { id, meshes, view, background, width, height })
    }

    /// Lisp request REQ for native model ID.  False while the channel is full.
    ///
    /// # Errors
    /// The render thread is gone.
    pub fn update_model(
        &self,
        id: u32,
        req: u64,
        change: romacs_display_protocol::model::ModelChange,
    ) -> Result<bool, String> {
        romacs_display_protocol::call_count::count("sender.update_model");
        self.send_model_request(crate::thread_comm::AssetCommand::ModelUpdate { id, req, change })
    }

    /// Free surface ID once no presented frame draws it: native models
    /// are the only surfaces the evaluator makes.  False while the channel
    /// is full.
    ///
    /// # Errors
    /// The render thread is gone.
    pub fn free_surface(&self, id: u32) -> Result<bool, String> {
        romacs_display_protocol::call_count::count("sender.free_surface");
        self.send_asset(crate::thread_comm::AssetCommand::ModelFree { id })
    }

    /// Send Lisp's model request COMMAND within `MODEL_REQUEST_ROOM`: false
    /// once that many commands wait, so requests never fill the channel a
    /// frame needs.
    fn send_model_request(&self, command: crate::thread_comm::AssetCommand) -> Result<bool, String> {
        self.send_asset_within(command, crate::thread_comm::MODEL_REQUEST_ROOM)
    }

    /// Send an asset COMMAND: whether it went, false while the channel is
    /// full (its sender sends it again later).
    fn send_asset(&self, command: crate::thread_comm::AssetCommand) -> Result<bool, String> {
        self.send_asset_within(command, usize::MAX)
    }

    /// Send asset COMMAND unless LIMIT commands already wait.
    fn send_asset_within(&self, command: crate::thread_comm::AssetCommand, limit: usize) -> Result<bool, String> {
        let result = match self.comms.cmd_tx.try_send_within(RenderCommand::Asset(command), limit) {
            Ok(()) => Ok(true),
            Err(crossbeam_channel::TrySendError::Full(_)) => Ok(false),
            Err(error) => Err(error.to_string()),
        };
        self.proxy.wake_up();
        result
    }

    /// Apply FRAME's `alpha' setter, the primary frame's or a child's: ALPHA
    /// while active and inactive (negative leaves its opacity alone),
    /// raised to LIMIT.
    pub fn set_frame_alpha(&self, frame: u64, alpha: [f32; 2], limit: f32) -> Result<(), String> {
        romacs_display_protocol::call_count::count("sender.set_frame_alpha");
        self.comms.frame_opacity.lock().unwrap_or_else(|e| e.into_inner()).accept(frame, alpha, limit);
        let result = self
            .comms
            .cmd_tx
            .try_send(RenderCommand::Window(WindowCommand::RefreshFrameOpacity))
            .map_err(|error| error.to_string());
        self.proxy.wake_up();
        result
    }

    /// A server's window for the primary frame, WIDTH by HEIGHT: made
    /// when none shows, asked for the focus when one does. The receiver
    /// answers once the window and its surface exist (or failed), within
    /// 30 s; paint the frame after that.
    pub fn show_primary(
        &self,
        width: u32,
        height: u32,
        title: String,
    ) -> Result<crossbeam_channel::Receiver<Result<(), String>>, String> {
        romacs_display_protocol::call_count::count("sender.show_primary");
        let (reply, ready) = crossbeam_channel::bounded(1);
        let none = crate::host_types::GuiFrameGeometryHints {
            base_width: 0, base_height: 0, min_width: 0, min_height: 0, width_inc: 0, height_inc: 0,
        };
        let result = self
            .comms
            .cmd_tx
            .try_send(RenderCommand::Window(WindowCommand::RealizeFrame {
                frame: FrameRef::Frame(PRIMARY_FRAME),
                width,
                height,
                title,
                geometry_hints: none,
                visual: None,
                adopt_primary: true,
                reply: Some(reply),
                live: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
            }))
            .map_err(|error| error.to_string());
        self.proxy.wake_up();
        result.map(|()| ready)
    }

    /// Where the pointer is over the primary window, if it is.
    pub fn pointer(&self) -> Option<(i32, i32)> {
        romacs_display_protocol::call_count::count("sender.pointer");
        self.comms.pointer.get()
    }

    /// What the render thread saw of native key repeat since the last take.
    #[must_use]
    pub fn take_key_repeats(&self) -> romacs_display_protocol::key_repeat_log::KeyRepeatCounts {
        romacs_display_protocol::call_count::count("sender.take_key_repeats");
        self.comms.key_repeats.take()
    }

    /// Title the primary window (xdisp.c gui_consider_frame_title).
    pub fn set_title(&self, title: String) -> Result<(), String> {
        romacs_display_protocol::call_count::count("sender.set_title");
        let result = self
            .comms
            .cmd_tx
            .try_send(RenderCommand::Window(WindowCommand::SetWindowTitle { title }))
            .map_err(|error| error.to_string());
        self.proxy.wake_up();
        result
    }

    /// Close a server's primary window; the render loop and the GPU stay.
    pub fn hide_primary(&self) -> Result<(), String> {
        romacs_display_protocol::call_count::count("sender.hide_primary");
        let result = self
            .comms
            .cmd_tx
            .try_send(RenderCommand::Window(WindowCommand::DestroyWindow { frame: FrameRef::Primary }))
            .map_err(|error| error.to_string());
        self.proxy.wake_up();
        result
    }

    /// Show CONFIG, the visual configuration Lisp asked for, on every
    /// window from the next paint. Sent rather than tried: a dropped
    /// configuration would leave an option silently without effect.
    pub fn set_visual_config(&self, config: romacs_display_protocol::VisualConfig) -> Result<(), String> {
        romacs_display_protocol::call_count::count("sender.set_visual_config");
        let result = self
            .comms
            .cmd_tx
            .send(RenderCommand::Config(crate::thread_comm::ConfigCommand::SetVisualConfig(config)))
            .map_err(|error| error.to_string());
        self.proxy.wake_up();
        result
    }

    /// Ask the compositor to bring the primary window to the user.
    pub fn raise_primary(&self) -> Result<(), String> {
        romacs_display_protocol::call_count::count("sender.raise_primary");
        let result = self
            .comms
            .cmd_tx
            .try_send(RenderCommand::Window(WindowCommand::RequestAttention { urgent: false }))
            .map_err(|error| error.to_string());
        self.proxy.wake_up();
        result
    }

    pub fn input(&self) -> &crossbeam_channel::Receiver<crate::thread_comm::InputEvent> {
        romacs_display_protocol::call_count::count("sender.input");
        &self.comms.input_rx
    }

    pub fn shutdown(&self) -> Result<(), String> {
        romacs_display_protocol::call_count::count("sender.shutdown");
        let result = self
            .comms
            .cmd_tx
            .try_send(RenderCommand::Lifecycle(LifecycleCommand::Shutdown))
            .map_err(|error| error.to_string());
        self.proxy.wake_up();
        result
    }
}

impl Drop for PaintSender {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// Children of ROOT, distinct and none the primary frame.
pub(crate) fn validate_children(root: &FrameGlyphBuffer, children: &[FrameGlyphBuffer]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for child in children {
        let placement = child.frame_placement;
        let id = placement.frame().get();
        if id == PRIMARY_FRAME
            || !ids.insert(id)
            || placement.parent().map(|p| p.get()) != Some(PRIMARY_FRAME)
            || child.presentation_id != root.presentation_id
            || placement.presentation() != root.presentation_id
            || !(child.width.is_finite() && child.height.is_finite() && child.width > 0.0 && child.height > 0.0)
        {
            return Err("paint host requires distinct child frames of the primary presentation".into());
        }
    }
    Ok(())
}

pub(crate) fn validate_frame(frame: &FrameGlyphBuffer) -> Result<(), String> {
    let rect = frame.frame_placement.outer_in_parent();
    if !frame.width.is_finite()
        || !frame.height.is_finite()
        || frame.width <= 0.0
        || frame.height <= 0.0
        || rect.x() != 0.0
        || rect.y() != 0.0
        || rect.width() != frame.width
        || rect.height() != frame.height
    {
        return Err("paint host requires coherent positive root geometry".into());
    }
    if frame.frame_placement.frame().get() != PRIMARY_FRAME
        || frame.frame_placement.parent().is_some()
        || frame.presentation_id.get() == 0
        || frame.frame_placement.presentation() != frame.presentation_id
    {
        return Err("paint host requires coherent nonzero primary presentation".into());
    }
    Ok(())
}
