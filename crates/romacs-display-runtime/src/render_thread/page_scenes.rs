//! Page scenes: a scene the host draws over a block of a window's text in
//! place of its cells (graphics design §2.4.3), the first placements of the
//! scene graph on screen.  The block is a text animation's
//! (romacs_display_protocol::text_animation); its kind names the scene, so
//! Lisp changes nothing to get it, and the cells it laid out stay in the
//! buffer as its text and are what a display without a GPU draws.
//!
//! Each frame the render thread keeps one scene per animation: it builds it
//! from the block's geometry, advances its document clock to the
//! animation's motion time (so a pause, a hidden window or a lost focus
//! stops it, never catching up), sends the shader nodes' uniforms to their
//! shader surfaces, lowers the scene and draws the placement into a surface
//! the frame shows as one glyph over the blanked block.  Nothing runs in
//! the evaluator.  A scene whose state did not change draws nothing.

pub(crate) mod landing;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use kurbo::{Rect, Size};
use romacs_display_protocol::frame_glyphs::{FrameGlyph, FrameGlyphBuffer, GlyphRowRole};
use romacs_display_protocol::text_animation::{TextAnimation, TextAnimationKind};
use romacs_display_protocol::types::{FaceId, SurfaceId};
use romacs_renderer_wgpu::shader_surface::{SurfaceChannelSource, SurfaceShaderLanguage, SurfaceUniformInit};
use romacs_renderer_wgpu::{WgpuGlyphAtlas, WgpuRenderer};
use romacs_scene::draw::{DrawList, Source};
use romacs_scene::layout::{self, Measure, Measured};
use romacs_scene::lower::{self, Resolve};
use romacs_scene::motion::Policy;
use romacs_scene::node::{Font, ImageSource, NodeId, SceneId, TextNode};
use romacs_scene::prop::{Prop, Value};
use romacs_scene::scene::Scene;

/// Surfaces page scenes own come from here up, far above the ids the
/// evaluator and the model store hand out.
const FIRST_SURFACE: u32 = 0xF000_0000;

fn next_surface() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(FIRST_SURFACE);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Where a text animation's block lies in its frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Block {
    /// The block's top left in frame coordinates (above the window when it
    /// is scrolled past its first rows).
    pub origin: kurbo::Point,
    pub cell: Size,
    pub clip: Option<romacs_display_protocol::types::Rect>,
    pub window: romacs_display_protocol::types::DisplayWindowId,
    pub face: FaceId,
}

/// ANIMATION's block in FRAME: placed from one of its text glyphs, as
/// `text_animation::paint` places its cells.
pub(crate) fn block(frame: &FrameGlyphBuffer, animation: &TextAnimation) -> Option<Block> {
    let in_window = |glyph: &&FrameGlyph| matches!(glyph,
        FrameGlyph::Char { slot_id, row_role: GlyphRowRole::Text, composed: None, .. }
            if slot_id.window_id.get() as u64 == animation.window);
    let first = frame.glyphs.iter().filter(in_window).fold(None, |origin: Option<(u32, u16)>, glyph| {
        let FrameGlyph::Char { slot_id, .. } = glyph else { return origin };
        Some(origin.map_or((slot_id.row, slot_id.col), |(r, c)| (r.min(slot_id.row), c.min(slot_id.col))))
    })?;
    // A glyph inside the block: its row and column fix the block's corner.
    frame.glyphs.iter().filter(in_window).find_map(|glyph| {
        let FrameGlyph::Char { slot_id, x, y, height, clip_rect, face_id, window_id, .. } = glyph else { return None };
        let row = i64::from(slot_id.row) - i64::from(first.0) - i64::from(animation.top);
        let col = i64::from(slot_id.col) - i64::from(first.1);
        ((0..i64::from(animation.rows)).contains(&row) && (0..i64::from(animation.cols)).contains(&col)).then(|| {
            let cell = Size::new(f64::from(frame.char_width), f64::from(*height));
            Block {
                origin: kurbo::Point::new(f64::from(*x) - col as f64 * cell.width, f64::from(*y) - row as f64 * cell.height),
                cell,
                clip: *clip_rect,
                window: *window_id,
                face: *face_id,
            }
        })
    })
}

/// What a scene was built for: when it changes, the scene starts over.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Built {
    geometry: landing::Geometry,
    policy: Policy,
    epoch: u64,
}

impl Built {
    /// Whether NEXT continues this one's motion: a new size redraws the
    /// same moment, a new policy or epoch starts over.
    fn continued_by(&self, next: &Self) -> bool {
        (self.policy, self.epoch) == (next.policy, next.epoch)
    }
}

/// The surfaces one landing placement draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Surfaces {
    field: u32,
    mark: u32,
    placement: u32,
}

