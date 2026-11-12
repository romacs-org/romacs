//! Rows: one line wrapped in one context, a row at a time.
//!
//! `Walk::row` is the body of the frame-at-once loop for a single row: it
//! reads characters from a `Standing` (the loop's state between two
//! characters) until the row breaks or the line ends, and answers the row,
//! the state the next row starts from, and, when asked, where a byte of
//! the row is. The row question measures; a paint also collects the row's
//! glyphs as `Op`s relative to the row's own origin. Both run this one
//! loop, so a row's geometry is the same number whoever asks.
use crate::faces::{Bound, Faces};
use crate::line::LineText;
use crate::{Shown, px, relief_strips, shown};
use romacs_display_protocol::BoxVerticalEdges as Edges;
use romacs_font::FontResolver;
use romacs_font::cluster::representative_char_for_cluster;
use romacs_layout_input::cluster::cluster_end;
use romacs_layout_input::{InlineImage, PointPlace, stretch_advance, tab_advance};
use std::rc::Rc;

/// How a line wider than its window shows: GNU's `it->line_wrap`, a
/// horizontally scrolled window truncating its lines (`init_iterator`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode { Truncate, Window, Word }

/// Everything a line's rows read that is neither the line nor the view:
/// the text area's WIDTH; the wrap MODE; `LAST_X`, where a row's next glyph
/// may not pass (the width less the continuation or truncation glyph a
/// window without a right fringe shows, `GLYPHS_MARK`, that glyph SPECIAL);
/// the glyph a scrolled row shows on its left without a left fringe and
/// its advance (`LEFT_GLYPH`, DOLLAR; 0 with a fringe); the tab width; the
/// line's gutter X0 and whether the window numbers its lines; the line's
/// horizontal scroll SHIFT; and GHOSTS, the slots whose face no character
/// of the window's text shows in (a bit each), which answer no metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Context {
    pub(crate) width: f32,
    pub(crate) mode: Mode,
    pub(crate) last_x: f32,
    pub(crate) glyphs_mark: bool,
    pub(crate) special: char,
    pub(crate) left_glyph: char,
    pub(crate) dollar: f32,
    pub(crate) tab_width: u32,
    /// The gutter before the line's rows, and the width of a line
    /// number's glyph, which a label inside the line gives its rows.
    pub(crate) x0: f32,
    pub(crate) number_cell: f32,
    pub(crate) numbered: bool,
    pub(crate) shift: f32,
    pub(crate) ghosts: u64,
}

impl Context {
    /// The advance of a scrolled row's left truncation glyph.
    pub(crate) fn left_mark(&self) -> f32 { if self.shift > 0.0 { self.dollar } else { 0.0 } }
}

/// GNU's word wrap (`move_it_in_display_line_to`), one row ahead of the
/// layout: the row it scans from (ROW), the width of the rows of the line
/// before that one, the break it found (NEXT), whether the line has no
/// more (DONE), and the box state where its row starts. A row breaks when
/// its next glyph would pass the last x, at its last wrap point (the first
/// glyph after whitespace) or with none at that glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ahead {
    row: usize,
    continued: f32,
    next: Option<usize>,
    done: bool,
    box_open: bool,
}

impl Ahead {
    /// At a line's start.
    pub(crate) const START: Ahead = Ahead { row: 0, continued: 0.0, next: None, done: false, box_open: false };

    /// This state in a line whose bytes from the row's start on moved as SHIFT says.
    fn moved(self, shift: Shift) -> Self {
        Ahead { row: shift.at(self.row), next: self.next.map(|next| shift.at(next)), ..self }
    }
}

/// What a row needs of the rows before it in its line: its first byte;
/// the width of those rows (GNU's `continuation_lines_width`, which TAB
/// stops and `:align-to` read); whether the glyph before it is in a box
/// run; whether it continues a line; word wrap's look-ahead; and the
/// composed sequence in progress (where it ends, its ascent and height,
/// its glyph's x and advance), which is live at a row's start only when a
/// row broke inside one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Restart {
    pub(crate) at: usize,
    pub(crate) continued: f32,
    pub(crate) box_open: bool,
    pub(crate) continuation: bool,
    pub(crate) ahead: Ahead,
    pub(crate) inside: usize,
    pub(crate) cluster_rise: (f32, f32),
    pub(crate) composed: (f32, f32),
}

impl Restart {
    /// A line's first row, in a line whose default face rises RISE.
    pub(crate) fn start(rise: (f32, f32)) -> Self {
        Restart { at: 0, continued: 0.0, box_open: false, continuation: false, ahead: Ahead::START, inside: 0, cluster_rise: rise, composed: (0.0, 0.0) }
    }

    /// This state in a line whose bytes from here on moved as SHIFT says.
    pub(crate) fn moved(self, shift: Shift) -> Self {
        Restart { at: shift.at(self.at), ahead: self.ahead.moved(shift), inside: if self.inside > self.at { shift.at(self.inside) } else { self.inside }, ..self }
    }
}

/// How the bytes after an edit moved: the line was OLD bytes long and is
/// NEW bytes long. A byte AT shifts from the edit's end in the old line on;
/// one BACK from there in the new line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shift {
    pub(crate) old: usize,
    pub(crate) new: usize,
}

impl Shift {
    /// Byte BYTE of the old line, at or after the edit's end, in the new one.
    pub(crate) fn at(self, byte: usize) -> usize { byte + self.new - self.old }
    /// Byte BYTE of the new line, at or after the edit's end, in the old one.
    pub(crate) fn back(self, byte: usize) -> usize { byte + self.old - self.new }
}

