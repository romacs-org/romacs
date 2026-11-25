//! O1b-3 pane layers: the ingest rules, residency by order, settling and
//! teardown (design O1b.md rev 3, §6.2; typing-modes o1b3/DESIGN.md).
use super::{Admit, admit};
use crate::render_thread::RenderApp;
use crate::thread_comm::{EmacsComms, FrameRef, RenderCommand, ThreadComms, WindowCommand};
use romacs_display_protocol::font::FontCatalogGeneration;
use romacs_display_protocol::foreign_pane::ForeignPaneEntry;
use romacs_display_protocol::pane_picture::{PaneFrame, PaneHead, PanePicture};
use romacs_display_protocol::pane_revisions::PaneRevisions;
use romacs_display_protocol::pane_sink::PaneSink;
use romacs_display_protocol::text_animation::{LogoColours, TextAnimation, TextAnimationKind};
use romacs_display_protocol::{
    DisplayFrameId, DisplayWindowId, FrameGlyphBuffer, ParentFrameRect, PresentationId,
    PresentedFramePlacement, Rect,
};
use std::sync::{Arc, Mutex};

const OWNER: u64 = 41;
const WINDOW: u64 = 900;
const VIEW: u64 = 11;
const OWN: i64 = 5;
const WIDTH: u32 = 400;
const HEIGHT: u32 = 100;
const CATALOG: FontCatalogGeneration = FontCatalogGeneration::INITIAL;

fn entry(request: u64) -> ForeignPaneEntry {
    ForeignPaneEntry { rect: Rect::new(10.0, 20.0, 400.0, 100.0), window: WINDOW, owner: OWNER, view: VIEW, request, cursor_off: false }
}

/// An animation of the owner's window, under KEY.
fn animation(key: u64) -> (u64, TextAnimation) {
    let own = OWN.cast_unsigned();
    let kind = TextAnimationKind::Logo { cell_aspect: 0.5, epoch: key, colours: LogoColours::default() };
    ((key << 32) | own, TextAnimation { window: own, top: 0, cols: 4, rows: 2, anchor: None, rise: None, fps: 25, moving: true, still: false, kind })
}

/// The owner's picture of WINDOW for REQUEST, minted as V, of the entry's
/// size, with ANIMATIONS.
fn picture_with(request: u64, v: u64, animations: Vec<(u64, TextAnimation)>) -> PanePicture {
    let head = PaneHead { owner: OWNER, window: WINDOW, view: VIEW, request, v, c: 1 };
    let frame = FrameGlyphBuffer::with_size(400.0, 100.0);
    PanePicture::from_frame(head, PaneFrame { frame, animations, submitted: None }, DisplayWindowId::new(OWN))
}

fn picture(request: u64, v: u64) -> PanePicture {
    picture_with(request, v, Vec::new())
}

/// A primary root frame, presentation PRESENTATION, naming ENTRIES.
fn root(presentation: u64, entries: Vec<ForeignPaneEntry>) -> Box<crate::thread_comm::PaintScene> {
    let mut frame = FrameGlyphBuffer::with_size(800.0, 600.0);
    frame.presentation_id = PresentationId::new(presentation);
    frame.frame_placement = PresentedFramePlacement::new(
        DisplayFrameId::new(1),
        frame.presentation_id,
        None,
        ParentFrameRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
        0,
    );
    frame.foreign_panes = entries;
    Box::new(frame.into())
}

/// A render thread with a primary window, its producer, and the heads it
/// settled, in order.
fn app() -> (RenderApp, EmacsComms, Arc<Mutex<Vec<u64>>>) {
    let (producer, render) = ThreadComms::new().split();
    let mut app = crate::render_thread::tests::make_test_app();
    app.comms = render;
    let settled = Arc::new(Mutex::new(Vec::new()));
    let record = settled.clone();
    assert!(app.panes.install(Box::new(move |owner, head: PaneHead| {
        assert_eq!(owner, OWNER);
        record.lock().unwrap().push(head.v);
    })));
    producer.cmd_tx.try_send(RenderCommand::Window(WindowCommand::AdoptPrimaryFrame { frame: FrameRef::Frame(1) })).unwrap();
    app.process_commands();
    (app, producer, settled)
}

