//! Opt-in native presentation receipts; observations never drive rendering.
mod wayland;
mod x11;

pub(crate) struct PresentationObserver {
    wayland: wayland::PresentationObserver,
    x11: x11::Observer,
}
impl PresentationObserver {
    pub(crate) fn new() -> Self {
        Self {
            wayland: wayland::PresentationObserver::new(),
            x11: x11::Observer::new(),
        }
    }
    pub(crate) fn before_present(
        &mut self,
        window: &dyn winit::window::Window,
        device: &wgpu::Device,
        surface: &wgpu::Surface<'_>,
        generation: u64,
        frame: u64,
        layout: romacs_display_protocol::PresentationId,
        size: (u32, u32),
        scale: f64,
    ) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Wayland(_)) => self
                .wayland
                .before_present(window, frame, layout, size, scale),
            Ok(RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)) => self
                .x11
                .before_present(device, surface, generation, frame, layout, size, scale),
            _ => {}
        }
    }
    pub(crate) fn dispatch_deadline(
        &mut self,
        now: romacs_display_protocol::frame_time::EventTime,
    ) -> Option<romacs_display_protocol::frame_time::EventTime> {
        self.wayland
            .dispatch_deadline(now)
            .into_iter()
            .chain(self.x11.deadline(now))
            .min()
    }
    pub(crate) fn dispatch_pending<'a>(
        &mut self,
        device: Option<&wgpu::Device>,
        surface: impl FnMut(u64) -> Option<(&'a wgpu::Surface<'static>, u64)>,
    ) {
        self.wayland.dispatch_pending();
        if let Some(device) = device {
            self.x11.dispatch(device, surface);
        }
    }
    pub(crate) fn shutdown(&mut self) {
        self.wayland.shutdown();
        self.x11.shutdown();
    }
}
