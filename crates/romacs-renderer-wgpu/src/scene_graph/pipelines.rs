//! The scene pass's pipelines: all 4× multisampled into an sRGB target with
//! premultiplied blending, over the renderer's existing vertex layouts.

use super::targets::{SAMPLE_COUNT, TARGET_FORMAT};
use crate::vertex::{GlyphVertex, RectVertex, RoundedRectVertex};

pub(super) struct Pipelines {
    pub uniform_layout: wgpu::BindGroupLayout,
    /// Texture + filtering sampler: the glyph atlas's layout, reused for
    /// sampled targets, images and surfaces.
    pub texture_layout: wgpu::BindGroupLayout,
    pub sampler: wgpu::Sampler,
    pub rect: wgpu::RenderPipeline,
    pub sample: wgpu::RenderPipeline,
    pub fill: wgpu::RenderPipeline,
    pub glyph_mask: wgpu::RenderPipeline,
    pub glyph_color: wgpu::RenderPipeline,
}

impl Pipelines {
    pub fn new(device: &wgpu::Device, texture_layout: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene pass"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let plain = layout(device, &[&uniform_layout]);
        let textured = layout(device, &[&uniform_layout, texture_layout]);
        let make =
            |name, layout, vs, fs, buffer| pipeline(device, &module, name, layout, vs, fs, buffer);
        Self {
            rect: make(
                "scene rect",
                &plain,
                "vs_rect",
                "fs_rect",
                RoundedRectVertex::desc(),
            ),
            sample: make(
                "scene sample",
                &textured,
                "vs_rect",
                "fs_sample",
                RoundedRectVertex::desc(),
            ),
            fill: make(
                "scene fill",
                &plain,
                "vs_fill",
                "fs_fill",
                RectVertex::desc(),
            ),
            glyph_mask: make(
                "scene glyph mask",
                &textured,
                "vs_glyph",
                "fs_glyph_mask",
                GlyphVertex::desc(),
            ),
            glyph_color: make(
                "scene glyph color",
                &textured,
                "vs_glyph",
                "fs_glyph_color",
                GlyphVertex::desc(),
            ),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("scene source sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            uniform_layout,
            texture_layout: texture_layout.clone(),
        }
    }

    pub fn texture_bind_group(
        &self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene source"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}

fn layout(device: &wgpu::Device, groups: &[&wgpu::BindGroupLayout]) -> wgpu::PipelineLayout {
    let groups: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().copied().map(Some).collect();
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("scene pass"),
        bind_group_layouts: &groups,
        immediate_size: 0,
    })
}

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    label: &str,
    layout: &wgpu::PipelineLayout,
    vs: &str,
    fs: &str,
    buffer: wgpu::VertexBufferLayout<'static>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vs),
            buffers: &[Some(buffer)],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            targets: &[Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: SAMPLE_COUNT,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}
