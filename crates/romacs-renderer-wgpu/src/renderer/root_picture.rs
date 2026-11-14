//! The root picture: the root glyph pass's pixels without any cursor, kept
//! in a texture across frames, so a frame costs the rows that changed.
//!
//! A frame is painted through it in three steps (`RootPicture::paint`):
//!
//! 1. Bring the picture up to date. `damage::plan` compares the frame with
//!    the one the picture shows: nothing to do, regions to repaint, or all
//!    of it. A region is repainted by the glyph pass itself, run over the
//!    frame's background as a rectangle and then every glyph of the groups
//!    (a window's row, its background) whose bounds touch the region, onto
//!    the picture loaded and scissored to the region: inside it, the same
//!    primitives in the same order as a whole repaint draws there, box runs
//!    and underline runs whole.
//! 2. Copy the picture to the target.
//! 3. Draw each cursor as a patch of the target, the same way: its extent
//!    (`WgpuRenderer::cursor_extents`, the box of everything its layers draw
//!    this frame, and every glyph in its slot) repainted with the cursor on.
//!    So a move of point, a blink and a glide frame change no row, and every
//!    cursor style takes one path.
//!
//! Ink can leave its cell (a mark above a tightly set row, a descender, a
//! wave underline, an overhang): every patch grows by the farthest reach the
//! picture's passes have drawn (`reach`), and takes each group whose bounds
//! so grown touch it. A patch whose glyphs reached farther than it allowed
//! is drawn again under the new reach: a patch is idempotent, so the second
//! covers the first.
//!
//! A frame that needs a whole repaint right after one that did (scrolling,
//! an animation the layout does not hold still) is drawn straight onto the
//! target, as without a picture; the picture is repainted whole only once a
//! frame repeats what the last showed.
//!
//! The picture holds pixels, not texture coordinates: an atlas eviction or
//! reset cannot stale it. It is valid for one device scale and raster
//! policy, is recreated with its surface's size and format, and is dropped
//! with its device.
use std::collections::HashMap;

use romacs_display_protocol::frame_glyphs::{DisplaySlotId, FrameGlyph, FrameGlyphBuffer};
use romacs_display_protocol::types::{AnimatedCursor, Rect};

use super::super::glyph_atlas::WgpuGlyphAtlas;
use super::{FullFrameTexture, RenderTarget, SnapshotSize, UnpooledTexture, WgpuRenderer};

mod damage;
pub(super) mod reach;
use damage::{Indexed, Plan, Run};
use reach::Reach;

/// How a frame was painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Painted {
    /// Through the picture, which already showed it.
    Clean,
    /// Through the picture, this many regions of it repainted.
    Regions(usize),
    /// Through the picture, repainted whole.
    Whole,
    /// Straight onto the target: it needed a whole repaint, as the frame
    /// before did.
    Direct,
}

/// Where a frame's root pass is painted and how its cursors show: the
/// target view and its mapping (its surface is the picture's size), whether
/// the cursors are on, and where a gliding cursor is this frame.
#[derive(Clone, Copy)]
pub struct PaintTarget<'a> {
    pub view: &'a wgpu::TextureView,
    pub mapping: romacs_display_protocol::PresentMapping,
    pub cursor_visible: bool,
    pub animated_cursor: Option<AnimatedCursor>,
}

/// The root glyph pass's cursorless pixels, and the frame they show.
pub struct RootPicture {
    texture: FullFrameTexture,
    bind_group: wgpu::BindGroup,
    shown: Option<Shown>,
    /// The farthest ink the picture's passes have drawn past its cells.
    reach: Reach,
}

/// The last frame painted, indexed, what it was rasterized under, and
/// whether the picture shows it (a frame drawn directly is kept to be
/// compared with the next, which the picture then shows if it repeats).
struct Shown {
    indexed: Indexed,
    scale_bits: u32,
    subpixel: bool,
    /// The renderer's effect settings it was drawn under: a pass reads some
    /// even with every effect off (a scroll bar's track opacity).
    effects: crate::effect_config::EffectsConfig,
    pictured: bool,
    /// Per cursor slot: the glyph that places a cursor there and the box of
    /// the glyphs in it (`slot_facts`), found once per shown frame.
    slots: HashMap<DisplaySlotId, (Option<usize>, Option<Rect>)>,
}

