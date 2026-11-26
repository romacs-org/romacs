//! The retained-static composition strategy: the fast path a compositor-only
//! frame takes when nothing but the cursor has moved.
//!
//! Owns: the rule deciding whether a frame is eligible, the retained
//! cursorless scene texture and its per-generation validity, and the
//! composition of front cursors over it.
//!
//! Front cursors paint over retained root text
//! (`offscreen_frame::composite_matches_full_render`). Filled boxes paint
//! beneath text, through the ordinary scene painter.
//!
//! Must not: acquire or present a surface, start or advance a transition, or
//! publish a projection. It exists precisely for frames where none of those
//! happen — the caller has already established that, and the eligibility rule
//! is where that establishment is written down.

use super::FrameDrawInputs;
use crate::render_thread::frame_stats;
use crate::render_thread::frame_windows::{
    GuiFrameNativeWindowState, GuiFrameRenderState, RetainedStatic,
};
use crate::render_thread::models::{ModelPicture, picture_of};
use crate::render_thread::state::PointerAppearanceState;
use romacs_display_protocol::frame_glyphs::FrameGlyphBuffer;
use romacs_renderer_wgpu::model::store::Snapshot;
use romacs_renderer_wgpu::{FullFrameTexture, SnapshotSize, UnpooledTexture, WgpuRenderer};
use std::collections::{BTreeMap, HashSet};

/// Whether this frame may be composed from the retained static scene:
/// the coordinator asked for a compositor-only cursor frame, and the scene is
/// eligible — no transition, no dynamic overlay, no pane motion, no owner's
/// pane (its over-pane hook runs every frame), and every cursor a clean
/// top-layer style. An ineligible frame falls through to the
/// ordinary full render. `ROMACS_DISABLE_RETAINED_STATIC` force-disables it.
///
/// The transition test asks whether one is *active*, not whether the frame
/// needs an offscreen: the latter is true whenever crossfade/scroll
/// transitions are merely enabled (the default), because the offscreen
/// snapshot only has to be kept current across scene commits. A
/// compositor-only frame changes no editor content, so it cannot start a
/// transition, and the "before" snapshot captured at the last scene-commit
/// full render stays correct. Gating on "needs an offscreen" would disable
/// this path entirely under the default transition policy.
///
/// A pane morph disqualifies the frame for the same reason an active
/// transition does: the retained scene is a picture of the panes where the
/// *presentation* puts them, and a moving frame needs them where the motion
/// puts them. Reproducing it would snap every pane to its destination for
/// that frame and back on the next — a blink landing mid-motion is enough to
/// show it.
pub(super) fn is_eligible(
    compositor_only_hint: bool,
    pane_placements: &[romacs_renderer_wgpu::PaneBlit],
    render: &GuiFrameRenderState,
) -> bool {
    compositor_only_hint
        && render.compositor.current_frame.as_deref().is_some_and(top_layer_cursors)
        && render.compositor.child_frames.frames.values().all(|child| top_layer_cursors(&child.frame))
        && pane_placements.is_empty()
        && render.compositor.current_frame.as_deref().is_none_or(|frame| frame.foreign_panes.is_empty())
        && !render.compositor.transitions.has_active()
        && !window_has_active_overlays(render)
        && pointer_appearance_allowed(&render.pointer_appearance)
        && !render.has_pointer_paint_damage()
        && std::env::var_os("ROMACS_DISABLE_RETAINED_STATIC").is_none()
}

fn top_layer_cursors(frame: &FrameGlyphBuffer) -> bool {
    frame.window_cursors.iter().all(|cursor| cursor.style.is_clean_top_layer())
}

pub(super) fn is_valid(render: &GuiFrameRenderState, size: SnapshotSize) -> bool {
    matches!(&render.compositor.retained_static,
        Some(rs) if rs.generation == render.compositor.current_scene_generation && rs.texture.size() == size)
}

