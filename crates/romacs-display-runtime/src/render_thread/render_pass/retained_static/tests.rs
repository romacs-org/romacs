use crate::render_thread::state::{PointerAppearanceState, PresentedAppearanceKey};
use romacs_display_protocol::frame_chrome::PresentationId;
use romacs_display_protocol::frame_glyphs::{
    CursorStyle, DisplaySlotId, FrameGlyphBuffer, WindowCursor,
};
use romacs_display_protocol::{
    Color, DisplayWindowId, PointerAppearanceId,
};

fn cursor_frame() -> FrameGlyphBuffer {
    let window_id = DisplayWindowId::new(1);
    let mut frame = FrameGlyphBuffer::with_size(20.0, 20.0);
    frame.window_cursors.push(WindowCursor {
        window_id,
        slot_id: DisplaySlotId::ZERO,
        x: 1.0,
        y: 2.0,
        width: 3.0,
        height: 4.0,
        style: CursorStyle::FilledBox,
        color: Color::WHITE,
        cursor_fg: Color::BLACK,
        ascent: 0.0,
        active: true,
    });
    frame
}

#[test]
fn the_retained_front_layer_excludes_filled_boxes() {
    let mut render = super::GuiFrameRenderState::new_without_device(
        42,
        false,
        romacs_display_protocol::frame_time::observe_platform_now(),
    );
    let mut frame = cursor_frame();
    render.set_current_frame(Some(frame.clone()), Default::default(), Default::default());
    assert!(!super::is_eligible(true, &[], &render));

    frame.window_cursors[0].style = CursorStyle::Bar(2.0);
    render.set_current_frame(Some(frame), Default::default(), Default::default());
    assert!(super::is_eligible(true, &[], &render));
    assert!(!super::is_eligible(false, &[], &render));
}

#[test]
fn a_hovered_or_pressed_pointer_appearance_forces_the_full_render() {
    let mut pointer = PointerAppearanceState::default();
    assert!(super::pointer_appearance_allowed(&pointer));

    let key = PresentedAppearanceKey::new(
        PresentationId::new(7),
        PointerAppearanceId::try_from(0usize).unwrap(),
    );
    pointer.hover(Some(key));
    assert!(!super::pointer_appearance_allowed(&pointer));

    pointer.press();
    assert!(!super::pointer_appearance_allowed(&pointer));
}

fn snapshot(rev: u64, yaw: f64) -> romacs_renderer_wgpu::model::store::Snapshot {
    romacs_renderer_wgpu::model::store::Snapshot {
        rev,
        req: 0,
        view: romacs_display_protocol::model::ModelView {
            yaw,
            pitch: 0.0,
            zoom: 1.0,
        },
        logical: (64, 32),
        size_px: (64, 32),
    }
}

fn record(
    drawn: &[u32],
    referenced: &[u32],
    current: &crate::render_thread::models::ModelPicture,
) -> super::RetainedModels {
    super::RetainedModels::record(
        Some(drawn.iter().copied().collect()),
        referenced.iter().copied().collect(),
        current,
    )
}

// Review MUST 5: a retained scene pictures each model it composited at the
// snapshot its texture showed then. Once any of them is redrawn (an orbit,
// a zoom, a Lisp request), blitting the retained raster would put the old
// camera back on screen with no editor scene change, so it must rebuild.
#[test]
fn a_retained_scene_is_stale_once_a_model_it_pictures_is_redrawn() {
    use crate::render_thread::models::ModelPicture;
    let size = romacs_renderer_wgpu::SnapshotSize::new(800, 600).unwrap();
    let current = ModelPicture::from([(5, snapshot(1, 0.0)), (7, snapshot(4, 10.0))]);
    let built = record(&[5, 7], &[5, 7], &current);
    assert_eq!(built.picture, current);
    assert!(super::retained_scene_is_current(
        (3, size, &built),
        (3, size, &current)
    ));
    // Model 7 orbited and redrew: same scene generation, same size.
    let now = ModelPicture::from([(5, snapshot(1, 0.0)), (7, snapshot(5, 30.0))]);
    assert!(
        !super::retained_scene_is_current((3, size, &built), (3, size, &now)),
        "the retained raster still shows model 7's old camera"
    );
    // Model 7 was freed: its pixels are still in the raster.
    let freed = ModelPicture::from([(5, snapshot(1, 0.0))]);
    assert!(!super::retained_scene_is_current(
        (3, size, &built),
        (3, size, &freed)
    ));
}

// SOL-REVIEW-2 MUST 3 / OPUS-REVIEW MUST 1: a raster whose glyphs reference
// a model that was not drawable at the build (still loading, or absent)
// composited nothing for it. When that model becomes drawable the raster
// is missing its pixels, so it must rebuild; a model the raster's glyphs do
// not reference stays irrelevant to it, loading or not.
#[test]
fn a_retained_scene_is_stale_once_a_model_it_references_becomes_drawable() {
    use crate::render_thread::models::ModelPicture;
    let size = romacs_renderer_wgpu::SnapshotSize::new(800, 600).unwrap();
    // Model 7 is referenced but loading: only 5 is drawn.
    let loading = ModelPicture::from([(5, snapshot(1, 0.0))]);
    let built = record(&[5], &[5, 7], &loading);
    assert_eq!(built.picture, loading, "the raster pictures only model 5");
    assert!(super::retained_scene_is_current(
        (3, size, &built),
        (3, size, &loading)
    ));
    // Model 7 uploaded: same scene generation, same size.
    let uploaded = ModelPicture::from([(5, snapshot(1, 0.0)), (7, snapshot(1, 0.0))]);
    assert!(
        !super::retained_scene_is_current((3, size, &built), (3, size, &uploaded)),
        "the retained raster lacks model 7's pixels"
    );
    // Model 9, which the raster does not reference, uploads or redraws:
    // the raster still pictures its scene.
    let unrelated = ModelPicture::from([(5, snapshot(1, 0.0)), (9, snapshot(2, 0.0))]);
    assert!(super::retained_scene_is_current(
        (3, size, &built),
        (3, size, &unrelated)
    ));
    // A referenced surface that is no model (a shader surface) never
    // appears among the models' snapshots and changes nothing.
    let built = record(&[5], &[5, 11], &loading);
    assert!(super::retained_scene_is_current(
        (3, size, &built),
        (3, size, &loading)
    ));
}
