//! Scene placements: one placement of a scene, drawn by its own
//! [`ScenePass`] and installed in the shader-surface cache as a
//! pipeline-less surface, so a frame shows it as an ordinary surface glyph
//! (both composite paths, the LRU touch and `mark_drawn`), as a model view is.
//! Its targets are charged to media_budget's Surface accounting under the
//! placement's id, never evictable.

use rustc_hash::FxHashMap;

use super::WgpuRenderer;
use crate::scene_graph::{CacheSources, SceneInputs, ScenePass, SceneStats, draw::DrawList};
use crate::shader_surface_cache::ShaderSurfaceCache;
use crate::WgpuGlyphAtlas;

/// Each placement's pass and the device size of the target it installed.
#[derive(Default)]
pub(crate) struct ScenePlacements {
    passes: FxHashMap<u32, (ScenePass, Option<[u32; 2]>)>,
}

impl WgpuRenderer {
    /// Draw `list` as placement `id` at the device scale and install its root
    /// target as surface `id`. The list's samples may name the renderer's
    /// images and shader surfaces; `atlas` is the window's glyph atlas.
    pub fn render_scene_placement(
        &mut self,
        id: u32,
        list: &DrawList,
        atlas: &mut WgpuGlyphAtlas,
    ) -> Result<SceneStats, String> {
        let root = list.targets.last().ok_or("scene draw list has no targets")?.id;
        let (pass, installed) = self.scene_placements.passes.entry(id).or_insert_with(|| {
            (ScenePass::new(&self.device, &self.queue, atlas.bind_group_layout()), None)
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Scene placement"),
        });
        let sources = CacheSources {
            images: Some(&self.caches.image),
            surfaces: Some(&self.caches.surface),
        };
        let stats = pass
            .render(
                &self.device,
                &self.queue,
                &mut encoder,
                list,
                self.scale_factor,
                SceneInputs { atlas, fonts: &|_| None, sources: &sources },
            )
            .map_err(|e| e.to_string())?;
        self.queue.submit([encoder.finish()]);
        pass.submitted();
        let texture = pass.texture(root).ok_or("scene placement drew no root")?;
        let size = [texture.width(), texture.height()];
        if *installed != Some(size) {
            let target = list.targets.last().map(|t| t.size).unwrap_or_default();
            let prepared = ShaderSurfaceCache::prepare_rendered(
                &self.device,
                self.caches.image.bind_group_layout(),
                self.caches.image.sampler(),
                texture.clone(),
                pass.view(root).ok_or("scene placement drew no root")?.clone(),
                (target.width.round() as u32, target.height.round() as u32),
                (size[0], size[1]),
            )?;
            self.caches.surface.install_prepared(id, prepared);
            *installed = Some(size);
            let bytes = pass.resident_bytes() as usize;
            self.media_budget.unregister(crate::media_budget::MediaType::Surface, id);
            self.media_budget.register(crate::media_budget::MediaType::Surface, id, bytes);
        }
        Ok(stats)
    }

    /// Forget placement `id`: its pass, its targets and its surface.
    pub fn free_scene_placement(&mut self, id: u32) {
        if self.scene_placements.passes.remove(&id).is_some() {
            self.free_surface(id);
        }
    }
}