/// Blit the retained cursorless scene and draw the cursor over it.
///
/// Rebuilds the retained texture first when the scene generation or the
/// window size has moved on — that rebuild is the one place this path runs
/// the glyph pipeline, once per generation rather than once per frame.
///
/// Draws onto `composition_view`, which is the frame-post source when the
/// post shader is on and the swapchain view otherwise; the caller owns that
/// choice and the frame-post step that follows.
///
/// Answers what the retained frame pictures of the native models.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    renderer: &mut WgpuRenderer,
    native: &GuiFrameNativeWindowState,
    render: &mut GuiFrameRenderState,
    composition_view: &wgpu::TextureView,
    frame: &romacs_display_protocol::frame_glyphs::FrameGlyphBuffer,
    inputs: &FrameDrawInputs<'_>,
    hovered_scroll_bar: Option<romacs_display_protocol::ScrollBarIdentity>,
) -> Result<ModelPicture, super::surface::FrameRenderFailure> {
    let Some(native_size) = SnapshotSize::new(native.content_size().0, native.content_size().1)
    else {
        // wgpu rejects a zero extent, so there is no scene to retain and
        // nothing to composite from. Reaching this would mean the window was
        // asked to draw at a size `resize` already refuses to configure.
        return Ok(ModelPicture::new());
    };
    let generation = render.compositor.current_scene_generation;
    let retained_valid = matches!(
        &render.compositor.retained_static,
        Some(rs) if retained_scene_is_current(
            (rs.generation, rs.texture.size(), &rs.models),
            (generation, native_size, inputs.models),
        )
    );

    if !retained_valid {
        ensure_retained_static_texture(renderer, render, native_size);
        let retained_view = render
            .compositor
            .retained_static
            .as_ref()
            .expect("retained texture just ensured")
            .texture
            .view()
            .clone();
        // Render the full cursorless static scene into the retained
        // texture (this runs the glyph pipeline once per generation), and
        // record which models (root and child) it composited and which it
        // referenced.
        let _ = renderer.take_drawn_surfaces();
        renderer.collect_referenced_surfaces();
        super::render_frame_window_contents(
            renderer,
            native,
            render,
            &retained_view,
            frame,
            inputs,
            false,
            true,
        )?;
        let models = RetainedModels::record(
            renderer.take_drawn_surfaces(),
            renderer.take_referenced_surfaces(),
            inputs.models,
        );
        if let Some(rs) = render.compositor.retained_static.as_mut() {
            rs.generation = generation;
            rs.models = models;
        }
        frame_stats::count(&frame_stats::RETAINED_STATIC_BUILDS);
    }
    let _non_root = romacs_renderer_wgpu::diagnostics::Span::new(
        romacs_display_protocol::present_trace::Stage::NonRootStart,
        romacs_display_protocol::present_trace::Stage::NonRootEnd,
        render.emacs_frame_id,
        frame.presentation_id,
    );
    if let Some(rs) = render.compositor.retained_static.as_ref() {
        renderer
            .begin_draw(romacs_renderer_wgpu::renderer::RenderTarget::new(
                composition_view,
                inputs.present_mapping.surface(),
            ))
            .blit_retained(&rs.bind_group);
    }
    renderer.cursor_light = render.cursor.light;
    renderer.render_cursor_only(
        composition_view,
        frame,
        inputs.present_mapping,
        true,
        inputs.animated_cursor,
        hovered_scroll_bar,
    );
    frame_stats::count(&frame_stats::COMPOSITE_ONLY_FRAMES);
    Ok(retained_frame_picture(
        render.compositor.retained_static.as_ref().map(|rs| &rs.models),
    ))
}

/// A blit preserves the model provenance originally recorded for the raster.
pub(in crate::render_thread) fn retained_frame_picture(retained: Option<&RetainedModels>) -> ModelPicture {
    retained.map(|r| r.picture.clone()).unwrap_or_default()
}

/// What a retained raster records of the native models when it is built.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::render_thread) struct RetainedModels {
    /// Provenance: every model the raster composited (root glyphs and
    /// child frames alike) at the snapshot its texture showed then. A frame
    /// blitting the raster presents exactly these.
    pub(in crate::render_thread) picture: ModelPicture,
    /// Dependencies: every surface the raster's glyph passes referenced,
    /// drawn or not, with what the store said the model's texture showed
    /// at the build (`None`: loading, absent, or not a model). A glyph whose
    /// model was not yet drawable composites nothing; once the model
    /// uploads, the raster no longer pictures the scene (SOL-REVIEW-2
    /// MUST 3, OPUS-REVIEW MUST 1).
    depends_on: BTreeMap<u32, Option<Snapshot>>,
}

