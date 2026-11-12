//! The layout by display line against the layout of a window whole.
//!
//! Under `cfg(test)` every window the engine lays out is also laid out
//! whole, and the two must give the same frame and the same rows
//! (`Store::window`), so these tests only make inputs: snapshots from a
//! seeded generator, and sequences of edits laid out by one engine, whose
//! cache then holds what the edits before left.
use crate::LayoutEngine;
use romacs_display_protocol::Color;
use romacs_layout_input::{BandStretches, BoxLine, CursorKinds, Decor, Extend, Face, FaceHandle, FaceRun, FontAlternativeState, FontSelectionState,
    FrameChrome, FrameSnapshot, Hscroll, InlineImage, LineNumbers, LineWrap, NoFont, PlacedWindowSnapshot, Stretch, StretchWidth, TextSlice,
    UnderlineRules, WindowChrome, WindowSnapshot};

/// N, a test's small count or byte, as `u32`.
fn small(n: usize) -> u32 { u32::try_from(n).expect("a test's numbers are small") }

/// SEED, a test's small seed, as `usize`.
fn small_seed(seed: u64) -> usize { usize::try_from(seed).expect("a test's seeds are small") }

/// xorshift64*: enough randomness for inputs, the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize { usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap_or(1)).unwrap_or(0) }
    fn chance(&mut self, percent: usize) -> bool { self.below(100) < percent }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T { &items[self.below(items.len())] }
}

fn face(family: &str, size: f32) -> Face {
    Face { decor: Decor::default(), foreground: [220; 3], background: [10; 3], family: family.into(), pixel_size: size, weight: 400, italic: false, width: 5 }
}

/// A face table: the default face, then faces of other sizes, families,
/// weights, boxes and colours.
fn faces(rng: &mut Rng) -> Vec<Face> {
    let mut faces = vec![face(rng.pick(&["Liberation Mono", "DejaVu Sans Mono"]), *rng.pick(&[14.0, 16.0, 20.0]))];
    for i in 0..rng.below(5) + 2 {
        let mut face = face(rng.pick(&["Liberation Mono", "Liberation Sans", "Liberation Serif", "monospace"]), *rng.pick(&[11.0, 14.0, 16.0, 20.0, 28.0]));
        let shade = u8::try_from(i * 20).unwrap_or(u8::MAX);
        face.foreground = [2 * shade, 200, 90];
        face.background = [10, shade, 30];
        face.weight = *rng.pick(&[400, 400, 700]);
        face.italic = rng.chance(20);
        if rng.chance(30) {
            face.decor = Decor { boxed: Some(BoxLine { vertical: *rng.pick(&[-1, 1, 2, 3]), horizontal: *rng.pick(&[-1, 1, 2]), color: None, style: 1 }), ..Decor::default() };
        }
        faces.push(face);
    }
    faces
}

/// A word of text: plain, or with TABs, controls, wide characters, a
/// character no font has, an emoji sequence or a placeholder space.
fn word(rng: &mut Rng) -> String {
    let plain = ["display", "row", "a", "of", "monoid", "wraps", "x", "the", "streamed", "line"];
    match rng.below(40) {
        0 => "\t".into(),
        1 => "\u{c}".into(),
        2 => "漢字かな".into(),
        3 => "\u{e000}".into(),
        4 => "\u{1F468}\u{200D}\u{1F469}".into(),
        5 => "\u{2764}\u{FE0F}".into(),
        6 => "é".into(),
        7 => "\u{3000}".into(),
        8 => "αβγ".into(),
        _ => rng.pick(&plain).to_string(),
    }
}

/// A word as `word` gives it, but none that composes: a line of these
/// can be resumed.
fn simple_word(rng: &mut Rng) -> String {
    let word = word(rng);
    if word.chars().any(|c| romacs_layout_input::cluster::is_composition_joiner(c) || c == '\u{1F468}') { "x".into() } else { word }
}

/// Text of LINES lines, each of up to WIDTH words, some long.
fn text(rng: &mut Rng, lines: usize, width: usize) -> String {
    let mut text = String::new();
    for i in 0..lines {
        if i > 0 { text.push('\n'); }
        let words = if rng.chance(15) { rng.below(width * 8) } else { rng.below(width) };
        for w in 0..words {
            if w > 0 { text.push(*rng.pick(&[' ', ' ', ' ', '\t'])); }
            text.push_str(&word(rng));
        }
    }
    if rng.chance(30) { text.push('\n'); }
    text
}

