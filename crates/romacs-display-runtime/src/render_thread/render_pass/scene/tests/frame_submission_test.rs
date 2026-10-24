//! One queue submission per frame: every pass of an open frame records into
//! it in order, and a later pass cannot overwrite what an earlier one uploaded.
use super::*;

fn stripe(x: f32, color: Color) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(W as f32, H as f32);
    frame.set_frame_identity(
        DisplayFrameId::new(42),
        DisplayFrameId::new(0),
        0.0,
        0.0,
        0,
        false,
        0.0,
        Color::BLACK,
        false,
        1.0,
    );
    frame.background = Color::BLACK;
    frame.background_alpha = 1.0;
    frame.set_face(
        FaceId::new(1),
        Color::WHITE,
        Some(color),
        400,
        false,
        0,
        None,
        0,
        None,
        0,
        None,
    );
    frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
    frame.add_stretch(x, 0.0, 16.0, H as f32, color, FaceId::new(1), false);
    frame
}

/// Two glyph passes, as root and child pictures are drawn in one frame.
/// Each pass starts by resetting the vertex arenas and atlas pins when it
/// runs standalone.
fn two_passes(
    renderer: &mut WgpuRenderer,
    render: &mut GuiFrameRenderState,
    one_frame: bool,
) -> (Vec<u8>, Vec<u8>, u64) {
    let first = target(renderer);
    let second = target(renderer);
    let before = renderer.queue_submit_count();
    if one_frame {
        renderer.begin_frame_submission();
    }
    draw_root(
        renderer,
        render,
        &first.create_view(&Default::default()),
        &stripe(8.0, Color::RED),
    );
    draw_root(
        renderer,
        render,
        &second.create_view(&Default::default()),
        &stripe(64.0, Color::BLUE),
    );
    if one_frame {
        renderer.finish_frame_submission();
    }
    let submits = renderer.queue_submit_count() - before;
    (pixels(renderer, &first), pixels(renderer, &second), submits)
}

#[test]
fn frame_passes_share_one_submission_without_overwriting_earlier_uploads() {
    let mut renderer =
        WgpuRenderer::new(None, W, H).expect("GPU required for frame submission regression");
    let mut render =
        GuiFrameRenderState::new(42, renderer.device(), 1.0, false, observe_platform_now());
    let (first_alone, second_alone, standalone_submits) =
        two_passes(&mut renderer, &mut render, false);
    assert_eq!(standalone_submits, 2);
    assert_eq!(
        px(&first_alone, 16, 32),
        [255, 0, 0, 255],
        "first stripe drawn"
    );
    assert_eq!(
        px(&first_alone, 72, 32),
        [0, 0, 0, 255],
        "second stripe absent"
    );
    assert_eq!(
        px(&second_alone, 72, 32),
        [0, 0, 255, 255],
        "second stripe drawn"
    );

    let (first, second, frame_submits) = two_passes(&mut renderer, &mut render, true);
    assert_eq!(frame_submits, 1, "one queue.submit for the whole frame");
    assert!(first == first_alone, "first pass kept its own vertices");
    assert!(second == second_alone, "second pass unchanged by batching");
}

#[test]
fn empty_frame_makes_no_submission_and_flush_keeps_it_open() {
    let mut renderer =
        WgpuRenderer::new(None, W, H).expect("GPU required for frame submission regression");
    let mut render =
        GuiFrameRenderState::new(42, renderer.device(), 1.0, false, observe_platform_now());
    let before = renderer.queue_submit_count();
    renderer.begin_frame_submission();
    renderer.finish_frame_submission();
    assert_eq!(renderer.queue_submit_count(), before);

    let first = target(&renderer);
    let second = target(&renderer);
    renderer.begin_frame_submission();
    draw_root(
        &mut renderer,
        &mut render,
        &first.create_view(&Default::default()),
        &stripe(8.0, Color::RED),
    );
    // A surface-loss path submits what it drew before dropping its target.
    renderer.flush_frame_submission();
    assert_eq!(renderer.queue_submit_count(), before + 1);
    draw_root(
        &mut renderer,
        &mut render,
        &second.create_view(&Default::default()),
        &stripe(64.0, Color::BLUE),
    );
    assert_eq!(
        renderer.queue_submit_count(),
        before + 1,
        "frame still open"
    );
    renderer.finish_frame_submission();
    assert_eq!(renderer.queue_submit_count(), before + 2);
    assert_eq!(px(&pixels(&renderer, &second), 72, 32), [0, 0, 255, 255]);
}
