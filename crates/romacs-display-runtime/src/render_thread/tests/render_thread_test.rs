use super::RenderApp;
use super::render_quality::RenderBackendProfile;
use romacs_display_protocol::frame_glyphs::FrameGlyphBuffer;
use romacs_display_protocol::types::DisplayWindowId;
use crate::host_types::GuiFrameGeometryHints;
use crate::thread_comm::{ThreadComms, WindowCommand};
use crate::thread_comm::FrameRef;
use romacs_display_protocol::{
    Color, CursorStyle, DisplaySlotId, EffectsConfig, FrameRate, PhysCursor,
};
use std::sync::{Arc, Mutex};
use winit::keyboard::{Key, NamedKey};

pub(super) fn make_test_app() -> RenderApp {
    let comms = ThreadComms::new();
    let (_emacs, render) = comms.split();
    RenderApp::new(
        render,
        800,
        600,
        "test".to_string(),
        Arc::new(crate::render_thread::ImageRenderState::default()),
        Arc::new((Mutex::new(Vec::new()), std::sync::Condvar::new())),
        true,
    )
}

#[test]
fn deferred_gui_pending_primary_close_rejects_exact_frame_and_recreates() {
    let mut app = make_test_app();
    app.comms.keep_alive_without_frames = true;
    app.frame_windows.adopt_primary_frame_id(0x42);
    let (reply, rx) = crossbeam_channel::bounded(1);
    app.frame_windows.await_ready(0x42, reply);
    assert!(matches!(
        rx.try_recv(),
        Err(crossbeam_channel::TryRecvError::Empty)
    ));
    app.retire_pending_primary();
    assert!(rx.try_recv().unwrap().is_err());
    assert!(app.frame_windows.primary_window().is_none());
    assert!(!app.lifecycle_flags.is_shutting_down());
    app.handle_window(WindowCommand::CreateWindow {
        frame: FrameRef::Frame(0x43),
        width: 901,
        height: 603,
        title: "replacement".into(),
        geometry_hints: GuiFrameGeometryHints {
            base_width: 0,
            base_height: 0,
            min_width: 1,
            min_height: 1,
            width_inc: 1,
            height_inc: 1,
        },
    });
    assert_eq!(app.frame_windows.primary_event_frame_id(), 0x43);
    assert_eq!(app.pending_content_size.width, 901);
    // A stale destroyed frame cannot remove the replacement.
    app.handle_window(WindowCommand::DestroyWindow {
        frame: FrameRef::Frame(0x42),
    });
    assert_eq!(app.frame_windows.primary_event_frame_id(), 0x43);
}

#[test]
fn pending_native_recreation_retains_last_usable_editor_size() {
    let mut content = super::startup::InitialWindowSize {
        width: 664,
        height: 574,
    };
    content.observe_content(901, 603);
    content.observe_content(0, 603);
    content.observe_content(901, 0);
    content.observe_content(0, 0);
    assert_eq!((content.width, content.height), (901, 603));

    let titlebar = romacs_display_protocol::ContentInsets::new(0, 32, 0, 0);
    let surface = titlebar.surface_size(content.width, content.height);
    assert_eq!(surface, (901, 635));
    let observed = titlebar.content_size(surface.0, surface.1);
    content.observe_content(observed.0, observed.1);
    assert_eq!(
        titlebar.surface_size(content.width, content.height),
        surface
    );
}

#[test]
fn child_frames_start_with_square_corners() {
    let app = make_test_app();

    assert_eq!(app.child_frame_style.corner_radius, 0.0);
}

#[test]
fn entering_cpu_compatibility_mode_discards_retained_effect_demand() {
    let mut app = make_test_app();
    let renderer_effects = &mut app
        .frame_windows
        .primary_window_mut()
        .expect("test app has a primary window")
        .render
        .compositor
        .renderer_effects;
    renderer_effects.spawn_ripple(10.0, 20.0);
    assert!(renderer_effects.needs_redraw());

    app.install_backend_profile(RenderBackendProfile::software());

    assert!(
        !app.frame_windows
            .primary_window()
            .expect("test app has a primary window")
            .render
            .compositor
            .renderer_effects
            .needs_redraw(),
        "disabled effects must not retain max-rate scheduler demand"
    );
}

#[test]
fn cursor_color_cycle_rate_respects_effect_and_display_limits() {
    let display_60_hz = std::num::NonZeroU16::new(60).unwrap();

    assert_eq!(
        RenderApp::cursor_color_cycle_rate(FrameRate::new(24).unwrap(), display_60_hz),
        std::num::NonZeroU16::new(24).unwrap()
    );
    assert_eq!(
        RenderApp::cursor_color_cycle_rate(FrameRate::new(144).unwrap(), display_60_hz),
        display_60_hz
    );
}

