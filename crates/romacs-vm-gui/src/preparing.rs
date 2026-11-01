//! What a first launch shows while it builds its world: the armillary
//! sphere (romacs_armillary), as the *Romacs* landing draws it, with
//! the message under it, painted by the host before any VM can.  It turns
//! at 25 frames a second, on a thread of its own, where the display
//! animates (`prompt::display_animates`); elsewhere, every headless
//! harness, it stays the first frame.
use romacs_display_runtime::paint_host::PaintSender;
use romacs_layout_input::{Face, FaceHandle, FaceRun, FrameSnapshot, PlacedWindowSnapshot, TextSlice, WindowSnapshot};
use romacs_armillary::{cells, shade_runs, text, View};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const MESSAGE: &str = "Preparing Romacs (first start)…";

/// The site's frame interval.
const PERIOD: Duration = Duration::from_millis(40);

/// The turning screen: [`Preparing::finish`] stops it.
pub struct Preparing {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<u64>,
}

impl Preparing {
    /// Stop turning; the presentation the launch's own frames start at,
    /// after every one this screen may have submitted.
    pub fn finish(self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().expect("preparing painter panicked")
    }
}

/// The sphere's shades, dark to light: the default face's foreground
/// blended toward its background, then itself.
fn shades(face: &Face) -> Vec<Face> {
    let blend = |t: f32| std::array::from_fn(|i| {
        (f32::from(face.background[i]) + (f32::from(face.foreground[i]) - f32::from(face.background[i])) * t).round() as u8
    });
    [0.4, 0.6, 0.8, 1.0].map(|t| Face { foreground: blend(t), ..face.clone() }).into()
}

/// The screen for the sphere's YAW and PHASE (the site's motion) in a
/// WIDTH by HEIGHT window of CELL (line height, character width) cells.
fn snapshot(face: &Face, width: u32, height: u32, cell: (u32, u32), yaw: f64, phase: f64) -> FrameSnapshot {
    let (line, char_width) = (cell.0.max(1), cell.1.max(1));
    let (columns, lines) = ((width / char_width) as usize, (height / line) as usize);
    let view = View {
        cols: columns.saturating_sub(1).clamp(1, 200), rows: lines.saturating_sub(3).clamp(1, 100),
        pitch: -0.25 + (phase * 0.47).sin() * 0.12, yaw, roll: -0.15 + (phase * 0.3).sin() * 0.1, zoom: 1.0,
        cell_aspect: (f64::from(char_width) / f64::from(line)).clamp(0.05, 2.0), stars: true,
    };
    let cells = cells(&view);
    let mut faces = vec![face.clone()];
    faces.extend(shades(face));
    let face_runs = shade_runs(&cells, view.cols, 4).into_iter()
        .map(|(bytes, shade)| FaceRun { bytes, face: FaceHandle(shade as u32 + 1) }).collect();
    let indent = " ".repeat(view.cols.saturating_sub(MESSAGE.chars().count()) / 2);
    let utf8 = format!("{}\n\n{indent}{MESSAGE}", text(&cells, view.cols));
    FrameSnapshot {
        font_selection: Default::default(), font_alternatives: Default::default(), width, height,
        background_alpha: 1.0, children: vec![], tab_bar: None, windows: vec![PlacedWindowSnapshot { x: 0, y: 0, snapshot: WindowSnapshot {
            id: 1, width, height, start_byte: 0, point_byte: 0, buffer_version: 0,
            text: TextSlice { utf8, face_runs, raw: vec![] }, mode_line: None,
            mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0), vscroll_px: 0, tab_width: 8,
            wrap: Default::default(), cursor_off: true, stretches: vec![], line_numbers: None, chrome: Default::default(), faces,
        } }],
    }
}

/// Paint the screen as presentation 1, before any VM can, laid out by an
/// engine of its own on a thread of its own; then turn it there until
/// [`Preparing::finish`].  Painted means submitted: the first frame never
/// waits for the GPU to start.
pub fn start(sender: Arc<PaintSender>, width: u32, height: u32) -> Result<Preparing, String> {
    let face = romacs_vm::Vm::new().display_default_face()?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let (first_tx, first) = std::sync::mpsc::channel();
    let thread = std::thread::Builder::new().name("romacs-preparing".into()).spawn(move || {
        let mut engine = None;
        let mut paint = |presentation: u64, yaw: f64, phase: f64| -> Result<(), String> {
            let engine = match &mut engine { Some(engine) => engine, None => engine.insert(romacs_layout2::LayoutEngine::new()?) };
            let cell = engine.frame_metrics(&face)?;
            let (frame, _) = engine.layout_frame(&snapshot(&face, width, height, cell, yaw, phase), presentation, None)?;
            sender.submit(frame)
        };
        let (mut yaw, mut phase, mut presentation) = (0.4, 0.0, 1);
        let painted = paint(presentation, yaw, phase);
        let failed = painted.is_err();
        let _ = first_tx.send(painted);
        if failed { return presentation; }
        let mut last = Instant::now();
        while !stopped.load(Ordering::Relaxed) {
            std::thread::sleep(PERIOD.saturating_sub(last.elapsed()));
            match crate::prompt::display_animates(&sender) {
                Some(true) => {}
                Some(false) => break,
                // The adapter is chosen as the first frame paints.
                None => { last = Instant::now(); continue; }
            }
            let dt = last.elapsed().min(Duration::from_millis(80)).as_secs_f64();
            last = Instant::now();
            (yaw, phase) = (yaw + dt * 0.16, phase + dt * 0.2);
            // Counted before it is painted: a frame submitted and not
            // rendered in time still holds its presentation.
            presentation += 1;
            if paint(presentation, yaw, phase).is_err() { break; }
            // Paced by the renderer: the next frame waits for this one, a
            // slow one only slows the turning.
            let _ = sender.wait_for_render(presentation, Duration::from_secs(1));
        }
        presentation + 1
    }).map_err(|e| e.to_string())?;
    first.recv().map_err(|_| "preparing painter stopped".to_owned())??;
    Ok(Preparing { stop, thread })
}
