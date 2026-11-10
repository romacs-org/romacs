//! Window and chrome render commands.

use super::RenderApp;
use crate::thread_comm::{InputEvent, WindowCommand};
use winit::window::UserAttentionType;

impl RenderApp {
    pub(super) fn retire_cancelled_frames(&mut self) {
        let cancelled: Vec<_> = self
            .frame_leases
            .iter()
            .filter_map(|(frame, live)| {
                (!live.load(std::sync::atomic::Ordering::Acquire)).then_some(*frame)
            })
            .collect();
        for frame in cancelled {
            self.frame_leases.remove(&frame);
            if let Some(waits) = &self.comms.native_window_waits {
                waits.remove(frame);
            }
            self.handle_window(WindowCommand::DestroyWindow {
                frame: crate::thread_comm::FrameRef::Frame(frame),
            });
        }
    }

    pub(super) fn refresh_frame_opacity(&mut self) {
        // Never hold this CPU lock across GPU work or callback-capable code.
        let controls = self
            .comms
            .frame_opacity
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for window in self.frame_windows.windows.values_mut() {
            let composites = window.lifecycle.native().is_some_and(|native| native.composites_alpha());
            if window.render.apply_surface_frame_opacity(&controls, composites) {
                window.request_redraw();
            }
        }
    }

    fn remove_pending_child_subtree(&mut self, frame_id: u64) {
        let mut subtree = std::collections::HashSet::from([frame_id]);
        loop {
            let before = subtree.len();
            for (&id, state) in &self.pending_child_frames {
                if state
                    .state
                    .frame_placement
                    .parent()
                    .is_some_and(|parent| subtree.contains(&parent.get()))
                {
                    subtree.insert(id);
                }
            }
            if subtree.len() == before {
                break;
            }
        }
        self.pending_child_frames
            .retain(|id, _| !subtree.contains(id));
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
        self.remove_pending_child_subtree(frame_id);
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
            WindowCommand::SetWindowTitle { .. } => true,
            WindowCommand::SetFrameGeometryHints { frame, .. } => {
                self.frame_windows.is_primary_frame_id(frame.raw_id())
            }
            _ => false,
        };
        match cmd {
            WindowCommand::RefreshFrameOpacity => self.refresh_frame_opacity(),
            WindowCommand::SetWindowTitle { title } => {
                if let Some(primary_state) = self.frame_windows.primary_window_mut() {
                    primary_state.chrome_mut().title = title.clone();
                    primary_state.set_title(title);
                    if !primary_state.chrome().decorations_enabled {
                        primary_state.render.mark_dirty();
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
            WindowCommand::RequestAttention { urgent } => {
                if let Some(window) = self
                    .frame_windows
                    .primary_window()
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
                reply,
                live,
                deadline,
            } => {
                if !live.load(std::sync::atomic::Ordering::Acquire) {
                    // Legacy adoption owns the pre-existing initial window.
                    // Never let a stale cancellation retire a newer identity.
                    if adopt_primary && self.frame_windows.primary_frame_id().is_none() {
                        self.handle_window(WindowCommand::DestroyWindow {
                            frame: crate::thread_comm::FrameRef::Primary,
                        });
                    }
                    return;
                }
                let id = frame.raw_id();
                if let Some(waits) = &self.comms.native_window_waits {
                    if waits.terminal() {
                        if let Some(reply) = reply {
                            let _ = reply.send(Err(
                                "Native connection closed during frame preparation".into(),
                            ));
                        }
                        return;
                    }
                    waits.register(id, live.clone(), deadline);
                }
                self.frame_leases.insert(id, live);
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
                    self.handle_window(WindowCommand::SetWindowTitle { title });
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
                if let Some(reply) = reply {
                    self.frame_windows.await_ready(id, reply);
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
                if self.frame_windows.is_primary_frame_id(emacs_frame_id) { self.comms.pointer.set(None); }
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
                    self.comms
                        .frame_opacity
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .focus(emacs_frame_id, true);
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
