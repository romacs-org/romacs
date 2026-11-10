use super::*;

fn heading(key: &str) -> MenuHeading {
    MenuHeading {
        frame: 1,
        parent: WindowId::from_raw(1),
        key: key.into(),
        index: 5,
        compact: false,
    }
}

#[test]
fn repeated_hover_over_the_open_heading_keeps_it() {
    let mut bar = MenuBar::default();
    assert!(matches!(
        bar.select(heading("help"), true),
        HeadingAction::Request(_)
    ));
    for _ in 0..100 {
        assert_eq!(bar.select(heading("help"), false), HeadingAction::Keep);
    }
}

#[test]
fn switching_headings_requests_each_afresh_and_a_press_closes() {
    let mut bar = MenuBar::default();
    let HeadingAction::Request(first) = bar.select(heading("help"), true) else {
        panic!()
    };
    assert!(matches!(
        bar.select(heading("interactively"), false),
        HeadingAction::Request(_)
    ));
    let HeadingAction::Request(again) = bar.select(heading("help"), false) else {
        panic!()
    };
    assert_ne!(first, again);
    assert_eq!(bar.select(heading("help"), true), HeadingAction::Close);
    assert!(bar.heading().is_none());
}

fn key(logical_key: Key) -> WindowEvent {
    WindowEvent::KeyboardInput {
        device_id: None,
        event: winit::event::KeyEvent {
            physical_key: winit::keyboard::PhysicalKey::Unidentified(
                winit::keyboard::NativeKeyCode::Unidentified,
            ),
            logical_key: logical_key.clone(),
            text: None,
            location: winit::keyboard::KeyLocation::Standard,
            state: ElementState::Pressed,
            repeat: false,
            text_with_all_modifiers: None,
            key_without_modifiers: logical_key,
        },
        is_synthetic: false,
    }
}

#[test]
fn an_open_heading_closes_with_control_g_and_escape() {
    let id = WindowId::from_raw(1);
    for (modifiers, cancel) in [
        (ModifiersState::CONTROL, Key::Character("g".into())),
        (ModifiersState::empty(), Key::Named(NamedKey::Escape)),
    ] {
        let mut bar = MenuBar::default();
        bar.select(heading("help-menu"), false);
        bar.event(
            id,
            &WindowEvent::ModifiersChanged(winit::event::Modifiers::new(
                modifiers,
                Default::default(),
            )),
        );
        assert!(bar.event(id, &key(Key::Character("x".into()))));
        assert!(bar.heading().is_some(), "other keys keep the heading open");
        assert!(
            bar.event(id, &key(cancel)),
            "the menu bar owns the cancel key"
        );
        assert!(bar.heading().is_none());
        assert!(!bar.event(id, &key(Key::Character("x".into()))));
    }
}

#[test]
fn closing_a_heading_keeps_its_release() {
    let mut bar = MenuBar::default();
    let open = heading("help-menu");
    bar.select(open.clone(), true);
    assert_eq!(bar.select(open.clone(), true), HeadingAction::Close);
    assert!(bar.heading().is_none());
    assert_eq!(bar.release_owner, Some((open.parent, MouseButton::Left)));
}

#[test]
fn arrows_step_between_headings_only_on_the_open_one() {
    let mut bar = MenuBar::default();
    let id = WindowId::from_raw(1);
    let right = Key::Named(NamedKey::ArrowRight);
    assert_eq!(bar.heading_step(id, &right), None);
    bar.select(heading("help"), false);
    assert_eq!(bar.heading_step(id, &right), Some(1));
    assert_eq!(
        bar.heading_step(id, &Key::Named(NamedKey::ArrowLeft)),
        Some(-1)
    );
    assert_eq!(bar.heading_step(WindowId::from_raw(2), &right), None);
}