fn send_root(app: &mut RenderApp, producer: &EmacsComms, presentation: u64, entries: Vec<ForeignPaneEntry>) {
    producer.cmd_tx.try_send(RenderCommand::Paint(root(presentation, entries))).unwrap();
    app.process_commands();
}

fn post(app: &RenderApp, picture: PanePicture) -> Arc<PaneRevisions> {
    let revisions = Arc::new(PaneRevisions::default());
    app.panes.post(OWNER, picture, revisions.clone());
    revisions
}

/// The head of the picture window WINDOW draws.
fn shown(app: &RenderApp) -> Option<PaneHead> {
    let layers = &app.frame_windows.primary_window().unwrap().render.compositor.pane_layers;
    layers.layer(WINDOW).and_then(|layer| layer.shown.as_ref()).map(|shown| shown.head)
}

fn settled(record: &Mutex<Vec<u64>>) -> Vec<u64> {
    record.lock().unwrap().clone()
}

// --- the ingest rules, on the pure function ---

#[test]
fn a_picture_for_no_entry_or_another_owner_or_view_is_dropped() {
    let mut other_owner = picture(3, 1);
    other_owner.head.owner = OWNER + 1;
    let mut other_view = picture(3, 1);
    other_view.head.view = VIEW + 1;
    assert_eq!(admit(Some(&entry(3)), CATALOG, &other_owner), Admit::Drop);
    assert_eq!(admit(Some(&entry(3)), CATALOG, &other_view), Admit::Drop);
    assert_eq!(admit(None, CATALOG, &picture(3, 1)), Admit::Drop);
}

#[test]
fn an_older_request_is_dropped_and_a_newer_one_held() {
    assert_eq!(admit(Some(&entry(3)), CATALOG, &picture(2, 1)), Admit::Drop);
    assert_eq!(admit(Some(&entry(3)), CATALOG, &picture(4, 1)), Admit::Hold);
}

#[test]
fn an_equal_request_must_match_the_entrys_size() {
    let mut wider = picture(3, 1);
    wider.width = WIDTH + 1;
    let mut shorter = picture(3, 1);
    shorter.height = HEIGHT - 1;
    assert_eq!(admit(Some(&entry(3)), CATALOG, &wider), Admit::Drop);
    assert_eq!(admit(Some(&entry(3)), CATALOG, &shorter), Admit::Drop);
    assert_eq!(admit(Some(&entry(3)), CATALOG, &picture(3, 1)), Admit::Swap);
}

#[test]
fn a_picture_of_another_font_catalog_is_dropped() {
    let mut stale = picture(3, 1);
    stale.catalog = FontCatalogGeneration::from_raw(7);
    assert_eq!(admit(Some(&entry(3)), CATALOG, &stale), Admit::Drop);
}

// --- ingest through the render round ---

#[test]
fn a_swap_marks_composed_with_its_v_and_settles_it() {
    let (mut app, producer, record) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    let revisions = post(&app, picture(3, 7));
    app.process_commands();
    assert_eq!(revisions.composed(), 7);
    assert_eq!(shown(&app).map(|head| head.v), Some(7));
    assert_eq!(settled(&record), vec![7]);
}

#[test]
fn a_dropped_picture_is_settled_and_never_composed() {
    let (mut app, producer, record) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    let revisions = post(&app, picture(2, 7));
    app.process_commands();
    assert_eq!(revisions.composed(), 0);
    assert_eq!(shown(&app), None);
    assert_eq!(settled(&record), vec![7]);
}

