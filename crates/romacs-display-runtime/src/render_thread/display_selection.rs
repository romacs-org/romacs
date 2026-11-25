use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PixelPolicy {
    Logical,
    Physical,
}

impl PixelPolicy {
    pub(super) fn effective_scale(self, compositor_scale: f64) -> f64 {
        match self {
            Self::Logical => compositor_scale,
            // Fontconfig already accounts for X11 DPI.
            Self::Physical => 1.0,
        }
    }
}

#[derive(Debug)]
/// The backend and display name selected at native GUI startup.
pub struct DisplaySelection {
    name: OsString,
    backend: Backend,
}

#[derive(Debug)]
enum Backend {
    Wayland(PathBuf),
    X11,
}

impl DisplaySelection {
    /// Select the native display for this launch. An explicit socket is
    /// authoritative even when opening it will fail.
    #[cfg(unix)]
    pub fn from_env(explicit_wayland: Option<&OsStr>) -> Result<Self, String> {
        use std::os::unix::fs::FileTypeExt;

        let runtime = std::env::var_os("XDG_RUNTIME_DIR");
        let resolve = |name: &OsStr| {
            let path = Path::new(name);
            if path.is_absolute() {
                Some(path.to_owned())
            } else {
                runtime.as_ref().map(|dir| Path::new(dir).join(path))
            }
        };
        let explicit = explicit_wayland
            .map(|name| {
                resolve(name)
                    .map(|path| (name, path))
                    .ok_or("XDG_RUNTIME_DIR is required for a relative Wayland socket")
            })
            .transpose()?;
        if explicit.is_some() {
            return select(explicit, None, None);
        }
        let wayland = std::env::var_os("WAYLAND_DISPLAY");
        let available = wayland.as_deref().and_then(|name| {
            let path = resolve(name)?;
            std::fs::metadata(&path)
                .ok()?
                .file_type()
                .is_socket()
                .then_some((name, path))
        });
        let x11 = std::env::var_os("DISPLAY");
        select(None, available, x11.as_deref())
    }

    pub fn name(&self) -> &OsStr {
        &self.name
    }

    pub fn wayland_socket(&self) -> Option<&Path> {
        match &self.backend {
            Backend::Wayland(path) => Some(path),
            Backend::X11 => None,
        }
    }

    pub fn pgtk_display_class(&self) -> &'static str {
        match self.backend {
            Backend::Wayland(_) => "GdkWaylandDisplay",
            Backend::X11 => "GdkX11Display",
        }
    }

    pub(crate) fn pixel_policy(&self) -> PixelPolicy {
        match self.backend {
            Backend::Wayland(_) => PixelPolicy::Logical,
            Backend::X11 => PixelPolicy::Physical,
        }
    }
}

fn select(
    explicit: Option<(&OsStr, PathBuf)>,
    available_wayland: Option<(&OsStr, PathBuf)>,
    x11: Option<&OsStr>,
) -> Result<DisplaySelection, String> {
    if let Some((name, socket)) = explicit.or(available_wayland) {
        return Ok(DisplaySelection {
            name: name.to_owned(),
            backend: Backend::Wayland(socket),
        });
    }
    if let Some(name) = x11.filter(|name| !name.is_empty()) {
        return Ok(DisplaySelection {
            name: name.to_owned(),
            backend: Backend::X11,
        });
    }
    Err("No display available: WAYLAND_DISPLAY must name a socket or DISPLAY must be set".into())
}

#[cfg(test)]
#[path = "tests/display_selection_test.rs"]
mod tests;