#[test]
fn reported_display_rate_is_a_hard_cap_even_below_30_hz() {
    assert_eq!(
        RenderApp::display_rate_limit(Some(24_000)),
        std::num::NonZeroU16::new(24).unwrap()
    );
    assert_eq!(
        RenderApp::display_rate_limit(Some(23_976)),
        std::num::NonZeroU16::new(23).unwrap(),
        "the integer scheduler must conservatively floor a fractional display rate"
    );
    assert_eq!(
        RenderApp::cursor_color_cycle_rate(
            FrameRate::new(60).unwrap(),
            RenderApp::display_rate_limit(Some(24_000)),
        ),
        std::num::NonZeroU16::new(24).unwrap()
    );
}

fn frame_with_cursor_effects(
    effects: Option<EffectsConfig>,
    cursor_style: CursorStyle,
) -> FrameGlyphBuffer {
    let window_id = DisplayWindowId::new(7);
    let mut frame = FrameGlyphBuffer::with_size(800.0, 600.0);
    frame.set_phys_cursor(PhysCursor {
        window_id,
        charpos: 0,
        row: 0,
        col: 0,
        slot_id: DisplaySlotId::ZERO,
        x: 0.0,
        y: 0.0,
        width: 8.0,
        height: 16.0,
        ascent: 12.0,
        style: cursor_style,
        color: Color::WHITE,
        cursor_fg: Color::BLACK,
    });
    if let Some(effects) = effects {
        frame.set_window_cursor_effects(window_id, effects);
    }
    frame
}

#[test]
fn s1b_empty_ime_clear_preserves_clean_and_dirty_state() {
    let mut app = make_test_app();
    let render = &mut app.frame_windows.primary_window_mut().unwrap().render;
    assert!(!render.has_ime_preedit());
    for dirty in [false, true] {
        render.set_dirty(dirty);
        for _ in 0..8 {
            render.clear_ime_preedit();
            assert!(!render.has_ime_preedit());
            assert_eq!(render.compositor.dirty, dirty);
        }
    }
}

#[test]
fn s1b_clearing_present_ime_preedit_invalidates_once() {
    let mut app = make_test_app();
    let render = &mut app.frame_windows.primary_window_mut().unwrap().render;
    render.set_ime_preedit("composition".into(), Some((0, 3)));
    assert!(render.has_ime_preedit());
    render.set_dirty(false);
    render.clear_ime_preedit();
    assert!(!render.has_ime_preedit());
    assert!(render.compositor.dirty);
    render.set_dirty(false);
    render.clear_ime_preedit();
    assert!(!render.compositor.dirty);
}

#[test]
fn s1b_hidden_cursor_repeated_poll_has_no_dirty_content_with_default_effects() {
    const CHILD: &str = "ROMACS_S1B_IDLE_POLL_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "render_thread::runtime_tests::s1b_hidden_cursor_repeated_poll_has_no_dirty_content_with_default_effects",
            ])
            .env(CHILD, "1")
            .env_remove("ROMACS_PRESENT_TRACE")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed;"));
        return;
    }
    let mut app = make_test_app();
    let frame = FrameGlyphBuffer::with_size(800.0, 600.0);
    assert!(frame.active_cursor().is_none());
    let render = &mut app.frame_windows.primary_window_mut().unwrap().render;
    render.set_current_frame(Some(frame), None, Default::default(), Default::default());
    render.set_dirty(false);
    let before = super::frame_stats::snapshot();
    for iteration in 0..2400 {
        app.refresh_frame_state();
        let render = &app.frame_windows.primary_window().unwrap().render;
        assert!(render.compositor.current_frame.is_some());
        assert!(render.cursor.target_cloned().is_none());
        assert!(
            !render.compositor.dirty,
            "idle poll {iteration} dirtied the frame"
        );
        assert!(!render.has_presentable_dirty_content());
    }
    let after = super::frame_stats::snapshot();
    assert_eq!(after.redraw_requests, before.redraw_requests);
    assert_eq!(after.presents, before.presents);
    assert_eq!(after.root_glyph_passes, before.root_glyph_passes);
}

#[test]
fn s1b_visible_cursor_poll_preserves_preedit_and_default_color_cycle() {
    let mut app = make_test_app();
    let frame = frame_with_cursor_effects(None, CursorStyle::FilledBox);
    let render = &mut app.frame_windows.primary_window_mut().unwrap().render;
    render.set_current_frame(Some(frame), None, Default::default(), Default::default());
    app.refresh_frame_state();
    let render = &mut app.frame_windows.primary_window_mut().unwrap().render;
    render.set_ime_preedit("composition".into(), Some((0, 3)));
    render.set_dirty(false);
    for _ in 0..8 {
        app.refresh_frame_state();
        let render = &app.frame_windows.primary_window().unwrap().render;
        assert!(render.cursor.target_cloned().is_some());
        assert!(render.has_ime_preedit());
        assert!(!render.compositor.dirty);
        assert_eq!(
            RenderApp::cursor_color_cycle_cadence(
                render.compositor.current_frame.as_ref().unwrap(),
                &EffectsConfig::default(),
                std::num::NonZeroU16::new(240).unwrap(),
            ),
            std::num::NonZeroU16::new(24),
        );
    }
}

