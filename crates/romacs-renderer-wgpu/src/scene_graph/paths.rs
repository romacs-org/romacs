//! Path paints: lyon fill and stroke tessellation, cached per node revision.
//!
//! Tessellation runs in node-local coordinates, so moving or fading a path
//! (the common motion) reuses its triangles; only the tolerance depends on
//! the transform, and it is bucketed by powers of two so a zoom re-tessellates
//! a handful of times rather than every frame.

use super::color::linear_premultiplied;
use super::draw::{Cap, FillRule, Join, NodeId, PathPaint, PathStroke};
use crate::vertex::RectVertex;
use kurbo::{Affine, BezPath, PathEl};
use lyon::math::point;
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, LineCap, LineJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// Device pixels a flattened curve may deviate from the true one.
const TOLERANCE_PX: f64 = 0.1;

/// Renders a cached tessellation survives without being drawn.
const CACHE_TTL: u64 = 120;

/// Local-space triangles for one path: the fill's indices, then the stroke's.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Mesh {
    vertices: Vec<[f32; 2]>,
    indices: Vec<u32>,
    /// `indices[..fill_end]` is the fill, the rest the stroke.
    fill_end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    node: NodeId,
    revision: u64,
    tolerance_bucket: i32,
}

/// Tessellations by node, revision and tolerance, evicted by age.
#[derive(Default)]
pub(super) struct PathCache {
    meshes: FxHashMap<Key, (Arc<Mesh>, u64)>,
    clock: u64,
    pub(super) tessellations: u64,
}

impl PathCache {
    pub(super) fn mesh(
        &mut self,
        node: NodeId,
        paint: &PathPaint,
        transform: Affine,
        scale: f64,
    ) -> Arc<Mesh> {
        let (tolerance, tolerance_bucket) = tolerance(transform, scale);
        let key = Key {
            node,
            revision: paint.revision,
            tolerance_bucket,
        };
        let clock = self.clock;
        if let Some((mesh, used)) = self.meshes.get_mut(&key) {
            *used = clock;
            return mesh.clone();
        }
        self.tessellations += 1;
        let mesh = Arc::new(tessellate(paint, tolerance));
        self.meshes.insert(key, (mesh.clone(), clock));
        mesh
    }

    /// Ends one render: forget what has not been drawn for a while.
    pub(super) fn sweep(&mut self) {
        let clock = self.clock;
        self.meshes
            .retain(|_, (_, used)| clock.saturating_sub(*used) < CACHE_TTL);
        self.clock += 1;
    }
}

/// Append `mesh` to the batch in target device pixels.
pub(super) fn emit(
    mesh: &Mesh,
    paint: &PathPaint,
    transform: Affine,
    opacity: f32,
    scale: f64,
    vertices: &mut Vec<RectVertex>,
    indices: &mut Vec<u32>,
) {
    let to_device = Affine::scale(scale) * transform;
    let mut part = |range: &[u32], color: [f32; 4]| {
        let base = vertices.len() as u32;
        // Every part re-emits the vertices it uses; fill and stroke never
        // share any, since lyon builds each into its own buffers.
        let (min, max) = match (range.iter().min(), range.iter().max()) {
            (Some(min), Some(max)) => (*min, *max),
            _ => return,
        };
        for local in &mesh.vertices[min as usize..=max as usize] {
            let device = to_device * kurbo::Point::new(f64::from(local[0]), f64::from(local[1]));
            vertices.push(RectVertex {
                position: [device.x as f32, device.y as f32],
                color,
            });
        }
        indices.extend(range.iter().map(|i| i - min + base));
    };
    if let Some(fill) = paint.fill {
        part(
            &mesh.indices[..mesh.fill_end],
            linear_premultiplied(fill.color, opacity),
        );
    }
    if let Some(stroke) = &paint.stroke {
        part(
            &mesh.indices[mesh.fill_end..],
            linear_premultiplied(stroke.color, opacity),
        );
    }
}

fn tolerance(transform: Affine, scale: f64) -> (f32, i32) {
    // The most stretched direction needs the finest flattening.
    let pixels_per_unit = (scale * super::rects::scale_range(transform).1).max(1e-6);
    let bucket = pixels_per_unit.log2().round() as i32;
    let tolerance = TOLERANCE_PX / 2f64.powi(bucket);
    (tolerance as f32, bucket)
}

pub(super) fn tessellate(paint: &PathPaint, tolerance: f32) -> Mesh {
    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    if let Some(fill) = paint.fill {
        let options = FillOptions::tolerance(tolerance).with_fill_rule(match fill.rule {
            FillRule::NonZero => lyon::path::FillRule::NonZero,
            FillRule::EvenOdd => lyon::path::FillRule::EvenOdd,
        });
        let path = lyon_path(paint.path.iter());
        let mut builder =
            BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position().to_array());
        if let Err(error) = FillTessellator::new().tessellate_path(&path, &options, &mut builder) {
            tracing::warn!(%error, "scene path fill tessellation failed");
        }
    }
    let fill_end = buffers.indices.len();
    if let Some(stroke) = paint.stroke.as_ref().filter(|s| s.width > 0.0) {
        let path = stroke_path(&paint.path, stroke);
        let options = stroke_options(stroke, tolerance);
        let mut builder =
            BuffersBuilder::new(&mut buffers, |v: StrokeVertex| v.position().to_array());
        if let Err(error) = StrokeTessellator::new().tessellate_path(&path, &options, &mut builder)
        {
            tracing::warn!(%error, "scene path stroke tessellation failed");
        }
    }
    Mesh {
        vertices: buffers.vertices,
        indices: buffers.indices,
        fill_end,
    }
}

