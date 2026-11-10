use super::super::tests::in_gpu_budget_child;
use super::*;
#[test]
fn mandatory_child_target_refuses_before_present_and_recovers() {
    if !in_gpu_budget_child(
        "render_thread::render_pass::composition_targets::tests::mandatory_child_target_refuses_before_present_and_recovers",
    ) {
        return;
    }
    let mut renderer = romacs_renderer_wgpu::gpu_policy::test_gpu::renderer(96, 64).expect("real GPU required");
    let size = SnapshotSize::new(96, 64).unwrap();
    let admitted = mandatory_child_target(&mut renderer, true, size)
        .unwrap()
        .unwrap();
    drop(admitted);
    let held = renderer
        .acquire_snapshot(SnapshotSize::new(512, 506).unwrap())
        .unwrap();
    assert!(matches!(
        mandatory_child_target(&mut renderer, true, size),
        Err(super::super::surface::FrameRenderFailure::WindowNotReady)
    ));
    assert!(
        mandatory_child_target(&mut renderer, false, size)
            .unwrap()
            .is_none()
    );
    drop(held);
    let recovered = mandatory_child_target(&mut renderer, true, size)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.size(), size);
    drop(recovered);
}

