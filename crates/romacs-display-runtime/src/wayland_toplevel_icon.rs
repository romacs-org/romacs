//! Native Wayland toplevel-icon integration.
//!
//! Winit 0.30 intentionally leaves `Window::set_window_icon` as a no-op on
//! Wayland. Keep the staging protocol inside this module, on a guest of
//! winit's display, so callers only express the application-level intent.

use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::globals::{BindError, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::toplevel_icon::v1::client::{
    xdg_toplevel_icon_manager_v1, xdg_toplevel_icon_v1,
};
use winit::event_loop::OwnedDisplayHandle;
use winit::platform::wayland::WindowExtWayland;
use winit::window::Window;

use crate::wayland_guest::Guest;
use crate::window_icon::RasterizedWindowIcon;
use crate::window_identity::ROMACS_APPLICATION;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ToplevelIconError {
    #[error("window has no usable Wayland display handle")]
    DisplayHandle,
    #[error("Wayland toplevel-icon setup failed: {0}")]
    Protocol(String),
    #[error("invalid icon dimensions")]
    InvalidDimensions,
    #[error("icon shared-memory setup failed: {0}")]
    SharedMemory(#[from] std::io::Error),
    #[error("icon memfd setup failed: {0}")]
    Memfd(#[from] rustix::io::Errno),
}

struct ProtocolState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for ProtocolState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(ProtocolState: ignore wl_shm::WlShm);
delegate_noop!(ProtocolState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(ProtocolState: ignore wl_buffer::WlBuffer);
delegate_noop!(ProtocolState: ignore xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1);
delegate_noop!(ProtocolState: ignore xdg_toplevel_icon_v1::XdgToplevelIconV1);

struct WaylandIconSession {
    guest: Guest,
    event_queue: EventQueue<ProtocolState>,
    state: ProtocolState,
    manager: xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1,
    shm: wl_shm::WlShm,
}

enum WaylandIconBackend {
    Protocol(WaylandIconSession),
    DesktopEntryOnly { display: OwnedDisplayHandle },
}

impl WaylandIconBackend {
    fn display(&self) -> &OwnedDisplayHandle {
        match self {
            Self::Protocol(session) => session.guest.display(),
            Self::DesktopEntryOnly { display } => display,
        }
    }
}

impl WaylandIconSession {
    fn connect(display: OwnedDisplayHandle) -> Result<WaylandIconBackend, ToplevelIconError> {
        let guest = Guest::new(display.clone()).ok_or(ToplevelIconError::DisplayHandle)?;
        let (globals, event_queue) = registry_queue_init::<ProtocolState>(guest.connection())
            .map_err(|error| ToplevelIconError::Protocol(error.to_string()))?;
        let queue_handle = event_queue.handle();
        let manager: xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1 =
            match globals.bind(&queue_handle, 1..=1, ()) {
                Ok(manager) => manager,
                Err(BindError::NotPresent | BindError::UnsupportedVersion) => {
                    tracing::debug!(
                        desktop_file_id = ROMACS_APPLICATION.desktop_file_id().as_str(),
                        "compositor has no xdg-toplevel-icon protocol; using desktop-entry icon"
                    );
                    return Ok(WaylandIconBackend::DesktopEntryOnly { display });
                }
            };
        let shm = globals
            .bind(&queue_handle, 1..=1, ())
            .map_err(|error| ToplevelIconError::Protocol(error.to_string()))?;

        Ok(WaylandIconBackend::Protocol(Self {
            guest,
            event_queue,
            state: ProtocolState,
            manager,
            shm,
        }))
    }

    fn set_icon(
        &mut self,
        window: &dyn Window,
        icon_pixels: &RasterizedWindowIcon,
    ) -> Result<(), ToplevelIconError> {
        let width =
            i32::try_from(icon_pixels.width()).map_err(|_| ToplevelIconError::InvalidDimensions)?;
        let height = i32::try_from(icon_pixels.height())
            .map_err(|_| ToplevelIconError::InvalidDimensions)?;
        let stride = width
            .checked_mul(4)
            .ok_or(ToplevelIconError::InvalidDimensions)?;
        let bytes = icon_pixels.to_wayland_argb8888();
        let byte_len =
            i32::try_from(bytes.len()).map_err(|_| ToplevelIconError::InvalidDimensions)?;

        let fd = memfd_create(c"romacs-window-icon", MemfdFlags::CLOEXEC)?;
        let mut file = File::from(fd);
        file.set_len(bytes.len() as u64)?;
        file.write_all(&bytes)?;

        let queue_handle = self.event_queue.handle();
        let pool = self
            .shm
            .create_pool(file.as_fd(), byte_len, &queue_handle, ());
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            stride,
            wl_shm::Format::Argb8888,
            &queue_handle,
            (),
        );
        let protocol_icon = self.manager.create_icon(&queue_handle, ());
        protocol_icon.set_name(ROMACS_APPLICATION.icon_name().as_str().to_owned());
        protocol_icon.add_buffer(&buffer, 1);

        self.guest
            .set_toplevel_icon(&self.manager, window, &protocol_icon)
            .map_err(ToplevelIconError::Protocol)?;

        // set_icon snapshots the immutable icon. Destroying these temporary
        // protocol objects also makes their Rust/file resources deterministic.
        protocol_icon.destroy();
        buffer.destroy();
        pool.destroy();
        self.guest
            .connection()
            .flush()
            .map_err(|error| ToplevelIconError::Protocol(error.to_string()))?;
        self.event_queue
            .dispatch_pending(&mut self.state)
            .map_err(|error| ToplevelIconError::Protocol(error.to_string()))?;
        Ok(())
    }
}

pub(crate) struct WaylandToplevelIconService {
    backend: Option<WaylandIconBackend>,
}

impl WaylandToplevelIconService {
    pub(crate) const fn new() -> Self {
        Self { backend: None }
    }

    pub(crate) fn apply(
        &mut self,
        window: &dyn Window,
        display: &OwnedDisplayHandle,
        icon_pixels: &RasterizedWindowIcon,
    ) -> Result<(), ToplevelIconError> {
        if window.xdg_toplevel().is_none() {
            return Ok(());
        }
        if self
            .backend
            .as_ref()
            .is_none_or(|current| current.display() != display)
        {
            self.shutdown();
            self.backend = Some(WaylandIconSession::connect(display.clone())?);
        }
        match self
            .backend
            .as_mut()
            .expect("backend was initialized above")
        {
            WaylandIconBackend::Protocol(session) => session.set_icon(window, icon_pixels),
            WaylandIconBackend::DesktopEntryOnly { .. } => Ok(()),
        }
    }

    /// Release the guest protocol queue while Winit's display is still live.
    pub(crate) fn shutdown(&mut self) {
        let Some(backend) = self.backend.take() else {
            return;
        };
        if let WaylandIconBackend::Protocol(mut session) = backend {
            let _ = session.event_queue.dispatch_pending(&mut session.state);
            session.manager.destroy();
            let _ = session.guest.connection().flush();
        }
    }
}

#[cfg(test)]
mod tests;