fn stroke_path(path: &BezPath, stroke: &PathStroke) -> Path {
    let valid_dash = !stroke.dash.is_empty()
        && stroke.dash.iter().all(|d| d.is_finite() && *d >= 0.0)
        && stroke.dash.iter().sum::<f64>() > 0.0;
    if valid_dash {
        lyon_path(kurbo::dash(path.iter(), stroke.dash_offset, &stroke.dash))
    } else {
        lyon_path(path.iter())
    }
}

fn stroke_options(stroke: &PathStroke, tolerance: f32) -> StrokeOptions {
    StrokeOptions::tolerance(tolerance)
        .with_line_width(stroke.width as f32)
        .with_line_join(match stroke.join {
            Join::Miter => LineJoin::Miter,
            Join::Round => LineJoin::Round,
            Join::Bevel => LineJoin::Bevel,
        })
        .with_line_cap(match stroke.cap {
            Cap::Butt => LineCap::Butt,
            Cap::Round => LineCap::Round,
            Cap::Square => LineCap::Square,
        })
}

/// kurbo elements to a lyon path. A segment without a preceding move starts
/// at the last point (or the origin), as SVG does.
fn lyon_path(elements: impl Iterator<Item = PathEl>) -> Path {
    let to = |p: kurbo::Point| point(p.x as f32, p.y as f32);
    let mut builder = Path::builder();
    let mut open = false;
    let mut last = kurbo::Point::ZERO;
    let ensure_open = |builder: &mut lyon::path::path::Builder, open: &mut bool, at| {
        if !*open {
            builder.begin(to(at));
            *open = true;
        }
    };
    for element in elements {
        match element {
            PathEl::MoveTo(p) => {
                if open {
                    builder.end(false);
                }
                builder.begin(to(p));
                open = true;
                last = p;
            }
            PathEl::LineTo(p) => {
                ensure_open(&mut builder, &mut open, last);
                builder.line_to(to(p));
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                ensure_open(&mut builder, &mut open, last);
                builder.quadratic_bezier_to(to(c), to(p));
                last = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                ensure_open(&mut builder, &mut open, last);
                builder.cubic_bezier_to(to(c1), to(c2), to(p));
                last = p;
            }
            PathEl::ClosePath => {
                if open {
                    builder.end(true);
                    open = false;
                }
            }
        }
    }
    if open {
        builder.end(false);
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_graph::draw::{Color, Fill};
    use kurbo::Shape;

    fn circle(revision: u64) -> PathPaint {
        PathPaint {
            path: Arc::new(kurbo::Circle::new((50.0, 50.0), 20.0).to_path(0.01)),
            revision,
            fill: Some(Fill {
                color: Color::rgb8(0xd4, 0x8a, 0x20),
                rule: FillRule::NonZero,
            }),
            stroke: Some(PathStroke {
                width: 2.0,
                color: Color::rgb8(0xf4, 0xe7, 0xc3),
                join: Join::Round,
                cap: Cap::Round,
                dash: Vec::new(),
                dash_offset: 0.0,
            }),
        }
    }

    #[test]
    fn fill_and_stroke_tessellate_into_separate_index_ranges() {
        let mesh = tessellate(&circle(0), 0.1);
        assert!(mesh.fill_end > 0 && mesh.fill_end < mesh.indices.len());
        assert_eq!(mesh.indices.len() % 3, 0);
    }

    #[test]
    fn the_cache_keeps_a_mesh_across_motion_and_drops_it_on_a_new_revision() {
        let mut cache = PathCache::default();
        let a = cache.mesh(7, &circle(1), Affine::IDENTITY, 1.0);
        let b = cache.mesh(7, &circle(1), Affine::translate((30.0, 0.0)), 1.0);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(cache.tessellations, 1);
        cache.mesh(7, &circle(2), Affine::IDENTITY, 1.0);
        cache.mesh(7, &circle(2), Affine::scale(4.0), 1.0);
        assert_eq!(cache.tessellations, 3);
    }

    #[test]
    fn tolerance_follows_the_most_stretched_axis() {
        assert_eq!(tolerance(Affine::scale_non_uniform(4.0, 0.25), 1.0).1, 2);
        assert_eq!(tolerance(Affine::IDENTITY, 2.0).1, 1);
    }

    #[test]
    fn unused_meshes_age_out() {
        let mut cache = PathCache::default();
        cache.mesh(1, &circle(0), Affine::IDENTITY, 1.0);
        for _ in 0..=CACHE_TTL {
            cache.sweep();
        }
        assert!(cache.meshes.is_empty());
    }

    #[test]
    fn a_dashed_stroke_has_gaps() {
        let mut solid = circle(0);
        solid.fill = None;
        let mut dashed = solid.clone();
        if let Some(stroke) = dashed.stroke.as_mut() {
            stroke.dash = vec![4.0, 4.0];
        }
        let solid = tessellate(&solid, 0.1);
        let dashed = tessellate(&dashed, 0.1);
        assert!(dashed.indices.len() > solid.indices.len());
    }

    #[test]
    fn emit_places_vertices_in_device_pixels() {
        let mut paint = circle(0);
        paint.stroke = None;
        let mesh = tessellate(&paint, 0.1);
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        emit(
            &mesh,
            &paint,
            Affine::translate((10.0, 0.0)),
            1.0,
            2.0,
            &mut vertices,
            &mut indices,
        );
        let max_x = vertices.iter().map(|v| v.position[0]).fold(0.0, f32::max);
        assert!((max_x - 160.0).abs() < 0.5, "max x {max_x}");
        assert_eq!(indices.len(), mesh.fill_end);
    }
}