#[test]
fn a_held_picture_swaps_in_once_the_root_names_its_request() {
    let (mut app, producer, record) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    let revisions = post(&app, picture(4, 7));
    app.process_commands();
    assert_eq!((revisions.composed(), settled(&record)), (0, vec![]), "held: neither composed nor settled");
    send_root(&mut app, &producer, 2, vec![entry(4)]);
    assert_eq!((revisions.composed(), settled(&record)), (7, vec![7]));
}

#[test]
fn a_root_and_the_picture_for_its_request_in_one_round_compose_in_that_round() {
    let (mut app, producer, _) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    let revisions = post(&app, picture(4, 7));
    producer.cmd_tx.try_send(RenderCommand::Paint(root(2, vec![entry(4)]))).unwrap();
    app.process_commands();
    assert_eq!(revisions.composed(), 7, "ingest ran after the channel drain");
}

/// Residency by order: a poster racing the round sends a root naming
/// request 4 on the channel, then posts picture 4, just before the round
/// takes the mailbox. Taking after the drain would check the picture
/// against the old root and hold it.
#[test]
fn what_the_channel_held_before_a_post_is_applied_before_the_picture_is_checked() {
    let (mut app, producer, _) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    let revisions = Arc::new(PaneRevisions::default());
    let (mailbox, theirs, commands) = (app.panes.clone(), revisions.clone(), producer.cmd_tx.clone());
    let mut once = Some(());
    *app.panes.before_take.lock().unwrap() = Some(Box::new(move || {
        if once.take().is_some() {
            commands.try_send(RenderCommand::Paint(root(2, vec![entry(4)]))).unwrap();
            mailbox.post(OWNER, picture(4, 7), theirs.clone());
        }
    }));
    app.process_commands();
    assert_eq!(revisions.composed(), 7);
}

#[test]
fn the_newest_post_wins_and_the_one_it_replaced_is_settled() {
    let (mut app, producer, record) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    post(&app, picture(3, 7));
    let revisions = post(&app, picture(3, 8));
    app.process_commands();
    assert_eq!(revisions.composed(), 8);
    assert_eq!(settled(&record), vec![7, 8]);
}

#[test]
fn a_held_picture_replaced_by_a_newer_one_is_settled() {
    let (mut app, producer, record) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    post(&app, picture(4, 7));
    app.process_commands();
    post(&app, picture(5, 8));
    app.process_commands();
    assert_eq!(settled(&record), vec![7], "8 is held in 7's place");
}

#[test]
fn each_swap_replaces_the_panes_animation_set() {
    let (mut app, producer, _) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    post(&app, picture_with(3, 7, vec![animation(1), animation(2)]));
    app.process_commands();
    post(&app, picture_with(3, 8, vec![animation(2), animation(3)]));
    app.process_commands();
    let layers = &app.frame_windows.primary_window().unwrap().render.compositor.pane_layers;
    let keys = layers.layer(WINDOW).unwrap().shown.as_ref().unwrap().animations.keys();
    let window = WINDOW & 0xffff_ffff;
    assert_eq!(keys, vec![(2 << 32) | window, (3 << 32) | window]);
}

#[test]
fn a_root_naming_another_view_tears_the_layer_down() {
    let (mut app, producer, record) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    post(&app, picture(3, 7));
    app.process_commands();
    post(&app, picture(4, 8));
    app.process_commands();
    send_root(&mut app, &producer, 2, vec![ForeignPaneEntry { view: VIEW + 1, ..entry(4) }]);
    assert_eq!(shown(&app), None);
    assert_eq!(settled(&record), vec![7, 8], "the held picture is settled as it goes");
}

#[test]
fn every_posted_picture_is_settled_exactly_once() {
    let (mut app, producer, record) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    for (request, v) in [(3, 1), (2, 2), (4, 3), (4, 4), (3, 5)] {
        post(&app, picture(request, v));
        app.process_commands();
    }
    send_root(&mut app, &producer, 2, vec![]);
    let mut all = settled(&record);
    all.sort_unstable();
    assert_eq!(all, vec![1, 2, 3, 4, 5]);
}

// --- teardown: no picture left without its credit ---