/// The loop's state between two characters of a row: the pen X, the
/// glyphs on the row so far, whether the row is past its truncation and
/// where it was cut, the row's height so far, its tallest box top, whether
/// a glyph of another font's metrics is on it; and what `Restart` holds.
/// MIXED outlives a row while the rows are above the window's start, as
/// the frame-at-once loop kept it.
#[derive(Clone, Copy, Debug)]
#[expect(clippy::struct_excessive_bools, reason = "the frame-at-once loop's state between two characters, as it kept it")]
pub(crate) struct Standing {
    pub(crate) at: usize,
    pub(crate) x: f32,
    hpos: usize,
    pub(crate) continued: f32,
    pub(crate) hidden: bool,
    cut: f32,
    box_open: bool,
    ahead: Ahead,
    inside: usize,
    cluster_rise: (f32, f32),
    composed: (f32, f32),
    row_height: f32,
    box_top: f32,
    pub(crate) mixed: bool,
    continuation: bool,
    /// The gutter of the row in progress.
    x0: f32,
}

/// Whose face a glyph is drawn in: a slot of its line, or the face that
/// slot's glyphless glyphs show their text in, or their box's lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ink { Face(u16), Digits(u16), Line(u16) }

/// The colour a stretch paints: a slot's background or foreground, or a pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fill { Background(u16), Foreground(u16), Pixel(u32) }

/// A glyph of a row, relative to the row's origin (its left edge before
/// any horizontal scroll, its top): Y is where the frame-at-once layout
/// first put it, DY how far settling the row on one baseline then moved it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Op {
    Char { ch: char, x: f32, y: f32, dy: f32, width: f32, height: f32, ascent: f32, ink: Ink, edges: Edges },
    /// A composed sequence's glyph.
    Composed { text: Box<str>, ch: char, x: f32, dy: f32, width: f32, height: f32, ascent: f32, slot: u16, edges: Edges },
    Stretch { x: f32, y: f32, dy: f32, width: f32, height: f32, fill: Fill, ink: Ink, edges: Edges },
    /// An image in its slot; SLOT is the slot's x, y, width and height.
    Image { image: u32, x: f32, y: f32, dy: f32, width: f32, height: f32, source: [f32; 4], slot: [f32; 4], face: u16 },
    Surface { surface: u32, x: f32, y: f32, dy: f32, width: f32, height: f32, face: u16 },
}

/// One row of a line in a context.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) restart: Restart,
    /// The first byte after it: the next row's first, or the line's length.
    pub(crate) end: usize,
    /// One past the last byte wrapping it read; `usize::MAX` when it read
    /// that the line ends.
    pub(crate) horizon: usize,
    /// Its height before and after it is settled on one baseline, and
    /// that baseline below its top when it was settled.
    pub(crate) raw_height: f32,
    pub(crate) height: f32,
    pub(crate) settled: Option<f32>,
    /// It breaks onto the next row; the text area truncated it.
    pub(crate) continued: bool,
    pub(crate) truncated: bool,
    /// A glyph of another font's metrics is on it.
    pub(crate) mixed: bool,
    /// Where the pen stood when it ended, and its gutter.
    pub(crate) pen: f32,
    pub(crate) x0: f32,
    /// Its glyphs, once something painted it.
    pub(crate) ops: Option<Rc<[Op]>>,
}

impl Row {
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Row>() + self.ops.as_ref().map_or(0, |ops| ops.len() * std::mem::size_of::<Op>())
    }
}

/// Where the row being walked is in its window, when part of it may be
/// out of view: its top Y in a text area BODY tall. A glyph wholly above
/// the text area is not drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Clip {
    pub(crate) y: f32,
    pub(crate) body: f32,
}

/// Point on a row: its cursor's x on the row, width, height and ascent
/// (the row's top is its y, before the row is settled), and its place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pointed {
    pub(crate) rect: (f32, f32, f32, f32),
    pub(crate) place: PointPlace,
}

/// What a walk is asked for beside the row.
#[derive(Default)]
pub(crate) struct Ask<'a> {
    /// The row's place in its window; none when all of it is in view.
    pub(crate) clip: Option<Clip>,
    /// A byte of the line to place: point, when it is in this line.
    pub(crate) point: Option<usize>,
    /// Where to collect the row's glyphs.
    pub(crate) ops: Option<&'a mut Vec<Op>>,
    /// Stop before this byte and answer the state there.
    pub(crate) until: Option<usize>,
    /// The walk starts above the window's start, which is its first byte:
    /// nothing is drawn until the row that starts there begins.
    pub(crate) starting: bool,
}

/// How the first row of a window that starts inside a line began: the
/// frame-at-once loop's three cases, and the fourth where it never did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Started {
    /// The walk began at a row's start.
    Row,
    /// In place: the pen stood at the row's left edge.
    InPlace,
    /// The row above broke there.
    Broke,
    /// The row was broken there.
    Forced,
    /// Nothing was drawn before the line ended: the start is past a
    /// truncation, or at the line's end.
    Never,
}

/// What a walk found.
pub(crate) enum Walked {
    /// A row, the state the line's next row starts from (none when the
    /// line ended), point when it was asked for and is on the row, whether
    /// the row needs its glyphs to know its height, and how it began.
    Row { row: Row, next: Option<Restart>, point: Option<Pointed>, unsettled: bool, started: Started },
    /// The state before the byte the walk was to stop at.
    At(Standing),
}

/// A box run at one character (xdisp.c `start_of_box_run_p`,
/// `end_of_box_run_p`): see `crate::BoxStep`.
#[derive(Clone, Copy)]
struct Step { boxed: bool, lead: f32, trail: f32, side: f32, top: f32, ends: bool, open: bool }

impl Step {
    const NONE: Step = Step { boxed: false, lead: 0.0, trail: 0.0, side: 0.0, top: 0.0, ends: false, open: false };
}

/// A line being wrapped in a context.
pub(crate) struct Walk<'a> {
    pub(crate) line: &'a LineText,
    pub(crate) context: &'a Context,
    pub(crate) faces: &'a mut Faces,
    pub(crate) resolver: &'a mut FontResolver,
}