/// One animation's scene and what it last drew.
pub(crate) struct PageScene {
    surfaces: Surfaces,
    built: Option<Built>,
    scene: Scene,
    /// The mark's uniforms as its surface has them.
    uniforms: Option<[f32; landing::UNIFORMS]>,
    /// The mark surface's logical size, once created.
    mark_size: Option<(u32, u32)>,
    /// The list lowered this frame, to draw after the shader surfaces.
    pending: Option<DrawList>,
    /// The list the placement surface shows.
    drawn: Option<DrawList>,
    /// The placement in block coordinates, once it has drawn.
    pub(crate) shown: Option<Rect>,
    /// The GPU refused it: the animation is drawn in cells from now on.
    pub(crate) failed: bool,
}

impl PageScene {
    pub(crate) fn new() -> Self {
        Self {
            surfaces: Surfaces { field: next_surface(), mark: next_surface(), placement: next_surface() },
            built: None,
            scene: Scene::new(SceneId(0), 0),
            uniforms: None,
            mark_size: None,
            pending: None,
            drawn: None,
            shown: None,
            failed: false,
        }
    }

    /// Free its surfaces.
    pub(crate) fn free(self, renderer: &mut WgpuRenderer) {
        renderer.free_surface(self.surfaces.field);
        renderer.free_surface(self.surfaces.mark);
        renderer.free_scene_placement(self.surfaces.placement);
    }

    /// Bring the scene to motion time MOVED for ANIMATION's block in FRAME
    /// and its shader surfaces up to date; the list to draw once they have
    /// rendered is kept for [`Self::draw`].
    pub(crate) fn prepare(&mut self, frame: &FrameGlyphBuffer, animation: &TextAnimation, moved: f64,
                          renderer: &mut WgpuRenderer) {
        self.pending = None;
        let TextAnimationKind::Logo { epoch, .. } = animation.kind;
        let Some(block) = block(frame, animation) else { self.shown = None; return };
        // Laid out in the cells as drawn, which may be taller than the
        // frame's character height Lisp sized the braille by.
        let cell_aspect = block.cell.width / block.cell.height;
        let view = romacs_logo::View { cols: animation.cols as usize, rows: animation.rows as usize, cell_aspect };
        let Some(geometry) = landing::geometry(view, block.cell) else { self.shown = None; return };
        // With motion off the scene shows its targets: the still frame.
        let policy = if animation.still { Policy::Reduced } else { Policy::On };
        let built = Built { geometry, policy, epoch };
        match self.built {
            Some(b) if b == built => {}
            // New geometry: the same scene, refitted by a
            // diff, so its motions run on and its nodes' revisions move.
            Some(b) if b.continued_by(&built) => {
                match self.scene.apply(&landing::refit(&geometry, self.scene.rev()), policy) {
                    Ok(scene) => self.scene = scene,
                    Err(reject) => {
                        tracing::warn!("landing scene refit rejected: {reject:?}");
                        self.failed = true;
                        return;
                    }
                }
                self.built = Some(built);
            }
            // A new start: a new scene, drawn by a new pass.
            _ => {
                let fresh = Scene::new(SceneId(1), 1).tick(seconds(moved), true);
                match fresh.apply(&landing::scene(&geometry, policy), policy) {
                    Ok(scene) => self.scene = scene,
                    Err(reject) => {
                        tracing::warn!("landing scene rejected: {reject:?}");
                        self.failed = true;
                        return;
                    }
                }
                renderer.free_scene_placement(self.surfaces.placement);
                self.built = Some(built);
                self.drawn = None;
            }
        }
        self.scene = self.scene.tick(seconds(moved), true);
        if !self.ensure_surfaces(&geometry, renderer) {
            return;
        }
        let uniforms = self.mark_uniforms();
        if self.uniforms != Some(uniforms) {
            for (i, value) in uniforms.iter().enumerate() {
                if self.uniforms.is_none_or(|u| u[i] != *value) {
                    renderer.set_surface_uniform(self.surfaces.mark, &format!("u{i}"), [*value, 0.0, 0.0, 0.0]);
                }
            }
        }
        let solved = layout::solve(&self.scene, geometry.placement.size(), &BTreeMap::new(), &mut NoText);
        let list = lower::lower(&self.scene, &solved, &mut Marks(self.surfaces.mark));
        if self.uniforms != Some(uniforms) || self.drawn.as_ref() != Some(&list) {
            self.pending = Some(list);
        }
        self.uniforms = Some(uniforms);
        self.shown = self.drawn.as_ref().or(self.pending.as_ref()).map(|_| geometry.placement);
    }

    /// Draw the list [`Self::prepare`] kept, now that its shader surfaces
    /// have rendered.
    pub(crate) fn draw(&mut self, renderer: &mut WgpuRenderer, atlas: &mut WgpuGlyphAtlas) {
        let Some(list) = self.pending.take() else { return };
        match renderer.render_scene_placement(self.surfaces.placement, &list, atlas) {
            Ok(_) => self.drawn = Some(list),
            Err(error) => {
                tracing::warn!("landing placement failed: {error}");
                self.failed = true;
                self.shown = None;
            }
        }
    }