impl RetainedModels {
    /// The record of a raster whose build composited `drawn` and referenced
    /// `referenced` surfaces, with `current` what every model's texture
    /// showed for that frame. The draw and its tests share this.
    pub(in crate::render_thread) fn record(
        drawn: Option<HashSet<u32>>,
        referenced: HashSet<u32>,
        current: &ModelPicture,
    ) -> Self {
        let picture = picture_of(drawn, current);
        let depends_on = referenced
            .into_iter()
            .chain(picture.keys().copied())
            .map(|id| (id, current.get(&id).copied()))
            .collect();
        Self {
            picture,
            depends_on,
        }
    }
}

/// Whether a retained scene built as `built` (editor scene generation,
/// size, and its model record) still pictures the scene `now`: same
/// generation and size, and every model its glyphs referenced is as the
/// store had it then. A model redrawn (orbit, zoom, Lisp request, rescale)
/// or freed since leaves the raster showing old pixels, and one uploaded
/// since leaves it missing pixels; no editor scene change would replace
/// either. A model the raster does not reference is irrelevant to it.
pub(in crate::render_thread) fn retained_scene_is_current(
    built: (u64, SnapshotSize, &RetainedModels),
    now: (u64, SnapshotSize, &ModelPicture),
) -> bool {
    built.0 == now.0
        && built.1 == now.1
        && built
            .2
            .depends_on
            .iter()
            .all(|(id, snapshot)| now.2.get(id) == snapshot.as_ref())
}

/// Whether any dynamic overlay is active. Overlays are not part of the
/// retained static scene, so their presence forces the full render path.
///
/// Idle dimming is included: it is a post-content overlay drawn *after* the
/// cursor (glyphs.rs draw_post_content_effects, after draw_cursor_layer), so
/// in a full render the cursor is dimmed too. The composite fast path draws
/// the cursor over the already-dimmed retained scene, which would leave the
/// cursor undimmed — and the retained scene's validity is not keyed on dim
/// alpha. Falling back to the full render whenever dimming is active keeps
/// both correct.
pub(super) fn window_has_active_overlays(render: &GuiFrameRenderState) -> bool {
    render.overlays.visual_bell_start.is_some()
        || render.has_ime_preedit()
        || render.overlays.idle_dim.active
        // The FPS counter is redrawn from a live timer every frame; the
        // retained scene would freeze it, so a full render is required
        // while it is shown.
        || render.overlays.fps.enabled
        // A child frame mid-lifecycle-animation (or its retained corpse)
        // must not be frozen into the retained texture at whatever alpha
        // the last build saw: the fade draws through full renders until
        // the corpse is gone and the texture rebuilds without it.
        || render.compositor.child_frames.has_animation_activity()
}

pub(super) fn pointer_appearance_allowed(pointer_appearance: &PointerAppearanceState) -> bool {
    pointer_appearance.active().is_none()
}

/// Ensure the window's retained-static texture exists at `size` in the
/// surface format, recreating it on a size change. Leaves the generation
/// stamp untouched (the caller sets it after rendering).
///
/// Not a pool lease: `RetainedStatic` outlives the frame that drew it, so it
/// owns a [`FullFrameTexture`] and is counted through the
/// [`UnpooledTexture::RetainedStaticScene`] census instead.
///
/// [`UnpooledTexture::RetainedStaticScene`]:
///     romacs_renderer_wgpu::UnpooledTexture::RetainedStaticScene
pub(super) fn ensure_retained_static_texture(
    renderer: &WgpuRenderer,
    render: &mut GuiFrameRenderState,
    size: SnapshotSize,
) {
    let needs_new = match &render.compositor.retained_static {
        Some(rs) => rs.texture.size() != size,
        None => true,
    };
    if !needs_new {
        return;
    }
    let texture = FullFrameTexture::allocate(
        renderer.device(),
        UnpooledTexture::RetainedStaticScene,
        "retained-static-scene",
        size,
        renderer.surface_format(),
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let bind_group = renderer.create_texture_bind_group(texture.view());
    render.compositor.retained_static = Some(RetainedStatic::new(texture, bind_group));
}

// Inactive parameters do not affect body pixels. In particular, startup's
// Lisp gallery rounds colors through sRGB strings even for disabled effects.
// Check activation rather than comparing complete configuration snapshots.
pub(super) fn static_body_effects(effects: &romacs_display_protocol::EffectsConfig) -> bool {
    !effects.has_enabled_effects_other_than_cursor_color_cycle()
        && effects.bg_pattern.style == 0
        && effects.mode_line_separator.style == 0
}

#[cfg(test)]
mod tests;
