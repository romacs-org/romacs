//! The scene graph's GPU pass: draws a [`draw::DrawList`] into textures.
//!
//! Every target of the list renders in list order (the list's dependency
//! order, so a target group is complete before anything samples it) into a
//! 4× multisampled attachment, resolved once into the target's sRGB texture.
//! Within a target, items draw in paint order through five pipelines over the
//! renderer's existing vertex layouts:
//!
//! - rects: SDF terms on [`RoundedRectVertex`](crate::vertex::RoundedRectVertex)
//!   (shadow, glow, fill and stroke of one distance field);
//! - paths: lyon triangles on [`RectVertex`](crate::vertex::RectVertex), cached
//!   per node revision;
//! - text and cells: the frame's glyph atlas on
//!   [`GlyphVertex`](crate::vertex::GlyphVertex);
//! - samples of earlier targets, images and shader surfaces through the
//!   same rounded-rect distance.
//!
//! The output textures hold linear premultiplied colour; a compositor samples
//! them as premultiplied. Timestamps bracket every target's pass when the
//! device has timestamp queries.

mod batch;
mod color;
mod paths;
mod pipelines;
mod rects;
mod targets;
mod text;
mod timing;

#[cfg(test)]
#[path = "tests/fixtures_test.rs"]
mod fixtures_test;

pub use romacs_scene::draw;
pub use targets::{SAMPLE_COUNT, TARGET_FORMAT};
pub use text::{FontLookup, SceneFont};
pub use timing::TargetTiming;

use crate::glyph_atlas::WgpuGlyphAtlas;
use crate::glyph_atlas::types::GlyphMaterialKind;
use crate::image_cache::ImageCache;
use crate::shader_surface_cache::ShaderSurfaceCache;
use batch::{Batch, Pipe, Scissor};
use draw::{Item, NodeId, Paint, Source, Target, TargetId};
use rustc_hash::{FxHashMap, FxHashSet};
use wgpu::util::DeviceExt;

/// Why the pass refused a draw list; nothing was drawn.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SceneError {
    #[error("scene draw list has no targets")]
    Empty,
    #[error("scene target {} has unusable size {size:?}", target.0)]
    TargetSize { target: TargetId, size: [u32; 2] },
    #[error("scene node {node} in target {} samples target {}, which is not earlier in the list", target.0, sampled.0)]
    SampleOrder {
        target: TargetId,
        node: NodeId,
        sampled: TargetId,
    },
    #[error("scene target {} appears twice", .0.0)]
    DuplicateTarget(TargetId),
}

/// An item the pass skipped, and why; the rest of the list still draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skipped {
    UnresolvedFont(NodeId),
    MissingSource(NodeId),
}

/// What one render did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneStats {
    pub draws: u32,
    pub vertices: u32,
    /// Path tessellations run (cache misses).
    pub tessellations: u64,
    pub skipped: Vec<Skipped>,
}

/// Textures a sample may name besides the list's own targets.
pub trait SceneSources {
    fn view(&self, source: Source) -> Option<&wgpu::TextureView>;
}

/// No external sources.
impl SceneSources for () {
    fn view(&self, _: Source) -> Option<&wgpu::TextureView> {
        None
    }
}

/// The renderer's image and shader-surface caches (a model's picture is a
/// shader surface too).
#[derive(Clone, Copy, Default)]
pub struct CacheSources<'a> {
    pub images: Option<&'a ImageCache>,
    pub surfaces: Option<&'a ShaderSurfaceCache>,
}

impl SceneSources for CacheSources<'_> {
    fn view(&self, source: Source) -> Option<&wgpu::TextureView> {
        match source {
            Source::Image(id) => Some(
                &self
                    .images?
                    .get(romacs_display_protocol::ImageId::new(id))?
                    .view,
            ),
            Source::Surface(id) => Some(&self.surfaces?.get(id)?.view),
            Source::Target(_) => None,
        }
    }
}

/// Everything a render reads besides the list.
pub struct SceneInputs<'a> {
    /// At the pass's device scale; its pages must stay pinned until the
    /// encoder is submitted, as for the frame's own glyphs.
    pub atlas: &'a mut WgpuGlyphAtlas,
    pub fonts: &'a FontLookup<'a>,
    pub sources: &'a dyn SceneSources,
}

/// Draws one placement's lists. Target ids are scene node ids and a render
/// releases the targets its list does not name, so each placement keeps its
/// own pass.
pub struct ScenePass {
    pipelines: pipelines::Pipelines,
    paths: paths::PathCache,
    targets: targets::TargetPool,
    timer: Option<timing::SceneTimer>,
    max_dimension: u32,
}

