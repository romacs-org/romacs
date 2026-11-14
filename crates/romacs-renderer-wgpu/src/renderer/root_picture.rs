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
use romacs_display_protocol::frame_glyphs::{FrameGlyph, FrameGlyphBuffer};
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
    pictured: bool,
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
    pub fn texture(&self) -> &FullFrameTexture {
        &self.texture
    }

    /// Whether it can picture a surface of SIZE (in its renderer's format,
    /// which is the renderer's for its life).
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
        let scale = renderer.scale_factor();
        let (scale_bits, subpixel) = (scale.to_bits(), atlas.subpixel_enabled());
        let surface = mapping.surface();
        let device = (surface.device_width().get(), surface.device_height().get());
        let shown = self
            .shown
            .take()
            .filter(|s| s.scale_bits == scale_bits && s.subpixel == subpixel);
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
        let keep = |frame: &FrameGlyphBuffer, runs: Vec<Run>, pictured| Shown {
            indexed: Indexed {
                frame: frame.glyph_pass_copy(),
                runs,
            },
            scale_bits,
            subpixel,
            pictured,
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
                self.shown = Some(keep(frame, runs, false));
                return Painted::Direct;
            }
            // Repeating, wholly or in part, a frame the picture does not
            // show: picture this one.
            (Plan::Clean { .. }, Some(mut shown)) if !shown.pictured => {
                adopt_presentation(&mut shown.indexed.frame, frame);
                (self.repaint(renderer, atlas, shown, mapping), Painted::Whole)
            }
            (Plan::Regions { .. }, Some(shown)) if !shown.pictured => (
                self.repaint(renderer, atlas, keep(frame, runs, true), mapping),
                Painted::Whole,
            ),
            (Plan::Clean { rows }, Some(mut shown)) => {
                adopt_presentation(&mut shown.indexed.frame, frame);
                crate::cost::add(crate::cost::ROWS_REUSED, rows);
                (shown, Painted::Clean)
            }
            (Plan::Regions { regions, rows_kept }, Some(_)) => {
                let mut shown = keep(frame, runs, true);
                let picture = self.texture.view().clone();
                self.reach = patched(self.reach, |reach| {
                    for (region, scissor) in pixel_regions(&regions, reach, scale, device) {
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
                crate::cost::add(crate::cost::ROWS_REUSED, rows_kept);
                (shown, Painted::Regions(regions.len()))
            }
            (_, None) => unreachable!("only a whole repaint is planned without a shown frame"),
        };
        self.compose(renderer, atlas, &mut shown, target);
        self.shown = Some(shown);
        painted
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
        let frame = &shown.indexed.frame;
        // The glyphs that place each cursor (`FrameGlyphBuffer::cursor_anchor`)
        // go into every cursor's patch, so it places the cursors as the frame does.
        let anchors: Vec<usize> = frame
            .window_cursors
            .iter()
            .filter_map(|c| frame.cursor_anchor(c.slot_id))
            .collect();
        let extents = cursor_regions(
            frame,
            renderer.cursor_extents(frame, cursor_visible, animated_cursor.as_ref()),
        );
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

/// What each cursor of FRAME, drawing in EXTENTS, changes of the cursorless
/// picture: its extent, and every glyph in its slot, which the pass draws in
/// the cursor's inverse colours wherever it is (a row set shorter than the
/// frame's cell can give two glyphs one slot).
fn cursor_regions(frame: &FrameGlyphBuffer, extents: Vec<Option<Rect>>) -> Vec<Rect> {
    frame
        .window_cursors
        .iter()
        .zip(extents)
        .filter_map(|(cursor, extent)| {
            let in_slot = frame
                .glyphs
                .iter()
                .filter(|g| g.slot_id() == Some(cursor.slot_id))
                .filter_map(FrameGlyph::geometry);
            extent.map(|extent| in_slot.fold(extent, damage::union))
        })
        .collect()
}

/// What a patch of REGION draws: the frame's background over it, then every
/// glyph of each group of SHOWN whose bounds grown by REACH touch it, and
/// ANCHORS, in frame order.
fn region_glyphs(shown: &Indexed, region: Rect, reach: Reach, anchors: &[usize]) -> Vec<FrameGlyph> {
    let frame = &shown.frame;
    let mut keys: Vec<_> = shown
        .runs
        .iter()
        .filter(|run| damage::overlaps(reach.grow(run.bounds), region))
        .map(|run| run.key)
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let mut glyphs = vec![FrameGlyph::Background {
        bounds: region,
        color: frame.background,
    }];
    for run in &shown.runs {
        if keys.binary_search(&run.key).is_ok() {
            glyphs.extend_from_slice(&frame.glyphs[run.glyphs.clone()]);
        } else {
            glyphs.extend(
                run.glyphs
                    .clone()
                    .filter(|at| anchors.contains(at))
                    .map(|at| frame.glyphs[at].clone()),
            );
        }
    }
    glyphs
}

/// A physical scissor (x, y, width, height) and the logical region it
/// covers: EXTENT grown to whole device pixels at SCALE, within DEVICE.
/// None when nothing of it is on the surface.
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
