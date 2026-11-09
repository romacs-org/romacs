//! Native display observation adapter for frame-scale policy.

use romacs_display_protocol::{
    DisplayHeightGeometry, DisplayObservation, Dpi, X11DisplayObservation, XServerKind,
};
use std::sync::OnceLock;
use winit::event_loop::EventLoop;

#[cfg(target_os = "linux")]
use std::ffi::CStr;
#[cfg(target_os = "linux")]
use std::ptr;
#[cfg(target_os = "linux")]
use winit::platform::wayland::EventLoopExtWayland;
#[cfg(target_os = "linux")]
use x11_dl::xlib;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowCoordinateSystem {
    WinitLogical,
    X11Physical,
}

static ACTIVE_WINDOW_COORDINATE_SYSTEM: OnceLock<WindowCoordinateSystem> = OnceLock::new();

fn coordinate_system_for_observation(observation: DisplayObservation) -> WindowCoordinateSystem {
    match observation {
        DisplayObservation::X11(_) => WindowCoordinateSystem::X11Physical,
        _ => WindowCoordinateSystem::WinitLogical,
    }
}

pub(crate) fn active_window_coordinate_system() -> Option<WindowCoordinateSystem> {
    ACTIVE_WINDOW_COORDINATE_SYSTEM.get().copied()
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectedLinuxBackend {
    Wayland,
    X11,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Default)]
struct RawX11DisplayObservation {
    has_xwayland_extension: bool,
    vendor: Option<String>,
    xft_dpi: Option<f32>,
    display_height_px: i32,
    display_height_mm: i32,
}

#[cfg(target_os = "linux")]
impl RawX11DisplayObservation {
    fn validate(self) -> X11DisplayObservation {
        let xft_dpi = self.xft_dpi.and_then(|dpi| Dpi::new(dpi).ok());
        let geometry = u32::try_from(self.display_height_px)
            .ok()
            .zip(u32::try_from(self.display_height_mm).ok())
            .and_then(|(height_px, height_mm)| {
                DisplayHeightGeometry::new(height_px, height_mm).ok()
            });
        X11DisplayObservation::new(
            classify_x_server(self.has_xwayland_extension, self.vendor.as_deref()),
            xft_dpi,
            geometry,
        )
    }
}

#[cfg(target_os = "linux")]
fn classify_x_server(has_xwayland_extension: bool, vendor: Option<&str>) -> XServerKind {
    if has_xwayland_extension {
        XServerKind::Xwayland
    } else if vendor.is_some_and(|vendor| vendor.contains("X.Org")) {
        XServerKind::Xorg
    } else {
        XServerKind::Unknown
    }
}

#[cfg(target_os = "linux")]
fn fallback_x11_observation() -> X11DisplayObservation {
    RawX11DisplayObservation::default().validate()
}

#[cfg(target_os = "linux")]
fn observe_linux_backend(
    backend: SelectedLinuxBackend,
    x11_probe: impl FnOnce() -> X11DisplayObservation,
) -> DisplayObservation {
    match backend {
        SelectedLinuxBackend::Wayland => DisplayObservation::Wayland,
        SelectedLinuxBackend::X11 => DisplayObservation::X11(x11_probe()),
    }
}

#[cfg(target_os = "linux")]
fn query_x11_display_with_timeout(
    timeout: std::time::Duration,
    probe: impl FnOnce() -> X11DisplayObservation + Send + 'static,
) -> X11DisplayObservation {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("x11-display-probe".into())
        .spawn(move || {
            let _ = tx.send(probe());
        });
    if spawned.is_err() {
        tracing::warn!("failed to spawn X11 display probe; using fallback DPI");
        return fallback_x11_observation();
    }
    match rx.recv_timeout(timeout) {
        Ok(observation) => observation,
        Err(_) => {
            tracing::warn!(
                timeout_ms = timeout.as_millis(),
                "X11 display probe timed out; using fallback DPI"
            );
            fallback_x11_observation()
        }
    }
}

// Every test here drives the X11/Wayland probe, whose types exist only
// on Linux; the module would not compile elsewhere.
#[cfg(all(test, target_os = "linux"))]
#[path = "display_scale/tests/display_scale_test.rs"]
mod tests;
