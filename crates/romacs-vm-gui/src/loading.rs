//! The launch screen: from the window's first moment until the session's
//! first frame, the Romacs logo (romacs_logo) as the *Romacs* landing draws
//! it, what startup is doing, and a bar of how far it got.  The host paints
//! it on a thread of its own before any VM can.  Its light sweeps at 25
//! frames a second where the display animates
//! (`presentation::display_animates`); elsewhere (every headless harness) the
//! screen is painted again only when its words or its bar change.
//!
//! Progress is reported, never guessed.  Every startup phase
//! (`startup::mark_phase`) and every step Lisp reports
//! (`romacs-startup-progress`: the init's loads, the agents a server
//! restores, a restart's buffers) is an event.  The bar is paced by the
//! last launch's events, which `launch-profile.json` in the user's Romacs
//! state directory keeps.  The bar sits between the times the last launch
//! reached the newest event seen and the one after it, and moves with the
//! clock in between.  It never passes an event before that event happens,
//! and an event the last launch did not have holds it still.  Nothing is
//! hidden or refused: the screen only shows what startup says it does.
use romacs_display_runtime::paint_host::PaintSender;
use romacs_layout_input::{Face, FaceHandle, FaceRun, FrameSnapshot, PlacedWindowSnapshot, TextSlice, WindowSnapshot};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What a first launch says while it builds its world.
pub const FIRST_START: &str = "Preparing Romacs (first start)…";

/// The screen's frame interval.
const PERIOD: Duration = Duration::from_millis(40);
/// How often a screen that does not animate looks for news.
const STILL_PERIOD: Duration = Duration::from_millis(100);
/// The bar's end until the session's first frame takes over.
const CAP: f64 = 0.97;
/// With no launch measured yet, the seconds at which the bar is half full:
/// it then nears its end and never reaches it.
const FIRST_GUESS: f64 = 4.0;
/// The bar's width at most, in columns.
const BAR: usize = 48;
/// The words over the bar once startup failed.
const FAILED: &str = "Startup failed";
/// The rows a failed startup's error shows at most.
const ERROR_ROWS: usize = 8;
/// A failed step's colour.
const FAILED_COLOUR: u32 = 0x00ff_5f59;
/// The stage of the step a failed startup was at.
const FAILED_STAGE: &str = "failed";

/// One launch's events in order: each key (a stage, `#N` after it when it
/// came N times) and its time in seconds since the screen began.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub events: Vec<(String, f64)>,
}

impl Profile {
    fn read(path: &std::path::Path) -> Self {
        let parsed = std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
        let events = parsed.as_ref().and_then(|value| value.get("events")).and_then(serde_json::Value::as_array)
            .map(|events| events.iter().filter_map(|event| Some((event.get(0)?.as_str()?.to_owned(), event.get(1)?.as_f64()?))).collect())
            .unwrap_or_default();
        Self { events }
    }

    fn write(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::json!({"events": self.events.iter().map(|(key, at)| serde_json::json!([key, (at * 1000.0).round() / 1000.0])).collect::<Vec<_>>()});
        let staged = path.with_extension("json.new");
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
        std::fs::write(&staged, json.to_string())?;
        std::fs::rename(staged, path)
    }
}

/// Where the bar is, 0 to [`CAP`], ELAPSED seconds into a launch that saw
/// SEEN, paced by EXPECTED (the last launch's events).
pub fn fraction(expected: &Profile, seen: &[(String, f64)], elapsed: f64) -> f64 {
    let Some(total) = expected.events.last().map(|event| event.1.max(0.1)) else {
        return (elapsed / (elapsed + FIRST_GUESS)).min(CAP);
    };
    let reached = seen.iter().rev().find_map(|(key, _)| expected.events.iter().position(|(k, _)| k == key));
    let (low, high) = match reached {
        Some(i) => (expected.events[i].1, expected.events.get(i + 1).map_or(total, |event| event.1)),
        None => (0.0, expected.events.first().map_or(total, |event| event.1)),
    };
    (elapsed.clamp(low, high.max(low)) / total).min(CAP)
}

