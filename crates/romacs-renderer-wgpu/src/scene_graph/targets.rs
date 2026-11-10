//! Target textures: list validation and a pool keyed by target id.
//!
//! Each target owns a single-sample sRGB texture other targets and the
//! compositor sample, and a 4× multisampled attachment it resolves into once
//! per render.

use super::SceneError;
use super::draw::{DrawList, Paint, Source, TargetId};
use rustc_hash::{FxHashMap, FxHashSet};

pub(crate) const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub(crate) const SAMPLE_COUNT: u32 = 4;

/// Device pixel size of every target, after checking the list's contract:
/// unique ids, sizes within the device limit, and samples only of targets
/// earlier in the list.
pub(super) fn validate(
    list: &DrawList,
    scale: f64,
    max_dimension: u32,
) -> Result<Vec<[u32; 2]>, SceneError> {
    if list.targets.is_empty() {
        return Err(SceneError::Empty);
    }
    let mut done: FxHashSet<TargetId> = FxHashSet::default();
    let mut sizes = Vec::with_capacity(list.targets.len());
    for target in &list.targets {
        let size = device_size(target.size, scale);
        if size[0] == 0 || size[1] == 0 || size[0] > max_dimension || size[1] > max_dimension {
            return Err(SceneError::TargetSize {
                target: target.id,
                size,
            });
        }
        for item in &target.items {
            let Paint::Sample(sample) = &item.paint else {
                continue;
            };
            // A shader surface's channels must be ready before it renders.
            let unrendered = std::iter::once(&Some(sample.source))
                .chain(sample.channels.iter())
                .find_map(|input| match input {
                    Some(Source::Target(id)) if !done.contains(id) => Some(*id),
                    _ => None,
                });
            if let Some(source) = unrendered {
                return Err(SceneError::SampleOrder {
                    target: target.id,
                    node: item.node,
                    sampled: source,
                });
            }
        }
        if !done.insert(target.id) {
            return Err(SceneError::DuplicateTarget(target.id));
        }
        sizes.push(size);
    }
    Ok(sizes)
}

fn device_size(size: kurbo::Size, scale: f64) -> [u32; 2] {
    let px = |v: f64| {
        let v = (v * scale).ceil();
        if v.is_finite() && v > 0.0 {
            v.min(f64::from(u32::MAX)) as u32
        } else {
            0
        }
    };
    [px(size.width), px(size.height)]
}

pub(super) struct TargetTexture {
    pub size: [u32; 2],
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub msaa_view: wgpu::TextureView,
    pub uniforms: wgpu::Buffer,
    pub uniform_bind_group: wgpu::BindGroup,
}

/// Textures by target id; a target that changes size gets fresh textures, a
/// target missing from a render is released.
#[derive(Default)]
pub(super) struct TargetPool {
    targets: FxHashMap<TargetId, TargetTexture>,
}

impl TargetPool {
    pub fn get(&self, id: TargetId) -> Option<&TargetTexture> {
        self.targets.get(&id)
    }

    pub fn ensure(
        &mut self,
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        id: TargetId,
        size: [u32; 2],
    ) -> &TargetTexture {
        let stale = self.targets.get(&id).is_some_and(|t| t.size != size);
        if stale {
            self.targets.remove(&id);
        }
        self.targets
            .entry(id)
            .or_insert_with(|| create(device, uniform_layout, id, size))
    }

    /// Keep only the targets of the list just rendered.
    pub fn retain(&mut self, live: &FxHashSet<TargetId>) {
        self.targets.retain(|id, _| live.contains(id));
    }

    /// Bytes resident: resolve textures plus their multisampled attachments.
    pub fn resident_bytes(&self) -> u64 {
        self.targets
            .values()
            .map(|t| u64::from(t.size[0]) * u64::from(t.size[1]) * 4 * u64::from(1 + SAMPLE_COUNT))
            .sum()
    }
}

