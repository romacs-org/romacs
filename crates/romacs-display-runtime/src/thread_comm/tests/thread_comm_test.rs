use super::*;
use romacs_display_protocol::frame_glyphs::FrameGlyphBuffer;
use romacs_display_protocol::glyph_matrix::FrameDisplayState;
use romacs_display_protocol::{
    ImageId, ImageStateEvent, SealedFramePresentation,
};

#[test]
fn native_repeat_tracking_does_not_fill_command_completion_ledger() {
    let (emacs, render) = ThreadComms::new().split();
    let mut progress = romacs_display_protocol::input_progress::InputProgress::default();
    let _command = progress.begin_command();
    // A single interactive command may read held-key input indefinitely.
    for _ in 0..1300 {
        let (observer, _, _) = render.send_key_input_with_receipt(InputEvent::Key {
            key: crate::host_types::FrontendKey::Character('p'),
            modifiers: 0,
            pressed: true,
            emacs_frame_id: 1,
        });
        let observer = observer.expect("every native key needs a read receipt");
        assert!(!observer.consumed_or_cancelled());
        let InputEvent::Tracked { receipt, .. } = emacs.input_rx.try_recv().unwrap() else {
            panic!("native key must retain read ownership through the bridge");
        };
        progress.consumed(receipt);
        assert!(observer.consumed_or_cancelled());
    }
    assert!(progress.current_command_receipts().is_empty());
}

#[test]
fn native_page_keys_keep_read_and_scroll_completion_receipts_separate() {
    let (emacs, render) = ThreadComms::new().split();
    let (read, completion, _) = render.send_key_input_with_receipt(InputEvent::Key {
        key: crate::host_types::FrontendKey::Keysym(0xff55),
        modifiers: 0,
        pressed: true,
        emacs_frame_id: 1,
    });
    let read = read.unwrap();
    let completion = completion.unwrap();
    assert!(!read.same_input(&completion));
    let mut progress = romacs_display_protocol::input_progress::InputProgress::default();
    let command = progress.begin_command();
    let InputEvent::Tracked { receipt, event } = emacs.input_rx.try_recv().unwrap() else {
        panic!("missing native read receipt");
    };
    progress.consumed(receipt);
    let InputEvent::Tracked { receipt, .. } = *event else {
        panic!("missing scroll completion receipt");
    };
    progress.consumed(receipt);
    assert!(read.consumed_or_cancelled());
    assert!(!completion.acknowledged_by(&progress.checkpoint()));
    assert_eq!(progress.current_command_receipts().len(), 1);
    assert!(progress.current_command_receipts()[0].same_input(&completion));
    drop(command);
    assert!(completion.acknowledged_by(&progress.checkpoint()));
    assert!(!read.acknowledged_by(&progress.checkpoint()));
}

#[test]
fn accepted_opacity_is_independent_of_superseded_visual_mailbox() {
    let (emacs, render) = ThreadComms::new().split();
    emacs.frame_opacity.lock().unwrap().accept(1, [0.5; 2], 0.2);
    let mut numeric = FrameDisplayState::new(12, 4, 8.0, 16.0);
    numeric.frame_alpha = [0.5; 2];
    assert!(
        emacs
            .frame_tx
            .submit(sealed_test_state(numeric))
            .unwrap()
            .is_none()
    );
    emacs
        .frame_opacity
        .lock()
        .unwrap()
        .accept(1, [-1.0; 2], 0.2);
    let mut nil = FrameDisplayState::new(12, 4, 8.0, 16.0);
    nil.frame_alpha = [-1.0; 2];
    assert!(
        emacs
            .frame_tx
            .submit(sealed_test_state(nil))
            .unwrap()
            .is_some()
    );
    let latest = render.frame_rx.try_recv().unwrap();
    assert_eq!(latest.frame_alpha, [-1.0; 2]);
    assert_eq!(render.frame_opacity.lock().unwrap().applied(1), Some(0.5));
}

