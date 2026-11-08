//! The retained root picture: the root glyph pass's fast path for a scene
//! commit whose root frame paints exactly as the last one did.
//!
//! A key typed into a layer (the minibuffer's, while completing) commits a
//! scene whose root frame is the last one again: the layout engine reused
//! it, and says so with the same `layout_identity`. Rasterising that root
//! again is most of such a frame's cost: on a software adapter every pixel
//! of the frame, on the GPU its tessellation. The first time a root
//! repeats (or is about to: a layer animation is running), this pass draws
//! it into a texture and blits that; from then on a repeat is the blit
//! alone, and the layers draw over it as before.
//!
//! Owns: the eligibility rule, the key a picture is valid for, and the
//! texture. Must not: draw anything but the root glyph pass's own pixels;
//! child frames and overlays are drawn after it by their passes, fresh.

use crate::render_thread::frame_compositor::RetainedRoot;
use crate::render_thread::frame_stats;
use crate::render_thread::frame_windows::GuiFrameRenderState;
use romacs_display_protocol::frame_glyphs::{FrameGlyph, FrameGlyphBuffer, WindowCursor};
use romacs_renderer_wgpu::{FullFrameTexture, SnapshotSize, UnpooledTexture, WgpuRenderer};

/// What a root picture shows: the producer's glyphs, with these cursors,
/// on a surface of this size and scale.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::render_thread) struct RootKey {
    identity: u64,
    cursors: Vec<WindowCursor>,
    blinked_off: bool,
    cursor_visible: bool,
    /// The look and focus light its cursors were drawn in
    /// (`romacs-cursor-look').
    look: romacs_display_protocol::CursorLookConfig,
    light_bits: u32,
    size: SnapshotSize,
    scale_bits: u32,
}

/// The key FRAME's root pass would paint under, when only its key decides
/// its pixels: an identified, opaque frame of text, with no effect,
/// animation, pointer highlight or native content that changes pixels
/// between two commits of the same glyphs.
pub(super) fn key(
    frame: &FrameGlyphBuffer,
    cursor_visible: bool,
    (look, light): (&romacs_display_protocol::CursorLookConfig, f32),
    size: SnapshotSize,
    scale: f32,
    stable: bool,
) -> Option<RootKey> {
    let plain = |glyph: &FrameGlyph| {
        matches!(
            glyph,
            FrameGlyph::Char { .. }
                | FrameGlyph::Stretch { .. }
                | FrameGlyph::Background { .. }
                | FrameGlyph::Border { .. }
                | FrameGlyph::FringeBitmap { .. }
                | FrameGlyph::ScrollBar { .. }
        )
    };
    (stable
        && frame.layout_identity != 0
        && frame.background_alpha == 1.0
        && frame.glyphs.iter().all(plain)
        && std::env::var_os("ROMACS_DISABLE_RETAINED_ROOT").is_none())
    .then(|| RootKey {
        identity: frame.layout_identity,
        cursors: frame.window_cursors.clone(),
        blinked_off: frame.cursor_blinked_off,
        cursor_visible,
        look: look.clone(),
        light_bits: light.to_bits(),
        size,
        scale_bits: scale.to_bits(),
    })
}

/// Paint the root under KEY onto VIEW: blit the retained picture when it
/// shows KEY; else, when the root will repeat (the last root painted had
/// KEY too, or REPEATS: a layer animation will draw it again next frame),
/// draw it into the picture through DRAW and blit that; else draw it onto
/// VIEW. Records KEY as the last root painted.
pub(super) fn paint(
    renderer: &mut WgpuRenderer,
    render: &mut GuiFrameRenderState,
    view: &wgpu::TextureView,
    present_mapping: romacs_display_protocol::PresentMapping,
    key: Option<RootKey>,
    repeats: bool,
    draw: impl FnOnce(&mut WgpuRenderer, &mut GuiFrameRenderState, &wgpu::TextureView),
) {
    let last = std::mem::replace(&mut render.compositor.last_root_key, key.clone());
    let Some(key) = key else {
        return draw(renderer, render, view);
    };
    let current = render
        .compositor
        .retained_root
        .as_ref()
        .is_some_and(|root| root.key.as_ref() == Some(&key));
    if !current {
        if !repeats && last.as_ref() != Some(&key) {
            return draw(renderer, render, view);
        }
        ensure_texture(renderer, render, key.size);
        let picture = render.compositor.retained_root.as_ref().expect("ensured above").texture.view().clone();
        draw(renderer, render, &picture);
        if let Some(root) = render.compositor.retained_root.as_mut() {
            root.key = Some(key);
        }
    } else {
        frame_stats::count(&frame_stats::RETAINED_ROOT_BLITS);
    }
    if let Some(root) = render.compositor.retained_root.as_ref() {
        renderer
            .begin_draw(romacs_renderer_wgpu::renderer::RenderTarget::new(
                view,
                present_mapping.surface(),
            ))
            .blit_retained(&root.bind_group);
    }
}

fn ensure_texture(renderer: &WgpuRenderer, render: &mut GuiFrameRenderState, size: SnapshotSize) {
    if render.compositor.retained_root.as_ref().is_some_and(|root| root.texture.size() == size) {
        return;
    }
    let texture = FullFrameTexture::allocate(
        renderer.device(),
        UnpooledTexture::RetainedRootScene,
        "retained-root-scene",
        size,
        renderer.surface_format(),
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let bind_group = renderer.create_texture_bind_group(texture.view());
    render.compositor.retained_root = Some(RetainedRoot { texture, bind_group, key: None });
}