fn create(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    id: TargetId,
    size: [u32; 2],
) -> TargetTexture {
    let extent = wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene target"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let msaa = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene target msaa"),
        size: extent,
        mip_level_count: 1,
        sample_count: SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scene target uniforms"),
        size: std::mem::size_of::<crate::vertex::Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene target uniforms"),
        layout: uniform_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    });
    tracing::debug!(target = id.0, ?size, "scene target allocated");
    TargetTexture {
        size,
        view: texture.create_view(&Default::default()),
        msaa_view: msaa.create_view(&Default::default()),
        texture,
        uniforms,
        uniform_bind_group,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_graph::draw::{Color, Item, SamplePaint, Target};
    use kurbo::{Affine, Rect, Size};
    use std::sync::Arc;

    fn target(id: u32, samples: &[u32]) -> Target {
        Target {
            id: TargetId(id),
            size: Size::new(10.0, 10.0),
            clear: Color::TRANSPARENT,
            items: samples
                .iter()
                .map(|s| Item {
                    node: 100 + s,
                    transform: Affine::IDENTITY,
                    opacity: 1.0,
                    clip: None,
                    paint: Paint::Sample(SamplePaint {
                        source: Source::Target(TargetId(*s)),
                        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                        radius: 0.0,
                        channels: Arc::default(),
                    }),
                })
                .collect(),
        }
    }

    #[test]
    fn a_shader_channel_may_bind_only_earlier_targets() {
        let shader = |channel: u32| Item {
            node: 7,
            transform: Affine::IDENTITY,
            opacity: 1.0,
            clip: None,
            paint: Paint::Sample(SamplePaint {
                source: Source::Surface(3),
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                channels: [None, Some(Source::Target(TargetId(channel)))].into(),
            }),
        };
        let with = |first: Target, channel| {
            let mut root = target(0, &[]);
            root.items.push(shader(channel));
            DrawList {
                targets: vec![first, root],
            }
        };
        assert!(validate(&with(target(1, &[]), 1), 1.0, 4096).is_ok());
        assert!(matches!(
            validate(&with(target(1, &[]), 2), 1.0, 4096),
            Err(SceneError::SampleOrder {
                target: TargetId(0),
                node: 7,
                sampled: TargetId(2)
            })
        ));
    }

    #[test]
    fn a_target_may_sample_only_earlier_targets() {
        let ok = DrawList {
            targets: vec![target(2, &[]), target(1, &[2]), target(0, &[1, 2])],
        };
        assert_eq!(validate(&ok, 1.5, 4096).unwrap(), vec![[15, 15]; 3]);
        let later = DrawList {
            targets: vec![target(1, &[2]), target(2, &[]), target(0, &[1])],
        };
        assert!(matches!(
            validate(&later, 1.0, 4096),
            Err(SceneError::SampleOrder {
                target: TargetId(1),
                node: 102,
                sampled: TargetId(2)
            })
        ));
        let itself = DrawList {
            targets: vec![target(0, &[0])],
        };
        assert!(matches!(
            validate(&itself, 1.0, 4096),
            Err(SceneError::SampleOrder { .. })
        ));
    }

    #[test]
    fn sizes_ids_and_emptiness_are_checked() {
        assert!(matches!(
            validate(&DrawList::default(), 1.0, 4096),
            Err(SceneError::Empty)
        ));
        let dup = DrawList {
            targets: vec![target(1, &[]), target(1, &[])],
        };
        assert!(matches!(
            validate(&dup, 1.0, 4096),
            Err(SceneError::DuplicateTarget(TargetId(1)))
        ));
        let mut huge = target(0, &[]);
        huge.size = Size::new(5000.0, 1.0);
        assert!(matches!(
            validate(
                &DrawList {
                    targets: vec![huge]
                },
                1.0,
                4096
            ),
            Err(SceneError::TargetSize { .. })
        ));
        let mut nan = target(0, &[]);
        nan.size = Size::new(f64::NAN, 1.0);
        assert!(validate(&DrawList { targets: vec![nan] }, 1.0, 4096).is_err());
    }
}