/// Runs of FACES over TEXT, ordered and disjoint, on character bounds.
fn runs(rng: &mut Rng, text: &str, faces: usize) -> Vec<FaceRun> {
    let bounds: Vec<usize> = text.char_indices().map(|(at, _)| at).chain(std::iter::once(text.len())).collect();
    let mut runs = vec![];
    let mut at = 0;
    while at + 1 < bounds.len() {
        let start = at + rng.below(12);
        let end = (start + 1 + rng.below(30)).min(bounds.len() - 1);
        if start >= end { break; }
        runs.push(FaceRun { bytes: bounds[start]..bounds[end], face: FaceHandle(1 + small(rng.below(faces - 1))) });
        at = end;
    }
    runs
}

/// A window snapshot of generated text and the rest of its inputs.
fn snapshot(rng: &mut Rng, id: u64) -> WindowSnapshot {
    let faces = faces(rng);
    let lines = rng.below(30) + 1;
    let utf8 = text(rng, lines, 14);
    let face_runs = runs(rng, &utf8, faces.len());
    let chars: Vec<(usize, char)> = utf8.char_indices().collect();
    let pick_char = |rng: &mut Rng| chars.get(rng.below(chars.len())).map_or(0, |c| c.0);
    // Placeholder spaces become stretches or images.
    let spaces: Vec<usize> = chars.iter().filter(|c| c.1 == ' ').map(|c| c.0).collect();
    let (mut stretches, mut images) = (vec![], vec![]);
    for &byte in &spaces {
        match rng.below(12) {
            0 => stretches.push(Stretch { byte, width: *rng.pick(&[StretchWidth::Width { width: 30, zero_ok: false }, StretchWidth::Width { width: 0, zero_ok: true },
                StretchWidth::AlignTo(200), StretchWidth::AlignTo(10), StretchWidth::Width { width: 900, zero_ok: true }]) }),
            1 => images.push(InlineImage { byte, image: 1 + small(rng.below(2)), surface: 0, width: 40, ascent: *rng.pick(&[10, 30, 50]), descent: 4, body_x: 1, body_y: 1,
                source: (0, 0, 38, 30), full: (38, 30), relief: *rng.pick(&[0, 0, 2, -1]), relief_background: None }),
            _ => {}
        }
    }
    let mut wrap_flips: Vec<u32> = (0..rng.below(6)).map(|_| small(pick_char(rng))).collect();
    wrap_flips.sort_unstable();
    wrap_flips.dedup();
    let raw: Vec<u32> = chars.iter().filter(|c| c.1 == 'é').filter(|_| rng.chance(50)).map(|c| small(c.0)).collect();
    let newlines: Vec<usize> = chars.iter().filter(|c| c.1 == '\n').map(|c| c.0).collect();
    let mut extends: Vec<Extend> = newlines.iter().filter(|_| rng.chance(25)).map(|&byte| Extend { byte: small(byte), face: FaceHandle(1) }).collect();
    if rng.chance(20) { extends.push(Extend { byte: small(utf8.len()), face: FaceHandle(2) }); }
    let starts: Vec<usize> = std::iter::once(0).chain(newlines.iter().map(|&n| n + 1)).collect();
    let line_numbers = rng.chance(25).then(|| LineNumbers { columns: 4, face: FaceHandle(0), current: FaceHandle(1),
        labels: {
            // Some labels inside a line: a buffer line a hidden newline
            // joined to the one before, its label wider.
            let mut labels: Vec<(usize, String, bool)> = starts.iter().enumerate().map(|(i, &byte)| (byte, format!("{:>3} ", i + 1), i == 2)).collect();
            for _ in 0..rng.below(4) { let byte = pick_char(rng); if byte > 0 { labels.push((byte, format!("{:>4} ", 1000 + byte), false)); } }
            labels.sort_by_key(|label| label.0);
            labels.dedup_by_key(|label| label.0);
            labels
        } });
    let start_byte = match rng.below(4) { 0 => pick_char(rng), 1 => *rng.pick(&starts), _ => 0 };
    let point_byte = if rng.chance(10) { utf8.len() } else { pick_char(rng) };
    let wrap = *rng.pick(&[LineWrap::Window, LineWrap::Window, LineWrap::Word, LineWrap::Truncate]);
    let hscroll = if rng.chance(15) { Hscroll { rows: 24, point_row: *rng.pick(&[24, 48]) } } else { Hscroll::default() };
    let chrome = if rng.chance(40) { WindowChrome { left_fringe: 8, right_fringe: 8, ..WindowChrome::default() } } else { WindowChrome::default() };
    let band = |rng: &mut Rng| rng.chance(30).then(|| TextSlice { utf8: "-:--  band  (Mode)".into(), face_runs: vec![], raw: vec![], extends: vec![] });
    let (mode_line, header_line) = (band(rng), band(rng));
    WindowSnapshot { id, width: *rng.pick(&[120, 333, 600, 1000]), height: *rng.pick(&[60, 300, 700]), start_byte, point_byte, buffer_version: 1,
        text: TextSlice { utf8, face_runs, raw, extends }, mode_line, mode_line_face: FaceHandle(0), header_line, header_line_face: FaceHandle(1),
        vscroll_px: if rng.chance(10) { *rng.pick(&[5, 30]) } else { 0 }, hscroll, faces, tab_width: *rng.pick(&[8, 4]), wrap, cursor_off: false,
        cursor: CursorKinds::default(), stretches, images, wrap_flips, line_numbers, chrome, underline_rules: UnderlineRules::default(), band_stretches: BandStretches::default() }
}