impl Walk<'_> {
    /// The state a row starts in from RESTART.
    pub(crate) fn standing(&self, restart: &Restart, mixed: bool) -> Standing {
        let x0 = self.x0_at(restart.at);
        Standing { x0, at: restart.at, x: x0, hpos: 0, continued: restart.continued, hidden: false, cut: 0.0, box_open: restart.box_open,
            ahead: restart.ahead, inside: restart.inside, cluster_rise: restart.cluster_rise, composed: restart.composed,
            row_height: self.line.faces[0].metrics.1, box_top: 0.0, mixed, continuation: restart.continuation }
    }

    /// The gutter of a row of the line that starts at BYTE: the line's,
    /// or a label's inside it at or before BYTE.
    pub(crate) fn x0_at(&self, byte: usize) -> f32 {
        let labels = &self.line.labels;
        if labels.is_empty() { return self.context.x0; }
        match labels.partition_point(|label| label.0 <= byte).checked_sub(1) {
            Some(at) => f32::from(labels[at].1) * self.context.number_cell,
            None => self.context.x0,
        }
    }

    /// The metrics of the face in SLOT: none of a face no character of
    /// the window's text shows in.
    fn metrics(&self, slot: u16) -> crate::Metrics {
        if self.context.ghosts >> slot.min(63) & 1 == 1 { (0.0, 0.0, 0.0, 0.0) } else { self.line.faces[slot as usize].metrics }
    }

    fn advance(&mut self, slot: u16, ch: char) -> f32 {
        self.line.faces[slot as usize].advance(ch, self.faces, self.resolver)
    }

    /// What the character CH at BYTE shows as, STRETCH the line's stretch
    /// there, INSIDE the end of the composed sequence the characters
    /// before it began (updated when one begins here), in the face in SLOT
    /// of nominal advance CELL.
    fn shown_at(&mut self, byte: usize, ch: char, slot: u16, cell: f32, inside: &mut usize) -> Shown {
        let line = self.line;
        if let Some(width) = line.stretch_at(byte) { return Shown::Stretch(width); }
        let raw = line.is_raw(byte);
        // No sequence starts at, or holds, an ASCII character; a simple
        // line has none.
        if ch.is_ascii() || line.simple { return shown(ch, raw); }
        if byte < *inside { return Shown::Inside; }
        if !raw && let Some(end) = cluster_end(&line.text, byte) {
            *inside = end;
            let representative = representative_char_for_cluster(&line.text[byte..end]);
            return Shown::Composed { end, advance: representative.map_or(cell, |c| self.advance(slot, c)) };
        }
        shown(ch, raw)
    }

    /// The ascent and height a character adds to its row, shown as SHOWN
    /// at BYTE: a glyph's are its font's, an escape's its face's, a
    /// composed sequence's its representative's, which a character within
    /// one takes (CLUSTER).
    fn rise(&mut self, byte: usize, ch: char, slot: u16, shown: &Shown, face: (f32, f32), cluster: &mut (f32, f32)) -> (f32, f32) {
        let real = &self.line.faces[slot as usize];
        match shown {
            // An escape draws ASCII, never the character itself.
            Shown::Glyphs(_) if ch.is_ascii() || ch.is_control() || self.line.is_raw(byte) => face,
            Shown::Glyphs(_) => real.rise(ch, self.faces, self.resolver),
            Shown::Composed { end, .. } => {
                let representative = representative_char_for_cluster(&self.line.text[byte..*end]).unwrap_or(ch);
                *cluster = if representative.is_ascii() { face } else { real.rise(representative, self.faces, self.resolver) };
                *cluster
            }
            Shown::Inside => *cluster,
            _ => face,
        }
    }

    /// The advance of SHOWN in SLOT's face (CELL wide, SPACE its space's
    /// width) at row X, line x `LINE_X`: a TAB to its next stop as from
    /// after the line number, a stretch as GNU's (up to the last x on a
    /// continued row), glyphs by their own advances.
    fn shown_advance(&mut self, shown: &Shown, slot: u16, space: f32, (x, line_x, x0): (f32, f32, f32)) -> f32 {
        let context = self.context;
        match shown {
            Shown::Tab => px::narrow(tab_advance(f64::from(line_x - x0), f64::from(space), context.tab_width)),
            Shown::Stretch(width) => {
                let last_x = (context.mode != Mode::Truncate).then_some(f64::from(context.last_x));
                px::narrow(stretch_advance(*width, f64::from(x), f64::from(line_x), last_x))
            }
            Shown::Image(image) => px::signed(image.width),
            Shown::Glyphs(glyphs) => glyphs.chars().map(|glyph| self.advance(slot, glyph)).sum(),
            Shown::Composed { advance, .. } => *advance,
            Shown::Inside => 0.0,
        }
    }

    /// The box run at CH, at BYTE in SLOT's face: PADS for text, whose
    /// run's ends are spacers outside it, not for a TAB or a stretch; OPEN
    /// when the glyph before is in a run. HORIZON takes the character
    /// after, whose face a boxed one reads.
    fn box_step(&self, slot: u16, byte: usize, ch: char, pads: bool, open: bool, horizon: &mut usize) -> Step {
        let line = self.line;
        if !line.boxed { return Step::NONE; }
        let Some(boxed) = line.faces[slot as usize].boxed() else { return Step::NONE };
        let next = byte + ch.len_utf8();
        let ends = match line.text[next..].chars().next() {
            // The line's end: its newline's, or the text's.
            None => { *horizon = usize::MAX; true }
            Some(after) => {
                *horizon = (*horizon).max(next + after.len_utf8());
                line.faces[line.slot_at(next) as usize].boxed().is_none()
            }
        };
        let side = f32::from(boxed.vertical.max(0));
        Step { boxed: true, lead: if open || !pads { 0.0 } else { side }, trail: if ends && pads { side } else { 0.0 }, side,
            top: f32::from(boxed.horizontal.max(0)), ends, open: !ends }
    }

    /// Whether a row breaks at BYTE under word wrap; BYTEs ascend.
    fn breaks_at(&mut self, ahead: &mut Ahead, byte: usize, horizon: &mut usize) -> bool {
        while !ahead.done && ahead.next.is_none_or(|next| next < byte) {
            ahead.next = self.row_break(ahead, horizon);
            ahead.done = ahead.next.is_none();
        }
        ahead.next == Some(byte)
    }

    /// The break of the row AHEAD scans, or none when the line ends on it.
    fn row_break(&mut self, ahead: &mut Ahead, horizon: &mut usize) -> Option<usize> {
        let (line, context) = (self.line, *self.context);
        let (row, x0) = (ahead.row, self.x0_at(ahead.row));
        let (mut x, mut hpos, mut may_wrap, mut wrap_at) = (x0, 0usize, false, None::<(usize, f32, bool)>);
        let mut inside = 0;
        for (byte, ch) in line.text[row..].char_indices().map(|(at, ch)| (row + at, ch)) {
            *horizon = (*horizon).max(byte + ch.len_utf8());
            let whitespace = (ch == ' ' || ch == '\t') != line.flipped(byte);
            // The box state before BYTE, where the row may restart.
            let before = ahead.box_open;
            if may_wrap && !whitespace { wrap_at = Some((byte, x, before)); }
            may_wrap = whitespace;
            let slot = line.slot_at(byte);
            let (_, _, cell, space) = self.metrics(slot);
            let shown = match line.image_at(byte) {
                Some(image) => Shown::Image(image),
                None => self.shown_at(byte, ch, slot, cell, &mut inside),
            };
            // A box run's sides count as painting counts them.
            let step = self.box_step(slot, byte, ch, crate::shows_glyphs(&shown), ahead.box_open, horizon);
            ahead.box_open = step.open;
            let advance = self.shown_advance_ahead(&shown, slot, space, (x, x + ahead.continued, x0)) + step.lead + step.trail;
            // No row breaks inside a composed sequence.
            if hpos > 0 && x + advance > context.last_x && !matches!(shown, Shown::Inside) {
                let (at, x, open) = wrap_at.unwrap_or((byte, if matches!(shown, Shown::Tab) { context.last_x } else { x }, before));
                ahead.box_open = open;
                (ahead.row, ahead.continued) = (at, ahead.continued + x);
                return Some(at);
            }
            if !matches!(shown, Shown::Stretch(_)) || advance > 0.0 { hpos += 1; }
            x += advance;
        }
        *horizon = usize::MAX;
        ahead.box_open = false;
        None
    }

    /// `shown_advance` as the look-ahead asks it: a stretch always ends
    /// short of the last x there.
    fn shown_advance_ahead(&mut self, shown: &Shown, slot: u16, space: f32, (x, line_x, x0): (f32, f32, f32)) -> f32 {
        match shown {
            Shown::Stretch(width) => px::narrow(stretch_advance(*width, f64::from(x), f64::from(line_x), Some(f64::from(self.context.last_x)))),
            _ => self.shown_advance(shown, slot, space, (x, line_x, x0)),
        }
    }

    /// The row from STANDING on: `Walked`.
    #[allow(clippy::too_many_lines, reason = "one row of the frame-at-once loop, kept in its order")]
    #[expect(clippy::float_cmp, reason = "layout coordinates are whole pixels, compared as the frame-at-once loop compared them")]
    pub(crate) fn row(&mut self, from: &Standing, restart: &Restart, mut ask: Ask) -> Walked {
        debug_assert!(ask.ops.as_ref().is_none_or(|ops| ops.is_empty()), "a row's glyphs start their own list");
        let (line, context) = (self.line, *self.context);
        let text = &*line.text;
        let default = line.faces[0].metrics;
        let (truncate, word) = (context.mode == Mode::Truncate, context.mode == Mode::Word);
        let (shift, last_x) = (context.shift, context.last_x);
        let mut x0 = from.x0;
        let left_mark = context.left_mark();
        let mut s = *from;
        let mut drawing = !ask.starting;
        let mut started = if ask.starting { Started::Never } else { Started::Row };
        // The first row drawn from a window start: where it starts and
        // what it starts from, known once drawing begins.
        let mut restart = *restart;
        let clip = ask.clip;
        // A glyph HEIGHT tall is drawn: in view vertically.
        let seen = |height: f32| clip.is_none_or(|clip| clip.body > 0.0 && clip.y < clip.body && clip.y + height > 0.0);
        let below = |height: f32| clip.is_none_or(|clip| clip.y < clip.body && clip.y + height > 0.0);
        let mut images: Vec<(usize, f32)> = vec![];
        let mut has_image = false;
        let mut horizon = from.at;
        let mut point: Option<PointPlace> = None;
        let mut rect: Option<(f32, f32, f32, f32)> = None;
        // The cursor's x on a row scrolled by SHIFT for a position at
        // unscrolled X: on the truncation glyph past the row's last
        // position, or on its first glyph when scrolled out on the left
        // (set_cursor_from_row's glyph_after and glyph_before).
        let cursor_x = |x: f32, x0: f32, hidden: bool, cut: f32| if hidden { cut - shift } else if x < x0 + shift { x0 } else { x - shift };
        let mut end = line.len();
        let mut next = None;
        let mut continued = false;
        let mut pos = from.at;
        while let Some(ch) = text[pos..].chars().next() {
            let byte = pos;
            pos += ch.len_utf8();
            if ask.until == Some(byte) { (s.at, s.x0) = (byte, x0); return Walked::At(s); }
            // The window start begins a row: at a line start, or where the
            // row before it breaks, which the character there decides
            // below, so that row's width counts toward `continued` first.
            let mut starting = !drawing;
            if starting && s.x == x0 {
                (drawing, starting, started) = (true, false, Started::InPlace);
                x0 = self.x0_at(byte);
                (s.x, s.row_height, s.hpos) = (x0, default.1, 0);
                restart = Restart { at: byte, continued: s.continued, box_open: s.box_open, continuation: s.continuation, ahead: s.ahead,
                    inside: s.inside, cluster_rise: s.cluster_rise, composed: s.composed };
            }
            let at_point = (drawing || starting) && ask.point == Some(byte);
            // Past the truncation, only point's x on its line still counts.
            let measuring = s.hidden && drawing && ask.point.is_some_and(|point| byte < point);
            if s.hidden && !measuring {
                if at_point {
                    rect = Some((s.cut - shift, default.2, s.row_height, default.0));
                    point = Some(PointPlace { x: s.cut - shift, line_x: s.x + s.continued, gutter: x0, truncated_left: shift > 0.0, truncated_right: true, at_eol: false });
                }
                // Nothing more of the row is read but point; a walk to a
                // byte stops there, in this state.
                if let Some(until) = ask.until { (s.at, s.x0) = (until, x0); return Walked::At(s); }
                match ask.point.filter(|&point| point > byte && point < text.len()) {
                    Some(point) if drawing || starting => { pos = point; continue; }
                    _ => break,
                }
            }
            horizon = horizon.max(byte + ch.len_utf8());
            let (before_box, before_inside, before_rise) = (s.box_open, s.inside, s.cluster_rise);
            let slot = line.slot_at(byte);
            let (face_ascent, face_height, cell, space) = self.metrics(slot);
            let image = line.image_at(byte);
            let shown = match image {
                Some(image) => Shown::Image(image),
                None => self.shown_at(byte, ch, slot, cell, &mut s.inside),
            };
            // An image's slot is its own height (produce_image_glyph).
            let (ascent, height) = match &shown {
                Shown::Image(i) => (px::signed(i.ascent), px::signed(i.ascent + i.descent)),
                _ => self.rise(byte, ch, slot, &shown, (face_ascent, face_height), &mut s.cluster_rise),
            };
            let own_mixed = (ascent, height) != (face_ascent, face_height) && !matches!(shown, Shown::Image(_));
            // A box run: its text's first and last glyphs own its left and
            // right edges; edges outside the text are spacers of their
            // width, the run's advance. The run's state past this
            // character holds on every path below; a row break alone opens
            // no new run (the iterator's box state carries).
            let step = self.box_step(slot, byte, ch, crate::shows_glyphs(&shown), s.box_open, &mut horizon);
            let opening = !s.box_open;
            s.box_open = step.open;
            let (lead, trail) = (step.lead, step.trail);
            let mut advance_px = self.shown_advance(&shown, slot, space, (s.x, s.x + s.continued, x0)) + lead + trail;
            if measuring { s.x += advance_px; continue; }
            // A word-wrapped row breaks where the look-ahead finds;
            // another, at the glyph that would pass the text area. No row
            // breaks inside a composed sequence. The character a row
            // starts at was asked when it ended the row before.
            let breaking = if word { (ask.starting || byte != from.at) && self.breaks_at(&mut s.ahead, byte, &mut horizon) }
                           else { !truncate && s.hpos > 0 && s.x + advance_px > last_x && !matches!(shown, Shown::Inside) };
            if breaking {
                if drawing {
                    if context.glyphs_mark && below(default.1) && let Some(ops) = ask.ops.as_deref_mut() { ops.push(self.glyph(context.special, s.x)); }
                    // The character starts the next row, from the state
                    // before it.
                    end = byte;
                    continued = true;
                    next = Some(Restart { at: byte, continued: s.continued + if matches!(shown, Shown::Tab) { last_x } else { s.x }, box_open: before_box,
                        continuation: true, ahead: s.ahead, inside: before_inside, cluster_rise: before_rise, composed: s.composed });
                    break;
                }
                // Above the window's start: the row that starts here is
                // the first drawn.
                s.continuation = true;
                // A TAB that does not fit continues from the text area's edge.
                s.continued += if matches!(shown, Shown::Tab) { last_x } else { s.x };
                x0 = self.x0_at(byte);
                (s.x, s.row_height, s.hpos) = (x0, default.1, 0);
                (drawing, starting, started) = (true, false, Started::Broke);
                restart = Restart { at: byte, continued: s.continued, box_open: before_box, continuation: true, ahead: s.ahead,
                    inside: before_inside, cluster_rise: before_rise, composed: s.composed };
                advance_px = self.shown_advance(&shown, slot, space, (x0, x0 + s.continued, x0)) + lead + trail;
            }
            if starting {
                // A start inside a row (the motion refuses one): the row
                // breaks there.
                s.continuation = true;
                s.continued += s.x;
                x0 = self.x0_at(byte);
                (drawing, s.x, s.row_height, s.hpos, started) = (true, x0, default.1, 0, Started::Forced);
                restart = Restart { at: byte, continued: s.continued, box_open: before_box, continuation: true, ahead: s.ahead,
                    inside: before_inside, cluster_rise: before_rise, composed: s.composed };
                advance_px = self.shown_advance(&shown, slot, space, (x0, x0 + s.continued, x0)) + lead + trail;
            }
            s.row_height = s.row_height.max(height);
            s.mixed |= own_mixed;
            let visible = drawing && seen(height) && s.x - shift < context.width;
            if truncate && s.x + advance_px > last_x + shift && !matches!(shown, Shown::Inside) {
                if visible && context.glyphs_mark && let Some(ops) = ask.ops.as_deref_mut() { ops.push(self.glyph(context.special, s.x)); }
                (s.hidden, s.cut) = (true, s.x);
                if at_point {
                    rect = Some((s.x - shift, default.2, s.row_height, ascent));
                    point = Some(PointPlace { x: s.x - shift, line_x: s.x + s.continued, gutter: x0, truncated_left: shift > 0.0, truncated_right: true, at_eol: false });
                }
                // The rest of the line still counts toward point's x on it.
                s.x += advance_px;
                continue;
            }
            // Scrolled out on the left, or under the left truncation
            // glyph; a glyph partly out shows its rest, but none overlaps
            // a line number.
            let left_edge = x0 + shift + left_mark;
            let visible = visible && s.x + advance_px > x0 + shift && (s.x >= left_edge || (!context.numbered && left_mark == 0.0));
            let drawn = Drawn { byte, ch, slot, x: s.x, advance: advance_px, ascent, height, cell, step, opening, visible };
            // Measuring away from point, a character leaves only what the
            // row keeps of it.
            let cursor = if ask.ops.is_some() && visible || at_point {
                self.draw(&shown, &drawn, &mut s, ask.ops.as_deref_mut(), &mut images, &mut has_image)
            } else {
                match &shown {
                    Shown::Glyphs(_) if step.boxed && visible => s.box_top = s.box_top.max(step.top),
                    Shown::Image(_) if visible => has_image = true,
                    Shown::Composed { advance, .. } => {
                        if visible { s.box_top = s.box_top.max(step.top); }
                        s.composed = (s.x, *advance + lead);
                    }
                    _ => {}
                }
                0.0
            };
            if at_point {
                // Point inside a composed sequence shows on its glyph.
                let (x, cursor) = if matches!(shown, Shown::Inside) { s.composed } else { (s.x, cursor) };
                let at = cursor_x(x, x0, false, s.cut);
                rect = Some((at, if at == x - shift { cursor } else { default.2 }, s.row_height, ascent));
                point = Some(PointPlace { x: at, line_x: x + s.continued, gutter: x0, truncated_left: shift > 0.0, truncated_right: false, at_eol: false });
            }
            if !matches!(shown, Shown::Stretch(_)) || advance_px > 0.0 { s.hpos += 1; }
            s.x += advance_px;
        }
        if ask.until.is_some() { (s.at, s.x0) = (line.len(), x0); return Walked::At(s); }
        if next.is_none() {
            horizon = if s.hidden { horizon } else { usize::MAX };
            // A window start at or past the last thing drawn of its line
            // begins a row at the newline; where the text ends instead,
            // no row begins, and the pen stays where the rows above left it.
            if !drawing {
                if line.newline.is_some() {
                    x0 = self.x0_at(line.len());
                    (s.x, s.row_height, s.hpos, started) = (x0, default.1, 0, Started::InPlace);
                }
                restart = Restart { at: from.at, continued: s.continued, box_open: s.box_open, continuation: s.continuation, ahead: s.ahead,
                    inside: s.inside, cluster_rise: s.cluster_rise, composed: s.composed };
            }
            if ask.point == Some(line.len()) {
                let at = cursor_x(s.x, x0, s.hidden, s.cut);
                rect = Some((at, default.2, s.row_height, default.0));
                // An empty last row shows no character: GNU truncates it on neither side.
                let empty = line.newline.is_none() && restart.at == line.len();
                point = Some(PointPlace { x: at, line_x: s.x + s.continued, gutter: x0, truncated_left: shift > 0.0 && !empty, truncated_right: s.hidden, at_eol: true });
            }
            // The row holding point is truncated as it ended.
            if let Some(point) = point.as_mut() {
                if line.newline.is_some() { point.truncated_right = s.hidden; } else { point.truncated_right |= s.hidden; }
            }
            // xdisp.c extend_face_to_end_of_line: the row paints past its
            // text, to the text area's edge, in its extend face.
            let shows = if line.newline.is_some() { seen(s.row_height) } else { below(s.row_height) };
            if let Some(slot) = line.extend && !s.hidden && shows {
                let (from, to) = ((s.x - shift).max(x0), context.width);
                if to > from && let Some(ops) = ask.ops.as_deref_mut() {
                    ops.push(Op::Stretch { x: from + shift, y: 0.0, dy: 0.0, width: to - from, height: s.row_height, fill: Fill::Background(slot), ink: Ink::Face(slot), edges: Edges::Both });
                }
            }
        }
        let unsettled = s.box_top > 0.0 || s.mixed || has_image;
        let raw_height = s.row_height;
        let (height, settled) = match ask.ops.as_deref_mut() {
            Some(ops) if unsettled => {
                let (bottom, ascent) = settle(ops, raw_height, &images, line);
                (bottom, Some(ascent))
            }
            _ => (raw_height, None),
        };
        let row = Row { restart, end, horizon, raw_height, height, settled, continued, truncated: s.hidden, mixed: s.mixed, pen: s.x, x0, ops: None };
        let point = point.zip(rect).map(|(place, rect)| Pointed { rect, place });
        Walked::Row { row, next, point, unsettled: unsettled && ask.ops.is_none(), started }
    }

    /// CH in the default face at X: a continuation or truncation glyph.
    fn glyph(&mut self, ch: char, x: f32) -> Op {
        let (ascent, height, ..) = self.line.faces[0].metrics;
        Op::Char { ch, x, y: 0.0, dy: 0.0, width: self.advance(0, ch), height, ascent, ink: Ink::Face(0), edges: Edges::Both }
    }

    /// Draw SHOWN as DRAWN places it, into OPS when it is visible and they
    /// are collected: the width the cursor takes on it.
    fn draw(&mut self, shown: &Shown, drawn: &Drawn, s: &mut Standing, mut ops: Option<&mut Vec<Op>>, images: &mut Vec<(usize, f32)>, has_image: &mut bool) -> f32 {
        let (line, context) = (self.line, *self.context);
        let Drawn { byte, ch, slot, x, advance: advance_px, ascent, height, cell, step, opening, visible } = *drawn;
        let default = line.faces[0].metrics;
        let face = Ink::Face(slot);
        let background = Fill::Background(slot);
        // A stretch in a box run owns the run's edges at its ends; its
        // advance has the sides in it.
        let stretch_edges = if step.boxed { edges(opening, step.ends) } else { Edges::Both };
        match shown {
            Shown::Tab => {
                if visible && let Some(ops) = ops.as_deref_mut() {
                    ops.push(Op::Stretch { x, y: 0.0, dy: 0.0, width: advance_px, height, fill: background, ink: face, edges: stretch_edges });
                }
                // GNU's cursor on a stretch is no wider than a column
                // (`x-stretch-cursor` nil).
                advance_px.min(default.2)
            }
            // A stretch that is not positive is no glyph (nglyphs 0).
            Shown::Stretch(_) => {
                if visible && advance_px > 0.0 && let Some(ops) = ops.as_deref_mut() {
                    ops.push(Op::Stretch { x, y: 0.0, dy: 0.0, width: advance_px, height, fill: background, ink: face, edges: stretch_edges });
                }
                advance_px.clamp(0.0, default.2)
            }
            // The slot in the face's background, the image's slice in it.
            Shown::Image(image) => {
                if visible {
                    *has_image = true;
                    if let Some(ops) = ops.as_deref_mut() { self.image(image, drawn, ops, images); }
                }
                advance_px
            }
            // Outside a box run, a character's glyphs are only themselves.
            Shown::Glyphs(glyphs) if !step.boxed => {
                let (mut at, mut first) = (x, None);
                for c in glyphs.chars() {
                    let (width, boxed) = self.glyph_of(slot, c);
                    if visible && at - context.shift < context.width && let Some(ops) = ops.as_deref_mut() {
                        match boxed {
                            Some(glyph) => { self.glyphless(ops, slot, &glyph, at, (ascent, height), 0.0); }
                            None => ops.push(Op::Char { ch: c, x: at, y: 0.0, dy: 0.0, width, height, ascent, ink: face, edges: Edges::Both }),
                        }
                    }
                    first.get_or_insert(width);
                    at += width;
                }
                first.unwrap_or(cell)
            }
            Shown::Glyphs(glyphs) => {
                let (lead, trail) = (step.lead, step.trail);
                // A side outside the text: the left a spacer before it,
                // the right the last glyph's own (its cell, the cursor's width).
                if visible && lead > 0.0 && let Some(ops) = ops.as_deref_mut() {
                    ops.push(Op::Stretch { x, y: 0.0, dy: 0.0, width: lead, height, fill: background, ink: face, edges: Edges::Left });
                }
                let (mut at, mut first) = (x + lead, None);
                let count = glyphs.chars().count();
                for (n, c) in glyphs.chars().enumerate() {
                    let last = n + 1 == count;
                    let (own, boxed) = self.glyph_of(slot, c);
                    let width = own + if last { trail } else { 0.0 };
                    if visible && at - context.shift < context.width && let Some(ops) = ops.as_deref_mut() {
                        // A glyphless character's cell in the run is its background.
                        let at_cell = if let Some(glyph) = boxed { self.glyphless(ops, slot, &glyph, at, (ascent, height), width - px::signed(i64::from(glyph.width))) }
                            else { ops.push(Op::Char { ch: c, x: at, y: 0.0, dy: 0.0, width, height, ascent, ink: face, edges: Edges::Both }); ops.len() - 1 };
                        let owned = edges(step.side == 0.0 && n == 0 && opening, last && step.ends);
                        if let Op::Char { edges, .. } | Op::Stretch { edges, .. } = &mut ops[at_cell] { *edges = owned; }
                    }
                    first.get_or_insert(width);
                    at += width;
                }
                if visible { s.box_top = s.box_top.max(step.top); }
                // The cursor covers the glyph with its box's sides.
                first.unwrap_or(cell) + lead
            }
            Shown::Composed { end, advance } => {
                let (lead, trail) = (step.lead, step.trail);
                // A sequence's glyph takes a box run's sides as a character's does.
                if visible && x - context.shift < context.width {
                    if let Some(ops) = ops {
                        if lead > 0.0 {
                            ops.push(Op::Stretch { x, y: 0.0, dy: 0.0, width: lead, height, fill: background, ink: face, edges: Edges::Left });
                        }
                        let owned = if step.boxed { edges(step.side == 0.0 && opening, step.ends) } else { Edges::Both };
                        ops.push(Op::Composed { text: line.text[byte..*end].into(), ch, x: x + lead, dy: 0.0, width: *advance + trail, height, ascent, slot, edges: owned });
                    }
                    s.box_top = s.box_top.max(step.top);
                }
                s.composed = (x, *advance + lead);
                *advance + lead
            }
            Shown::Inside => 0.0,
        }
    }

    /// The advance of C, a glyph of GLYPHS drawn in SLOT's face, and its
    /// box when no font has it. An escape's glyphs are ASCII.
    fn glyph_of(&mut self, slot: u16, c: char) -> (f32, Option<Rc<crate::glyphless::Glyph>>) {
        let real = &self.line.faces[slot as usize];
        if c.is_ascii() { return (real.advance(c, self.faces, self.resolver), None); }
        let cell = real.metrics.2;
        match real.bound(c, self.faces, self.resolver) {
            _ if crate::is_icon(c) => (cell, real.boxed_char(c, self.faces, self.resolver)),
            Bound::Cell => (cell, None),
            Bound::Font(glyph, _) => (glyph.advance_px, None),
            Bound::Glyphless(glyph, _, boxed) => (glyph.advance_px, Some(boxed)),
        }
    }

    /// IMAGE's slot at DRAWN: the slot in the face's background, the
    /// image, a surface, or a failed image's outline in it, and its
    /// relief; each on the image's own ascent (IMAGES).
    fn image(&mut self, image: &InlineImage, drawn: &Drawn, ops: &mut Vec<Op>, images: &mut Vec<(usize, f32)>) {
        let Drawn { slot, x, advance: advance_px, ascent, height, .. } = *drawn;
        let face = Ink::Face(slot);
        ops.push(Op::Stretch { x, y: 0.0, dy: 0.0, width: advance_px, height, fill: Fill::Background(slot), ink: face, edges: Edges::Both });
        // The body: where the image, a surface, or a failed image's outline goes.
        let (_, _, sw, sh) = image.source;
        let (bx, by, bw, bh) = (x + px::signed(image.body_x), px::signed(image.body_y), px::signed(sw), px::signed(sh));
        if image.image != 0 {
            let (sx, sy, _, _) = image.source;
            let (fw, fh) = (px::signed(image.full.0.max(1)), px::signed(image.full.1.max(1)));
            ops.push(Op::Image { image: image.image, x: bx, y: by, dy: 0.0, width: bw, height: bh,
                source: [px::signed(sx) / fw, px::signed(sy) / fh, px::signed(sw) / fw, px::signed(sh) / fh], slot: [x, 0.0, advance_px, height], face: slot });
        } else if image.surface != 0 {
            // A native surface (a model) composited in the slot.
            ops.push(Op::Surface { surface: image.surface, x: bx, y: by, dy: 0.0, width: bw, height: bh, face: slot });
        }
        // The background slot and the image in it move together, listed
        // in glyph order.
        let shown = image.image != 0 || image.surface != 0;
        let slot_at = ops.len() - 1 - usize::from(shown);
        images.extend((slot_at..ops.len()).map(|at| (at, ascent)));
        // An image that failed to load: a rectangle of its size in the
        // face's foreground (x_draw_image_foreground).
        if !shown {
            for (sx, sy, w, h) in [(bx, by, bw, 1.0), (bx, by + bh - 1.0, bw, 1.0), (bx, by, 1.0, bh), (bx + bw - 1.0, by, 1.0, bh)] {
                ops.push(Op::Stretch { x: sx, y: sy, dy: 0.0, width: w, height: h, fill: Fill::Foreground(slot), ink: face, edges: Edges::Both });
                images.push((ops.len() - 1, ascent));
            }
        }
        let base = image.relief_background.unwrap_or_else(|| self.line.faces[slot as usize].background_pixel());
        for (sx, sy, sw, sh, pixel) in relief_strips(image, x, 0.0, base) {
            ops.push(Op::Stretch { x: sx, y: sy, dy: 0.0, width: sw, height: sh, fill: Fill::Pixel(pixel), ink: face, edges: Edges::Both });
            images.push((ops.len() - 1, ascent));
        }
    }

    /// GLYPH, a character's of SLOT's face no font has, at X on a row
    /// HEIGHT tall with its baseline ASCENT below its top: the face's
    /// background, the box in its foreground, and inside it the box's
    /// text in the digit face. The background, TRAIL wider for the side
    /// of a box run it ends, is the glyph's cell in that run: its index.
    fn glyphless(&mut self, ops: &mut Vec<Op>, slot: u16, glyph: &crate::glyphless::Glyph, x: f32, (ascent, height): (f32, f32), trail: f32) -> usize {
        let width = px::signed(i64::from(glyph.width));
        let stretch = |x: f32, y: f32, width: f32, height: f32, fill: Fill, ink: Ink| Op::Stretch { x, y, dy: 0.0, width, height, fill, ink, edges: Edges::Both };
        ops.push(stretch(x, 0.0, width + trail, height, Fill::Background(slot), Ink::Face(slot)));
        let cell = ops.len() - 1;
        // The text's cells stay inside the bars, which their backgrounds
        // would otherwise cover.
        let (top, inside) = if glyph.boxed { (1.0, (height - 2.0).max(0.0)) } else { (0.0, height) };
        if glyph.boxed && width >= 1.0 && height >= 1.0 {
            for (rx, ry, rw, rh) in [(x, 0.0, width, 1.0), (x, height - 1.0, width, 1.0), (x, 0.0, 1.0, height), (x + width - 1.0, 0.0, 1.0, height)] {
                ops.push(stretch(rx, ry, rw, rh, Fill::Foreground(slot), Ink::Line(slot)));
            }
        }
        let Some(digits) = self.line.faces[slot as usize].digits(self.faces, self.resolver) else { return cell };
        for half in &glyph.halves {
            let mut at = x + px::signed(i64::from(half.x));
            for c in half.text.chars() {
                let advance = digits.advance(c, self.faces, self.resolver);
                ops.push(Op::Char { ch: c, x: at, y: top, dy: 0.0, width: advance, height: inside, ascent: ascent + px::signed(i64::from(half.y)) - top, ink: Ink::Digits(slot), edges: Edges::Both });
                at += advance;
            }
        }
        cell
    }
}