impl ScenePass {
    /// `atlas_layout` is the glyph atlas's bind group layout, shared by every
    /// sampled texture.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        Self {
            pipelines: pipelines::Pipelines::new(device, atlas_layout),
            paths: paths::PathCache::default(),
            targets: targets::TargetPool::default(),
            timer: timing::SceneTimer::new(device, queue),
            max_dimension: device.limits().max_texture_dimension_2d,
        }
    }

    /// Record `list` at device `scale` into `encoder`. Targets missing from
    /// `list` are released.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        list: &draw::DrawList,
        scale: f32,
        inputs: SceneInputs<'_>,
    ) -> Result<SceneStats, SceneError> {
        let scale = f64::from(scale);
        let sizes = targets::validate(list, scale, self.max_dimension)?;
        let mut stats = SceneStats::default();
        let tessellations = self.paths.tessellations;
        let timed = self
            .timer
            .as_ref()
            .is_some_and(|t| t.begin(list.targets.len()));
        let mut glyphs = text::GlyphContext {
            atlas: inputs.atlas,
            device,
            queue,
            scale,
        };
        for (index, (target, size)) in list.targets.iter().zip(sizes).enumerate() {
            let batch = build(
                target,
                size,
                scale,
                &mut glyphs,
                inputs.fonts,
                inputs.sources,
                &mut self.paths,
                &mut stats,
            );
            self.targets
                .ensure(device, &self.pipelines.uniform_layout, target.id, size);
            let timestamps = self
                .timer
                .as_ref()
                .filter(|_| timed)
                .map(|t| t.writes(index));
            encode(
                device,
                queue,
                encoder,
                &self.pipelines,
                &self.targets,
                glyphs.atlas,
                inputs.sources,
                target,
                &batch,
                timestamps,
            );
            stats.draws += batch.cmds.len() as u32;
            stats.vertices += (batch.rects.len() + batch.fills.len() + batch.glyphs.len()) as u32;
        }
        let live: FxHashSet<TargetId> = list.targets.iter().map(|t| t.id).collect();
        self.targets.retain(&live);
        self.paths.sweep();
        if let (true, Some(timer)) = (timed, self.timer.as_ref()) {
            timer.resolve(encoder, list.targets.iter().map(|t| t.id).collect());
        }
        stats.tessellations = self.paths.tessellations - tessellations;
        Ok(stats)
    }

    /// Call once the encoder given to [`Self::render`] has been submitted.
    pub fn submitted(&self) {
        if let Some(timer) = &self.timer {
            timer.submitted();
        }
    }

    /// GPU time per target of the latest timed render, once it is readable
    /// (after the device has been polled past it).
    pub fn take_timings(&self) -> Option<Vec<TargetTiming>> {
        self.timer.as_ref()?.take()
    }

    pub fn timing_available(&self) -> bool {
        self.timer.is_some()
    }

    pub fn view(&self, id: TargetId) -> Option<&wgpu::TextureView> {
        Some(&self.targets.get(id)?.view)
    }

    pub fn texture(&self, id: TargetId) -> Option<&wgpu::Texture> {
        Some(&self.targets.get(id)?.texture)
    }

    /// GPU bytes the pass holds in target textures.
    pub fn resident_bytes(&self) -> u64 {
        self.targets.resident_bytes()
    }
}

/// One target's geometry, in paint order.
#[allow(clippy::too_many_arguments)]
fn build(
    target: &Target,
    size: [u32; 2],
    scale: f64,
    glyphs: &mut text::GlyphContext<'_>,
    fonts: &FontLookup<'_>,
    sources: &dyn SceneSources,
    paths: &mut paths::PathCache,
    stats: &mut SceneStats,
) -> Batch {
    let mut batch = Batch::default();
    for item in &target.items {
        if item.opacity <= 0.0 {
            continue;
        }
        let Some(scissor) = scissor(item, size, scale) else {
            continue;
        };
        let (transform, opacity) = (item.transform, item.opacity);
        match &item.paint {
            Paint::Rect(paint) => {
                let quads = rects::rect_quads(paint, transform, opacity, scale);
                batch.push_rects(Pipe::Rect, scissor, quads);
            }
            Paint::Path(paint) => {
                let mesh = paths.mesh(item.node, paint, transform, scale);
                let start = batch.fill_indices.len() as u32;
                paths::emit(
                    &mesh,
                    paint,
                    transform,
                    opacity,
                    scale,
                    &mut batch.fills,
                    &mut batch.fill_indices,
                );
                batch.push_fill(scissor, start);
            }
            Paint::Text(paint) => match fonts(&paint.font) {
                Some(font) => text::emit_text(
                    glyphs, &font, paint, transform, opacity, scissor, &mut batch,
                ),
                None => stats.skipped.push(Skipped::UnresolvedFont(item.node)),
            },
            Paint::Cells(paint) => match fonts(&paint.font) {
                Some(font) => text::emit_cells(
                    glyphs, &font, paint, transform, opacity, scissor, &mut batch,
                ),
                None => stats.skipped.push(Skipped::UnresolvedFont(item.node)),
            },
            Paint::Sample(paint) => {
                let present = matches!(paint.source, Source::Target(_))
                    || sources.view(paint.source).is_some();
                if !present {
                    stats.skipped.push(Skipped::MissingSource(item.node));
                    continue;
                }
                let straight = !matches!(paint.source, Source::Target(_));
                let quad = rects::sample_quad(
                    paint.rect,
                    paint.radius,
                    straight,
                    transform,
                    opacity,
                    scale,
                );
                batch.push_rects(Pipe::Sample(paint.source), scissor, quad);
            }
        }
    }
    batch
}