/// Posted, then the render thread shuts down before its next round: the
/// exit closes the mailbox and settles what it held, and a post after it
/// settles at once.
#[test]
fn a_picture_posted_before_shutdown_is_settled_at_exit_and_one_after_at_once() {
    let (mut app, producer, record) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    post(&app, picture(3, 7));
    post(&app, picture(3, 8));
    producer.cmd_tx.try_send(RenderCommand::Lifecycle(crate::thread_comm::LifecycleCommand::Shutdown)).unwrap();
    assert!(app.process_commands(), "the shutdown ends the loop");
    app.handle_exiting();
    let mut all = settled(&record);
    all.sort_unstable();
    assert_eq!(all, vec![7, 8]);
    post(&app, picture(3, 9));
    assert_eq!(settled(&record).last(), Some(&9));
}

#[test]
fn a_held_picture_is_settled_when_the_primary_window_goes() {
    let (mut app, producer, record) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    post(&app, picture(4, 7));
    app.process_commands();
    assert_eq!(settled(&record), Vec::<u64>::new(), "held");
    producer.cmd_tx.try_send(RenderCommand::Window(WindowCommand::DestroyWindow { frame: FrameRef::Frame(1) })).unwrap();
    app.process_commands();
    assert_eq!(settled(&record), vec![7]);
}

// --- owners' states ---

fn state(code: u32) -> romacs_display_protocol::pane_sink::PaneState {
    romacs_display_protocol::pane_sink::PaneState { code, since: romacs_display_protocol::frame_time::observe_platform_now() }
}

#[test]
fn a_state_changes_the_frame_only_for_an_owner_it_shows() {
    use romacs_display_protocol::pane_sink::PaneState;
    let mut states = super::PaneStates::default();
    assert!(!states.update(vec![(OWNER + 1, state(PaneState::BUSY))], |owner| owner == OWNER));
    assert!(states.update(vec![(OWNER, state(PaneState::BUSY))], |owner| owner == OWNER));
}

#[test]
fn a_dead_owners_state_is_kept_while_shown_and_forgotten_after() {
    use romacs_display_protocol::pane_sink::PaneState;
    let mut states = super::PaneStates::default();
    let dead = state(PaneState::DEAD);
    states.update(vec![(OWNER, dead), (OWNER + 1, state(PaneState::BUSY))], |owner| owner == OWNER);
    assert_eq!(states.get(OWNER), Some(dead), "its pane still shows: the veil reads it");
    states.update(Vec::new(), |_| false);
    assert_eq!(states.get(OWNER), None, "dead and no longer shown");
    assert!(states.get(OWNER + 1).is_some(), "a live owner's state stays");
}

/// Owners' states are the app's: one posted while the window is hidden
/// reaches the panes of the window reopened.
#[test]
fn an_owners_state_outlives_a_hidden_window_and_reaches_the_reopened_one() {
    use romacs_display_protocol::pane_sink::PaneState;
    let (mut app, producer, _) = app();
    send_root(&mut app, &producer, 1, vec![entry(3)]);
    let pane_state = |app: &RenderApp| app.frame_windows.primary_window().unwrap().render.compositor.pane_layers.state(OWNER);
    let busy = state(PaneState::BUSY);
    app.panes.state(OWNER, busy);
    app.process_commands();
    assert_eq!(pane_state(&app), Some(busy));
    producer.cmd_tx.try_send(RenderCommand::Window(WindowCommand::DestroyWindow { frame: FrameRef::Frame(1) })).unwrap();
    app.process_commands();
    let wedged = state(PaneState::WEDGED | PaneState::QUITS);
    app.panes.state(OWNER, wedged);
    app.process_commands();
    app.frame_windows.prepare_primary(1, 800, 600, "reopened".into(), None);
    send_root(&mut app, &producer, 2, vec![entry(3)]);
    assert_eq!(pane_state(&app), Some(wedged), "the hook of the reopened pane reads it");
}
