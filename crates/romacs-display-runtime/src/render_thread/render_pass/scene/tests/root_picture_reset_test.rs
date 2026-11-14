//! The root picture is GPU-resident: a device reset drops it, and the next
//! frame's root pass is drawn whole, as a fresh renderer draws it.
use super::*;
use crate::render_thread::frame_windows::{FrameLifecycle, GuiFrameWindowManager, GuiFrameWindowState};
use romacs_display_protocol::frame_glyphs::{CursorStyle, DisplaySlotId, GlyphRowRole, PhysCursor};

/// Rows of coloured cells with a box cursor, the second row's cells in
/// SECOND's order of colours (no font: this crate's tests have none).
#[expect(clippy::cast_precision_loss, reason = "a test surface's few pixels")]
fn text(second: &str) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(W as f32, H as f32);
    (frame.char_width, frame.char_height) = (7.0, 12.0);
    frame.background = Color::rgb(0.1, 0.12, 0.16);
    let background = frame.background;
    frame.set_face(FaceId::new(0), Color::rgb(0.9, 0.9, 0.85), Some(background), 400, false, 0, None, 0, None, 0, None);
    frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
    frame.add_background(0.0, 0.0, W as f32, H as f32, background);
    for (row, line) in ["first", second, "third"].iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            let shade = f32::from(u8::try_from(ch).unwrap() % 8) / 8.0;
            let (x, y) = (2.0 + 7.0 * f32::from(u8::try_from(col).unwrap()), 1.0 + 12.0 * f32::from(u8::try_from(row).unwrap()));
            frame.add_stretch(x, y, 7.0, 12.0, Color::rgb(shade, 0.4, 1.0 - shade), FaceId::new(0), false);
        }
    }
    let slot = DisplaySlotId { window_id: DisplayWindowId::new(1), row: 0, col: 1 };
    frame.set_phys_cursor(PhysCursor {
        window_id: DisplayWindowId::new(1),
        charpos: 0,
        row: 0,
        col: 1,
        slot_id: slot,
        x: 9.0,
        y: 1.0,
        width: 7.0,
        height: 12.0,
        ascent: 9.0,
        style: CursorStyle::FilledBox,
        color: Color::rgb(1.0, 0.5, 0.0),
        cursor_fg: Color::rgb(0.05, 0.05, 0.05),
    });
    frame
}

/// A renderer whose pixels depend on nothing but the frames it draws.
fn still(renderer: &mut WgpuRenderer) {
    renderer.effects = romacs_display_protocol::EffectsConfig::default();
    renderer.effects.cursor_color_cycle.enabled = false;
}

/// FRAME's root pass through RENDER onto a new target, read back.
fn root(renderer: &mut WgpuRenderer, render: &mut GuiFrameRenderState, frame: &FrameGlyphBuffer) -> Vec<u8> {
    let target = target(renderer);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    render_frame_root_glyphs(renderer, render, &view, frame, mapping(frame, W, H), true, None, None);
    pixels(renderer, &target)
}

#[test]
#[expect(clippy::default_trait_access, reason = "a pending window's chrome, as the device-recreation test builds it")]
fn a_reset_drops_the_root_picture_and_the_next_frame_is_drawn_whole() {
    let mut a = romacs_renderer_wgpu::gpu_policy::test_gpu::renderer(W, H).expect("real device A required");
    still(&mut a);
    let origin = observe_platform_now();
    let mut render = GuiFrameRenderState::new(42, a.device(), 1.0, false, origin);
    // A frame drawn directly, then one the picture shows and patches.
    root(&mut a, &mut render, &text("second"));
    root(&mut a, &mut render, &text("secXnd"));
    assert!(render.compositor.root_picture.is_some(), "the root pass drew through the picture");
    let mut manager = GuiFrameWindowManager::new();
    manager.set_primary_pending(GuiFrameWindowState {
        pending_scale_factor: None,
        lifecycle: FrameLifecycle::Pending {
            width: W,
            height: H,
            scale_factor: 1.0,
            mouse_hidden_for_typing: false,
            ime_enabled: false,
            last_ime_cursor_area: None,
            chrome: Default::default(),
            geometry_hints: None,
        },
        render,
    });
    drop(a);
    manager.clear_gpu_resident_state();
    let render = &mut manager.primary_window_mut().unwrap().render;
    assert!(render.compositor.root_picture.is_none(), "the picture went with its device");
    let mut b = romacs_renderer_wgpu::gpu_policy::test_gpu::renderer(W, H).expect("real device B required");
    still(&mut b);
    render.populate_glyph_atlas(b.device(), 1.0);
    let frame = text("secXnd");
    let after_reset = root(&mut b, render, &frame);
    drop(b);
    // A fresh renderer and window state drawing the same frame whole.
    let mut c = romacs_renderer_wgpu::gpu_policy::test_gpu::renderer(W, H).expect("real device C required");
    still(&mut c);
    let mut fresh = GuiFrameRenderState::new(43, c.device(), 1.0, false, origin);
    let target = target(&c);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    c.render_frame_glyphs(&view, &frame, fresh.compositor.glyph_atlas.as_mut().unwrap(), mapping(&frame, W, H), true, None, None, None, None);
    assert_eq!(after_reset, pixels(&c, &target), "the first frame after a reset is the whole pass's pixels");
}