fn sealed_test_state(mut state: FrameDisplayState) -> SealedFramePresentation {
    if state.presentation_id == romacs_display_protocol::PresentationId::default() {
        state.presentation_id = romacs_display_protocol::PresentationId::new(1);
    }
    state.frame_placement = romacs_display_protocol::PresentedFramePlacement::new(
        romacs_display_protocol::DisplayFrameId::new(1),
        state.presentation_id,
        None,
        romacs_display_protocol::ParentFrameRect::new(0.0, 0.0, 1.0, 1.0).unwrap(),
        0,
    );
    state.presented_hit_index = romacs_display_protocol::PresentedHitIndex::from_parts(
        state.presentation_id,
        vec![],
        vec![],
    )
    .unwrap();
    SealedFramePresentation::seal(state).unwrap()
}

fn unpresented_pointer(x: f32, y: f32, target_frame_id: u64, action: PointerAction) -> InputEvent {
    InputEvent::PositionedPointer(PositionedPointerInput {
        position: PointerPosition {
            x,
            y,
            target_frame_id,
        },
        target: PointerTarget::Unpresented,
        action,
    })
}

// ===================================================================
// Constants
// ===================================================================

#[test]
fn channel_capacity_constants() {
    assert_eq!(INPUT_CHANNEL_CAPACITY, 4096);
    assert_eq!(COMMAND_CHANNEL_CAPACITY, 64);
}

// ===================================================================
// ThreadComms
// ===================================================================

#[test]
fn thread_comms_new_constructs_all_channels() {
    let (_emacs, _render) = ThreadComms::new().split();
}

#[test]
fn thread_comms_input_channel_roundtrip() {
    let comms = ThreadComms::new();

    let event = InputEvent::Key {
        key: crate::host_types::FrontendKey::Keysym(65), // 'A'
        modifiers: 0,
        pressed: true,
        emacs_frame_id: 0,
    };

    comms.input_tx.send(event.clone()).unwrap();

    let received = comms.input_rx.try_recv().unwrap();
    match received {
        InputEvent::Key {
            key,
            modifiers,
            pressed,
            emacs_frame_id,
        } => {
            assert_eq!(key, crate::host_types::FrontendKey::Keysym(65));
            assert_eq!(modifiers, 0);
            assert!(pressed);
            assert_eq!(emacs_frame_id, 0);
        }
        other => panic!("Expected Key event, got {:?}", other),
    }
}

#[test]
fn thread_comms_cmd_channel_roundtrip() {
    let comms = ThreadComms::new();

    comms
        .cmd_tx
        .send(RenderCommand::Lifecycle(LifecycleCommand::Shutdown))
        .unwrap();

    let received = comms.cmd_rx.try_recv().unwrap();
    match received {
        RenderCommand::Lifecycle(LifecycleCommand::Shutdown) => {} // ok
        other => panic!("Expected Shutdown, got {:?}", other),
    }
}

#[test]
fn thread_comms_frame_channel_roundtrip() {
    let comms = ThreadComms::new();

    let buf = FrameGlyphBuffer::new();
    let state = FrameDisplayState::from_frame_glyph_buffer(&buf);
    comms.frame_tx.submit(sealed_test_state(state)).unwrap();

    let received = comms.frame_rx.try_recv().unwrap();
    assert_eq!(received.frame_pixel_width, 0.0);
    assert_eq!(received.frame_pixel_height, 0.0);
}

#[test]
fn pending_frames_keep_only_the_latest_revision() {
    let comms = ThreadComms::new();
    for i in 0..100 {
        let buf = FrameGlyphBuffer::with_size(i as f32, i as f32);
        let state = FrameDisplayState::from_frame_glyph_buffer(&buf);
        let old = comms.frame_tx.submit(sealed_test_state(state)).unwrap();
        assert_eq!(
            old.map(|state| state.frame_pixel_width),
            (i > 0).then_some((i - 1) as f32)
        );
    }
    let received = comms.frame_rx.try_recv().unwrap();
    assert_eq!(received.frame_pixel_width, 99.0);
    assert!(received.skipped_predecessor);
    assert!(comms.frame_rx.try_recv().is_err());
}

