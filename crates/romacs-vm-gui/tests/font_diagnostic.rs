//! Supplemental renderer diagnostic, not a compositor acceptance judge.
use romacs_display_protocol::{
    DeviceScale, GeometrySize, LogicalPixels, PresentMapping, PresentationExtent, SurfaceState,
};
use romacs_renderer_wgpu::WgpuGlyphAtlas;
#[test]
fn scene_projection_uses_zero_emergency_font_lookups() {
    let mut vm = romacs_vm::Vm::new();
    vm.eval_source(include_str!("../scene.el")).unwrap();
    let mut renderer = romacs_renderer_wgpu::gpu_policy::test_gpu::renderer(400, 300).expect("real GPU renderer");
    let mut atlas = WgpuGlyphAtlas::new(renderer.device());
    let target = renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("G1a font diagnostic"),
        size: wgpu::Extent3d {
            width: 400,
            height: 300,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    for presentation in 1..=2 {
        if presentation == 2 {
            vm.eval_source(
                "(with-current-buffer \"*g1a*\" (goto-char (point-max)) (insert \"Δ\"))",
            )
            .unwrap();
        }
        let snapshot = vm.display_snapshot(400, 300).unwrap();
        vm.collect_garbage();
        let frame = romacs_layout::layout(&snapshot, presentation).unwrap();
        atlas.set_current_frame_fonts(frame.font_bindings());
        let SurfaceState::Drawable(surface) =
            SurfaceState::from_device_size(400, 300, DeviceScale::new(1.0).unwrap()).unwrap()
        else {
            panic!("drawable")
        };
        let mapping = PresentMapping::top_left_clip(
            surface,
            PresentationExtent::new(
                frame.presentation_id,
                GeometrySize::<LogicalPixels>::from_px(400.0, 300.0).unwrap(),
            ),
        );
        renderer.render_frame_glyphs(
            &view, &frame, &mut atlas, mapping, false, None, None, None, None, None,
        );
        eprintln!(
            "presentation={presentation} unresolved_face_text_total={}",
            atlas.unresolved_face_text_total()
        );
        assert_eq!(atlas.unresolved_face_text_total(), 0);
    }
}