#[test]
fn cursor_color_cycle_cadence_follows_the_effective_frame_profile() {
    let display_60_hz = std::num::NonZeroU16::new(60).unwrap();
    let mut global = EffectsConfig::default();
    global.cursor_color_cycle.enabled = false;
    let frame = frame_with_cursor_effects(None, CursorStyle::FilledBox);
    assert_eq!(
        RenderApp::cursor_color_cycle_cadence(&frame, &global, display_60_hz),
        None
    );

    let mut local = EffectsConfig::cursor_profile_baseline();
    local.cursor_color_cycle.enabled = true;
    local.cursor_color_cycle.fps = FrameRate::new(12).unwrap();
    let frame = frame_with_cursor_effects(Some(local), CursorStyle::FilledBox);
    assert_eq!(
        RenderApp::cursor_color_cycle_cadence(&frame, &global, display_60_hz),
        std::num::NonZeroU16::new(12)
    );

    global.cursor_color_cycle.enabled = true;
    let frame = frame_with_cursor_effects(
        Some(EffectsConfig::cursor_profile_baseline()),
        CursorStyle::FilledBox,
    );
    assert_eq!(
        RenderApp::cursor_color_cycle_cadence(&frame, &global, display_60_hz),
        None
    );
}

#[test]
fn cursor_color_cycle_cadence_pauses_when_the_cycle_cannot_change_pixels() {
    let display_60_hz = std::num::NonZeroU16::new(60).unwrap();
    let global = EffectsConfig::default();
    // blink-cursor-mode's off phase publishes no cursor (internal-show-cursor).
    let mut off = frame_with_cursor_effects(None, CursorStyle::FilledBox);
    off.window_cursors.clear();
    assert_eq!(
        RenderApp::cursor_color_cycle_cadence(&off, &global, display_60_hz),
        None,
        "a frame whose cursor is shown off has no visible cycle work"
    );

    let hollow = frame_with_cursor_effects(None, CursorStyle::Hollow);
    assert_eq!(
        RenderApp::cursor_color_cycle_cadence(&hollow, &global, display_60_hz),
        None,
        "the renderer deliberately does not color-cycle hollow cursors"
    );
}

#[test]
fn cursor_color_cycle_cadence_covers_every_rendered_window_cursor() {
    let display_60_hz = std::num::NonZeroU16::new(60).unwrap();
    let global = EffectsConfig::cursor_profile_baseline();

    let mut selected = EffectsConfig::cursor_profile_baseline();
    selected.cursor_color_cycle.enabled = true;
    selected.cursor_color_cycle.fps = FrameRate::new(12).unwrap();
    let mut frame = frame_with_cursor_effects(Some(selected), CursorStyle::FilledBox);

    let decorative_window = DisplayWindowId::new(8);
    frame.add_cursor(
        decorative_window,
        80.0,
        0.0,
        2.0,
        16.0,
        CursorStyle::Bar(2.0),
        Color::WHITE,
    );
    let mut decorative = EffectsConfig::cursor_profile_baseline();
    decorative.cursor_color_cycle.enabled = true;
    decorative.cursor_color_cycle.fps = FrameRate::new(24).unwrap();
    frame.set_window_cursor_effects(decorative_window, decorative);

    assert_eq!(
        RenderApp::cursor_color_cycle_cadence(&frame, &global, display_60_hz),
        std::num::NonZeroU16::new(24),
        "demand must use the fastest visible cursor rendered in a split frame"
    );

    frame.active_cursor_mut().unwrap().style = CursorStyle::Hollow;
    assert_eq!(
        RenderApp::cursor_color_cycle_cadence(&frame, &global, display_60_hz),
        std::num::NonZeroU16::new(24),
        "a hollow selected cursor must not suppress a cycling decorative cursor"
    );
}

#[test]
fn cursor_color_cycle_state_translates_to_an_exact_scheduler_demand() {
    use super::frame_sched::{Cadence, DemandReason, FrameDemand, Invalidation, LayerMask};

    let display_60_hz = std::num::NonZeroU16::new(60).unwrap();
    let global = EffectsConfig::default();
    let frame = frame_with_cursor_effects(None, CursorStyle::FilledBox);
    let mut off = frame.clone();
    off.window_cursors.clear();
    let expected = FrameDemand {
        invalidation: Invalidation::CompositeOnly {
            layers: LayerMask::CURSOR_EFFECTS,
        },
        cadence: Cadence::MaxRate(std::num::NonZeroU16::new(24).unwrap()),
        reason: DemandReason::CursorColorCycle,
    };

    assert_eq!(
        RenderApp::cursor_color_cycle_demand(&frame, &global, display_60_hz, true),
        Some(expected)
    );
    assert_eq!(
        RenderApp::cursor_color_cycle_demand(&off, &global, display_60_hz, true),
        None,
        "a cursor shown off must retract standing demand"
    );
    assert_eq!(
        RenderApp::cursor_color_cycle_demand(&frame, &global, display_60_hz, false),
        None,
        "unfocused windows must retract standing demand"
    );
}

