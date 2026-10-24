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

#[cfg(feature = "core-free")]
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
#[cfg(feature = "core-free")]
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
        .try_send(RenderCommand::Paint(Box::new(malformed)))
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
        .try_send(RenderCommand::Paint(Box::new(frame)))
        .unwrap();
    assert!(!app.process_commands());
    producer
        .cmd_tx
        .try_send(RenderCommand::Paint(Box::new(paint(1, Color::WHITE))))
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
#[cfg(feature = "core-free")]
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
            .try_send(RenderCommand::Paint(Box::new(paint(revision, color))))
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