impl RootPicture {
    /// A picture of SIZE device pixels in RENDERER's surface format,
    /// showing nothing yet.
    pub fn new(renderer: &WgpuRenderer, size: SnapshotSize) -> Self {
        let texture = FullFrameTexture::allocate(
            renderer.device(),
            UnpooledTexture::RetainedRootScene,
            "root-picture",
            size,
            renderer.surface_format(),
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let bind_group = renderer.create_texture_bind_group(texture.view());
        Self {
            texture,
            bind_group,
            shown: None,
            reach: Reach::default(),
        }
    }

    /// Its texture, for the GPU budget's census.
    #[must_use]
    pub fn texture(&self) -> &FullFrameTexture {
        &self.texture
    }

    /// Whether it can picture a surface of SIZE (in its renderer's format,
    /// which is the renderer's for its life).
    #[must_use]
    pub fn fits(&self, size: SnapshotSize) -> bool {
        self.texture.size() == size
    }

    /// Paint FRAME's root glyph pass onto TARGET: through the picture
    /// brought up to date, with the cursors drawn over it, or straight onto
    /// the target's view.
    pub fn paint(
        &mut self,
        renderer: &mut WgpuRenderer,
        atlas: &mut WgpuGlyphAtlas,
        frame: &FrameGlyphBuffer,
        target: PaintTarget<'_>,
    ) -> Painted {
        let PaintTarget {
            view,
            mapping,
            cursor_visible,
            animated_cursor,
        } = target;
        let (scale_bits, subpixel) = (renderer.scale_factor().to_bits(), atlas.subpixel_enabled());
        let shown = self
            .shown
            .take()
            .filter(|s| s.scale_bits == scale_bits && s.subpixel == subpixel && s.effects == renderer.effects);
        let repeats = shown
            .as_ref()
            .is_some_and(|s| damage::repeats(&s.indexed.frame, frame));
        let runs = if repeats {
            Vec::new()
        } else {
            damage::runs(&frame.glyphs)
        };
        let plan = match &shown {
            None => Plan::Whole,
            Some(s) if repeats => Plan::Clean {
                rows: damage::text_rows(&s.indexed, |_| true),
            },
            Some(s) => damage::plan(&s.indexed, frame, &runs),
        };
        let keep = |frame: &FrameGlyphBuffer, runs: Vec<Run>, pictured, effects| Shown {
            indexed: Indexed {
                frame: frame.glyph_pass_copy(),
                runs,
            },
            scale_bits,
            subpixel,
            effects,
            pictured,
            slots: HashMap::new(),
        };
        let (mut shown, painted) = match (plan, shown) {
            // A frame that needs a whole repaint is drawn as it comes: the
            // next frame may not repeat it (scrolling, a change of theme).
            (Plan::Whole, _) => {
                renderer.render_frame_glyphs(
                    view,
                    frame,
                    atlas,
                    mapping,
                    cursor_visible,
                    animated_cursor,
                    None,
                    None,
                    None,
                    None,
                );
                self.shown = Some(keep(frame, runs, false, renderer.effects.clone()));
                return Painted::Direct;
            }
            // Repeating, wholly or in part, a frame the picture does not
            // show: picture this one.
            (Plan::Clean { .. }, Some(mut shown)) if !shown.pictured => {
                adopt_presentation(&mut shown.indexed.frame, frame);
                (self.repaint(renderer, atlas, shown, mapping), Painted::Whole)
            }
            (Plan::Regions { .. }, Some(shown)) if !shown.pictured => (
                self.repaint(renderer, atlas, keep(frame, runs, true, renderer.effects.clone()), mapping),
                Painted::Whole,
            ),
            (Plan::Clean { rows }, Some(mut shown)) => {
                adopt_presentation(&mut shown.indexed.frame, frame);
                crate::cost::add(crate::cost::ROWS_REUSED, rows);
                (shown, Painted::Clean)
            }
            (Plan::Regions { regions, rows_kept }, Some(_)) => {
                let mut shown = keep(frame, runs, true, renderer.effects.clone());
                self.patch_regions(renderer, atlas, &mut shown, &regions, mapping);
                crate::cost::add(crate::cost::ROWS_REUSED, rows_kept);
                (shown, Painted::Regions(regions.len()))
            }
            (_, None) => unreachable!("only a whole repaint is planned without a shown frame"),
        };
        self.compose(renderer, atlas, &mut shown, target);
        self.shown = Some(shown);
        painted
    }

    /// Repaint REGIONS of the picture from SHOWN's frame, without its cursors.
    fn patch_regions(
        &mut self,
        renderer: &mut WgpuRenderer,
        atlas: &mut WgpuGlyphAtlas,
        shown: &mut Shown,
        regions: &[Rect],
        mapping: romacs_display_protocol::PresentMapping,
    ) {
        let surface = mapping.surface();
        let device = (surface.device_width().get(), surface.device_height().get());
        let scale = renderer.scale_factor();
        let picture = self.texture.view().clone();
        self.reach = patched(self.reach, |reach| {
            for (region, scissor) in pixel_regions(regions, reach, scale, device) {
                let glyphs = region_glyphs(&shown.indexed, region, reach, &[]);
                with_glyphs(&mut shown.indexed.frame, glyphs, false, |sub| {
                    renderer.render_frame_glyphs_impl(
                        &picture,
                        sub,
                        atlas,
                        mapping,
                        false,
                        None,
                        None,
                        None,
                        None,
                        None,
                        true,
                        Some(scissor),
                    );
                });
            }
        });
    }

    /// Repaint the picture whole from SHOWN's frame, without its cursors;
    /// SHOWN, pictured.
    fn repaint(
        &mut self,
        renderer: &mut WgpuRenderer,
        atlas: &mut WgpuGlyphAtlas,
        mut shown: Shown,
        mapping: romacs_display_protocol::PresentMapping,
    ) -> Shown {
        let picture = self.texture.view().clone();
        reach::take();
        let cursors = std::mem::take(&mut shown.indexed.frame.window_cursors);
        renderer.render_frame_glyphs(
            &picture,
            &shown.indexed.frame,
            atlas,
            mapping,
            false,
            None,
            None,
            None,
            None,
            None,
        );
        shown.indexed.frame.window_cursors = cursors;
        self.reach = reach::take();
        shown.pictured = true;
        shown
    }

    /// Copy the picture onto TARGET and draw SHOWN's cursors over it.
    fn compose(
        &mut self,
        renderer: &mut WgpuRenderer,
        atlas: &mut WgpuGlyphAtlas,
        shown: &mut Shown,
        target: PaintTarget<'_>,
    ) {
        let PaintTarget {
            view: target,
            mapping,
            cursor_visible,
            animated_cursor,
        } = target;
        let surface = mapping.surface();
        let device = (surface.device_width().get(), surface.device_height().get());
        let scale = renderer.scale_factor();
        renderer
            .begin_draw(RenderTarget::new(target, surface))
            .blit_retained(&self.bind_group);
        // Per cursor, the glyph that places it (`FrameGlyphBuffer::cursor_anchor`),
        // which goes into every cursor's patch so it places the cursors as the
        // frame does, and the glyphs in its slot, which its patch covers.
        let cursors = shown.indexed.frame.window_cursors.clone();
        let facts: Vec<(Option<usize>, Option<Rect>)> = cursors
            .iter()
            .map(|cursor| {
                *shown
                    .slots
                    .entry(cursor.slot_id)
                    .or_insert_with(|| slot_facts(&shown.indexed.frame, cursor.slot_id))
            })
            .collect();
        let anchors: Vec<usize> = facts.iter().filter_map(|fact| fact.0).collect();
        let extents: Vec<Rect> = renderer
            .cursor_extents(&shown.indexed.frame, cursor_visible, animated_cursor.as_ref())
            .into_iter()
            .zip(&facts)
            .filter_map(|(extent, fact)| extent.map(|extent| fact.1.map_or(extent, |slot| damage::union(extent, slot))))
            .collect();
        self.reach = patched(self.reach, |reach| {
            for &extent in &extents {
                let Some((region, scissor)) = pixel_region(reach.grow(extent), scale, device) else {
                    continue;
                };
                let glyphs = region_glyphs(&shown.indexed, region, reach, &anchors);
                with_glyphs(&mut shown.indexed.frame, glyphs, true, |sub| {
                    crate::cost::for_cursor(|| {
                        renderer.render_frame_glyphs_impl(
                            target,
                            sub,
                            atlas,
                            mapping,
                            cursor_visible,
                            animated_cursor,
                            None,
                            None,
                            None,
                            None,
                            true,
                            Some(scissor),
                        );
                    });
                });
            }
        });
    }
}

/// Draw patches with PATCH under REACH, and again under the farther reach
/// their glyphs showed, until it covers what they drew; the reach then.
fn patched(mut reach: Reach, mut patch: impl FnMut(Reach)) -> Reach {
    loop {
        reach::take();
        patch(reach);
        let seen = reach::take();
        if reach.covers(seen) {
            return reach;
        }
        reach = reach.max(seen);
    }
}

/// Run DRAW over FRAME holding GLYPHS in place of its own, and its cursors
/// unless CURSORS is false; FRAME as it was after. A patch borrows the
/// shown frame's tables this way instead of copying them.
fn with_glyphs(
    frame: &mut FrameGlyphBuffer,
    glyphs: Vec<FrameGlyph>,
    cursors: bool,
    draw: impl FnOnce(&FrameGlyphBuffer),
) {
    let all = std::mem::replace(&mut frame.glyphs, glyphs);
    let kept = if cursors {
        Vec::new()
    } else {
        std::mem::take(&mut frame.window_cursors)
    };
    draw(frame);
    frame.glyphs = all;
    if !cursors {
        frame.window_cursors = kept;
    }
}

/// Take into SHOWN, a frame whose glyphs, tables and setting FRAME repeats,
/// what FRAME holds of its own presentation: its cursors and its identity.
fn adopt_presentation(shown: &mut FrameGlyphBuffer, frame: &FrameGlyphBuffer) {
    shown.window_cursors.clone_from(&frame.window_cursors);
    shown.cursor_blinked_off = frame.cursor_blinked_off;
    shown.presentation_id = frame.presentation_id;
    shown.frame_placement = frame.frame_placement;
}

/// For a cursor on SLOT of FRAME: the glyph that places it, and the box of
/// every glyph in the slot, which the pass draws in the cursor's inverse
/// colours wherever it is (a row set shorter than the frame's cell can give
/// two glyphs one slot).
fn slot_facts(frame: &FrameGlyphBuffer, slot: DisplaySlotId) -> (Option<usize>, Option<Rect>) {
    let in_slot = frame
        .glyphs
        .iter()
        .filter(|g| g.slot_id() == Some(slot))
        .filter_map(FrameGlyph::geometry)
        .reduce(damage::union);
    (frame.cursor_anchor(slot), in_slot)
}

/// What a patch of REGION draws: the frame's background over it, then the
/// glyphs of every run of SHOWN whose group touches it (bounds grown by
/// REACH), with the whole visual rows they lie on (the text runs of their
/// window that overlap them, so underline and box runs that span runs of
/// different heights lay out as in the whole frame), and ANCHORS, in frame
/// order.
fn region_glyphs(shown: &Indexed, region: Rect, reach: Reach, anchors: &[usize]) -> Vec<FrameGlyph> {
    let (frame, runs) = (&shown.frame, &shown.runs);
    let mut keys: Vec<_> = runs
        .iter()
        .filter(|run| damage::overlaps(reach.grow(run.bounds), region))
        .map(|run| run.key)
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let mut picked: Vec<bool> = runs.iter().map(|run| keys.binary_search(&run.key).is_ok()).collect();
    // Close over rows: a text run of a picked text run's window that
    // overlaps it vertically is picked too, until nothing more is.
    let rows = |a: &damage::Run, b: &damage::Run| {
        a.text && b.text && a.key.0 == b.key.0 && a.bounds.y < b.bounds.y + b.bounds.height && b.bounds.y < a.bounds.y + a.bounds.height
    };
    loop {
        let more: Vec<usize> = (0..runs.len())
            .filter(|&j| !picked[j] && (0..runs.len()).any(|i| picked[i] && rows(&runs[i], &runs[j])))
            .collect();
        if more.is_empty() {
            break;
        }
        for j in more {
            picked[j] = true;
        }
    }
    let mut glyphs = vec![FrameGlyph::Background {
        bounds: region,
        color: frame.background,
    }];
    for (run, picked) in runs.iter().zip(picked) {
        if picked {
            glyphs.extend_from_slice(&frame.glyphs[run.glyphs.clone()]);
        } else {
            let mut held: Vec<usize> = anchors.iter().copied().filter(|at| run.glyphs.contains(at)).collect();
            held.sort_unstable();
            held.dedup();
            glyphs.extend(held.into_iter().map(|at| frame.glyphs[at].clone()));
        }
    }
    glyphs
}

/// A physical scissor (x, y, width, height) and the logical region it
/// covers: EXTENT grown to whole device pixels at SCALE, within DEVICE.
/// None when nothing of it is on the surface.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "device pixels: whole numbers clamped to the surface, far below f32's exact range"
)]
fn pixel_region(
    extent: Rect,
    scale: f32,
    (width, height): (u32, u32),
) -> Option<(Rect, (u32, u32, u32, u32))> {
    let x0 = (extent.x * scale).floor().max(0.0) as u32;
    let y0 = (extent.y * scale).floor().max(0.0) as u32;
    let x1 = ((extent.x + extent.width) * scale)
        .ceil()
        .clamp(0.0, width as f32) as u32;
    let y1 = ((extent.y + extent.height) * scale)
        .ceil()
        .clamp(0.0, height as f32) as u32;
    (x1 > x0 && y1 > y0).then(|| {
        let logical = Rect::new(
            x0 as f32 / scale,
            y0 as f32 / scale,
            (x1 - x0) as f32 / scale,
            (y1 - y0) as f32 / scale,
        );
        (logical, (x0, y0, x1 - x0, y1 - y0))
    })
}

/// REGIONS grown by REACH and to whole device pixels, merged where growing
/// them made them meet.
fn pixel_regions(
    regions: &[Rect],
    reach: Reach,
    scale: f32,
    device: (u32, u32),
) -> Vec<(Rect, (u32, u32, u32, u32))> {
    let grown: Vec<Rect> = regions
        .iter()
        .filter_map(|&r| pixel_region(reach.grow(r), scale, device))
        .map(|(r, _)| r)
        .collect();
    damage::merge(grown)
        .into_iter()
        .filter_map(|r| pixel_region(r, scale, device))
        .collect()
}

#[cfg(test)]
mod tests;
