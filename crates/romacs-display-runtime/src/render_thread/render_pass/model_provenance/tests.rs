use super::blends_older_pictures;
use crate::render_thread::frame_windows::GuiFrameRenderState;
use romacs_display_protocol::frame_time;
use romacs_display_protocol::motion_spec::{MotionDuration, MotionSpec, TweenSpec};
use romacs_display_protocol::scroll_animation::TransitionEasing;
use romacs_renderer_wgpu::SnapshotSize;

const ROOT: u64 = 1;

fn tween_100ms() -> MotionSpec {
    MotionSpec::Tween(TweenSpec {
        duration: MotionDuration::new(std::time::Duration::from_millis(100)).expect("positive"),
        easing: TransitionEasing::Linear,
        bezier: None,
    })
}

/// SOL-REVIEW-2 MUST 2: each composition that shows an older picture
/// besides the fresh glyph passes is recognised, through the state the
/// composition itself reads (pane placements, the active transitions), and
/// only while it runs.
#[test]
fn compositions_showing_an_older_picture_are_recognised_while_they_run() {
    let Ok(mut renderer) = romacs_renderer_wgpu::gpu_policy::test_gpu::renderer(80, 40) else {
        assert!(std::env::var_os("ROMACS_REQUIRE_GPU_TESTS").is_none());
        return;
    };
    let origin = frame_time::observe_platform_now();
    let mut render = GuiFrameRenderState::new(ROOT, renderer.device(), 1.0, false, origin);
    let size = SnapshotSize::new(80, 40).unwrap();


    // A plain frame shows only its fresh passes.
    assert!(!blends_older_pictures(&render, false));
    // A pane morph places ring pictures.
    assert!(blends_older_pictures(&render, true), "pane morph");

    // A theme crossfade blends the previous composition in.
    let old = renderer.acquire_snapshot(size).unwrap();
    render
        .compositor
        .transitions
        .start_theme_crossfade_for_test(old, tween_100ms(), origin);
    assert!(
        blends_older_pictures(&render, false),
        "theme crossfade"
    );
    render.compositor.transitions.clear_active_for_test();
    assert!(!blends_older_pictures(&render, false));
}