#[test]
fn cursor_color_cycle_reconciliation_drives_attributed_frames_and_retracts() {
    use super::frame_sched::{
        ClockSource, DemandReason, FrameCoordinator, FrameTick, NativeWindowId, PacingAction,
        RenderWork,
    };

    let mut coordinator = FrameCoordinator::new();
    let id = NativeWindowId(7);
    let now = romacs_display_protocol::frame_time::observe_platform_now();
    let display_60_hz = std::num::NonZeroU16::new(60).unwrap();
    let global = EffectsConfig::default();
    let frame = frame_with_cursor_effects(None, CursorStyle::FilledBox);
    let mut off = frame.clone();
    off.window_cursors.clear();
    let tick = |at| FrameTick {
        frame_time: at,
        target_presentation_time: at,
        estimated_interval: std::time::Duration::from_secs_f64(1.0 / 60.0),
        source: ClockSource::Synthetic,
    };

    assert_eq!(
        RenderApp::reconcile_cursor_color_cycle_demand(
            &mut coordinator,
            id,
            Some(&frame),
            &global,
            display_60_hz,
            now,
        ),
        PacingAction::RequestRedraw
    );
    let plan = coordinator.begin_frame(id, tick(now));
    assert_eq!(
        plan.work,
        RenderWork::CompositeOnly {
            layers: super::frame_sched::LayerMask::CURSOR_EFFECTS,
        }
    );
    assert!(plan.reasons.contains(DemandReason::CursorColorCycle));
    assert!(!plan.reasons.contains(DemandReason::PlatformRedraw));

    assert!(matches!(
        RenderApp::reconcile_cursor_color_cycle_demand(
            &mut coordinator,
            id,
            Some(&frame),
            &global,
            display_60_hz,
            now.plus(std::time::Duration::from_millis(1)),
        ),
        PacingAction::WakeAt(_)
    ));
    assert_eq!(
        RenderApp::reconcile_cursor_color_cycle_demand(
            &mut coordinator,
            id,
            Some(&off),
            &global,
            display_60_hz,
            now.plus(std::time::Duration::from_millis(2)),
        ),
        PacingAction::Sleep,
        "a cursor shown off must withdraw its standing deadline"
    );
    assert!(coordinator.active_reasons(id).is_empty());
    assert_eq!(coordinator.next_wake_deadline_unserviced(), None);

    assert_eq!(
        RenderApp::reconcile_cursor_color_cycle_demand(
            &mut coordinator,
            id,
            Some(&frame),
            &global,
            display_60_hz,
            now.plus(std::time::Duration::from_millis(3)),
        ),
        PacingAction::RequestRedraw
    );
    let _ = coordinator.begin_frame(id, tick(now.plus(std::time::Duration::from_millis(3))));
    assert!(matches!(
        RenderApp::reconcile_cursor_color_cycle_demand(
            &mut coordinator,
            id,
            Some(&frame),
            &global,
            display_60_hz,
            now.plus(std::time::Duration::from_millis(4)),
        ),
        PacingAction::WakeAt(_)
    ));
    coordinator.set_focused(id, false);
    assert_eq!(
        RenderApp::reconcile_cursor_color_cycle_demand(
            &mut coordinator,
            id,
            Some(&frame),
            &global,
            display_60_hz,
            now.plus(std::time::Duration::from_millis(5)),
        ),
        PacingAction::Sleep,
        "an unfocused window must withdraw its standing deadline"
    );
    assert!(coordinator.active_reasons(id).is_empty());
    assert_eq!(coordinator.next_wake_deadline_unserviced(), None);
}

fn presentation_frame(frame_id: u64, parent_id: u64, presentation: u64) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(800.0, 600.0);
    frame.presentation_id = romacs_display_protocol::PresentationId::new(presentation);
    frame.set_frame_identity(
        romacs_display_protocol::DisplayFrameId::new(frame_id),
        romacs_display_protocol::DisplayFrameId::new(parent_id),
        0.0,
        0.0,
        0,
        false,
        0.0,
        romacs_display_protocol::Color::BLACK,
        false,
        1.0,
    );
    frame
}

fn make_test_device() -> Option<wgpu::Device> {
    romacs_renderer_wgpu::gpu_policy::test_gpu::device(&wgpu::DeviceDescriptor {
        label: Some("render-thread test device"),
        ..Default::default()
    })
    .map(|(device, _queue)| device)
}

