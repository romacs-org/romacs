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

#[test]
fn an_open_heading_closes_with_control_g_and_escape() {
    for key in [Key::Character("g".into()), Key::Named(NamedKey::Escape)] {
        let mut bar = MenuBar::default();
        bar.select(heading("help-menu"), false);
        bar.modifiers = ModifiersState::CONTROL;
        assert!(cancels(&key, bar.modifiers));
        bar.close();
        assert!(bar.heading().is_none());
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