/// A step that counts how far it is (`romacs-progress-functions': a
/// restore's buffers, the agent sessions), as it last said.
#[derive(Clone, Debug, PartialEq)]
struct Step {
    stage: String,
    label: String,
    /// Done of how many.
    count: Option<(u64, u64)>,
    failed: bool,
}

/// What startup reported so far, while the screen shows.
struct Report {
    began: Instant,
    expected: Profile,
    seen: Vec<(String, f64)>,
    counts: HashMap<String, u32>,
    label: String,
    /// The steps that count, in the order they first spoke.
    steps: Vec<Step>,
    /// When startup failed, in seconds since the screen began, and why.
    failed: Option<(f64, String)>,
}

static REPORT: Mutex<Option<Report>> = Mutex::new(None);

fn report() -> std::sync::MutexGuard<'static, Option<Report>> {
    REPORT.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Report {
    fn new(expected: Profile) -> Self {
        Self { began: Instant::now(), expected, seen: vec![], counts: HashMap::new(), label: "Opening the world".to_owned(), steps: vec![], failed: None }
    }

    /// STAGE reached, now at LABEL (empty: the words stay); a step that
    /// counts or failed (COUNT, FAILED) says LABEL on its own line.
    fn step(&mut self, stage: &str, label: &str, count: Option<(u64, u64)>, failed: bool) {
        let n = self.counts.entry(stage.to_owned()).or_insert(0);
        *n += 1;
        let key = if *n == 1 { stage.to_owned() } else { format!("{stage}#{n}") };
        self.seen.push((key, self.began.elapsed().as_secs_f64()));
        let own_line = count.is_some() || failed;
        if !label.is_empty() && !own_line { label.clone_into(&mut self.label); }
        if own_line {
            let line = Step { stage: stage.to_owned(), label: if label.is_empty() { stage.to_owned() } else { label.to_owned() }, count, failed };
            match self.steps.iter_mut().find(|s| s.stage == stage) {
                Some(known) => *known = line,
                None => self.steps.push(line),
            }
        }
    }

    /// Startup failed with ERROR: the step it was at fails, and the bar
    /// stops where it was.
    /// Startup failed with ERROR: the step it was at fails, FIRST (the
    /// first error of Romacs's Lisp startup went on past, often its cause)
    /// shows above ERROR, and the bar stops where it was.
    fn fail(&mut self, error: &str, first: Option<&str>) {
        let at = std::mem::replace(&mut self.label, FAILED.to_owned());
        self.step(FAILED_STAGE, &at, None, true);
        let shown = if let Some(first) = first.filter(|first| *first != error) {
            format!("{first}\n\nThen: {error}")
        } else {
            error.to_owned()
        };
        self.failed = Some((self.began.elapsed().as_secs_f64(), shown));
    }

    /// What the screen says now.
    fn now(&self) -> Now {
        let elapsed = self.failed.as_ref().map_or_else(|| self.began.elapsed().as_secs_f64(), |(at, _)| *at);
        Now { label: self.label.clone(), filled: fraction(&self.expected, &self.seen, elapsed), steps: self.steps.clone(),
              error: self.failed.as_ref().map(|(_, error)| error.clone()) }
    }
}

/// Startup reached STAGE, now at LABEL (empty: the words stay).  Nothing
/// once the screen has gone, or when none shows.
pub fn step(stage: &str, label: &str) { step_counted(stage, label, None, false); }

/// [`step`], for a step that counts (COUNT: done of how many) or FAILED:
/// whether a screen showed it.
pub fn step_counted(stage: &str, label: &str, count: Option<(u64, u64)>, failed: bool) -> bool {
    report().as_mut().map(|report| report.step(stage, label, count, failed)).is_some()
}

/// Startup failed with ERROR: the screen shows the step it was at failed,
/// ERROR under it with FIRST above it (the first error of Romacs's Lisp
/// startup went on past), and its bar stopped.  Nothing when no screen
/// shows.
pub fn fail(error: &str, first: Option<&str>) {
    if let Some(report) = report().as_mut() { report.fail(error, first); }
}