/// The item's clip in device pixels within the target; `None` when empty.
fn scissor(item: &Item, size: [u32; 2], scale: f64) -> Option<Scissor> {
    let full = kurbo::Rect::new(0.0, 0.0, f64::from(size[0]), f64::from(size[1]));
    let clip = match item.clip {
        Some(clip) => kurbo::Affine::scale(scale)
            .transform_rect_bbox(clip)
            .intersect(full),
        None => full,
    };
    let (x0, y0) = (clip.x0.floor().max(0.0), clip.y0.floor().max(0.0));
    let (x1, y1) = (clip.x1.ceil().min(full.x1), clip.y1.ceil().min(full.y1));
    (x1 > x0 && y1 > y0).then_some([x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32])
}

/// Record one target's render pass.
#[allow(clippy::too_many_arguments)]
fn encode(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &pipelines::Pipelines,
    pool: &targets::TargetPool,
    atlas: &WgpuGlyphAtlas,
    sources: &dyn SceneSources,
    target: &Target,
    batch: &Batch,
    timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
) {
    let Some(texture) = pool.get(target.id) else {
        return;
    };
    let uniforms = crate::vertex::Uniforms {
        screen_size: [texture.size[0] as f32, texture.size[1] as f32],
        time: 0.0,
        content_alpha: 1.0,
        content_scale: 1.0,
        _pivot_padding: 0.0,
        content_pivot: [0.0, 0.0],
    };
    queue.write_buffer(&texture.uniforms, 0, bytemuck::bytes_of(&uniforms));
    let buffer = |label, contents: &[u8], usage| {
        (!contents.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        })
    };
    let vertex = wgpu::BufferUsages::VERTEX;
    let rects = buffer("scene rects", bytemuck::cast_slice(&batch.rects), vertex);
    let fills = buffer("scene fills", bytemuck::cast_slice(&batch.fills), vertex);
    let fill_indices = buffer(
        "scene fill indices",
        bytemuck::cast_slice(&batch.fill_indices),
        wgpu::BufferUsages::INDEX,
    );
    let glyph_vertices = buffer("scene glyphs", bytemuck::cast_slice(&batch.glyphs), vertex);
    let samples = sample_bind_groups(device, pipelines, pool, sources, batch);

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("scene target"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &texture.msaa_view,
            resolve_target: Some(&texture.view),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color::clear_color(target.clear)),
                store: wgpu::StoreOp::Discard,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: timestamps,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_bind_group(0, &texture.uniform_bind_group, &[]);
    for cmd in &batch.cmds {
        let [x, y, w, h] = cmd.scissor;
        pass.set_scissor_rect(x, y, w, h);
        match cmd.pipe {
            Pipe::Rect => {
                let Some(rects) = &rects else { continue };
                pass.set_pipeline(&pipelines.rect);
                pass.set_vertex_buffer(0, rects.slice(..));
                pass.draw(cmd.range.clone(), 0..1);
            }
            Pipe::Sample(source) => {
                let (Some(rects), Some(group)) = (&rects, samples.get(&source)) else {
                    continue;
                };
                pass.set_pipeline(&pipelines.sample);
                pass.set_bind_group(1, group, &[]);
                pass.set_vertex_buffer(0, rects.slice(..));
                pass.draw(cmd.range.clone(), 0..1);
            }
            Pipe::Fill => {
                let (Some(fills), Some(indices)) = (&fills, &fill_indices) else {
                    continue;
                };
                pass.set_pipeline(&pipelines.fill);
                pass.set_vertex_buffer(0, fills.slice(..));
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(cmd.range.clone(), 0, 0..1);
            }
            Pipe::Glyph(entry) => {
                let (Some(glyph_vertices), Ok(group)) =
                    (&glyph_vertices, atlas.atlas_bind_group(entry))
                else {
                    continue;
                };
                let pipeline = match entry.material_kind() {
                    GlyphMaterialKind::ColorRgba => &pipelines.glyph_color,
                    GlyphMaterialKind::AlphaMask | GlyphMaterialKind::SubpixelMask => {
                        &pipelines.glyph_mask
                    }
                };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(1, group, &[]);
                pass.set_vertex_buffer(0, glyph_vertices.slice(..));
                pass.draw(cmd.range.clone(), 0..1);
            }
        }
    }
}

fn sample_bind_groups(
    device: &wgpu::Device,
    pipelines: &pipelines::Pipelines,
    pool: &targets::TargetPool,
    sources: &dyn SceneSources,
    batch: &Batch,
) -> FxHashMap<Source, wgpu::BindGroup> {
    let mut groups = FxHashMap::default();
    for cmd in &batch.cmds {
        let Pipe::Sample(source) = cmd.pipe else {
            continue;
        };
        if groups.contains_key(&source) {
            continue;
        }
        let view = match source {
            Source::Target(id) => pool.get(id).map(|t| &t.view),
            other => sources.view(other),
        };
        if let Some(view) = view {
            groups.insert(source, pipelines.texture_bind_group(device, view));
        }
    }
    groups
}
