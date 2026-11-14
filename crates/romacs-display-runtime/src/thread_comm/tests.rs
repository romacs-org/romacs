use super::*;
use romacs_display_protocol::{
    ImageId, ImageStateEvent,
};

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

#[test]
fn a_native_key_receipt_is_taken_once_the_evaluator_drops_the_event() {
    let (emacs, render) = ThreadComms::new().split();
    // A single interactive command may read held-key input indefinitely.
    for _ in 0..1300 {
        let observer = render.send_key_input_with_receipt(InputEvent::Key {
            key: crate::host_types::FrontendKey::Character('p'),
            modifiers: 0,
            pressed: true,
            emacs_frame_id: 1,
        });
        let observer = observer.expect("every native key needs a read receipt");
        assert!(!observer.taken());
        let event = emacs.input_rx.try_recv().unwrap();
        assert!(matches!(event, InputEvent::Tracked { .. }), "native key must keep its delivery through the bridge");
        drop(event);
        assert!(observer.taken());
    }
}

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
    assert!(!is_lossy_input_event(&cancelled));
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
fn thread_comms_cmd_channel_bounded_capacity() {
    let comms = ThreadComms::new();

    // Ordinary work occupies the bounded FIFO; shutdown is independent.
    for _ in 0..COMMAND_CHANNEL_CAPACITY {
        comms
            .cmd_tx
            .try_send(RenderCommand::Window(WindowCommand::RequestAttention { urgent: false }))
            .unwrap();
    }

    // Next ordinary try_send should fail (channel full)
    let result = comms
        .cmd_tx
        .try_send(RenderCommand::Window(WindowCommand::RequestAttention { urgent: false }));
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
        .send(RenderCommand::Window(WindowCommand::RequestAttention { urgent: false }))
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
        RenderCommand::Window(WindowCommand::RequestAttention { urgent: false }) => {}
        other => panic!("Expected RefreshFrameOpacity, got {:?}", other),
    }
    match comms.cmd_rx.try_recv().unwrap() {
        RenderCommand::Asset(AssetCommand::ImageRetire { image }) => assert_eq!(image.get(), 3),
        other => panic!("Expected ImageRetire, got {:?}", other),
    }

    assert!(comms.cmd_rx.try_recv().is_err());
}

/// SOURCE's fields that are not plain data, by the type that holds them:
/// those of the MESSAGES, of every type SOURCE declares that they reach
/// (aliases included), and of the HANDLES, which may hold their transport.
/// Fields are read whole with whitespace removed; comments are dropped.
fn seam_violations(source: &str, messages: &[&str], handles: &[&str]) -> (Vec<(String, String)>, Vec<String>) {
    const TRANSPORT: &[&str] = &["CommandSender", "CommandReceiver", "Sender<InputEvent>", "Receiver<InputEvent>",
                                 "Sender<Fact>", "Receiver<Fact>"];
    const FORBIDDEN: &[&str] = &["Arc<", "Rc<", "Mutex", "RwLock", "Cell<", "Atomic", "dyn", "Instant", "Sender<",
                                 "Receiver<", "InputDelivery", "CommandSender", "CommandReceiver"];
    let source: String = source.lines().map(|line| line.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    // NAME's declaration in SOURCE: an enum's or struct's body, or an alias's type.
    let declared = |name: &str| -> Option<String> {
        let at = ["enum ", "struct ", "type "].iter().find_map(|kind| {
            source.match_indices(&format!("{kind}{name}")).map(|(at, _)| at + kind.len() + name.len())
                .find(|&end| !source[end..].starts_with(|c: char| c.is_alphanumeric() || c == '_'))
        })?;
        let rest = &source[at..];
        let open = rest.find(['{', '(', '=', ';'])?;
        if rest[open..].starts_with(';') { return Some(String::new()); }
        if rest[open..].starts_with('=') { return Some(rest[open + 1..rest.find(';')?].to_owned()); }
        let mut depth = 0;
        let close = rest[open..].char_indices().find_map(|(index, c)| {
            depth += match c { '{' | '(' => 1, '}' | ')' => -1, _ => 0 };
            (depth == 0).then_some(open + index)
        })?;
        Some(rest[open + 1..close].to_owned())
    };
    // BODY's leaf fields, whitespace removed: split at commas, and at a
    // variant's braces and parentheses, outside generic arguments.
    let fields = |body: &str| -> Vec<String> {
        let (mut out, mut field, mut generic) = (Vec::new(), String::new(), 0i32);
        for c in body.chars().filter(|c| !c.is_whitespace()) {
            generic += match c { '<' => 1, '>' => -1, _ => 0 };
            if generic == 0 && matches!(c, ',' | '{' | '}' | '(' | ')') { out.push(std::mem::take(&mut field)); } else { field.push(c); }
        }
        out.push(field);
        out.into_iter().filter(|field| !field.is_empty()).collect()
    };
    let mut found = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut work: Vec<(String, bool)> = messages.iter().map(|name| ((*name).to_owned(), false))
        .chain(handles.iter().map(|name| ((*name).to_owned(), true))).collect();
    while let Some((name, handle)) = work.pop() {
        if seen.contains(&name) { continue; }
        seen.push(name.clone());
        let Some(body) = declared(&name) else { continue };
        for field in fields(&body) {
            let kind = field.split_once(':').map_or(field.as_str(), |(_, kind)| kind);
            if handle && TRANSPORT.contains(&kind) { continue; }
            if FORBIDDEN.iter().any(|form| field.contains(form)) { found.push((name.clone(), field.clone())); }
        }
        for word in body.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|word| !word.is_empty()) {
            if word != name && declared(word).is_some() { work.push((word.to_owned(), false)); }
        }
    }
    (found, seen)
}

