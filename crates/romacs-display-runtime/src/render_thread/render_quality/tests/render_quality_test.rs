use super::*;

#[test]
fn backend_profile_projects_wgpu_device_cost_class_once() {
    assert_eq!(
        RenderBackendProfile::from_device_type(wgpu::DeviceType::Cpu).adapter_class(),
        AdapterClass::Software
    );
    for device_type in [
        wgpu::DeviceType::Other,
        wgpu::DeviceType::IntegratedGpu,
        wgpu::DeviceType::DiscreteGpu,
        wgpu::DeviceType::VirtualGpu,
    ] {
        assert_eq!(
            RenderBackendProfile::from_device_type(device_type).adapter_class(),
            AdapterClass::Hardware
        );
    }
}

#[test]
fn software_compat_policy_preserves_request_and_owns_every_feature_decision() {
    let mut requested = VisualConfig::default();
    requested.cursor_motion.enabled = true;
    requested.cursor_size_transition.enabled = true;
    requested.buffer_transition.enabled = true;
    requested.scroll_transition.enabled = true;
    requested.effects.cursor_glow.enabled = true;
    requested.effects.bg_pattern.style = 1;

    let policy = RenderQualityPolicy::negotiate(RenderBackendProfile::software(), &requested);

    assert_eq!(policy.mode(), QualityMode::SoftwareCompatibility);
    assert_eq!(
        policy.dynamic_animation_rate(NonZeroU16::new(144).unwrap()),
        None,
        "software compatibility has a hard zero-cadence bound"
    );
    assert!(!policy.effective_visual_config().cursor_motion.enabled);
    assert!(!policy.effective_visual_config().effects.cursor_glow.enabled);
    assert_eq!(requested.effects.bg_pattern.style, 1);
    assert_eq!(
        policy.frame_post_disposition(),
        FramePostDisposition::SuppressedByQualityPolicy
    );
    assert!(!policy.frame_post_scheduler_active(true, true));
    assert_eq!(
        policy.plan_frame(true, true),
        RenderFeaturePlan {
            compose_offscreen: false,
            accept_transition_hints: false,
            accept_derived_effects: false,
            accept_cursor_effects: false,
            apply_frame_post: false,
        }
    );
}

#[test]
fn full_policy_preserves_requested_features() {
    let requested = VisualConfig::default();
    let policy = RenderQualityPolicy::negotiate(RenderBackendProfile::hardware(), &requested);

    assert_eq!(policy.mode(), QualityMode::Full);
    assert_eq!(policy.effective_visual_config(), &requested);
    assert_eq!(
        policy.frame_post_disposition(),
        FramePostDisposition::Enabled
    );
    assert!(policy.frame_post_scheduler_active(true, true));
    assert!(
        !policy.frame_post_scheduler_active(true, false),
        "an awaiting-content window must not inherit global shader demand"
    );
    assert_eq!(
        policy.dynamic_animation_rate(NonZeroU16::new(144).unwrap()),
        NonZeroU16::new(144)
    );
}

#[test]
fn layers_animate_on_hardware_and_not_on_a_software_adapter() {
    use romacs_display_protocol::LayerAnimation;
    let requested = VisualConfig::default();
    let full = RenderQualityPolicy::negotiate(RenderBackendProfile::hardware(), &requested).child_frame_motion();
    assert!(!full.open.is_instant() && !full.close.is_instant());
    assert!(!full.resize.is_instant() && !full.selection.is_instant());
    assert_eq!(full.open_slide, 8.0);
    // Screenshots and parity under a software adapter must not depend on
    // when a frame was sampled; ROMACS_LAYER_ANIMATION=1 opts a test in.
    if std::env::var_os("ROMACS_LAYER_ANIMATION").is_none() {
        let software = RenderQualityPolicy::negotiate(RenderBackendProfile::software(), &requested);
        assert_eq!(software.child_frame_motion(), ChildFrameMotionSpecs::INSTANT);
    }
    let reduced = full.for_layer(LayerAnimation::Reduced);
    assert_eq!((reduced.open, reduced.close), (full.open, full.close));
    assert_eq!((reduced.open_slide, reduced.close_slide), (0.0, 0.0));
    assert!(reduced.resize.is_instant() && reduced.selection.is_instant() && reduced.movement.is_instant());
    assert_eq!(full.for_layer(LayerAnimation::Off), ChildFrameMotionSpecs::INSTANT);
    assert_eq!(full.for_layer(LayerAnimation::Full), full);
}