#[test]
fn frame_mailbox_drains_after_producer_disconnect_and_returns_rejected_submission() {
    let (emacs, render) = ThreadComms::new().split();
    emacs
        .frame_tx
        .submit(sealed_test_state(FrameDisplayState::new(80, 24, 8.0, 16.0)))
        .unwrap();
    drop(emacs);
    assert!(render.frame_rx.recv().is_ok());
    assert!(render.frame_rx.recv().is_err());

    let (emacs, render) = ThreadComms::new().split();
    drop(render);
    let rejected = emacs
        .frame_tx
        .submit(sealed_test_state(FrameDisplayState::new(80, 24, 8.0, 16.0)))
        .unwrap_err();
    assert_eq!(rejected.0.presentation().get(), 1);
}

#[test]
fn concurrent_take_and_replace_assign_every_revision_exactly_once() {
    let (emacs, render) = ThreadComms::new().split();
    let reader = std::thread::spawn(move || {
        let mut taken = Vec::new();
        while let Ok(state) = render.frame_rx.recv() {
            taken.push(state.presentation().get());
        }
        taken
    });
    let mut retired = Vec::new();
    for revision in 1..=1000 {
        let mut state = FrameDisplayState::new(80, 24, 8.0, 16.0);
        state.presentation_id = romacs_display_protocol::PresentationId::new(revision);
        if let Some(old) = emacs.frame_tx.submit(sealed_test_state(state)).unwrap() {
            retired.push(old.presentation().get());
        }
    }
    drop(emacs);
    let taken = reader.join().unwrap();
    assert_eq!(taken.last(), Some(&1000));
    assert!(taken.windows(2).all(|pair| pair[0] < pair[1]));
    retired.extend(taken);
    retired.sort_unstable();
    assert_eq!(retired, (1..=1000).collect::<Vec<_>>());
}

#[test]
fn thread_comms_input_channel_bounded_capacity() {
    let comms = ThreadComms::new();

    // Fill up the input channel to capacity
    for _ in 0..INPUT_CHANNEL_CAPACITY {
        let event = InputEvent::Key {
            key: crate::host_types::FrontendKey::Keysym(0),
            modifiers: 0,
            pressed: false,
            emacs_frame_id: 0,
        };
        comms.input_tx.try_send(event).unwrap();
    }

    // Next try_send should fail (channel full)
    let result = comms.input_tx.try_send(InputEvent::Key {
        key: crate::host_types::FrontendKey::Keysym(0),
        modifiers: 0,
        pressed: false,
        emacs_frame_id: 0,
    });
    assert!(
        result.is_err(),
        "input channel should be full after {} sends",
        INPUT_CHANNEL_CAPACITY
    );
}

#[test]
fn presentation_lifecycle_events_roundtrip_without_losing_frame_identity() {
    let comms = ThreadComms::new();

    comms
        .input_tx
        .send(InputEvent::PresentationActivated {
            presentation: 41,
            emacs_frame_id: 0x1_0000_0000,
        })
        .unwrap();
    comms
        .input_tx
        .send(InputEvent::PresentationDiscarded {
            presentation: 42,
            emacs_frame_id: 0x1_0000_0000,
        })
        .unwrap();

    assert!(matches!(
        comms.input_rx.try_recv().unwrap(),
        InputEvent::PresentationActivated {
            presentation: 41,
            emacs_frame_id: 0x1_0000_0000,
        }
    ));
    assert!(matches!(
        comms.input_rx.try_recv().unwrap(),
        InputEvent::PresentationDiscarded {
            presentation: 42,
            emacs_frame_id: 0x1_0000_0000,
        }
    ));
}

// ===================================================================
// ThreadComms::split()
// ===================================================================

// ===================================================================
// RenderComms::send_input()
// ===================================================================

#[test]
fn render_comms_send_input_delivers_event() {
    let comms = ThreadComms::new();
    let (emacs, render) = comms.split();

    render.send_input(unpresented_pointer(
        100.0,
        200.0,
        0,
        PointerAction::Move { modifiers: 0 },
    ));

    // Event should be receivable
    let evt = emacs.input_rx.try_recv().unwrap();
    match evt {
        InputEvent::PositionedPointer(PositionedPointerInput { position, .. }) => {
            assert_eq!(position.x, 100.0);
            assert_eq!(position.y, 200.0);
        }
        other => panic!("Expected PositionedPointer, got {:?}", other),
    }
}

// ===================================================================
// InputEvent enum variant construction
// ===================================================================