#[test]
fn test_translate_key_named() {
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::Escape)),
        0xff1b
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::Enter)),
        0xff0d
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::Tab)),
        0xff09
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::Backspace)),
        0xff08
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::Delete)),
        0xffff
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::Home)),
        0xff50
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::End)),
        0xff57
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::PageUp)),
        0xff55
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::PageDown)),
        0xff56
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::ArrowLeft)),
        0xff51
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::ArrowUp)),
        0xff52
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::ArrowRight)),
        0xff53
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::ArrowDown)),
        0xff54
    );
    assert_eq!(
        RenderApp::translate_key_input(&Key::Character(" ".into())),
        Some(crate::host_types::FrontendKey::Character(' '))
    );
}

#[test]
fn test_translate_key_character() {
    assert_eq!(
        RenderApp::translate_key_input(&Key::Character("a".into())),
        Some(crate::host_types::FrontendKey::Character('a'))
    );
    assert_eq!(
        RenderApp::translate_key_input(&Key::Character("A".into())),
        Some(crate::host_types::FrontendKey::Character('A'))
    );
    assert_eq!(
        RenderApp::translate_key_input(&Key::Character("1".into())),
        Some(crate::host_types::FrontendKey::Character('1'))
    );
}

#[test]
fn test_translate_key_function_keys() {
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::F1)),
        0xffbe
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::F12)),
        0xffc9
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::Insert)),
        0xff63
    );
    assert_eq!(
        RenderApp::translate_keysym(&Key::Named(NamedKey::PrintScreen)),
        0xff61
    );
}

#[test]
fn test_translate_key_unknown() {
    assert_eq!(RenderApp::translate_keysym(&Key::Dead(None)), 0);
}

#[test]
fn test_render_thread_creation() {
    let comms = ThreadComms::new();
    let (emacs, render) = comms.split();

    assert!(emacs.input_rx.is_empty());
    assert!(render.cmd_rx.is_empty());
}

#[test]
fn destroy_primary_window_command_prevents_lifecycle_recreate() {
    let mut app = make_test_app();
    app.frame_windows.adopt_primary_frame_id(0x1000);

    app.handle_window(WindowCommand::DestroyWindow {
        frame: FrameRef::Primary,
    });

    assert!(app.frame_windows.primary_window().is_none());
    assert!(
        app.frame_windows
            .primary_window()
            .map(|ws| &ws.render)
            .is_none()
    );
    assert!(
        app.frame_windows
            .primary_window()
            .and_then(|ws| ws.render.compositor.current_frame.as_ref())
            .is_none()
    );
    assert!(
        !app.frame_windows
            .primary_window()
            .is_some_and(|ws| ws.render.compositor.dirty)
    );
    assert!(app.frame_windows.primary_window().is_none());
    assert_eq!(app.frame_windows.primary_frame_id(), None);
}

#[test]
fn destroy_adopted_primary_window_by_real_frame_id_prevents_lifecycle_recreate() {
    let mut app = make_test_app();
    app.frame_windows.adopt_primary_frame_id(0x1000);

    app.handle_window(WindowCommand::DestroyWindow {
        frame: FrameRef::Frame(0x1000),
    });

    assert!(app.frame_windows.primary_window().is_none());
    assert!(
        app.frame_windows
            .primary_window()
            .map(|ws| &ws.render)
            .is_none()
    );
    assert!(
        !app.frame_windows
            .primary_window()
            .is_some_and(|ws| ws.render.compositor.dirty)
    );
    assert!(app.frame_windows.primary_window().is_none());
    assert_eq!(app.frame_windows.primary_frame_id(), None);
    assert!(app.frame_windows.pending_destroys.is_empty());
}

#[test]
fn pending_dirty_primary_window_is_not_redrawable_active_work() {
    let mut app = make_test_app();
    let primary = app.frame_windows.primary_window_mut().unwrap();
    primary.render.compositor.dirty = true;

    assert!(
        app.frame_windows
            .primary_window()
            .unwrap()
            .render
            .compositor
            .dirty
    );
    assert!(
        !app.frame_windows
            .windows
            .values()
            .any(|window_state| window_state.has_presentable_dirty_content()),
        "a pending window has no native surface to receive RedrawRequested"
    );
}

#[test]
fn adopt_primary_window_command_updates_existing_primary_render_state_identity() {
    let mut app = make_test_app();
    let Some(device) = make_test_device() else {
        return;
    };
    let __render = super::frame_windows::GuiFrameRenderState::new(
        0,
        &device,
        app.frame_windows
            .primary_window()
            .map_or(1.0, |ws| ws.scale_factor()),
        app.frame_windows.fps_enabled,
        romacs_display_protocol::frame_time::observe_platform_now(),
    );
    if let Some(window_state) = app.frame_windows.primary_window_mut() {
        window_state.render = __render;
    }

    app.handle_window(WindowCommand::AdoptPrimaryFrame {
        frame: FrameRef::Frame(0x1000),
    });

    assert_eq!(app.frame_windows.primary_frame_id(), Some(0x1000));
    assert_eq!(
        app.frame_windows
            .primary_window()
            .map(|ws| &ws.render)
            .map(|frame| frame.emacs_frame_id),
        Some(0x1000)
    );
}

