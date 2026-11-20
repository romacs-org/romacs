use crate::host_types::GuiFrameGeometryHints;
use winit::window::Window;

use super::state::window_size_from_emacs_pixels;

pub(crate) fn apply_window_geometry_hints(
    window: &dyn Window,
    geometry_hints: GuiFrameGeometryHints,
) {
    // GNU gtkutil.c:xg_wm_set_size_hint supplies the minimum with resize
    // increments. This is essential on Wayland: winit snaps floating windows
    // relative to their minimum surface size, not relative to zero. Leaving
    // its default minimum in place shrinks even an already aligned text grid.
    // Hints and resize requests describe the same Emacs coordinate space.
    // Wayland/Cocoa use logical units; X11's font metrics are already physical.
    // Labeling logical hints PhysicalSize halves the grid at scale 2 and lets
    // native configure snap a correctly sized 91-column window to 90 columns.
    window.set_min_surface_size(Some(window_size_from_emacs_pixels(
        geometry_hints.min_width.max(1),
        geometry_hints.min_height.max(1),
    )));
    window.set_surface_resize_increments(Some(window_size_from_emacs_pixels(
        geometry_hints.width_inc.max(1),
        geometry_hints.height_inc.max(1),
    )));
}