/// The seam between the evaluator and the render thread carries plain data,
/// so thread or process is a transport choice (design: process split).
/// No field may hold shared memory, a callback, a channel end or a
/// process-local clock (`seam_violations`), except those listed with the
/// slice that removes them.  Types from other modules (`FrameGlyphBuffer`,
/// `VisualConfig`) are leaves here: flat frames hold those.  An exception
/// that no longer occurs must leave the list: the list only shrinks.
#[test]
fn the_seam_carries_plain_data_but_its_listed_exceptions() {
    // (type, the field with its whitespace removed, what removes it)
    const EXCEPTIONS: &[(&str, &str, &str)] = &[
        ("AssetCommand", "decoded:std::sync::Arc<romacs_renderer_wgpu::image_cache::SemanticImageDecoded>",
         "S4: pixels by a shared-memory region"),
        ("ClipboardCommand", "expires_at:Instant", "S4: deadlines on a clock both processes share"),
        ("InputEvent", "delivery:romacs_display_protocol::input_progress::InputDelivery",
         "S2b: key receipts as a control-page word"),
        ("RenderComms", "pubinput_interrupt:Option<Arc<dynInputInterrupt>>", "S2b: the quit test on the control page"),
    ];
    let (found, seen) = seam_violations(include_str!("../thread_comm.rs"),
        &["PaintScene", "RenderCommand", "InputEvent", "Fact", "Reply"], &["EmacsComms", "RenderComms", "ThreadComms"]);
    for (name, field) in &found {
        assert!(EXCEPTIONS.iter().any(|(known, text, _)| known == name && text == field), "{name}: `{field}` is not plain data");
    }
    for (name, field, by) in EXCEPTIONS {
        assert!(found.iter().any(|(known, text)| known == name && text == field), "{name}: `{field}` is gone ({by}): take it off the list");
    }
    assert!(seen.iter().any(|name| name == "MonitorInfo") && seen.iter().any(|name| name == "WindowCommand"),
            "payload types are followed: {seen:?}");
}

/// The scanner finds a field split over lines, one behind an alias or in
/// a nested payload, and a handle's field beside its transport.
#[test]
fn seam_violations_are_found_however_they_are_written() {
    let source = "
        pub enum Message { Plain(u64), Split { bytes: std::sync::Arc
            <u8> }, Aliased(Shared), Nested(Inner) }
        pub(crate) type Shared = Arc<u8>;
        pub struct Inner { at: std::time::Instant, ok: Vec<(u32, u32)> }
        pub(crate) struct Handle { pub tx: Sender<Fact>, pub hook: Box<dyn Fn()> }
    ";
    let (found, _) = seam_violations(source, &["Message"], &["Handle"]);
    let found: Vec<String> = found.into_iter().map(|(name, field)| format!("{name} {field}")).collect();
    assert_eq!(found.len(), 4, "{found:?}");
    for expected in ["Message bytes:std::sync::Arc<u8>", "Shared Arc<u8>", "Inner at:std::time::Instant", "Handle pubhook:Box<dynFn()>"] {
        assert!(found.iter().any(|item| item == expected), "missed {expected}: {found:?}");
    }
}