#[test]
fn keyboard_heading_switch_uses_controller_identity_and_wraps() {
    use romacs_display_protocol::frame_chrome::*;
    let mut app = make_test_app();
    let items = ["help-menu", "lisp-interaction"]
        .iter()
        .enumerate()
        .map(|(index, key)| {
            romacs_display_protocol::frame_chrome::PositionedMenuHeading::measure(
                romacs_display_protocol::MenuBarItem {
                    index: index as u32,
                    key: (*key).into(),
                    label: (*key).into(),
                },
                index as f32 * 100.0,
                18.0,
                0.0,
                |_| romacs_display_protocol::frame_chrome::MenuHeadingText::Cells { width: 100.0 },
            )
        })
        .collect();
    let mut frame = FrameGlyphBuffer::with_size(800.0, 600.0);
    frame.frame_chrome = FrameChrome::layout(
        FrameSize::new(800.0, 600.0).unwrap(),
        vec![ChromeBandRequest::new(
            FrameChromeKind::MenuBar,
            18.0,
            FrameChromeContent::MenuBar(MenuBarContent::new(items, Color::WHITE, Color::BLACK)),
        )],
    )
    .unwrap();
    let window = app.frame_windows.primary_window_mut().unwrap();
    let owner = window.render.emacs_frame_id;
    window.render.compositor.current_frame = Some(frame);
    app.menu_bar.select(
        crate::menu_bar::MenuHeading {
            frame: owner,
            parent: winit::window::WindowId::from_raw(1),
            key: "help-menu".into(),
            index: 0,
            compact: false,
        },
        false,
    );
    assert!(app.switch_native_menu_heading(-1));
    assert_eq!(app.menu_bar.heading().unwrap().key, "lisp-interaction");
    assert!(app.switch_native_menu_heading(1));
    assert_eq!(app.menu_bar.heading().unwrap().key, "help-menu");
    let heading = app.menu_bar.heading().unwrap().clone();
    assert_eq!(
        app.menu_bar.select(heading, false),
        crate::menu_bar::HeadingAction::Keep
    );
}

#[test]
fn adopted_primary_pointer_target_uses_real_frame_id() {
    let mut app = make_test_app();
    let Some(device) = make_test_device() else {
        return;
    };
    let __render = super::frame_windows::GuiFrameRenderState::new(
        0,
        &device,
        app.frame_windows
            .primary_window()
            .map_or(1.0, |ws| ws.scale_factor()),
        app.frame_windows.fps_enabled,
        romacs_display_protocol::frame_time::observe_platform_now(),
    );
    if let Some(window_state) = app.frame_windows.primary_window_mut() {
        window_state.render = __render;
    }
    app.frame_windows.adopt_primary_frame_id(0x1000);
    if let Some(ws) = app.frame_windows.primary_window_mut() {
        ws.render.set_current_frame(
            Some(FrameGlyphBuffer::with_size(800.0, 600.0)),
            None,
            Default::default(),
            Default::default(),
        );
    };

    let (x, y, frame_id) = app.pointer_target_at(12.0, 34.0);

    assert_eq!((x, y), (12.0, 34.0));
    assert_eq!(frame_id, 0x1000);
}

#[test]
fn installing_frame_emits_activation_before_replaced_presentation_retirement() {
    let comms = ThreadComms::new();
    let (emacs, render) = comms.split();
    let mut app = RenderApp::new(
        render,
        800,
        600,
        "test".to_string(),
        Arc::new(crate::render_thread::ImageRenderState::default()),
        Arc::new((Mutex::new(Vec::new()), std::sync::Condvar::new())),
        true,
    );
    let primary = crate::paint_host::PRIMARY_FRAME;
    app.frame_windows.adopt_primary_frame_id(primary);

    app.ingest_paint(presentation_frame(primary, 0, 41).into());
    app.refresh_frame_state();
    let events = emacs.input_rx.try_iter().collect::<Vec<_>>();
    assert!(matches!(
        events.as_slice(),
        [crate::thread_comm::InputEvent::PresentationActivated {
            presentation: 41,
            emacs_frame_id,
        }] if *emacs_frame_id == primary
    ));

    app.ingest_paint(presentation_frame(primary, 0, 42).into());
    app.refresh_frame_state();
    let events = emacs.input_rx.try_iter().collect::<Vec<_>>();
    assert!(matches!(
        events.as_slice(),
        [
            crate::thread_comm::InputEvent::PresentationActivated {
                presentation: 42,
                emacs_frame_id,
            },
            crate::thread_comm::InputEvent::PresentationRetired { presentation: 41 },
        ] if *emacs_frame_id == primary
    ));
}

