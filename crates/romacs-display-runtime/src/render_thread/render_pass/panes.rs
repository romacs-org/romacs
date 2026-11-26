//! The pane pass: each owner-laid window of the root, drawn over the root's
//! glyphs and under its child frames and overlays (design O1b.md rev 3,
//! §6.2; `pane_layers.rs` holds what it draws).
//!
//! Owns: per root entry, the pane's picture at the entry's rect (its own
//! font tables bound, its text animations painted over a copy, its cursor
//! hidden by the entry's `cursor_off`), then the over-pane hook; the root's
//! font tables bound again after. A frame with panes never takes the
//! retained-static path, so the hook runs every frame; the pass says when
//! one still animates, for the caller's frame demand.
//!
//! Must not: change which picture a pane shows (ingest does), or draw a
//! picture of another font catalog than the root's: its font ids name
//! tables the atlas would clear itself for.
use super::surface::FrameRenderFailure;
use crate::render_thread::frame_windows::GuiFrameRenderState;
use crate::render_thread::pane_layers::DrawnPane;
use romacs_display_protocol::foreign_pane::ForeignPaneEntry;
use romacs_display_protocol::frame_glyphs::FrameGlyphBuffer;
use romacs_display_protocol::pane_picture::PaneHead;
use romacs_display_protocol::RootSurfaceRect;
use romacs_renderer_wgpu::{SnapshotSize, WgpuRenderer};
use std::sync::Arc;

/// Draw FRAME's panes onto VIEW, a surface of CONTENT_SIZE device px:
/// whether a hook still animates, and wants the next frame.
pub(super) fn render_pane_layers(
    renderer: &mut WgpuRenderer,
    render: &mut GuiFrameRenderState,
    view: &wgpu::TextureView,
    frame: &FrameGlyphBuffer,
    content_size: (u32, u32),
) -> Result<bool, FrameRenderFailure> {
    if frame.foreign_panes.is_empty() {
        return Ok(false);
    }
    let size = SnapshotSize::new(content_size.0, content_size.1).ok_or(FrameRenderFailure::WindowNotReady)?;
    let sample = renderer.frame_sample();
    let mut animating = false;
    let mut drawn = Ok(());
    for entry in &frame.foreign_panes {
        let Ok(rect) = RootSurfaceRect::new(entry.rect.x, entry.rect.y, entry.rect.width, entry.rect.height) else {
            continue;
        };
        let picture = match draw_picture(renderer, render, view, frame, entry, rect, size) {
            Ok(picture) => picture,
            Err(error) => {
                drawn = Err(error);
                break;
            }
        };
        let layers = &render.compositor.pane_layers;
        let pane = DrawnPane { owner: entry.owner, window: entry.window, rect, picture, state: layers.state(entry.owner) };
        if let Some(hook) = layers.over_pane() {
            let mut hook = hook.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            animating |= (*hook)(renderer, view, &pane, sample);
        }
    }
    // The root's tables again, even when a pane failed preparation.
    if let Some(atlas) = render.compositor.glyph_atlas.as_mut() {
        atlas.set_current_frame_fonts(frame.font_bindings());
    }
    drawn.map(|()| animating)
}

/// Draw ENTRY's picture at RECT, when it has one of ROOT's font catalog:
/// its head, or None when it draws none.
fn draw_picture(
    renderer: &mut WgpuRenderer,
    render: &mut GuiFrameRenderState,
    view: &wgpu::TextureView,
    root: &FrameGlyphBuffer,
    entry: &ForeignPaneEntry,
    rect: RootSurfaceRect,
    size: SnapshotSize,
) -> Result<Option<PaneHead>, FrameRenderFailure> {
    let compositor = &mut render.compositor;
    let (Some(shown), Some(atlas)) = (compositor.pane_layers.shown_mut(entry.window), compositor.glyph_atlas.as_mut()) else {
        return Ok(None);
    };
    if shown.frame.font_catalog_generation != root.font_catalog_generation {
        return Ok(None);
    }
    let mut pane = Arc::clone(&shown.frame);
    if shown.animations.is_running() {
        shown.animations.paint(Arc::make_mut(&mut pane));
    }
    atlas.set_current_frame_fonts(pane.font_bindings());
    let prepared = romacs_renderer_wgpu::renderer::PreparedChildFrame::new(&pane, 1.0, size, None).map_err(|error| {
        tracing::error!(?error, window = entry.window, "pane preparation rejected; retrying without present");
        FrameRenderFailure::WindowNotReady
    })?;
    renderer.render_child_frame_prepared(
        view,
        prepared,
        rect.x(),
        rect.y(),
        rect,
        atlas,
        !entry.cursor_off,
        None,
        0.0,
        false,
        0,
        0.0,
        0.0,
        None,
        1.0,
        [rect.x(), rect.y()],
    );
    Ok(Some(shown.head))
}

#[cfg(test)]
#[path = "tests/panes_test.rs"]
mod tests;