/// What a finished startup phase (`startup::mark_phase`) leads to next,
/// in the user's words; None keeps the words.
fn after(phase: &str) -> Option<&'static str> {
    Some(match phase {
        "world-open" => "Restoring the world",
        "world-restored" | "world-none" => "Starting Lisp",
        "session-lisp" => "Loading your init",
        "user-init" => "Finishing startup",
        "finish-loadup" => "Starting the session",
        "server" => "Drawing",
        _ => return None,
    })
}

/// Startup phase PHASE ended: an event, with what comes next.
pub fn phase(phase: &str) {
    if let Some(report) = report().as_mut() { report.phase(phase); }
}

impl Report {
    fn phase(&mut self, phase: &str) { self.step(phase, after(phase).unwrap_or(""), None, false); }
}

/// What the screen says at one moment.
#[derive(Clone, Debug, Default, PartialEq)]
struct Now {
    label: String,
    /// The bar, 0 to 1.
    filled: f64,
    steps: Vec<Step>,
    /// Why startup failed, once it did.
    error: Option<String>,
}

impl Now {
    /// Whether NEXT shows what this does: the same words and steps, the
    /// bar at the same cell.
    fn looks_like(&self, next: &Self) -> bool {
        let cells = |f: f64| (f * BAR as f64).round() as usize;
        self.label == next.label && self.steps == next.steps && self.error == next.error && cells(self.filled) == cells(next.filled)
    }
}

/// The words, the bar, the counting steps and a failure's error now.
fn view() -> Option<Now> {
    report().as_ref().map(Report::now)
}

/// The user's Romacs state directory's profile of launches.
pub fn profile_path() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|state| state.join("romacs/launch-profile.json"))
}

/// The screen: [`Loading::finish`] hands its window to the session;
/// dropped unfinished (startup failed first) it stops, and keeps no pace.
pub struct Loading {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<u64>>,
    profile: Option<PathBuf>,
}

impl Loading {
    /// End the screen; the presentation the session's own frames start
    /// at, after every one this screen may have submitted.  This launch's
    /// events become the next one's pace.
    pub fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        let presentation = self.thread.take().map_or(1, |thread| thread.join().expect("loading painter panicked"));
        if let Some(report) = report().take() && let Some(path) = self.profile.take() {
            let mut events = report.seen;
            events.push(("first-frame".to_owned(), report.began.elapsed().as_secs_f64()));
            // Beside the first frame, not before it.
            std::thread::spawn(move || {
                if let Err(error) = (Profile { events }).write(&path) {
                    eprintln!("romacs: launch profile {}: {error}", path.display());
                }
            });
        }
        presentation
    }
}

impl Drop for Loading {
    /// Unfinished: the painter stops and lets the window go, and what was
    /// reported is dropped.  After `finish`, nothing is left to do.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
        report().take();
    }
}

/// The colour of COLOUR (0xRRGGBB).
fn rgb(colour: u32) -> [u8; 3] { [(colour >> 16) as u8, (colour >> 8) as u8, colour as u8] }

/// Halfway between A and B.
fn between(a: [u8; 3], b: [u8; 3]) -> [u8; 3] { [0, 1, 2].map(|i| ((u16::from(a[i]) + u16::from(b[i])) / 2) as u8) }

/// What one frame of the screen shows.
struct Shown<'a> {
    /// Seconds into the logo's motion.
    t: f64,
    /// The logo's share (at most 1) of the rows the words leave it.
    scale: f64,
    /// What startup is doing.
    label: &'a str,
    /// How full the bar is, 0 to 1.
    filled: f64,
    /// A line each, under the bar.
    steps: &'a [Step],
    /// Why startup failed, under the steps.
    error: Option<&'a str>,
}