#[test]
fn reduced_motion_ends_the_layer_animations_already_running() {
    use romacs_display_protocol::motion_spec::{MotionDuration, MotionSpec, TweenSpec};
    let mut app = make_test_app();
    app.install_backend_profile(RenderBackendProfile::hardware());
    let origin = romacs_display_protocol::frame_time::observe_platform_now();
    let spec = MotionSpec::Tween(TweenSpec {
        duration: MotionDuration::new(std::time::Duration::from_millis(100)).expect("positive duration"),
        easing: romacs_display_protocol::scroll_animation::TransitionEasing::EaseOutQuad,
        bezier: None,
    });
    fn manager(app: &mut RenderApp) -> &mut super::child_frames::ChildFrameManager {
        &mut app.frame_windows.primary_window_mut().expect("test app has a primary window").render.compositor.child_frames
    }
    for (id, parent, z) in [(u64::MAX, 0, i32::MIN), (7, u64::MAX, 0)] {
        let mut frame = FrameGlyphBuffer::with_size(100.0, 100.0);
        frame.set_frame_identity(
            romacs_display_protocol::DisplayFrameId::new(id),
            romacs_display_protocol::DisplayFrameId::new(parent),
            0.0, 0.0, z, false, 0.0, romacs_display_protocol::Color::BLACK, false, 1.0,
        );
        manager(&mut app).update_frame(frame);
    }
    manager(&mut app).begin_open_animation(7, spec, origin, 8.0, 1.0);
    assert!(manager(&mut app).has_animation_activity(), "a layer opening");

    app.requested_visual_config.motion.reduced = true;
    app.apply_requested_visual_config();
    assert!(!manager(&mut app).has_animation_activity(), "reduced motion ends it where Lisp put it");
}

#[test]
fn reduced_motion_settles_a_pane_resize_already_running() {
    use super::frame_compositor::layout_driver::{LayoutDelta, LayoutDriver};
    use romacs_display_protocol::frame_glyphs::WindowInfo;
    use romacs_display_protocol::motion_spec::{MotionDuration, MotionSpec, TweenSpec};
    use romacs_display_protocol::types::Rect;
    let mut app = make_test_app();
    app.install_backend_profile(RenderBackendProfile::hardware());
    let window = |id: i64, bounds: Rect| WindowInfo {
        window_id: DisplayWindowId::new(id),
        buffer_id: id as u64 + 100,
        window_start: 1,
        window_end: 100,
        buffer_size: 1000,
        buffer_modiff: romacs_display_protocol::presentation_origin::BufferModiff::new(1),
        bounds,
        geometry: romacs_display_protocol::PresentedWindowGeometry::default(),
        line_number_field: None,
        mode_line_height: 20.0,
        header_line_height: 0.0,
        tab_line_height: 0.0,
        selected: false,
        is_minibuffer: false,
        char_height: 16.0,
        buffer_name: String::from("scratch"),
        buffer_file_name: String::new(),
        modified: false,
    };
    let tween = MotionSpec::Tween(TweenSpec {
        duration: MotionDuration::new(std::time::Duration::from_millis(100)).expect("positive duration"),
        easing: romacs_display_protocol::scroll_animation::TransitionEasing::Linear,
        bezier: None,
    });
    let specs = super::render_quality::WindowAnimationSpecs { resize: tween, movement: tween, open: tween, close: tween };
    let one = vec![window(1, Rect::new(0.0, 0.0, 800.0, 600.0))];
    let two = vec![window(1, Rect::new(0.0, 0.0, 400.0, 600.0)), window(2, Rect::new(400.0, 0.0, 400.0, 600.0))];
    let driver = LayoutDriver::default().on_commit(
        LayoutDelta { previous: &one, next: &two },
        specs,
        romacs_display_protocol::frame_time::observe_platform_now(),
    );
    assert!(driver.wants_frames(), "a split resizing the panes");
    // A frame drawn mid-morph publishes where its panes were.
    let presentation = romacs_display_protocol::PresentationId::new(1);
    let sample = romacs_display_protocol::frame_time::FrameSample::new(
        romacs_display_protocol::frame_time::observe_platform_now().plus(std::time::Duration::from_millis(50)),
        std::time::Duration::ZERO,
    );
    let grid = super::frame_compositor::continuity::pane_layout::PixelGrid::new(1.0);
    let (driver, composition) = driver.on_frame(presentation, sample, grid);
    assert!(composition.projection.is_some(), "a mid-morph projection");
    let render = &mut app.frame_windows.primary_window_mut().expect("test app has a primary window").render;
    render.compositor.layout = driver;
    render.publish_submitted_projection(composition.projection);

    app.requested_visual_config.motion.reduced = true;
    app.apply_requested_visual_config();
    let render = &mut app.frame_windows.primary_window_mut().expect("primary").render;
    assert!(!render.compositor.layout.wants_frames(), "reduced motion lands the panes where the presentation puts them");
    // The next submitted frame is settled: clicks map by identity again.
    render.publish_submitted_projection(None);
    assert!(render.compositor.interaction.is_none(), "no mid-morph mapping left");
}

