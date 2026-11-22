//! Opt-in native presentation receipts; observations never drive rendering.
mod wayland;

pub(crate) struct PresentationObserver {
    wayland: wayland::PresentationObserver,
}
impl PresentationObserver {
    pub(crate) fn new() -> Self {
        Self {
            wayland: wayland::PresentationObserver::new(),
        }
    }
    /// Winit's display, where receipts are requested.
    pub(crate) fn attach(&mut self, display: winit::event_loop::OwnedDisplayHandle) {
        self.wayland.attach(display);
    }
    pub(crate) fn before_present(
        &mut self,
        window: &dyn winit::window::Window,
        frame: u64,
        layout: romacs_display_protocol::PresentationId,
        size: (u32, u32),
        scale: f64,
    ) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(RawWindowHandle::Wayland(_)) = window.window_handle().map(|handle| handle.as_raw()) {
            self.wayland.before_present(window, frame, layout, size, scale);
        }
    }
    pub(crate) fn dispatch_deadline(
        &mut self,
        now: romacs_display_protocol::frame_time::EventTime,
    ) -> Option<romacs_display_protocol::frame_time::EventTime> {
        self.wayland.dispatch_deadline(now)
    }
    pub(crate) fn dispatch_pending(&mut self) {
        self.wayland.dispatch_pending();
    }
    pub(crate) fn shutdown(&mut self) {
        self.wayland.shutdown();
    }
}