/// TEXT in rows of at most WIDTH characters, its own lines kept, and at
/// most ROWS of them: the last of rows cut short ends in an ellipsis.
fn wrapped(text: &str, width: usize, rows: usize) -> Vec<String> {
    let width = width.max(1);
    let mut all = text.lines().flat_map(|line| {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() { vec![String::new()] } else { chars.chunks(width).map(|row| row.iter().collect()).collect() }
    });
    let mut kept: Vec<String> = all.by_ref().take(rows).collect();
    if all.next().is_some() && let Some(last) = kept.last_mut() {
        if last.chars().count() >= width { last.pop(); }
        last.push('…');
    }
    kept
}

/// SHOWN in a WIDTH by HEIGHT window of CELL (line height, character
/// width) cells: the logo, and under it the words over the bar.
fn snapshot(face: &Face, (width, height): (u32, u32), cell: (u32, u32), shown: &Shown<'_>) -> FrameSnapshot {
    let Shown { t, scale, label, filled, steps, error } = *shown;
    let (line, char_width) = (cell.0.max(1), cell.1.max(1));
    let (columns, lines) = ((width / char_width) as usize, (height / line) as usize);
    let cols = columns.saturating_sub(1).clamp(1, 400);
    // A failure's words come first: when the other steps leave no row for
    // its error, only the step it was at shows; its error gets the rows
    // the words and the bar leave, and the logo only rows left over.
    let fits = error.is_none() || lines >= 3 + steps.len();
    let steps: Vec<&Step> = steps.iter().filter(|step| fits || step.stage == FAILED_STAGE).collect();
    let error_rows = ERROR_ROWS.min(lines.saturating_sub(2 + steps.len()).max(1));
    let error = error.map(|error| wrapped(error, cols.saturating_sub(4), error_rows)).unwrap_or_default();
    let room = lines.saturating_sub(5 + steps.len() + error.len());
    let view = romacs_logo::View {
        cols,
        rows: ((room as f64 * scale) as usize).clamp(1, 200),
        cell_aspect: (f64::from(char_width) / f64::from(line)).clamp(0.05, 2.0),
    };
    let cells = if room > 0 || error.is_empty() { romacs_logo::frame(view, t) } else { Vec::new() };
    // A face per colour, like the default one.
    let mut faces = vec![face.clone()];
    let mut face_runs: Vec<FaceRun> = vec![];
    let handle = |faces: &mut Vec<Face>, foreground: [u8; 3]| match faces.iter().position(|f| f.foreground == foreground) {
        Some(index) if index > 0 => FaceHandle(index as u32),
        _ => { faces.push(Face { foreground, ..face.clone() }); FaceHandle((faces.len() - 1) as u32) }
    };
    let run = |runs: &mut Vec<FaceRun>, bytes: std::ops::Range<usize>, face: FaceHandle| match runs.last_mut() {
        Some(last) if last.face == face && last.bytes.end == bytes.start => last.bytes.end = bytes.end,
        _ => runs.push(FaceRun { bytes, face }),
    };
    let mut at = 0;
    for (i, &(c, colour)) in cells.iter().enumerate() {
        if i > 0 && i % view.cols == 0 { at += 1; }
        let end = at + c.len_utf8();
        if let Some(colour) = colour {
            let face = handle(&mut faces, rgb(colour));
            run(&mut face_runs, at..end, face);
        }
        at = end;
    }
    let centred = |text_width: usize| " ".repeat(view.cols.saturating_sub(text_width) / 2);
    let logo = if cells.is_empty() { String::new() } else { format!("{}\n\n", romacs_logo::text(&cells, view.cols)) };
    let mut utf8 = format!("{logo}{}{label}\n", centred(label.chars().count()));
    let bar = BAR.min(view.cols.saturating_sub(4)).max(1);
    let full = ((filled.clamp(0.0, 1.0) * bar as f64).round() as usize).min(bar);
    utf8.push_str(&centred(bar));
    let accent = romacs_logo::mark_paths().first().map_or(face.foreground, |(colour, _)| rgb(*colour));
    let (lit, dim) = (handle(&mut faces, accent), handle(&mut faces, between(face.foreground, face.background)));
    let start = utf8.len();
    utf8.push_str(&"━".repeat(full));
    run(&mut face_runs, start..utf8.len(), lit);
    let rest = utf8.len();
    utf8.push_str(&"─".repeat(bar - full));
    run(&mut face_runs, rest..utf8.len(), dim);
    // Each counting step: its words over a bar of its own, or its failure.
    let short = (bar / 3).max(4);
    for step in &steps {
        let line = match (step.failed, step.count) {
            (true, _) => format!("✗ {}", step.label),
            (false, Some((done, of))) => {
                let lit_cells = if of == 0 { short } else { ((done.min(of) as f64 / of as f64) * short as f64).round() as usize };
                format!("{}  {}{}  {done}/{of}", step.label, "━".repeat(lit_cells), "─".repeat(short - lit_cells))
            }
            (false, None) => step.label.clone(),
        };
        utf8.push('\n');
        utf8.push_str(&centred(line.chars().count()));
        let start = utf8.len();
        utf8.push_str(&line);
        if step.failed { run(&mut face_runs, start..utf8.len(), handle(&mut faces, rgb(FAILED_COLOUR))); }
    }
    // Why startup failed: a block, centred as one.
    let block = centred(error.iter().map(|row| row.chars().count()).max().unwrap_or(0));
    for row in &error {
        utf8.push('\n');
        utf8.push_str(&block);
        utf8.push_str(row);
    }
    FrameSnapshot {
        font_selection: Default::default(), font_alternatives: Default::default(), fontsets: Default::default(), no_font: Default::default(), width, height,
        background_alpha: 1.0, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None, windows: vec![PlacedWindowSnapshot { x: 0, y: 0, snapshot: WindowSnapshot { underline_rules: Default::default(), band_stretches: Default::default(),
            id: 1, width, height, start_byte: 0, point_byte: 0, buffer_version: 0,
            text: TextSlice { extends: vec![], utf8, face_runs, raw: vec![] }, mode_line: None,
            mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0), vscroll_px: 0, hscroll: Default::default(), tab_width: 8,
            wrap: Default::default(), cursor_off: true, cursor: Default::default(), stretches: vec![], images: vec![], wrap_flips: vec![], line_numbers: None, chrome: Default::default(), faces,
        } }],
    }
}