#[test]
fn input_event_key_construction() {
    let event = InputEvent::Key {
        key: crate::host_types::FrontendKey::Keysym(0xFF0D), // Return
        modifiers: 4,                                        // Ctrl
        pressed: true,
        emacs_frame_id: 0,
    };
    match event {
        InputEvent::Key {
            key,
            modifiers,
            pressed,
            emacs_frame_id,
        } => {
            assert_eq!(key, crate::host_types::FrontendKey::Keysym(0xFF0D));
            assert_eq!(modifiers, 4);
            assert!(pressed);
            assert_eq!(emacs_frame_id, 0);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn input_event_mouse_button_construction() {
    let event = unpresented_pointer(
        50.5,
        100.3,
        0,
        PointerAction::Button {
            button: 1,
            pressed: true,
            modifiers: 0,
        },
    );
    match event {
        InputEvent::PositionedPointer(PositionedPointerInput {
            position,
            target: PointerTarget::Unpresented,
            action:
                PointerAction::Button {
                    button,
                    pressed,
                    modifiers,
                },
        }) => {
            assert_eq!(
                position,
                PointerPosition {
                    x: 50.5,
                    y: 100.3,
                    target_frame_id: 0
                }
            );
            assert_eq!(button, 1);
            assert!(pressed);
            assert_eq!(modifiers, 0);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn input_event_mouse_move_construction() {
    let event = unpresented_pointer(200.0, 300.0, 42, PointerAction::Move { modifiers: 1 });
    match event {
        InputEvent::PositionedPointer(PositionedPointerInput {
            position,
            target: PointerTarget::Unpresented,
            action: PointerAction::Move { modifiers },
        }) => {
            assert_eq!(position.x, 200.0);
            assert_eq!(position.y, 300.0);
            assert_eq!(modifiers, 1);
            assert_eq!(position.target_frame_id, 42);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn input_event_mouse_scroll_construction() {
    let event = unpresented_pointer(
        400.0,
        500.0,
        0,
        PointerAction::Scroll {
            delta: ScrollDelta::Lines { x: 0.0, y: -3.0 },
            modifiers: 0,
        },
    );
    match event {
        InputEvent::PositionedPointer(PositionedPointerInput {
            action: PointerAction::Scroll { delta, .. },
            ..
        }) => {
            assert_eq!(delta, ScrollDelta::Lines { x: 0.0, y: -3.0 });
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn input_event_pixel_scroll_uses_a_distinct_delta_variant() {
    let event = unpresented_pointer(
        0.0,
        0.0,
        0,
        PointerAction::Scroll {
            delta: ScrollDelta::Pixels { x: 10.5, y: -25.3 },
            modifiers: 0,
        },
    );
    match event {
        InputEvent::PositionedPointer(PositionedPointerInput {
            action: PointerAction::Scroll { delta, .. },
            ..
        }) => assert_eq!(delta, ScrollDelta::Pixels { x: 10.5, y: -25.3 }),
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn input_event_window_resize_construction() {
    let event = InputEvent::WindowResize {
        width: 1920,
        height: 1080,
        scale_factor: 1.0,
        emacs_frame_id: 0,
    };
    match event {
        InputEvent::WindowResize {
            width,
            height,
            scale_factor,
            emacs_frame_id,
        } => {
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
            assert_eq!(scale_factor, 1.0);
            assert_eq!(emacs_frame_id, 0);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn input_event_window_close_construction() {
    let event = InputEvent::WindowClose {
        emacs_frame_id: 123,
    };
    match event {
        InputEvent::WindowClose { emacs_frame_id } => assert_eq!(emacs_frame_id, 123),
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn input_event_window_focus_construction() {
    let focused = InputEvent::WindowFocus {
        focused: true,
        emacs_frame_id: 0,
    };
    match focused {
        InputEvent::WindowFocus {
            focused,
            emacs_frame_id,
        } => {
            assert!(focused);
            assert_eq!(emacs_frame_id, 0);
        }
        _ => panic!("Wrong variant"),
    }

    let unfocused = InputEvent::WindowFocus {
        focused: false,
        emacs_frame_id: 5,
    };
    match unfocused {
        InputEvent::WindowFocus {
            focused,
            emacs_frame_id,
        } => {
            assert!(!focused);
            assert_eq!(emacs_frame_id, 5);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn input_event_image_terminal_state_changed_construction() {
    let state = ImageStateEvent::Evicted(ImageId::new(7));
    let event = InputEvent::ImageStateChanged { event: state };
    match event {
        InputEvent::ImageStateChanged { event } => {
            assert_eq!(event, state);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn input_event_menu_selection_construction() {
    let selected = InputEvent::MenuSelection {
        index: 3,
        token: None,
    };
    match selected {
        InputEvent::MenuSelection { index, .. } => assert_eq!(index, 3),
        _ => panic!("Wrong variant"),
    }

    let cancelled = InputEvent::MenuSelection {
        index: -1,
        token: None,
    };
    assert!(!RenderComms::is_lossy_input_event(&cancelled));
    match cancelled {
        InputEvent::MenuSelection { index, .. } => assert_eq!(index, -1),
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn a_file_drop_reports_the_dropped_paths_and_nothing_that_stands_in_for_a_posn() {
    // A drop's position is GNU's `make_lispy_position` posn, resolved against
    // the presentation the way `PresentedPointer` resolves a click. If this
    // variant regained a bare coordinate pair, the day drops are wired to a
    // window position the nearest thing to hand would be the pointer's raw
    // surface coordinates, which name a different pixel for the length of a
    // pane morph.
    let event = InputEvent::FileDrop {
        paths: vec![
            "/home/user/file.txt".to_string(),
            "/tmp/image.png".to_string(),
        ],
    };
    match event {
        InputEvent::FileDrop { paths } => {
            assert_eq!(paths.len(), 2);
            assert_eq!(paths[0], "/home/user/file.txt");
            assert_eq!(paths[1], "/tmp/image.png");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn input_event_clone() {
    let original = InputEvent::Key {
        key: crate::host_types::FrontendKey::Keysym(42),
        modifiers: 8,
        pressed: false,
        emacs_frame_id: 0,
    };
    let cloned = original.clone();
    match cloned {
        InputEvent::Key {
            key,
            modifiers,
            pressed,
            emacs_frame_id,
        } => {
            assert_eq!(key, crate::host_types::FrontendKey::Keysym(42));
            assert_eq!(modifiers, 8);
            assert!(!pressed);
            assert_eq!(emacs_frame_id, 0);
        }
        _ => panic!("Clone changed variant"),
    }
}

#[test]
fn input_event_debug() {
    let event = InputEvent::Key {
        key: crate::host_types::FrontendKey::Keysym(65),
        modifiers: 0,
        pressed: true,
        emacs_frame_id: 0,
    };
    let debug = format!("{:?}", event);
    assert!(
        debug.contains("Key"),
        "Debug output should contain variant name: {}",
        debug
    );
}

// ===================================================================
// RenderCommand enum variant construction
// ===================================================================

#[test]
fn render_command_shutdown() {
    let cmd = RenderCommand::Lifecycle(LifecycleCommand::Shutdown);
    match cmd {
        RenderCommand::Lifecycle(LifecycleCommand::Shutdown) => {}
        other => panic!("Expected Shutdown, got {:?}", other),
    }
}

#[test]
fn render_command_image_retire() {
    let image = ImageId::new(42);
    let cmd = RenderCommand::Asset(AssetCommand::ImageRetire { image });
    match cmd {
        RenderCommand::Asset(AssetCommand::ImageRetire { image: actual }) => {
            assert_eq!(actual, image)
        }
        other => panic!("Expected ImageRetire, got {:?}", other),
    }
}

#[test]
fn render_command_set_window_title() {
    let cmd = RenderCommand::Window(WindowCommand::SetWindowTitle {
        title: "Romacs - main.rs".to_string(),
    });
    match cmd {
        RenderCommand::Window(WindowCommand::SetWindowTitle { title }) => {
            assert_eq!(title, "Romacs - main.rs");
        }
        other => panic!("Expected SetWindowTitle, got {:?}", other),
    }
}

#[test]
fn render_command_set_frame_geometry_hints() {
    let geometry_hints = GuiFrameGeometryHints {
        base_width: 42,
        base_height: 58,
        min_width: 42,
        min_height: 58,
        width_inc: 26,
        height_inc: 58,
    };
    let cmd = RenderCommand::Window(WindowCommand::SetFrameGeometryHints {
        frame: FrameRef::Primary,
        geometry_hints,
    });
    match cmd {
        RenderCommand::Window(WindowCommand::SetFrameGeometryHints {
            frame,
            geometry_hints: actual_hints,
        }) => {
            assert_eq!(frame.raw_id(), 0);
            assert_eq!(actual_hints, geometry_hints);
        }
        other => panic!("Expected SetFrameGeometryHints, got {:?}", other),
    }
}

#[test]
fn render_command_request_attention() {
    let cmd = RenderCommand::Window(WindowCommand::RequestAttention { urgent: true });
    match cmd {
        RenderCommand::Window(WindowCommand::RequestAttention { urgent }) => assert!(urgent),
        other => panic!("Expected RequestAttention, got {:?}", other),
    }
}

#[test]
fn render_command_update_visual_config() {
    let cmd = RenderCommand::Config(ConfigCommand::SetVisualConfig(VisualConfig::default()));
    match cmd {
        RenderCommand::Config(ConfigCommand::SetVisualConfig(_)) => {}
        other => panic!("Expected SetVisualConfig, got {:?}", other),
    }
}

#[test]
fn render_command_create_window() {
    let geometry_hints = GuiFrameGeometryHints {
        base_width: 42,
        base_height: 58,
        min_width: 42,
        min_height: 58,
        width_inc: 26,
        height_inc: 58,
    };
    let cmd = RenderCommand::Window(WindowCommand::CreateWindow {
        frame: FrameRef::Frame(99),
        width: 1024,
        height: 768,
        title: "New Frame".to_string(),
        geometry_hints,
    });
    match cmd {
        RenderCommand::Window(WindowCommand::CreateWindow {
            frame,
            width,
            height,
            title,
            geometry_hints: actual_hints,
        }) => {
            assert_eq!(frame.raw_id(), 99);
            assert_eq!(width, 1024);
            assert_eq!(height, 768);
            assert_eq!(title, "New Frame");
            assert_eq!(actual_hints, geometry_hints);
        }
        other => panic!("Expected CreateWindow, got {:?}", other),
    }
}

#[test]
fn render_command_destroy_window() {
    let cmd = RenderCommand::Window(WindowCommand::DestroyWindow {
        frame: FrameRef::Frame(99),
    });
    match cmd {
        RenderCommand::Window(WindowCommand::DestroyWindow { frame }) => {
            assert_eq!(frame.raw_id(), 99)
        }
        other => panic!("Expected DestroyWindow, got {:?}", other),
    }
}

#[test]
fn render_command_debug() {
    let cmd = RenderCommand::Lifecycle(LifecycleCommand::Shutdown);
    let debug = format!("{:?}", cmd);
    assert!(debug.contains("Shutdown"), "Debug output: {}", debug);
}

// ===================================================================
// PopupMenuItem
// ===================================================================

#[test]
fn popup_menu_item_construction() {
    let item = PopupMenuItem {
        kind: romacs_display_protocol::menu::MenuItemKind::Command {
            availability: romacs_display_protocol::menu::MenuAvailability::Enabled,
            indicator: romacs_display_protocol::menu::MenuIndicator::None,
        },
        help: None,
        label: "Save".to_string(),
        shortcut: "C-x C-s".to_string(),
        depth: 0,
    };
    assert_eq!(item.label, "Save");
    assert_eq!(item.shortcut, "C-x C-s");
    assert!(item.enabled());
    assert!(!item.separator());
    assert!(!item.submenu());
    assert_eq!(item.depth, 0);
}

#[test]
fn popup_menu_item_separator() {
    let sep = PopupMenuItem {
        kind: romacs_display_protocol::menu::MenuItemKind::Separator,
        help: None,
        label: String::new(),
        shortcut: String::new(),
        depth: 0,
    };
    assert!(sep.separator());
    assert!(!sep.enabled());
}

#[test]
fn popup_menu_item_submenu() {
    let sub = PopupMenuItem {
        kind: romacs_display_protocol::menu::MenuItemKind::Submenu {
            availability: romacs_display_protocol::menu::MenuAvailability::Enabled,
        },
        help: None,
        label: "Recent Files".to_string(),
        shortcut: String::new(),
        depth: 1,
    };
    assert!(sub.submenu());
    assert_eq!(sub.depth, 1);
}

#[test]
fn popup_menu_item_clone() {
    let item = PopupMenuItem {
        kind: romacs_display_protocol::menu::MenuItemKind::Command {
            availability: romacs_display_protocol::menu::MenuAvailability::Enabled,
            indicator: romacs_display_protocol::menu::MenuIndicator::None,
        },
        help: None,
        label: "Test".to_string(),
        shortcut: "M-x".to_string(),
        depth: 2,
    };
    let cloned = item.clone();
    assert_eq!(cloned.label, "Test");
    assert_eq!(cloned.depth, 2);
}

#[test]
fn popup_menu_item_debug() {
    let item = PopupMenuItem {
        kind: romacs_display_protocol::menu::MenuItemKind::Command {
            availability: romacs_display_protocol::menu::MenuAvailability::Enabled,
            indicator: romacs_display_protocol::menu::MenuIndicator::None,
        },
        help: None,
        label: "Debug".to_string(),
        shortcut: String::new(),
        depth: 0,
    };
    let debug = format!("{:?}", item);
    assert!(debug.contains("PopupMenuItem"), "Debug output: {}", debug);
}

// ===================================================================
// Channel operations: send through crossbeam, receive correctly
// ===================================================================

#[test]
fn channel_sends_multiple_input_events_in_order() {
    let comms = ThreadComms::new();

    let events = vec![
        InputEvent::Key {
            key: crate::host_types::FrontendKey::Keysym(1),
            modifiers: 0,
            pressed: true,
            emacs_frame_id: 0,
        },
        InputEvent::Key {
            key: crate::host_types::FrontendKey::Keysym(2),
            modifiers: 0,
            pressed: true,
            emacs_frame_id: 0,
        },
        InputEvent::Key {
            key: crate::host_types::FrontendKey::Keysym(3),
            modifiers: 0,
            pressed: true,
            emacs_frame_id: 0,
        },
        unpresented_pointer(10.0, 20.0, 0, PointerAction::Move { modifiers: 0 }),
        InputEvent::WindowResize {
            width: 800,
            height: 600,
            scale_factor: 1.0,
            emacs_frame_id: 0,
        },
    ];

    for e in &events {
        comms.input_tx.send(e.clone()).unwrap();
    }

    // Receive and verify order
    for (i, expected) in events.iter().enumerate() {
        let received = comms.input_rx.try_recv().unwrap();
        let expected_debug = format!("{:?}", expected);
        let received_debug = format!("{:?}", received);
        assert_eq!(
            expected_debug, received_debug,
            "Event {} mismatch: expected {:?}, got {:?}",
            i, expected_debug, received_debug
        );
    }

    // No more events
    assert!(comms.input_rx.try_recv().is_err());
}

#[test]
fn channel_empty_recv_returns_error() {
    let comms = ThreadComms::new();
    assert!(comms.input_rx.try_recv().is_err());
    assert!(comms.cmd_rx.try_recv().is_err());
    assert!(comms.frame_rx.try_recv().is_err());
}

// ===================================================================
// Cross-thread usage simulation
// ===================================================================

#[test]
fn cross_thread_input_event_delivery() {
    let comms = ThreadComms::new();
    let (emacs, render) = comms.split();

    let handle = std::thread::spawn(move || {
        render.send_input(InputEvent::Key {
            key: crate::host_types::FrontendKey::Keysym(0x61), // 'a'
            modifiers: 0,
            pressed: true,
            emacs_frame_id: 0,
        });
        render.send_input(InputEvent::WindowResize {
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
            emacs_frame_id: 0,
        });
    });

    handle.join().unwrap();

    // Both events should be receivable on the Emacs side
    let evt1 = emacs.input_rx.try_recv().unwrap();
    match evt1 {
        InputEvent::Key { key, .. } => {
            assert_eq!(key, crate::host_types::FrontendKey::Keysym(0x61))
        }
        other => panic!("Expected Key, got {:?}", other),
    }

    let evt2 = emacs.input_rx.try_recv().unwrap();
    match evt2 {
        InputEvent::WindowResize { width, height, .. } => {
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
        }
        other => panic!("Expected WindowResize, got {:?}", other),
    }
}

#[test]
fn cross_thread_command_delivery() {
    let comms = ThreadComms::new();
    let (emacs, render) = comms.split();

    let handle = std::thread::spawn(move || {
        let cmd = render.cmd_rx.recv().unwrap();
        match cmd {
            RenderCommand::Window(WindowCommand::SetWindowTitle { title }) => {
                assert_eq!(title, "test-title");
            }
            other => panic!("Expected SetWindowTitle, got {:?}", other),
        }
    });

    emacs
        .cmd_tx
        .send(RenderCommand::Window(WindowCommand::SetWindowTitle {
            title: "test-title".to_string(),
        }))
        .unwrap();

    handle.join().unwrap();
}

#[test]
fn cross_thread_frame_delivery() {
    let comms = ThreadComms::new();
    let (emacs, render) = comms.split();

    let handle = std::thread::spawn(move || {
        let frame = render.frame_rx.recv().unwrap();
        assert_eq!(frame.frame_pixel_width, 1920.0);
        assert_eq!(frame.frame_pixel_height, 1080.0);
    });

    let buf = FrameGlyphBuffer::with_size(1920.0, 1080.0);
    let state = FrameDisplayState::from_frame_glyph_buffer(&buf);
    emacs.frame_tx.submit(sealed_test_state(state)).unwrap();

    handle.join().unwrap();
}

#[test]
fn pointer_cell_keeps_a_position_and_its_absence() {
    let cell = PointerCell::default();
    assert_eq!(cell.get(), None);
    for at in [(0, 0), (45, 61), (-3, 7), (i32::MIN, i32::MAX)] {
        cell.set(Some(at));
        assert_eq!(cell.get(), Some(at));
    }
    cell.set(None);
    assert_eq!(cell.get(), None);
}


#[test]
fn thread_comms_cmd_channel_bounded_capacity() {
    let comms = ThreadComms::new();

    // Ordinary work occupies the bounded FIFO; shutdown is independent.
    for _ in 0..COMMAND_CHANNEL_CAPACITY {
        comms
            .cmd_tx
            .try_send(RenderCommand::Window(WindowCommand::RefreshFrameOpacity))
            .unwrap();
    }

    // Next ordinary try_send should fail (channel full)
    let result = comms
        .cmd_tx
        .try_send(RenderCommand::Window(WindowCommand::RefreshFrameOpacity));
    assert!(
        result.is_err(),
        "cmd channel should be full after {} sends",
        COMMAND_CHANNEL_CAPACITY
    );
}

#[test]
fn channel_sends_multiple_commands_in_order() {
    let comms = ThreadComms::new();

    comms
        .cmd_tx
        .send(RenderCommand::Lifecycle(LifecycleCommand::Shutdown))
        .unwrap();
    comms
        .cmd_tx
        .send(RenderCommand::Window(WindowCommand::RefreshFrameOpacity))
        .unwrap();
    comms
        .cmd_tx
        .send(RenderCommand::Asset(AssetCommand::ImageRetire {
            image: romacs_display_protocol::ImageId::new(3),
        }))
        .unwrap();

    match comms.cmd_rx.try_recv().unwrap() {
        RenderCommand::Lifecycle(LifecycleCommand::Shutdown) => {}
        other => panic!("Expected Shutdown, got {:?}", other),
    }
    match comms.cmd_rx.try_recv().unwrap() {
        RenderCommand::Window(WindowCommand::RefreshFrameOpacity) => {}
        other => panic!("Expected RefreshFrameOpacity, got {:?}", other),
    }
    match comms.cmd_rx.try_recv().unwrap() {
        RenderCommand::Asset(AssetCommand::ImageRetire { image }) => assert_eq!(image.get(), 3),
        other => panic!("Expected ImageRetire, got {:?}", other),
    }

    assert!(comms.cmd_rx.try_recv().is_err());
}
