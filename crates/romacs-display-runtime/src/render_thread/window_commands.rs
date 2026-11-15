//! Window and chrome render commands.

use super::RenderApp;
use crate::thread_comm::{InputEvent, WindowCommand};
use winit::window::UserAttentionType;

impl RenderApp {
    /// Answer the window-readiness requests decided since the last pass.
    pub(super) fn send_answers(&mut self) {
        for (request, made) in self.frame_windows.take_answers() {
            self.comms.tell(crate::thread_comm::Fact::Reply { request, reply: crate::thread_comm::Reply::Done(made) });
        }
    }

    pub(super) fn refresh_frame_opacity(&mut self) {
        for window in self.frame_windows.windows.values_mut() {
            let composites = window.lifecycle.native().is_some_and(|native| native.composites_alpha());
            if window.render.apply_surface_frame_opacity(&self.frame_opacity, composites) {
                window.request_redraw();
            }
        }
    }

    /// Remove child FRAME_ID and its descendants from every top-level
    /// window, retiring their presentations.
    pub(super) fn remove_child_frame(&mut self, frame_id: u64) {
        let mut retirements = Vec::new();
        self.frame_windows.for_each_top_level_window_mut(|window| {
            for presentation in window.render.child_subtree_presentations(frame_id) {
                if let Some(presentation) =
                    window.render.route_presentation_retirement(presentation)
                {
                    retirements.push(presentation);
                }
            }
        });
        self.frame_windows
            .remove_child_frame_from_top_level_windows(frame_id);
        for presentation in retirements {
            self.comms
                .send_input(InputEvent::PresentationRetired { presentation });
        }
    }