/// Show the screen as presentation 1, before any VM can, laid out by an
/// engine of its own on a thread of its own; it keeps reporting until
/// [`Loading::finish`].  Painted means submitted: the first frame never
/// waits for the GPU to start.  PROFILE paces the bar, and this launch's
/// events are written there when it ends.
pub fn start(sender: Arc<PaintSender>, width: u32, height: u32, profile: Option<PathBuf>) -> Result<Loading, String> {
    let face = romacs_vm::Vm::new().display_default_face()?;
    let expected = profile.as_deref().map(Profile::read).unwrap_or_default();
    *report() = Some(Report::new(expected));
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let (first_tx, first) = std::sync::mpsc::channel();
    let thread = std::thread::Builder::new().name("romacs-loading".into()).spawn(move || {
        let mut engine = None;
        // The logo's share of its rows, by window size: its braille rows
        // may stand taller than the face's cell, so the words and the bar
        // under it are laid out again until they show.
        let mut scales: HashMap<(u32, u32), f64> = HashMap::new();
        let mut paint = |presentation: u64, (width, height): (u32, u32), t: f64, now: &Now| -> Result<(), String> {
            let engine = match &mut engine { Some(engine) => engine, None => engine.insert(romacs_layout::LayoutEngine::new()?) };
            let cell = engine.frame_metrics(&face)?;
            let scale = scales.entry((width, height)).or_insert(1.0);
            let mut tries = 0;
            loop {
                let screen = snapshot(&face, (width, height), cell, &Shown { t, scale: *scale, label: &now.label, filled: now.filled, steps: &now.steps,
                                                                              error: now.error.as_deref() });
                let length = screen.windows[0].snapshot.text.utf8.len();
                let (frame, ends) = engine.layout_frame(&screen, presentation, None)?;
                tries += 1;
                if tries > 5 || ends.first().is_none_or(|end| end.visible_end > length) { return sender.submit(frame); }
                *scale *= 0.88;
            }
        };
        // The window's size as the compositor made it, once it says.
        let size = || sender.window_size(romacs_display_runtime::paint_host::INITIAL_FRAME).unwrap_or((width, height));
        let (mut t, mut presentation, mut shown) = (0.0, 1, size());
        let mut now = view().unwrap_or_default();
        let painted = paint(presentation, shown, t, &now);
        let failed = painted.is_err();
        let _ = first_tx.send(painted);
        if failed { return presentation; }
        let mut last = Instant::now();
        while !stopped.load(Ordering::Relaxed) {
            let animates = crate::presentation::display_animates(&sender);
            std::thread::sleep(if animates == Some(true) { PERIOD } else { STILL_PERIOD }.saturating_sub(last.elapsed()));
            // The adapter is chosen as the first frame paints.
            if animates.is_none() { last = Instant::now(); continue; }
            let dt = last.elapsed().min(Duration::from_millis(80)).as_secs_f64();
            last = Instant::now();
            let (next, size_now) = (view().unwrap_or_default(), size());
            if animates == Some(true) {
                t += dt;
            } else if now.looks_like(&next) && size_now == shown {
                continue;
            }
            (now, shown) = (next, size_now);
            // Counted before it is painted: a frame submitted and not
            // rendered in time still holds its presentation.
            presentation += 1;
            if paint(presentation, shown, t, &now).is_err() { break; }
            // Paced by the renderer: the next frame waits for this one, a
            // slow one only slows the motion. In slices, so a stop (the
            // session's first frame) ends the wait at once rather than
            // after a frame's second.
            let end = Instant::now() + Duration::from_secs(1);
            while !stopped.load(Ordering::Relaxed)
                && sender.wait_for_render(romacs_display_runtime::paint_host::INITIAL_FRAME, presentation, Duration::from_millis(20)).is_err()
                && Instant::now() < end {}
        }
        presentation + 1
    }).map_err(|e| e.to_string())?;
    if let Err(error) = first.recv().map_err(|_| "loading painter stopped".to_owned()).and_then(|painted| painted) {
        report().take();
        return Err(error);
    }
    Ok(Loading { stop, thread: Some(thread), profile })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(events: &[(&str, f64)]) -> Profile {
        Profile { events: events.iter().map(|(k, t)| ((*k).to_owned(), *t)).collect() }
    }

    fn seen(events: &[(&str, f64)]) -> Vec<(String, f64)> { profile(events).events }

    #[test]
    fn the_bar_moves_with_the_clock_between_the_events_it_reached() {
        let last = profile(&[("world-open", 1.0), ("init", 2.0), ("init#2", 3.0), ("first-frame", 4.0)]);
        assert_eq!(fraction(&last, &[], 0.5), 0.125, "before any event: the clock, up to the first");
        assert_eq!(fraction(&last, &[], 3.0), 0.25, "and no further than the first event's time");
        assert_eq!(fraction(&last, &seen(&[("world-open", 0.2)]), 0.2), 0.25, "an event early: its place at once");
        assert_eq!(fraction(&last, &seen(&[("world-open", 1.0), ("init", 1.5)]), 2.5), 0.625);
        assert_eq!(fraction(&last, &seen(&[("world-open", 1.0), ("init", 1.5)]), 9.0), 0.75, "late: held at the next event");
    }

    #[test]
    fn an_event_the_last_launch_lacked_holds_the_bar_and_it_never_fills() {
        let last = profile(&[("a", 1.0), ("b", 2.0)]);
        let before = fraction(&last, &seen(&[("a", 1.0)]), 1.5);
        assert_eq!(fraction(&last, &seen(&[("a", 1.0), ("new", 1.5)]), 1.5), before);
        assert_eq!(fraction(&last, &seen(&[("a", 1.0), ("b", 2.0)]), 60.0), CAP);
        let first = |elapsed| fraction(&Profile::default(), &[], elapsed);
        assert_eq!(first(FIRST_GUESS), 0.5, "a first launch paces by a guess");
        assert!(first(20.0) < first(40.0) && first(40.0) < CAP, "and never fills");
    }

    #[test]
    fn repeated_stages_count_and_a_profile_round_trips() {
        let mut taken = Report::new(Profile::default());
        taken.step("init", "Loading your init: org", None, false);
        taken.step("init", "", None, false);
        taken.step("restore", "Restoring buffers 1/4", Some((1, 4)), false);
        taken.step("restore", "Restoring buffers 2/4", Some((2, 4)), false);
        taken.phase("user-init");
        assert_eq!(taken.seen.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["init", "init#2", "restore", "restore#2", "user-init"]);
        assert_eq!(taken.steps, [Step { stage: "restore".into(), label: "Restoring buffers 2/4".into(), count: Some((2, 4)), failed: false }]);
        assert_eq!(taken.label, "Finishing startup");
        let dir = std::env::temp_dir().join(format!("romacs-loading-{}", std::process::id()));
        let path = dir.join("launch-profile.json");
        let written = profile(&[("world-open", 0.25), ("first-frame", 1.5)]);
        written.write(&path).expect("written");
        assert_eq!(Profile::read(&path), written);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_screen_says_what_startup_does_over_its_bar() {
        let face = romacs_vm::Vm::new().display_default_face().expect("a default face");
        let steps = [Step { stage: "agents".into(), label: "Agents".into(), count: Some((3, 12)), failed: false },
                     Step { stage: "restore".into(), label: "Restoring buffers".into(), count: None, failed: true }];
        let frame = snapshot(&face, (800, 600), (20, 10), &Shown { t: 0.0, scale: 1.0, label: "Loading your init: org", filled: 0.5, steps: &steps, error: None });
        let text = &frame.windows[0].snapshot.text.utf8;
        assert!(text.contains("Loading your init: org"));
        let short = (BAR / 3).max(4);
        let agents_lit = (3.0 / 12.0 * short as f64).round() as usize;
        assert_eq!(text.matches('━').count(), BAR / 2 + agents_lit);
        assert_eq!(text.matches('─').count(), BAR - BAR / 2 + short - agents_lit);
        assert!(text.contains("Agents") && text.contains("3/12") && text.contains("✗ Restoring buffers"));
        let cell = romacs_layout::LayoutEngine::new().unwrap().frame_metrics(&face).unwrap();
        let long = format!("{}\n{}", "e".repeat(600), "f".repeat(20));
        for (width, height) in [(1280, 900), (800, 600), (1600, 1000), (640, 1400)] {
            for (steps, error) in [(&[][..], None), (&steps[..], Some(long.as_str()))] {
                let frame = snapshot(&face, (width, height), cell, &Shown { t: 0.0, scale: 1.0, label: "x", filled: 0.0, steps, error });
                let rows = frame.windows[0].snapshot.text.utf8.lines().count();
                assert!(rows as u32 * cell.0 <= height, "{width}x{height}: {rows} rows of {} px", cell.0);
            }
        }
    }

    #[test]
    fn a_failed_startup_says_where_and_why_and_its_bar_stops() {
        let mut report = Report::new(Profile::default());
        report.phase("world-restored");
        let first = report.now();
        std::thread::sleep(Duration::from_millis(20));
        assert!(report.now().filled > first.filled, "a first launch's bar moves with the clock");
        report.fail("lisp/session/romacs-loadup-bootstrap.el: No such file or directory", None);
        let failed = report.now();
        assert_eq!(failed.label, FAILED);
        assert_eq!(failed.steps, [Step { stage: FAILED_STAGE.into(), label: "Starting Lisp".into(), count: None, failed: true }]);
        assert_eq!(failed.error.as_deref(), Some("lisp/session/romacs-loadup-bootstrap.el: No such file or directory"));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(report.now(), failed, "nothing moves once startup failed");
        let face = romacs_vm::Vm::new().display_default_face().expect("a default face");
        let frame = snapshot(&face, (800, 600), (20, 10), &Shown { t: 0.0, scale: 1.0, label: &failed.label, filled: failed.filled,
                                                                   steps: &failed.steps, error: failed.error.as_deref() });
        let text = &frame.windows[0].snapshot.text.utf8;
        let (failed_at, error_at) = (text.find("✗ Starting Lisp").expect("the failed step"), text.find("lisp/session/").expect("the error"));
        assert!(text.contains(FAILED) && failed_at < error_at, "{text}");
        let red = frame.windows[0].snapshot.faces.iter().position(|f| f.foreground == rgb(FAILED_COLOUR)).expect("a red face");
        let red = FaceHandle(u32::try_from(red).expect("a face index"));
        assert!(frame.windows[0].snapshot.text.face_runs.iter().any(|run| run.face == red
            && text[run.bytes.clone()] == *"✗ Starting Lisp"), "the failed step is red");
    }

    #[test]
    fn a_failure_after_a_noted_lisp_error_shows_that_error_first() {
        let mut report = Report::new(Profile::default());
        report.fail("void-function romacs-gui-split", Some("session/romacs-startup.el: line 1 form 1: broken on purpose"));
        assert_eq!(report.now().error.as_deref(),
                   Some("session/romacs-startup.el: line 1 form 1: broken on purpose\n\nThen: void-function romacs-gui-split"));
        let mut alone = Report::new(Profile::default());
        alone.fail("x.el: line 1 form 1: gone", Some("x.el: line 1 form 1: gone"));
        assert_eq!(alone.now().error.as_deref(), Some("x.el: line 1 form 1: gone"), "a failure that is the noted error says it once");
    }

    #[test]
    fn a_short_window_shows_a_failure_s_words_before_the_logo() {
        let face = romacs_vm::Vm::new().display_default_face().expect("a default face");
        let cell = romacs_layout::LayoutEngine::new().unwrap().frame_metrics(&face).unwrap();
        let steps = [Step { stage: "agents".into(), label: "Agents".into(), count: Some((3, 12)), failed: false },
                     Step { stage: "init".into(), label: "Loading your init".into(), count: None, failed: true },
                     Step { stage: FAILED_STAGE.into(), label: "Starting Lisp".into(), count: None, failed: true }];
        let error = format!("{}\nNo such file or directory", "x".repeat(300));
        for lines in [4, 5, 8, 10] {
            let height = lines * cell.0;
            let frame = snapshot(&face, (800, height), cell, &Shown { t: 0.0, scale: 1.0, label: FAILED, filled: 0.1, steps: &steps,
                                                                     error: Some(&error) });
            let text = &frame.windows[0].snapshot.text.utf8;
            let rows = text.lines().count();
            assert!(u32::try_from(rows).expect("rows") * cell.0 <= height, "{lines} lines: {rows} rows\n{text}");
            assert!(text.trim_start().starts_with(FAILED), "no logo above the words: {text}");
            assert!(text.contains("✗ Starting Lisp") && text.contains("xxx"), "{lines} lines: {text}");
            assert_eq!(text.contains("Agents"), lines >= 6, "{lines} lines: the other steps, where they fit: {text}");
            assert_eq!(text.contains("Loading your init"), lines >= 6, "{lines} lines: an earlier failed step too: {text}");
        }
    }

    #[test]
    fn an_error_wraps_to_the_screen_and_stops_at_its_rows() {
        assert_eq!(wrapped("abcdef\n\nxyz", 4, 8), ["abcd", "ef", "", "xyz"]);
        assert_eq!(wrapped(&"x".repeat(25), 10, 2), ["x".repeat(10), format!("{}…", "x".repeat(9))]);
        assert_eq!(wrapped("ab\ncd\nef", 4, 2), ["ab", "cd…"]);
        assert_eq!(wrapped("abc", 0, 8), ["a", "b", "c"]);
    }
}