#[test]
fn clipboard_command_before_display_initialization_returns_an_explicit_error() {
    let comms = ThreadComms::new();
    let (emacs, render) = comms.split();
    let mut app = RenderApp::new(
        render,
        800,
        600,
        "test".to_string(),
        Arc::new(crate::render_thread::ImageRenderState::default()),
        Arc::new((Mutex::new(Vec::new()), std::sync::Condvar::new())),
        true,
    );
    let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
    emacs
        .cmd_tx
        .send(crate::thread_comm::RenderCommand::Clipboard(
            crate::thread_comm::ClipboardCommand::GetText {
                selection: crate::thread_comm::ClipboardSelection::Clipboard,
                expires_at: std::time::Instant::now() + std::time::Duration::from_secs(5),
                reply: reply_tx,
            },
        ))
        .unwrap();

    assert!(!app.process_commands());
    assert_eq!(
        reply_rx.recv().unwrap(),
        Err("clipboard is unavailable before display initialization".to_owned())
    );
}

#[test]
fn a_blinked_off_cursor_keeps_the_drawn_ime_preedit() {
    let comms = ThreadComms::new();
    let (_emacs, render) = comms.split();
    let mut app = RenderApp::new(render, 800, 600, "test".to_string(),
        Arc::new(crate::render_thread::ImageRenderState::default()),
        Arc::new((Mutex::new(Vec::new()), std::sync::Condvar::new())), true);
    let id = crate::paint_host::PRIMARY_FRAME;
    let frame = |presentation: u64, blinked_off: bool| {
        let mut frame = frame_with_cursor_effects(None, CursorStyle::FilledBox);
        frame.presentation_id = romacs_display_protocol::PresentationId::new(presentation);
        frame.set_frame_identity(romacs_display_protocol::DisplayFrameId::new(id), romacs_display_protocol::DisplayFrameId::new(0),
            0.0, 0.0, 0, false, 0.0, romacs_display_protocol::Color::BLACK, false, 1.0);
        if blinked_off {
            frame.window_cursors.clear();
            frame.cursor_blinked_off = true;
        }
        frame
    };
    let drawn = |app: &RenderApp| {
        let render = &app.frame_windows.primary_window().unwrap().render;
        crate::render_thread::render_pass::chrome::transient_overlays::ime_preedit_overlay(render)
            .map(|overlay| (overlay.text.to_owned(), overlay.x, overlay.y, overlay.height))
    };
    // Composing: the cursor's frame places the IME and its panel.
    app.frame_windows.primary_window_mut().unwrap().render.set_ime_preedit("composition".into(), Some((0, 3)));
    app.ingest_paint(frame(1, false).into());
    app.refresh_frame_state();
    let shown = drawn(&app);
    let area = app.frame_windows.primary_window().unwrap().lifecycle.last_ime_cursor_area();
    assert!(shown.is_some() && area.is_some(), "the cursor placed the IME and its panel");
    app.ingest_paint(frame(2, true).into());
    app.refresh_frame_state();
    let window = app.frame_windows.primary_window().unwrap();
    assert!(RenderApp::cursor_blinked_off(&window.render), "the blink-off frame is current");
    assert!(window.render.compositor.current_frame.as_ref().unwrap().active_cursor().is_none(), "no cursor is painted");
    assert_eq!(drawn(&app), shown, "the pre-edit panel draws as before the blink");
    assert_eq!(app.frame_windows.primary_window().unwrap().lifecycle.last_ime_cursor_area(), area, "the IME keeps its anchor");
}

#[test]
fn backend_recovery_rederives_quality_from_the_unchanged_request() {
    let mut app = make_test_app();
    app.requested_visual_config.cursor_motion.enabled = true;
    let requested = app.requested_visual_config.clone();

    app.install_backend_profile(RenderBackendProfile::hardware());
    assert_eq!(
        app.render_policy.effective_visual_config(),
        &super::render_quality::with_motion(requested.clone())
    );

    app.install_backend_profile(RenderBackendProfile::software());
    assert!(app.requested_visual_config.cursor_motion.enabled);
    assert!(
        !app.render_policy
            .effective_visual_config()
            .cursor_motion
            .enabled
    );

    app.install_backend_profile(RenderBackendProfile::hardware());
    assert_eq!(
        app.render_policy.effective_visual_config(),
        &super::render_quality::with_motion(requested.clone())
    );
}