    pub(super) fn handle_window(&mut self, cmd: WindowCommand) {
        // PendingGpu already owns the exact primary native window, while the
        // frame lifecycle still retains CPU state. Replay only native-state
        // commands targeting that identity, never stale Lisp size requests.
        let replay_primary = match &cmd {
            WindowCommand::SetWindowTitle { frame, .. } => {
                self.frame_windows.is_primary_frame_id(frame.raw_id())
            }
            WindowCommand::SetFrameGeometryHints { frame, .. } => {
                self.frame_windows.is_primary_frame_id(frame.raw_id())
            }
            _ => false,
        };
        match cmd {
            WindowCommand::SetFrameAlpha { frame, alpha, limit } => {
                self.frame_opacity.accept(frame, alpha, limit);
                self.refresh_frame_opacity();
            }
            WindowCommand::SetWindowTitle { frame, title } => {
                if let Some(window_state) = self.frame_windows.get_mut(frame.raw_id()) {
                    window_state.chrome_mut().title = title.clone();
                    window_state.set_title(title);
                    if !window_state.chrome().decorations_enabled {
                        window_state.render.mark_dirty();
                    }
                }
            }
            WindowCommand::SetFrameGeometryHints {
                frame,
                geometry_hints,
            } => {
                let emacs_frame_id = frame.raw_id();
                tracing::debug!(
                    "WindowCommand::SetFrameGeometryHints frame_id=0x{:x} base={}x{} inc={}x{}",
                    emacs_frame_id,
                    geometry_hints.base_width,
                    geometry_hints.base_height,
                    geometry_hints.width_inc,
                    geometry_hints.height_inc
                );
                if let Some(window_state) = self.frame_windows.get_mut(emacs_frame_id) {
                    window_state.apply_geometry_hints(geometry_hints);
                } else if let Some(pending) = self.frame_windows.pending_creates.iter_mut()
                    .find(|request| request.emacs_frame_id == emacs_frame_id)
                {
                    pending.geometry_hints = geometry_hints;
                } else {
                    tracing::warn!(
                        "SetFrameGeometryHints requested for unknown frame_id=0x{:x}",
                        emacs_frame_id
                    );
                }
            }
            WindowCommand::RequestAttention { frame, urgent } => {
                if let Some(window) = self
                    .frame_windows
                    .get(frame.raw_id())
                    .and_then(|ws| ws.window())
                {
                    let attention = if urgent {
                        Some(UserAttentionType::Critical)
                    } else {
                        Some(UserAttentionType::Informational)
                    };
                    window.request_user_attention(attention);
                }
            }
            WindowCommand::RealizeFrame {
                frame,
                width,
                height,
                title,
                geometry_hints,
                visual,
                adopt_primary,
                request,
            } => {
                let id = frame.raw_id();
                if let Some(visual) = visual {
                    self.handle_config(crate::thread_comm::ConfigCommand::SetVisualConfig(visual));
                }
                if adopt_primary
                    && self.comms.keep_alive_without_frames
                    && self.gpu.is_some()
                    && self.frame_windows.primary_window().is_none()
                {
                    // A server's primary window closed and is asked for
                    // again: the window made for the frame becomes primary.
                    self.frame_windows.primary_emacs_frame_id = Some(id);
                }
                if adopt_primary && self.frame_windows.primary_window().is_some() {
                    self.handle_window(WindowCommand::SetWindowTitle {
                        frame: crate::thread_comm::FrameRef::Primary,
                        title,
                    });
                    self.handle_window(WindowCommand::SetFrameGeometryHints {
                        frame: crate::thread_comm::FrameRef::Primary,
                        geometry_hints,
                    });
                    self.handle_window(WindowCommand::AdoptPrimaryFrame { frame });
                } else {
                    self.handle_window(WindowCommand::CreateWindow {
                        frame,
                        width,
                        height,
                        title,
                        geometry_hints,
                    });
                }
                if let Some(request) = request {
                    self.frame_windows.await_ready(id, request);
                }
            }
            WindowCommand::CreateWindow {
                frame,
                width,
                height,
                title,
                geometry_hints,
            } => {
                let emacs_frame_id = frame.raw_id();
                tracing::info!(
                    "CreateWindow request: frame_id=0x{:x} {}x{} \"{}\"",
                    emacs_frame_id,
                    width,
                    height,
                    title
                );
                if self.gpu.is_none()
                    && self.comms.keep_alive_without_frames
                    && self.frame_windows.primary_window().is_none()
                {
                    self.frame_windows.prepare_primary(
                        emacs_frame_id,
                        width,
                        height,
                        title,
                        Some(geometry_hints),
                    );
                    self.pending_content_size = super::startup::InitialWindowSize { width, height };
                    return;
                }
                self.frame_windows.request_create(
                    emacs_frame_id,
                    width,
                    height,
                    title,
                    geometry_hints,
                );
            }
            WindowCommand::DestroyWindow { frame } => {
                let emacs_frame_id = frame.raw_id();
                tracing::info!("DestroyWindow request: frame_id=0x{:x}", emacs_frame_id);
                // A retired window's PointerLeft is discarded.
                let pointer_frame = if emacs_frame_id == 0 { self.frame_windows.primary_event_frame_id() } else { emacs_frame_id };
                self.comms.tell(crate::thread_comm::Fact::PointerOver { frame: pointer_frame, at: None });
                // Cancel CPU preparation immediately, not after GPU readiness.
                self.frame_windows.pending_creates.retain(|request| {
                    request.emacs_frame_id != emacs_frame_id
                });
                self.frame_windows.reject_ready(emacs_frame_id, "Native frame creation cancelled");
                let mut retirements = Vec::new();
                if let Some(window) = self.frame_windows.get_mut(emacs_frame_id) {
                    // Destruction cancels capture first, flushing any older
                    // pinned generation before retiring buffers still shown.
                    retirements.extend(window.render.cancel_pointer_interaction().1);
                    let mut presentations: Vec<_> = window
                        .render
                        .displayed_presentations()
                        .into_iter()
                        .collect();
                    presentations.sort_unstable();
                    for presentation in presentations {
                        if let Some(presentation) =
                            window.render.route_presentation_retirement(presentation)
                        {
                            retirements.push(presentation);
                        }
                    }
                }
                for presentation in retirements {
                    self.comms
                        .send_input(InputEvent::PresentationRetired { presentation });
                }
                if self.frame_windows.is_primary_frame_id(emacs_frame_id) {
                    self.cancel_gpu_startup();
                    self.frame_windows
                        .reject_ready(emacs_frame_id, "Native frame creation cancelled");
                    self.frame_windows.take_primary_window();
                    self.frame_windows.clear_primary_mapping();
                } else if self.frame_windows.get(emacs_frame_id).is_some() {
                    self.frame_windows.request_destroy(emacs_frame_id);
                }
            }
            WindowCommand::AdoptPrimaryFrame { frame } => {
                let emacs_frame_id = frame.raw_id();
                tracing::info!("AdoptPrimaryFrame request: frame_id=0x{:x}", emacs_frame_id);
                self.frame_windows.adopt_primary_frame_id(emacs_frame_id);
                if self
                    .frame_windows
                    .get(emacs_frame_id)
                    .and_then(|state| state.window())
                    .is_some_and(|window| window.has_focus())
                {
                    self.frame_opacity.focus(emacs_frame_id, true);
                    self.refresh_frame_opacity();
                }
                // The window's scheduling identity moves from the pending id
                // (0) to the adopted Emacs frame id; retire the old entry so
                // its deadlines and request token cannot go stale.
                self.frame_coordinator
                    .remove_window(super::frame_sched::NativeWindowId(0));
            }
        }
        if replay_primary
            && let Some(pending) = &self.gpu_startup
            && let Some(primary) = self.frame_windows.primary_window()
        {
            primary.replay_pending_native_state(pending.window());
        }
    }
}
