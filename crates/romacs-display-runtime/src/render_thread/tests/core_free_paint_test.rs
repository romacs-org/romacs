use super::RenderApp;
use crate::thread_comm::{FrameRef, RenderCommand, ThreadComms, WindowCommand};
use romacs_display_protocol::{
    Color, DisplayFrameId, FrameGlyphBuffer, ParentFrameRect, PresentationId,
    PresentedFramePlacement,
};

pub(super) fn make_test_app() -> RenderApp {
    let (_producer, render) = ThreadComms::new().split();
    RenderApp::new(
        render,
        800,
        600,
        "core-free test".into(),
        std::sync::Arc::default(),
        std::sync::Arc::default(),
        true,
    )
}

fn paint(revision: u64, background: Color) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(800.0, 600.0);
    frame.presentation_id = PresentationId::new(revision);
    frame.frame_placement = PresentedFramePlacement::new(
        DisplayFrameId::new(1),
        frame.presentation_id,
        None,
        ParentFrameRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
        0,
    );
    frame.background = background;
    frame
        .char_fonts
        .insert(romacs_display_protocol::FaceId::new(77), Default::default());
    frame
}

#[test]
fn core_free_paint_rejects_incoherent_and_stale_presentations() {
    let (producer, render) = ThreadComms::new().split();
    let mut app = make_test_app();
    app.comms = render;
    producer
        .cmd_tx
        .try_send(RenderCommand::Window(WindowCommand::AdoptPrimaryFrame {
            frame: FrameRef::Frame(1),
        }))
        .unwrap();
    let frame = paint(2, Color::RED);
    assert!(crate::paint_host::validate_frame(&frame).is_ok());
    let mut malformed = frame.clone();
    malformed.presentation_id = PresentationId::new(3);
    assert!(crate::paint_host::validate_frame(&malformed).is_err());
    producer
        .cmd_tx
        .try_send(RenderCommand::Paint(Box::new(malformed.into())))
        .unwrap();
    assert!(!app.process_commands());
    assert!(
        app.frame_windows
            .primary_window()
            .unwrap()
            .render
            .compositor
            .current_frame
            .is_none()
    );
    producer
        .cmd_tx
        .try_send(RenderCommand::Paint(Box::new(frame.into())))
        .unwrap();
    assert!(!app.process_commands());
    producer
        .cmd_tx
        .try_send(RenderCommand::Paint(Box::new(paint(1, Color::WHITE).into())))
        .unwrap();
    assert!(!app.process_commands());
    let active = app
        .frame_windows
        .primary_window()
        .unwrap()
        .render
        .compositor
        .current_frame
        .as_ref()
        .unwrap();
    assert_eq!(active.presentation_id.get(), 2);
    assert_eq!(active.background, Color::RED);
    assert!(producer.input_rx.try_iter().any(|event| matches!(
        event,
        crate::thread_comm::InputEvent::PresentationDiscarded {
            presentation: 1,
            ..
        }
    )));
}

#[test]
fn core_free_paint_command_installs_and_replaces_scene() {
    let (producer, render) = ThreadComms::new().split();
    let mut app = make_test_app();
    app.comms = render;
    producer
        .cmd_tx
        .try_send(RenderCommand::Window(WindowCommand::AdoptPrimaryFrame {
            frame: FrameRef::Frame(1),
        }))
        .unwrap();
    for (revision, color) in [(1, Color::RED), (2, Color::WHITE)] {
        app.frame_windows
            .primary_window_mut()
            .unwrap()
            .render
            .compositor
            .dirty = false;
        producer
            .cmd_tx
            .try_send(RenderCommand::Paint(Box::new(paint(revision, color).into())))
            .unwrap();
        assert!(!app.process_commands());
        let active = app
            .frame_windows
            .primary_window()
            .unwrap()
            .render
            .compositor
            .current_frame
            .as_ref();
        assert!(
            active.is_some(),
            "Seam P command must install a frame in the shared compositor"
        );
        let active = active.unwrap();
        assert_eq!(
            active.presentation_id.get(),
            revision,
            "mutation must replace the scene"
        );
        assert_eq!(active.background, color);
        assert!(
            active
                .char_fonts
                .contains_key(&romacs_display_protocol::FaceId::new(77)),
            "font projection must survive the host ingress"
        );
        assert!(
            app.frame_windows
                .primary_window()
                .unwrap()
                .render
                .compositor
                .dirty,
            "a submitted mutation must demand repaint"
        );
    }
}

fn child(id: u64, revision: u64, x: f32) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(120.0, 40.0);
    frame.presentation_id = PresentationId::new(revision);
    frame.frame_placement = PresentedFramePlacement::new(
        DisplayFrameId::new(id),
        frame.presentation_id,
        Some(DisplayFrameId::new(1)),
        ParentFrameRect::new(x, 30.0, 120.0, 40.0).unwrap(),
        0,
    );
    frame
}

/// A paint scene's child frames are installed over the primary frame, move
/// with the next scene, and go when a scene leaves them out; a child that
/// is not the primary presentation's is refused with its scene.
#[test]
fn core_free_paint_scene_installs_moves_and_removes_child_frames() {
    let (producer, render) = ThreadComms::new().split();
    let mut app = make_test_app();
    app.comms = render;
    producer
        .cmd_tx
        .try_send(RenderCommand::Window(WindowCommand::AdoptPrimaryFrame {
            frame: FrameRef::Frame(1),
        }))
        .unwrap();
    let scene = |revision: u64, children: Vec<FrameGlyphBuffer>| {
        Box::new(crate::thread_comm::PaintScene { root: paint(revision, Color::RED), children, text_animations: Vec::new() })
    };
    let placed = |app: &RenderApp| -> Vec<(u64, f32)> {
        let frames = &app.frame_windows.primary_window().unwrap().render.compositor.child_frames.frames;
        let mut placed: Vec<_> = frames.iter().map(|(&id, entry)| (id, entry.frame.frame_placement.outer_in_parent().x())).collect();
        placed.sort_by_key(|&(id, _)| id);
        placed
    };
    producer.cmd_tx.try_send(RenderCommand::Paint(scene(1, vec![child(2, 1, 10.0), child(3, 1, 50.0)]))).unwrap();
    app.process_commands();
    assert_eq!(placed(&app), vec![(2, 10.0), (3, 50.0)]);
    producer.cmd_tx.try_send(RenderCommand::Paint(scene(2, vec![child(3, 2, 70.0)]))).unwrap();
    app.process_commands();
    assert_eq!(placed(&app), vec![(3, 70.0)], "the next scene moves one child and drops the other");
    producer.cmd_tx.try_send(RenderCommand::Paint(scene(3, vec![child(3, 2, 90.0)]))).unwrap();
    app.process_commands();
    assert_eq!(placed(&app), vec![(3, 70.0)], "a child of another presentation is refused with its scene");
    producer.cmd_tx.try_send(RenderCommand::Paint(scene(4, vec![]))).unwrap();
    app.process_commands();
    assert!(placed(&app).is_empty());
    producer.cmd_tx.try_send(RenderCommand::Paint(scene(5, vec![child(3, 5, 20.0)]))).unwrap();
    app.process_commands();
    assert_eq!(placed(&app), vec![(3, 20.0)], "a removed child shows again in a later scene");
}
