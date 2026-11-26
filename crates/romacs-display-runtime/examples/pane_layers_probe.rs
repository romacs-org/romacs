//! O1b-3 GUI probe: a root frame naming one foreign pane, and a real pane
//! picture for it (laid out in owner slot 0, carrying a working thread)
//! posted through the pane sink: the render thread draws it as a layer
//! and runs the thread's motion. Driven by
//! `romacs-vm-gui/harness/pane_layers_live.py` on a private compositor;
//! never on the user's live desktop.
//!
//! usage: pane_layers_probe [--still] [--seconds N]
//! Prints `activated`, `posted` and `settled v=V` as they happen.
use romacs_display_protocol::foreign_pane::ForeignPaneEntry;
use romacs_display_protocol::pane_picture::{PaneFrame, PaneHead, PanePicture};
use romacs_display_protocol::pane_revisions::PaneRevisions;
use romacs_display_protocol::text_animation::{TextAnimation, TextAnimationKind};
use romacs_display_protocol::{
    Color, DisplayFrameId, DisplayWindowId, FrameGlyph, FrameGlyphBuffer, ParentFrameRect,
    PresentationId, PresentedFramePlacement, Rect,
};
use romacs_display_runtime::paint_host::{INITIAL_FRAME, PaintHost, PaintSender};
use romacs_display_runtime::thread_comm::InputEvent;
use romacs_layout::LayoutEngine;
use romacs_layout_input::{
    BandStretches, CursorKinds, Decor, Face, FaceHandle, FontAlternativeState, FontSelectionState,
    FrameChrome, FrameSnapshot, Hscroll, LineWrap, NoFont, PlacedWindowSnapshot, TextSlice,
    UnderlineRules, WindowChrome, WindowSnapshot,
};
use std::sync::Arc;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 400;
/// The same, in the frame's logical px.
const SIZE: (f32, f32) = (640.0, 400.0);
const PANE: Rect = Rect::new(40.0, 40.0, 480.0, 240.0);
const OWNER: u64 = 1;
const WINDOW: u64 = 900;
const OWN: u64 = 5;
const COLS: u32 = 24;

fn main() -> Result<(), String> {
    let still = std::env::args().any(|arg| arg == "--still");
    let seconds = std::env::args().skip_while(|arg| arg != "--seconds").nth(1).and_then(|n| n.parse().ok()).unwrap_or(20);
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR unset")?;
    let display = std::env::var_os("WAYLAND_DISPLAY").ok_or("WAYLAND_DISPLAY unset")?;
    let socket = std::path::Path::new(&runtime).join(display);
    let (host, sender) = PaintHost::wayland(&socket, WIDTH, HEIGHT, "pane layers probe".into())?;
    let producer = std::thread::spawn(move || produce(&sender, still, seconds));
    host.run().map_err(|error| error.to_string())?;
    producer.join().map_err(|_| "producer panicked")?
}

fn produce(sender: &PaintSender, still: bool, seconds: u64) -> Result<(), String> {
    let sink = sender.pane_sink(Box::new(|_, head| println!("settled v={}", head.v)));
    sender.submit(root()?)?;
    await_activation(sender, 1)?;
    println!("activated");
    // The root alone for a while: the harness shoots it before the pane.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let picture = picture(still)?;
    sink.post(OWNER, picture, Arc::new(PaneRevisions::default()));
    println!("posted");
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    sender.shutdown()
}

fn await_activation(sender: &PaintSender, revision: u64) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let timeout = deadline.saturating_duration_since(std::time::Instant::now());
        match sender.input().recv_timeout(timeout).map_err(|error| error.to_string())? {
            InputEvent::PresentationActivated { presentation, .. } if presentation == revision => return Ok(()),
            InputEvent::WindowClose { .. } => return Err("native host closed early".into()),
            _ => (),
        }
    }
}

/// The master's frame: a dark field naming the pane.
fn root() -> Result<FrameGlyphBuffer, String> {
    let mut frame = FrameGlyphBuffer::with_size(SIZE.0, SIZE.1);
    frame.presentation_id = PresentationId::new(1);
    frame.frame_placement = PresentedFramePlacement::new(
        DisplayFrameId::new(INITIAL_FRAME),
        frame.presentation_id,
        None,
        ParentFrameRect::new(0.0, 0.0, SIZE.0, SIZE.1).map_err(|error| format!("{error:?}"))?,
        0,
    );
    frame.background = Color::from_pixel(0x0020_2020);
    frame.foreign_panes = vec![ForeignPaneEntry { rect: PANE, window: WINDOW, owner: OWNER, view: 1, request: 1, cursor_off: false }];
    Ok(frame)
}

/// The owner's picture: a working thread's still frame on the first row,
/// then text; the thread moving unless STILL.
fn picture(still: bool) -> Result<PanePicture, String> {
    let mut engine = LayoutEngine::with_font_base(0)?;
    let text = format!("{} running\npane text, laid out by an owner\n", "\u{28ff}".repeat(COLS as usize));
    let frame = engine.layout_frame(&snapshot(&text), 1, Some((OWN, Color::from_pixel(0x00ff_ffff))))?.0;
    let animation = working(&frame, still);
    let head = PaneHead { owner: OWNER, window: WINDOW, view: 1, request: 1, v: 1, c: 1 };
    let pane = PaneFrame { frame, animations: vec![((1 << 32) | OWN, animation)], submitted: None };
    Ok(PanePicture::from_frame(head, pane, DisplayWindowId::new(OWN.cast_signed())))
}

/// A working thread on FRAME's first text row, COLS cells.
fn working(frame: &FrameGlyphBuffer, still: bool) -> TextAnimation {
    let top = frame.glyphs.iter().filter_map(|glyph| match glyph {
        FrameGlyph::Char { y, .. } => Some(*y),
        _ => None,
    }).fold(f32::INFINITY, f32::min);
    let kind = TextAnimationKind::Working { work: romacs_logo::working::Work::Running, beats: 0, epoch: 1, colour: [0x7a, 0xa2, 0xf7], bead: [0xff; 3] };
    TextAnimation { window: OWN, top: 0, cols: COLS, rows: 1, anchor: None, rise: Some((top, frame.char_height)), fps: 30, moving: !still, still: false, kind }
}

/// One window, the pane's box, showing TEXT with point at its start.
fn snapshot(text: &str) -> FrameSnapshot {
    let face = Face { decor: Decor::default(), foreground: [220; 3], background: [24, 24, 40], family: "Liberation Mono".into(), pixel_size: 18.0, weight: 400, italic: false, width: 5 };
    let text = TextSlice { extends: vec![], raw: vec![], utf8: text.into(), face_runs: vec![] };
    let (width, height) = (480, 240);
    let window = WindowSnapshot { underline_rules: UnderlineRules::default(), band_stretches: BandStretches::default(), id: OWN, width, height, start_byte: 0, point_byte: 0, buffer_version: 1,
        mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0), vscroll_px: 0, hscroll: Hscroll::default(),
        tab_width: 8, wrap: LineWrap::default(), cursor_off: false, cursor: CursorKinds::default(), stretches: vec![], images: vec![], wrap_flips: vec![], line_numbers: None, chrome: WindowChrome::default(),
        text, faces: vec![face] };
    FrameSnapshot { font_selection: FontSelectionState::default(), font_alternatives: FontAlternativeState::default(), fontsets: Arc::default(), width, height,
        windows: vec![PlacedWindowSnapshot { x: 0, y: 0, snapshot: window }], background_alpha: 1.0, children: vec![], tab_bar: None,
        chrome: FrameChrome::default(), status_bar: None, no_font: NoFont::default() }
}