fn frame(windows: Vec<WindowSnapshot>) -> FrameSnapshot {
    let width = windows.iter().map(|w| w.width + w.chrome.left_fringe + w.chrome.right_fringe).max().unwrap_or(1);
    let mut y = 0;
    let windows = windows.into_iter().map(|snapshot| {
        let placed = PlacedWindowSnapshot { x: snapshot.chrome.left_fringe, y, snapshot };
        y += placed.snapshot.height;
        placed
    }).collect::<Vec<_>>();
    FrameSnapshot { font_selection: FontSelectionState::default(), font_alternatives: FontAlternativeState::default(), fontsets: std::sync::Arc::default(), width, height: y.max(1),
        windows, background_alpha: 1.0, children: vec![], tab_bar: None, chrome: FrameChrome::default(), status_bar: None, no_font: NoFont::default() }
}

/// Lay FRAME out with ENGINE, asked and painted: the engine checks each
/// window against its layout whole.
fn both(engine: &mut LayoutEngine, frame: &FrameSnapshot, presentation: u64) {
    let selected = frame.windows[0].snapshot.id;
    engine.lay_out(frame).unwrap();
    engine.layout_frame(frame, presentation, Some((selected, Color::from_pixel(0x00ff_ffff)))).unwrap();
}

#[test]
fn generated_windows_lay_out_as_whole() {
    let mut engine = LayoutEngine::new().unwrap();
    for seed in 1..400u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let count = rng.below(2) + 1;
        let windows = (1..=count as u64).map(|id| snapshot(&mut rng, id)).collect();
        both(&mut engine, &frame(windows), seed);
    }
}

/// Every start and point in a few windows: the window starts on each
/// character, inside rows and lines, and point is on each.
#[test]
fn every_start_and_point_lays_out_as_whole() {
    let mut engine = LayoutEngine::new().unwrap();
    for seed in 1..12u64 {
        let mut rng = Rng(seed.wrapping_mul(0x2545_f491) | 1);
        let base = snapshot(&mut rng, 1);
        let bounds: Vec<usize> = base.text.utf8.char_indices().map(|(at, _)| at).chain(std::iter::once(base.text.utf8.len())).collect();
        for (n, &start) in bounds.iter().enumerate().step_by(3) {
            let mut window = base.clone();
            window.start_byte = start;
            window.point_byte = bounds[(n * 7 + small_seed(seed)) % bounds.len()];
            both(&mut engine, &frame(vec![window]), seed * 1000 + u64::from(small(n)));
        }
    }
}