    /// The glyph that shows the placement over BLOCK, once it has drawn.
    pub(crate) fn glyph(&self, block: &Block) -> Option<FrameGlyph> {
        let shown = self.shown.filter(|_| self.drawn.is_some() && !self.failed)?;
        Some(FrameGlyph::Surface {
            window_id: block.window,
            row_role: GlyphRowRole::Text,
            clip_rect: block.clip,
            slot_id: None,
            surface_id: SurfaceId::new(self.surfaces.placement),
            x: (block.origin.x + shown.x0) as f32,
            y: (block.origin.y + shown.y0) as f32,
            width: shown.width() as f32,
            height: shown.height() as f32,
            face_id: block.face,
            box_vertical_edges: Default::default(),
        })
    }

    /// Create the field and the mark surfaces, and keep the mark's size;
    /// whether the mark can draw.  A new field renders at once, so the mark
    /// created after it samples a finished field in this very frame: no
    /// later frame may come to finish it (a still landing asks for none).
    fn ensure_surfaces(&mut self, geometry: &landing::Geometry, renderer: &mut WgpuRenderer) -> bool {
        let Surfaces { field, mark, .. } = self.surfaces;
        if !renderer.has_surface(field) {
            self.mark_size = None;
            self.drawn = None;
            renderer.free_surface(mark);
            let (width, height) = landing::field_size();
            let created = renderer.create_shader_surface(field, SurfaceShaderLanguage::Wgsl, landing::field_wgsl(),
                                                          &[], width, height, false, None, None, false);
            if let Err(error) = created {
                tracing::warn!("landing field surface: {error}");
                self.failed = true;
                return false;
            }
            renderer.process_shader_surfaces();
        }
        let size = (geometry.mark.width().round() as u32, geometry.mark.height().round() as u32);
        if renderer.has_surface(mark) && self.mark_size.is_some() {
            if self.mark_size != Some(size) && !renderer.resize_shader_surface(mark, size.0, size.1) {
                self.mark_size = None;
                return false;
            }
            self.mark_size = Some(size);
            return true;
        }
        let uniforms: Vec<SurfaceUniformInit> = (0..landing::UNIFORMS)
            .map(|i| SurfaceUniformInit { name: format!("u{i}"), value: [0.0; 4], components: 1 })
            .collect();
        let created = renderer.create_shader_surface(mark, SurfaceShaderLanguage::Wgsl, landing::mark_wgsl(),
                                                      &uniforms, size.0, size.1, false, None,
                                                      Some(SurfaceChannelSource::Surface(field)), false);
        match created {
            Ok(()) => {
                self.mark_size = Some(size);
                self.uniforms = None;
                true
            }
            Err(error) => {
                tracing::warn!("landing mark surface: {error}");
                self.failed = true;
                false
            }
        }
    }

    fn mark_uniforms(&self) -> [f32; landing::UNIFORMS] {
        std::array::from_fn(|i| match self.scene.value(landing::MARK, Prop::Uniform(i as u8)) {
            Some(Value::Num(v)) => v,
            _ => 0.0,
        })
    }
}

fn seconds(moved: f64) -> Duration {
    Duration::from_secs_f64(moved.max(0.0))
}

/// Page scenes hold no text, so nothing measures.
struct NoText;

impl Measure for NoText {
    fn text(&mut self, _: &TextNode, _: Option<f64>) -> Measured {
        Measured::default()
    }

    fn cell(&mut self, _: &Font) -> Size {
        Size::ZERO
    }

    fn image(&mut self, _: &ImageSource) -> Option<Size> {
        None
    }
}

/// The landing's one shader node draws into the mark surface.
struct Marks(u32);

impl Resolve for Marks {
    fn image(&mut self, _: SceneId, _: NodeId, _: &ImageSource) -> Option<(Source, Size)> {
        None
    }

    fn surface(&mut self, _: SceneId, node: NodeId) -> Option<u32> {
        (node == landing::MARK).then_some(self.0)
    }
}

impl super::RenderApp {
    /// Before shader surfaces render: free the scenes of animations that
    /// ended, and bring every GPU-drawn animation's scene of the primary
    /// window to its motion time.
    pub(super) fn prepare_page_scenes(&mut self) {
        let gpu = self.render_policy.allows_dynamic_effects();
        let (Some(renderer), Some(window)) = (self.renderer.as_mut(), self.frame_windows.primary_window_mut()) else {
            return;
        };
        let compositor = &mut window.render.compositor;
        for scene in compositor.text_animations.take_retired() {
            scene.free(renderer);
        }
        if let Some(frame) = compositor.current_frame.as_ref() {
            compositor.text_animations.prepare_scenes(frame, gpu, renderer);
        }
    }

    /// After shader surfaces rendered: draw the placements they feed.
    pub(super) fn render_page_scenes(&mut self) {
        let (Some(renderer), Some(window)) = (self.renderer.as_mut(), self.frame_windows.primary_window_mut()) else {
            return;
        };
        let compositor = &mut window.render.compositor;
        if let Some(atlas) = compositor.glyph_atlas.as_mut() {
            compositor.text_animations.draw_scenes(renderer, atlas);
        }
    }
}

#[cfg(test)]
#[path = "page_scenes/tests/page_scenes_test.rs"]
mod tests;
