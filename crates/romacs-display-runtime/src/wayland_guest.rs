//! Romacs's own Wayland protocol queues on winit's display.
//!
//! Winit owns the `wl_display`. Romacs binds what winit does not
//! (presentation-time, data-control, the toplevel icon) on guests of that
//! display: each a backend with its own libwayland event queue, so whoever
//! reads the socket sorts every event into its own queue, and winit's
//! `prepare_read` still sees what is pending for it. A clone of winit's
//! `Connection` would not do: its queues share one libwayland queue, a guest's
//! read would sort winit's events into winit's Rust queue, and wayland-client
//! 0.31's `prepare_read` would let winit sleep on them.
//!
//! The crate's unsafe code is here, behind safe functions. A guest keeps
//! winit's display connected for the rest of the process: its event queues
//! hold its backend beyond the guest, so no drop order could bound them, and
//! winit makes one event loop, so one display, a process. A window's objects
//! are taken only from a window on the guest's display, and only for the one
//! request a function here makes: winit owns them. (A window's `wl_surface`
//! needs none of this: winit lends it as a proxy, `Window::surface`.)

use std::ffi::c_void;
use std::ptr::NonNull;

use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::{Connection, Proxy};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_v1::XdgToplevelIconV1;
use winit::event_loop::OwnedDisplayHandle;
use winit::platform::wayland::WindowExtWayland;
use winit::window::Window;

/// The `wl_display` behind HANDLE, or `None` off Wayland.
fn wayland_display(handle: &(impl HasDisplayHandle + ?Sized)) -> Option<NonNull<c_void>> {
    match handle.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(display) => Some(display.display),
        _ => None,
    }
}

/// DISPLAY's `wl_display`, connected from now until the process exits, or
/// `None` off Wayland. Winit's `OwnedDisplayHandle` holds winit's
/// `Connection`, which disconnects the display only when its last clone
/// drops; one clone is never dropped.
fn kept_display(display: &OwnedDisplayHandle) -> Option<NonNull<c_void>> {
    let raw = wayland_display(display)?;
    std::mem::forget(display.clone());
    Some(raw)
}

/// A connection of Romacs's own on winit's Wayland display.
pub(crate) struct Guest {
    connection: Connection,
    display: OwnedDisplayHandle,
    /// The address of the `wl_display` the guest borrows, as it made it.
    raw: usize,
}

impl Guest {
    /// A guest on DISPLAY, or `None` when DISPLAY is not a Wayland display.
    pub(crate) fn new(display: OwnedDisplayHandle) -> Option<Self> {
        let raw = kept_display(&display)?;
        // SAFETY: RAW is DISPLAY's wl_display, connected until the process
        // exits (kept_display), so it outlives this backend and every queue
        // and proxy that holds it. A foreign backend never disconnects the
        // display it borrows.
        // UNSAFE(winit exposes its Wayland connection only as a raw wl_display, and wayland-backend makes a guest of one only through from_foreign_display)
        let backend = unsafe { Backend::from_foreign_display(raw.as_ptr().cast()) };
        Some(Self {
            connection: Connection::from_backend(backend),
            display,
            raw: raw.as_ptr() as usize,
        })
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) fn display(&self) -> &OwnedDisplayHandle {
        &self.display
    }

    /// Show ICON as WINDOW's toplevel icon, through MANAGER.
    pub(crate) fn set_toplevel_icon(
        &self,
        manager: &XdgToplevelIconManagerV1,
        window: &dyn Window,
        icon: &XdgToplevelIconV1,
    ) -> Result<(), String> {
        // MANAGER and ICON on this guest's backend: their liveness check and
        // their use in the request then hold one lock, as their destruction does.
        if !self.owns(manager) || !self.owns(icon) {
            return Err("the icon is not this guest's".into());
        }
        let id = self.window_toplevel(window)?;
        let toplevel = XdgToplevel::from_id(&self.connection, id).map_err(|error| error.to_string())?;
        manager.set_icon(&toplevel, Some(icon));
        Ok(())
    }

    /// Whether PROXY is an object of this guest's backend.
    fn owns(&self, proxy: &impl Proxy) -> bool {
        proxy.backend().upgrade() == Some(self.connection.backend())
    }

    /// WINDOW's `xdg_toplevel`, for one request on this guest before the
    /// caller returns: the proxy made of it never leaves this module.
    fn window_toplevel(&self, window: &dyn Window) -> Result<ObjectId, String> {
        if wayland_display(window).map(|display| display.as_ptr() as usize) != Some(self.raw) {
            return Err("the window is not on this Wayland display".into());
        }
        let toplevel = window.xdg_toplevel().ok_or("the window has no xdg_toplevel")?;
        // SAFETY: TOPLEVEL is WINDOW's xdg_toplevel. Winit hands it out only
        // as this pointer, never as a proxy, and destroys it only when the
        // Window drops, so the WINDOW borrow keeps it alive through this call
        // and the caller's one request; from_ptr checks its interface. WINDOW
        // is on the display the guest borrows (checked above), so that request
        // reaches the object's owner.
        // UNSAFE(winit hands out a window's xdg_toplevel only as a raw pointer, and wayland-backend takes one only through ObjectId::from_ptr)
        unsafe { ObjectId::from_ptr(XdgToplevel::interface(), toplevel.as_ptr().cast()) }
            .map_err(|error| error.to_string())
    }
}

/// smithay-clipboard on winit's Wayland display.
pub(crate) struct SmithayClipboard {
    clipboard: smithay_clipboard::Clipboard,
}

impl SmithayClipboard {
    /// smithay-clipboard on DISPLAY, or `None` when DISPLAY is not Wayland.
    pub(crate) fn new(display: &OwnedDisplayHandle) -> Option<Self> {
        let raw = kept_display(display)?;
        // SAFETY: RAW is DISPLAY's wl_display, connected until the process
        // exits (kept_display), so it outlives the clipboard and its worker.
        // UNSAFE(smithay-clipboard takes its display only as a raw wl_display)
        let clipboard = unsafe { smithay_clipboard::Clipboard::new(raw.as_ptr()) };
        Some(Self { clipboard })
    }

    pub(crate) fn get(&self) -> &smithay_clipboard::Clipboard {
        &self.clipboard
    }
}