/// A long line edited in place, step by step, by one engine: each step
/// resumes the line's rows from its last version.
#[test]
fn edits_to_a_long_line_lay_out_as_whole() {
    for (seed, wrap, simple) in [(3u64, LineWrap::Word, true), (5, LineWrap::Window, true), (7, LineWrap::Word, true), (11, LineWrap::Truncate, true),
                                 (13, LineWrap::Window, true), (19, LineWrap::Word, true), (23, LineWrap::Word, false)] {
        let word = |rng: &mut Rng| if simple { simple_word(rng) } else { word(rng) };
        let mut rng = Rng(seed);
        let mut engine = LayoutEngine::new().unwrap();
        let mut window = snapshot(&mut rng, 1);
        (window.wrap, window.hscroll, window.height, window.start_byte, window.vscroll_px) = (wrap, Hscroll::default(), 700, 0, 0);
        let long = (0..300).map(|i| word(&mut Rng(i + seed))).collect::<Vec<_>>().join(if seed % 3 == 1 { " \t" } else { " " });
        window.text = TextSlice { utf8: format!("head\n{long}\ntail"), face_runs: vec![], raw: vec![], extends: vec![] };
        window.text.face_runs = runs(&mut rng, &window.text.utf8, window.faces.len());
        (window.stretches, window.images, window.wrap_flips, window.line_numbers) = (vec![], vec![], vec![], None);
        for step in 0..60 {
            let text = &mut window.text.utf8;
            let line_end = text.rfind('\n').unwrap();
            let bounds: Vec<usize> = text[..line_end].char_indices().map(|(at, _)| at).filter(|&at| at > 5).collect();
            let at = match step % 4 { 0 | 1 => line_end, _ => *rng.pick(&bounds) };
            if step % 5 == 4 {
                let Some(ch) = text[at..].chars().next().filter(|&c| c != '\n') else { continue };
                let gone = ch.len_utf8();
                text.replace_range(at..at + gone, "");
                for run in &mut window.text.face_runs {
                    let shift = |b: usize| if b > at { b.saturating_sub(gone).max(at) } else { b };
                    run.bytes = shift(run.bytes.start)..shift(run.bytes.end);
                }
                window.text.face_runs.retain(|run| run.bytes.start < run.bytes.end);
            } else {
                let insert = word(&mut rng) + " ";
                text.insert_str(at, &insert);
                for run in &mut window.text.face_runs {
                    if run.bytes.start >= at { run.bytes.start += insert.len(); }
                    if run.bytes.end > at { run.bytes.end += insert.len(); }
                }
            }
            window.point_byte = at.min(window.text.utf8.len());
            while !window.text.utf8.is_char_boundary(window.point_byte) { window.point_byte -= 1; }
            window.start_byte = if step % 3 == 0 { 5 } else { 0 };
            both(&mut engine, &frame(vec![window.clone()]), step + 1);
        }
        let counts = engine.counts().0;
        eprintln!("{wrap:?}: {counts:?}");
        // A truncated line is one row, read to its cut: laid out whole.
        assert_eq!(counts.lines_resumed > 20, simple && wrap != LineWrap::Truncate, "{counts:?}");
    }
}

/// An append to a long line wraps its last rows only.
#[test]
fn an_append_wraps_its_last_rows() {
    let mut engine = LayoutEngine::new().unwrap();
    let mut rng = Rng(17);
    let mut window = snapshot(&mut rng, 1);
    let long: String = (0..2000).map(|i| ["reply", "streams", "a", "row"][i % 4]).collect::<Vec<_>>().join(" ");
    (window.wrap, window.hscroll, window.height, window.width, window.vscroll_px) = (LineWrap::Word, Hscroll::default(), 700, 600, 0);
    (window.stretches, window.images, window.wrap_flips, window.line_numbers, window.mode_line, window.header_line) = (vec![], vec![], vec![], None, None, None);
    window.faces.truncate(1);
    window.text = TextSlice { utf8: format!("{long}\n"), face_runs: vec![], raw: vec![], extends: vec![] };
    window.start_byte = 0;
    window.point_byte = window.text.utf8.len() - 1;
    both(&mut engine, &frame(vec![window.clone()]), 1);
    for step in 0..10 {
        let before = engine.counts().0;
        let end = window.text.utf8.len() - 1;
        window.text.utf8.insert_str(end, " and twenty more ch");
        window.point_byte = window.text.utf8.len() - 1;
        both(&mut engine, &frame(vec![window.clone()]), step + 2);
        let after = engine.counts().0;
        assert!(after.rows_reused > before.rows_reused, "step {step}: the rows before the append are kept: {before:?} -> {after:?}");
    }
}

/// Point moving through generated windows, nothing else changing: each
/// frame after the first places cursors in the last frame's glyphs, and
/// must equal a whole layout.
#[test]
fn point_moves_lay_out_as_whole() {
    let mut engine = LayoutEngine::new().unwrap();
    for seed in 1..40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x5851_f42d_4c95_7f2d) | 1);
        let mut window = snapshot(&mut rng, 1);
        let bounds: Vec<usize> = window.text.utf8.char_indices().map(|(at, _)| at).chain(std::iter::once(window.text.utf8.len())).collect();
        let bar = seed % 2 == 0;
        for step in 0..20 {
            window.point_byte = bounds[rng.below(bounds.len())];
            let mut frame = frame(vec![window.clone()]);
            // A status bar that shows where point is, as Romacs's does.
            if bar {
                let text = TextSlice { utf8: format!("{}:0   *scratch*", window.point_byte), face_runs: vec![], raw: vec![], extends: vec![] };
                frame.height += 30;
                frame.status_bar = Some(romacs_layout_input::StatusBarSnapshot { x: 0, y: frame.height - 30, width: frame.width, height: 30, text, face: FaceHandle(0),
                    faces: vec![face("Liberation Sans", 16.0)], window: 1, focused: true, marks: romacs_layout_input::BandMarks::default() });
            }
            both(&mut engine, &frame, seed * 100 + step);
        }
    }
}