/// One character as the row loop places it for drawing.
#[derive(Clone, Copy)]
struct Drawn {
    byte: usize,
    ch: char,
    slot: u16,
    x: f32,
    advance: f32,
    ascent: f32,
    height: f32,
    cell: f32,
    step: Step,
    opening: bool,
    visible: bool,
}

/// A box run glyph's edges: the LEFT and the RIGHT, either or neither.
fn edges(left: bool, right: bool) -> Edges {
    match (left, right) { (true, true) => Edges::Both, (true, false) => Edges::Left, (false, true) => Edges::Right, (false, false) => Edges::Neither }
}

/// Settle the row of OPS, HEIGHT tall, on one baseline, as
/// `produce_glyphs`'s row ascent is the largest of its glyphs' (a boxed
/// glyph's grown by its box's top): each character and each stretch
/// shorter than the row moves down by that ascent less its own (the
/// ascent its face gives a stretch; IMAGES' for an image's slot and what
/// is in it, in glyph order): the row's new height and its ascent.
fn settle(ops: &mut [Op], height: f32, images: &[(usize, f32)], line: &LineText) -> (f32, f32) {
    let ascent_of = |at: usize, op: &Op| match images.binary_search_by_key(&at, |&(i, _)| i) {
        Ok(k) => Some(images[k].1),
        Err(_) => match op {
            Op::Char { ascent, .. } | Op::Composed { ascent, .. } => Some(*ascent),
            Op::Stretch { height: h, ink: Ink::Face(slot), .. } if *h < height => Some(line.faces[*slot as usize].metrics.0),
            _ => None,
        },
    };
    let row_ascent = ops.iter().enumerate().filter_map(|(k, op)| ascent_of(k, op)).fold(0.0f32, f32::max);
    let mut bottom = height;
    for (k, op) in ops.iter_mut().enumerate() {
        let Some(own) = ascent_of(k, op) else { continue };
        let shift = row_ascent - own;
        match op {
            Op::Char { y, dy, height: h, .. } | Op::Stretch { y, dy, height: h, .. } | Op::Surface { y, dy, height: h, .. } => {
                *dy += shift;
                bottom = bottom.max(*y + *dy + *h);
            }
            Op::Composed { dy, height: h, .. } => {
                *dy += shift;
                bottom = bottom.max(*dy + *h);
            }
            Op::Image { dy, slot, .. } => {
                *dy += shift;
                bottom = bottom.max(slot[1] + *dy + slot[3]);
            }
        }
    }
    (bottom, row_ascent)
}

/// The cursor rectangle RECT (x, width, height, ascent) of a row settled
/// on ASCENT and HEIGHT tall: how far below the row's top it sits, and
/// its height, as the row's glyph does.
pub(crate) fn settled_cursor(rect: (f32, f32, f32, f32), ascent: f32, height: f32) -> (f32, f32) {
    let shift = (ascent - rect.3).max(0.0);
    (shift, rect.2.min(height - shift))
}
