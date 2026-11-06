//! GNU xdisp.c's display iterator motion (move_it_in_display_line_to,
//! move_it_to, move_it_by_lines with 0) and window_text_pixel_size, on a
//! terminal frame (term.c produce_glyphs): x counts columns, every glyph
//! is one column, and every element produced makes its row one line
//! tall. A host's layout supplies `Shape::Pixel` glyphs instead, measured
//! in its pixels (VM2-WTPS1), where a row is as tall as its tallest. window_text_pixel_size iterates without bidi reordering; VM2
//! has no line numbers, fringes or applied hscroll, so those branches are
//! absent. The elements come from `Elements`, so nothing here reads the
//! VM: the motion is a function of the element sequence and the window
//! geometry.
use super::Fault;

pub(super) type R<T> = std::result::Result<T, Fault>;

pub(super) use romacs_layout_input::LineWrap as Wrap;

/// How the iterator delivers the element (`it->method`, and whether
/// `it->object` is the buffer).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Method { Buffer, Vector { last: bool, buffer: bool }, String, Stretch }

/// How a glyphless character shows (`glyphless-char-display'): a thin
/// space, an empty box, or a box holding its code or an ACRONYM string.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Glyphless { ThinSpace, EmptyBox, HexCode, Acronym(crate::Value) }

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Shape {
    /// A character glyph WIDTH columns wide; a newline is zero wide.
    Glyph { c: u32, width: i64 },
    /// A character the host lays out as one glyph WIDTH pixels wide in a
    /// row at least HEIGHT pixels tall, ASCENT of them above the baseline;
    /// a newline is zero wide.
    Pixel { c: u32, width: i64, height: i64, ascent: i64 },
    Tab,
    /// A TAB the host lays out as a stretch to the next tab stop in its
    /// pixels, SPACE being its face's space width, in a row at least
    /// HEIGHT pixels tall (romacs_layout_input::tab_advance).
    PixelTab { space: i64, height: i64, ascent: i64 },
    Stretch { width: i64, zero_ok: bool },
    /// `:align-to', TARGET columns from the text area's left edge.
    Align { target: i64 },
    /// A glyphless character shown as WIDTH columns (term.c
    /// produce_glyphless_glyph).
    Glyphless { width: i64, c: u32, how: Glyphless },
    /// `:relative-width' FACTOR of a TAB, whose width depends on x.
    TabStretch { factor: f64 },
    /// `:relative-width' FACTOR of the glyph C would make, in pixels once
    /// a host measures it (a host's elements only).
    RelativeStretch { factor: f64, c: u32 },
    /// A stretch the host lays out, WIDTH pixels or to TARGET pixels from
    /// the text area's left edge, in a row at least HEIGHT pixels tall
    /// (romacs_layout_input::stretch_advance).
    /// UNNUMBERED, when given, is its width while the row's line number is
    /// not yet produced (calc_pixel_width_or_height counts it only then).
    PixelStretch { width: romacs_layout_input::StretchWidth, unnumbered: Option<romacs_layout_input::StretchWidth>, height: i64, ascent: i64 },
    /// An image's glyph (produce_image_glyph): its slot's width, ascent
    /// and descent; what the host paints in it and its spec are the
    /// source's image AT (Source::image), so elements stay small.
    Image { width: i64, ascent: i64, descent: i64, at: u32 },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Elem {
    pub shape: Shape,
    pub method: Method,
    /// IT_CHARPOS while the element is loaded.
    pub charpos: i64,
    /// IT_DISPLAYING_WHITESPACE.
    pub whitespace: bool,
    pub text_area: bool,
    /// IT_STRING_CHARPOS, inside a string.
    pub strpos: Option<usize>,
    pub from_display: bool,
    /// The string delivering the element, else nil.
    pub object: crate::Value,
    /// A glyph of the ellipsis that stands for invisible text, not of the
    /// character at CHARPOS (or STRPOS), with its glyph code's face, else nil.
    pub ellipsis: Option<crate::Value>,
    /// A display vector glyph's own face, merged over the text's: the
    /// `escape-glyph' of an escape or control sequence, `nobreak-space' or
    /// `nobreak-hyphen', or a display table glyph code's face; else nil.
    pub face: Option<crate::Value>,
    /// A stretch's shape with no line number produced on its row, where
    /// that differs.
    pub unnumbered: Option<Shape>,
}

impl Elem {
    fn end_of_line(&self) -> bool { matches!(self.shape, Shape::Glyph { c: 10, .. } | Shape::Pixel { c: 10, .. }) }
    /// BUFFER_POS_REACHED_P's object and method test.
    fn reaches(&self) -> bool {
        matches!(self.method, Method::Buffer | Method::Vector { last: true, buffer: true })
    }
}

/// One delivery: the element (None at the end), and the buffer position
/// the iterator reads from before loading it, when that is the buffer
/// (`GET_FROM_BUFFER` at a character or at a stop position).
#[derive(Clone, Copy, Debug)]
pub(super) struct Slot { pub pre: Option<i64>, pub elem: Option<Elem> }

pub(super) trait Elements {
    fn slot(&mut self, index: usize) -> R<Slot>;
    /// handle_line_prefix before element INDEX: a pushed prefix's id,
    /// and whether it aligns visually (a wrap-prefix space spec).
    fn prefix(&mut self, index: usize, loaded: bool, wrap: bool) -> R<Option<(usize, bool)>>;
    fn prefix_elem(&self, id: usize, k: usize) -> Option<Elem>;
    /// The index of the buffer line element INDEX shows, among the
    /// accessible text's (display_count_lines_logically).
    fn line_at(&mut self, index: usize) -> R<i64>;
}

/// The iterator state SAVE_IT copies.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct It {
    pub index: usize,
    pub prefix: Option<(usize, usize)>,
    pub align_visually: bool,
    pub loaded: bool,
    pub current_x: i64,
    pub current_y: i64,
    pub vpos: i64,
    pub hpos: i64,
    /// max_ascent and max_descent of the row so far.
    pub max_h: Rise,
    /// ascent and descent of the last element produced.
    pub h: Rise,
    pub continuation_lines_width: i64,
    pub wrap_prefix_width: i64,
    pub pixel_width: i64,
    pub nglyphs: i64,
    /// The x this row's line number took, once produced
    /// (line_number_produced_p, lnum_pixel_width), else 0.
    pub lnum_x: i64,
    /// The x the last row's line number took (lnum_pixel_width), kept
    /// across rows, 0 before any.
    pub lnum_last: i64,
    /// The glyphs this row's line number took (GNU's lnum_width + 2), else
    /// 0: the hpos of the row's first text glyph.
    pub lnum_hpos: i64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Ctx {
    /// The x the hscroll hides each line's text before (first_visible_x):
    /// a glyph counts in a row's hpos only when it ends past it.
    pub first_visible_x: i64,
    pub last_visible_x: i64,
    pub wrap: Wrap,
    pub tab_width: i64,
    pub zv: i64,
    pub zv_after_newline: bool,
    /// `display-line-numbers': how each row's number is labelled.
    pub lnum: Option<super::text_pixel_size::LineLabels>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Move { Undefined, PosMatchOrZv, NewlineOrCr, LineTruncated, LineContinued, XReached }

#[derive(Clone, Copy, Default)]
pub(super) struct Op { pub pos: bool, pub x: bool, pub y: bool, pub vpos: bool }
const POS: Op = Op { pos: true, x: false, y: false, vpos: false };


fn wrap(cx: &Ctx, it: &It) -> Wrap {
    // handle_line_prefix truncates while the prefix is shown.
    if it.prefix.is_some() { Wrap::Truncate } else { cx.wrap }
}

fn element<E: Elements>(src: &mut E, it: &It) -> R<Option<Elem>> {
    match it.prefix {
        Some((id, k)) => Ok(src.prefix_elem(id, k)),
        None => Ok(src.slot(it.index)?.elem),
    }
}

/// get_next_display_element.
fn load<E: Elements>(src: &mut E, it: &mut It) -> R<Option<Elem>> {
    it.loaded = true;
    element(src, it)
}

/// set_iterator_to_next; a finished prefix pops back to the text.
fn advance<E: Elements>(src: &mut E, it: &mut It) {
    it.loaded = false;
    match it.prefix {
        Some((id, k)) if src.prefix_elem(id, k + 1).is_some() => it.prefix = Some((id, k + 1)),
        Some(_) => { it.prefix = None; it.align_visually = false; }
        None => it.index += 1,
    }
}

/// The iterator's method is GET_FROM_BUFFER on the buffer, and IT_CHARPOS.
pub(super) fn position<E: Elements>(src: &mut E, it: &It) -> R<(bool, i64)> {
    if it.prefix.is_some() {
        return Ok((false, element(src, it)?.map_or(0, |e| e.charpos)));
    }
    let slot = src.slot(it.index)?;
    Ok(match (it.loaded, slot.elem, slot.pre) {
        (true, Some(e), _) => (e.method == Method::Buffer, e.charpos),
        (_, _, Some(pre)) => (true, pre),
        (_, Some(e), None) => (false, e.charpos),
        (_, None, None) => (true, 0),
    })
}

pub(super) fn charpos<E: Elements>(src: &mut E, it: &It) -> R<i64> { Ok(position(src, it)?.1) }

/// IT_POS_VALID_AFTER_MOVE_P, and string_from_display_prop_p.
fn valid_after_move<E: Elements>(src: &mut E, it: &It) -> R<(bool, bool)> {
    let slot_elem = element(src, it)?;
    let unloaded_buffer = !it.loaded && it.prefix.is_none() && src.slot(it.index)?.pre.is_some();
    Ok(match slot_elem {
        Some(e) if !unloaded_buffer && e.method == Method::String => (e.strpos == Some(0), e.from_display),
        _ => (true, false),
    })
}

/// An element's or a row's extent about the baseline: GNU's ascent and
/// descent, a row's the largest of its elements' (max_ascent, max_descent),
/// and its tallest element's height.  The host aligns a row's elements on
/// one baseline only once it holds an image (SETTLED); other rows it lays
/// out each element from the row's top, its tallest element's height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Rise { pub ascent: i64, pub descent: i64, pub tallest: i64, pub settled: bool }

impl Rise {
    pub(super) fn height(self) -> i64 {
        if self.settled { (self.ascent + self.descent).max(self.tallest) } else { self.tallest }
    }
    pub(super) fn max(self, other: Rise) -> Rise {
        Rise { ascent: self.ascent.max(other.ascent), descent: self.descent.max(other.descent),
               tallest: self.tallest.max(other.tallest), settled: self.settled || other.settled }
    }
    fn of(height: i64, ascent: i64) -> Rise { Rise { ascent, descent: height - ascent, tallest: height, settled: false } }
    /// How far below the row's top an element of OWN ascent sits.
    pub(super) fn drop(self, own: Rise) -> i64 { if self.settled || own.settled { (self.ascent - own.ascent).max(0) } else { 0 } }
}

/// produce_glyphs (term.c) and produce_stretch_glyph.
fn produce(cx: &Ctx, it: &mut It, e: &Elem) {
    let from_prefix = it.prefix.is_some();
    // A terminal's line is one row tall.
    let mut rise = Rise::of(1, 1);
    let (width, glyphs) = match e.shape {
        Shape::Glyph { c: 10, .. } => (0, 0),
        Shape::Pixel { c, width, height, ascent } => { rise = Rise::of(height, ascent); if c == 10 { (0, 0) } else { (width, 1) } }
        Shape::Glyph { width, .. } | Shape::Glyphless { width, .. } => (width, width),
        Shape::Tab => { let spaces = tab_spaces(cx, it); (spaces, spaces) }
        Shape::PixelTab { space, height, ascent } => { rise = Rise::of(height, ascent); (pixel_tab(cx, it, space), 1) }
        Shape::TabStretch { factor } => stretch(cx, it, e, (factor * tab_spaces(cx, it) as f64) as i64, false),
        Shape::RelativeStretch { .. } => (0, 0),
        Shape::Stretch { width, zero_ok } => stretch(cx, it, e, width, zero_ok),
        Shape::Align { target } => {
            let x = it.current_x + if it.align_visually { 0 } else { it.continuation_lines_width };
            stretch(cx, it, e, (target - x).max(0), true)
        }
        Shape::Image { width, ascent, descent, .. } => {
            rise = Rise { ascent, descent, tallest: ascent + descent, settled: true };
            (width, 1)
        }
        Shape::PixelStretch { width, unnumbered, height, ascent } => {
            rise = Rise::of(height, ascent);
            let width = pixel_stretch(cx, it, e, if it.lnum_x > 0 { width } else { unnumbered.unwrap_or(width) });
            (width, i64::from(width > 0))
        }
    };
    it.pixel_width = width;
    it.nglyphs = glyphs;
    if e.text_area {
        it.current_x += width;
        if it.continuation_lines_width != 0 && from_prefix { it.wrap_prefix_width = it.current_x; }
    }
    it.h = rise;
    it.max_h = it.max_h.max(rise);
}

/// The spaces a TAB at the iterator's x takes to the next tab stop.
fn tab_spaces(cx: &Ctx, it: &It) -> i64 {
    let x = it.current_x + continued_width(it) - it.lnum_x;
    (x + cx.tab_width) / cx.tab_width * cx.tab_width - x
}

/// The pixels a host's TAB at the iterator's x stretches, SPACE being its
/// face's space width.
pub(super) fn pixel_tab(cx: &Ctx, it: &It, space: i64) -> i64 {
    let x = it.current_x + continued_width(it) - it.lnum_x;
    romacs_layout_input::tab_advance(x as f64, space as f64, cx.tab_width as u32) as i64
}

/// The element the iterator has loaded, if any.
pub(super) fn loaded<E: Elements>(src: &mut E, it: &It) -> R<Option<Elem>> {
    if it.loaded { element(src, it) } else { Ok(None) }
}

/// pos_visible_p's walk out of the display vector showing the character
/// at IT: the iterator after the last element it produced before an
/// element outside a vector, or at or after CHARPOS, and whether that
/// element ends its line.
pub(super) fn past_vector<E: Elements>(src: &mut E, cx: &Ctx, it: &mut It, charpos: i64) -> R<(It, bool)> {
    loop {
        let Some(e) = load(src, it)? else { return Ok((*it, false)) };
        produce(cx, it, &e);
        let produced = *it;
        advance(src, it);
        let next = element(src, it)?;
        if !next.is_some_and(|n| matches!(n.method, Method::Vector { .. }) && n.charpos < charpos) { return Ok((produced, e.end_of_line())); }
    }
}

/// pos_visible_p's walk along a display line from IT for a position a
/// replacing display covers: the x where the first element at or after
/// CHARPOS starts, or the line's last, and whether an element of STRING
/// was the last one produced, with the x before the first of them.
pub(super) fn x_at_or_after<E: Elements>(src: &mut E, cx: &Ctx, it: &mut It, charpos: i64, string: crate::Value) -> R<(i64, bool, i64)> {
    let mut before = it.current_x;
    let mut moved = false;
    while let Some(e) = load(src, it)? {
        if e.object != string { before = it.current_x; }
        produce(cx, it, &e);
        if e.charpos >= charpos || e.end_of_line() { return Ok((it.current_x - it.pixel_width, moved && e.object == string && e.charpos != charpos, before)); }
        moved = true;
        advance(src, it);
    }
    Ok((it.current_x - it.pixel_width, false, before))
}

/// The pixels a host's stretch of WIDTH in E takes at the iterator's x.
pub(super) fn pixel_stretch(cx: &Ctx, it: &It, e: &Elem, width: romacs_layout_input::StretchWidth) -> i64 {
    let line_x = it.current_x + if it.align_visually { 0 } else { it.continuation_lines_width };
    let last = (e.text_area && wrap(cx, it) != Wrap::Truncate).then_some(cx.last_visible_x as f64);
    romacs_layout_input::stretch_advance(width, it.current_x as f64, line_x as f64, last) as i64
}

/// The width of the screen lines before this one that a TAB's stop counts
/// from, less a wrap prefix's.
fn continued_width(it: &It) -> i64 {
    if it.prefix.is_some() || it.continuation_lines_width == 0 { 0 }
    else { it.continuation_lines_width - it.wrap_prefix_width }
}

fn stretch(cx: &Ctx, it: &It, e: &Elem, width: i64, zero_ok: bool) -> (i64, i64) {
    let mut width = if width <= 0 && (width < 0 || !zero_ok) { 1 } else { width };
    if width > 0 && e.text_area && wrap(cx, it) != Wrap::Truncate && it.current_x + width > cx.last_visible_x {
        width = cx.last_visible_x - it.current_x;
    }
    (width, width)
}

fn handle_line_prefix<E: Elements>(src: &mut E, it: &mut It) -> R<()> {
    if let Some((id, align)) = src.prefix(it.index, it.loaded, it.continuation_lines_width > 0)? {
        it.prefix = Some((id, 0));
        it.align_visually = align;
    }
    Ok(())
}

/// move_it_in_display_line_to. TO is ignored unless OP.pos, TO_X unless OP.x.
pub(super) fn move_in_line<E: Elements>(src: &mut E, cx: &Ctx, it: &mut It, to: i64, to_x: i64, op: Op) -> R<Move> {
    let mut result = Move::Undefined;
    let (mut wrap_it, mut atpos_it, mut atx_it): (Option<It>, Option<It>, Option<It>) = (None, None, None);
    let mut may_wrap = false;
    let mut subject = 0u8;
    let reached = |e: &Elem| op.pos && e.reaches() && e.charpos >= to;
    // A line number for a row whose text starts left of the window's first
    // visible x (hscrolled) waits for the first glyph that shows.
    let mut lnum_pending = false;
    if it.hpos == 0 {
        // maybe_produce_line_number, unless moving to the first visible x
        // (GNU tests `op && MOVE_TO_X').
        let moving = op.pos || op.x || op.y || op.vpos;
        if let Some(lnum) = cx.lnum && !(moving && to_x == cx.first_visible_x) {
            if it.current_x == cx.first_visible_x {
                let (glyphs, x) = lnum.prefix(src.line_at(it.index)?);
                (it.current_x, it.hpos, it.lnum_x, it.lnum_last, it.lnum_hpos) = (it.current_x + x, glyphs, x, x, glyphs);
            } else {
                lnum_pending = it.current_x < cx.first_visible_x;
            }
        }
        if it.prefix.is_none() { handle_line_prefix(src, it)?; }
        subject = u8::from(it.prefix.is_some());
    }
    'line: loop {
        if op.pos {
            let (buffer, at) = position(src, it)?;
            if buffer && at > to {
                if wrap(cx, it) != Wrap::Word || wrap_it.is_none() { result = Move::PosMatchOrZv; break; }
                if atpos_it.is_none() { atpos_it = Some(*it); }
            }
        }
        let Some(e) = load(src, it)? else { result = Move::PosMatchOrZv; break };
        let line_wrap = wrap(cx, it);
        if line_wrap == Wrap::Truncate {
            if reached(&e) && (it.pixel_width > 0 || e.charpos > to || !e.text_area) {
                result = Move::PosMatchOrZv;
                break;
            }
        } else if line_wrap == Wrap::Word && e.text_area {
            let next_may_wrap = e.whitespace;
            if may_wrap && !e.whitespace {
                if let Some(saved) = atpos_it { *it = saved; return Ok(Move::PosMatchOrZv); }
                if let Some(saved) = atx_it { *it = saved; return Ok(Move::XReached); }
                wrap_it = Some(*it);
            }
            may_wrap = next_may_wrap;
        }
        let max_h = it.max_h;
        let x0 = it.current_x;
        produce(cx, it, &e);
        if !e.text_area {
            advance(src, it);
            continue;
        }
        if it.nglyphs > 0 {
            let single = it.pixel_width / it.nglyphs;
            let hpos_before = it.hpos;
            let mut x = x0;
            for i in 0..it.nglyphs {
                let new_x = x + single;
                let line_wrap = wrap(cx, it);
                if op.x && new_x > to_x {
                    if reached(&e) {
                        if line_wrap != Wrap::Word || wrap_it.is_none() {
                            (it.current_x, it.max_h) = (x, max_h);
                            result = Move::PosMatchOrZv;
                            break 'line;
                        }
                        if atpos_it.is_none() { let mut a = *it; (a.current_x, a.max_h) = (x, max_h); atpos_it = Some(a); }
                    } else {
                        if line_wrap != Wrap::Word || wrap_it.is_none() {
                            it.current_x = x;
                            result = Move::XReached;
                            break;
                        }
                        if atx_it.is_none() { let mut a = *it; (a.current_x, a.max_h) = (x, max_h); atx_it = Some(a); }
                    }
                }
                let beyond = new_x > cx.last_visible_x && (subject == 0 || (i != 0 && x > cx.last_visible_x));
                if line_wrap != Wrap::Truncate && beyond {
                    result = continue_line(src, cx, it, &e, ContinueAt { i, x, new_x, x0, hpos_before, max_h },
                        &mut wrap_it, &mut atpos_it, &mut atx_it, &reached)?;
                    break;
                }
                if reached(&e) {
                    if line_wrap != Wrap::Word || wrap_it.is_none() {
                        (it.current_x, it.max_h) = (x, max_h);
                        result = Move::PosMatchOrZv;
                        break 'line;
                    }
                    if atpos_it.is_none() { let mut a = *it; (a.current_x, a.max_h) = (x, max_h); atpos_it = Some(a); }
                }
                if new_x > cx.first_visible_x {
                    if lnum_pending && let Some(lnum) = cx.lnum {
                        // The row's first visible glyph: its number shows
                        // at the first visible x, the text after it.
                        lnum_pending = false;
                        let (glyphs, width) = lnum.prefix(src.line_at(it.index)?);
                        it.current_x = cx.first_visible_x + width + new_x - cx.first_visible_x;
                        (it.hpos, it.lnum_x, it.lnum_last, it.lnum_hpos) = (it.hpos + glyphs, width, width, it.hpos + glyphs);
                    }
                    it.hpos += 1;
                }
                x = new_x;
            }
            if result != Move::Undefined { break; }
        } else if reached(&e) {
            (it.current_x, it.max_h) = (x0, max_h);
            result = Move::PosMatchOrZv;
            break;
        } else if op.x && it.current_x >= to_x {
            result = Move::XReached;
            break;
        }
        if e.end_of_line() {
            result = Move::NewlineOrCr;
            break;
        }
        advance(src, it);
        if subject == 1 && it.prefix.is_none() { subject = 2; } else if subject == 2 && it.prefix.is_none() { subject = 0; }
        if wrap(cx, it) == Wrap::Truncate && it.current_x >= cx.last_visible_x {
            result = match load(src, it)? {
                None => Move::PosMatchOrZv,
                Some(n) if reached(&n) => Move::PosMatchOrZv,
                Some(n) if n.end_of_line() => Move::NewlineOrCr,
                Some(_) => Move::LineTruncated,
            };
            break;
        }
    }
    let ww = wrap(cx, it) == Wrap::Word;
    match (wrap_it, atpos_it, atx_it) {
        (Some(w), a, b) if result == Move::LineContinued && ww
            && (a.is_some_and(|a| w.current_x < a.current_x) || b.is_some_and(|b| w.current_x < b.current_x)) => *it = w,
        (_, Some(a), _) => *it = a,
        (_, None, Some(b)) => *it = b,
        _ => {}
    }
    Ok(result)
}

struct ContinueAt { i: i64, x: i64, new_x: i64, x0: i64, hpos_before: i64, max_h: Rise }

/// The branch of move_it_in_display_line_to for a glyph that does not fit
/// a continued line. A terminal has no fringe, so a newline after the last
/// glyph that fits ends this row rather than starting a continuation.
#[allow(clippy::too_many_arguments)]
fn continue_line<E: Elements>(src: &mut E, cx: &Ctx, it: &mut It, e: &Elem, at: ContinueAt,
    wrap_it: &mut Option<It>, atpos_it: &mut Option<It>, atx_it: &mut Option<It>, reached: &impl Fn(&Elem) -> bool) -> R<Move> {
    let line_wrap = wrap(cx, it);
    // The row's first text glyph does not fit (a wide image, or a TAB
    // after the line number): it stays on the row (hpos == 0 modulo the
    // line number's width, move_it_in_display_line_to).
    if it.hpos == it.lnum_hpos {
        it.hpos += 1;
        it.current_x = at.new_x;
        if at.i == it.nglyphs - 1 {
            if reached(e) {
                if line_wrap != Wrap::Word || wrap_it.is_none() {
                    (it.hpos, it.current_x) = (at.hpos_before, at.x0);
                    return Ok(Move::PosMatchOrZv);
                }
                if atpos_it.is_none() { let mut a = *it; (a.current_x, a.hpos) = (at.x0, at.hpos_before); *atpos_it = Some(a); }
            }
            let prev_stretch = e.method == Method::Stretch;
            advance(src, it);
            let Some(n) = load(src, it)? else { return Ok(Move::PosMatchOrZv) };
            if reached(&n) {
                if n.end_of_line() { return Ok(Move::PosMatchOrZv); }
                if !prev_stretch || line_wrap != Wrap::Word { return Ok(Move::LineContinued); }
            }
            if n.end_of_line() && (line_wrap != Wrap::Word || wrap_it.is_none()) { return Ok(Move::NewlineOrCr); }
        }
    } else {
        (it.current_x, it.max_h) = (at.x, at.max_h);
    }
    if let Some(w) = *wrap_it {
        *it = w;
        *atpos_it = None;
        *atx_it = None;
    }
    Ok(Move::LineContinued)
}

/// move_it_to; returns the widest line scanned, never beyond the last
/// visible x. TO, TO_X, TO_Y and TO_VPOS count only when OP names them.
pub(super) fn move_to<E: Elements>(src: &mut E, cx: &Ctx, it: &mut It, to: i64, to_x: i64, to_y: i64, to_vpos: i64, op: Op) -> R<i64> {
    let mut max_x = 0;
    let mut line_start_x = 0;
    loop {
        let mut skip;
        if op.vpos {
            if !op.x && !op.pos {
                if it.vpos == to_vpos { break; }
                skip = move_in_line(src, cx, it, -1, -1, Op::default())?;
            } else {
                if it.vpos == to_vpos { break; }
                skip = move_in_line(src, cx, it, to, to_x, op)?;
                if skip == Move::PosMatchOrZv || it.vpos == to_vpos { break; }
                if skip == Move::XReached {
                    skip = move_in_line(src, cx, it, to, -1, POS)?;
                    if skip == Move::PosMatchOrZv { break; }
                }
            }
        } else if op.y {
            let line_backup = (cx.wrap == Wrap::Word).then_some(*it);
            let mut reached = false;
            skip = move_in_line(src, cx, it, to, if op.x { to_x } else { 0 }, Op { x: true, pos: op.pos, ..Op::default() })?;
            let in_line = |it: &It| to_y >= it.current_y && to_y < it.current_y + it.max_h.height();
            if skip == Move::PosMatchOrZv {
                reached = true;
            } else if skip == Move::XReached {
                if in_line(it) { break; }
                let backup = *it;
                let skip2 = move_in_line(src, cx, it, to, -1, Op { pos: op.pos, ..Op::default() })?;
                if in_line(it) {
                    let h = it.max_h;
                    *it = backup;
                    it.max_h = h;
                    reached = true;
                } else {
                    skip = skip2;
                    if skip == Move::PosMatchOrZv {
                        reached = true;
                        let (now, then) = (charpos(src, it)?, charpos(src, &backup)?);
                        if to > 0 && now != to && ((then > to) == (now > to)) {
                            let h = it.max_h;
                            *it = backup;
                            it.max_h = h;
                        }
                    }
                }
            } else if in_line(it) {
                if to_y > it.current_y { max_x = max_x.max(it.current_x); }
                if skip == Move::LineContinued && cx.wrap == Wrap::Word && let Some(saved) = line_backup {
                    let prev_x = (it.current_x - 1).max(0);
                    *it = saved;
                    move_in_line(src, cx, it, -1, prev_x, Op { x: true, ..Op::default() })?;
                }
                reached = true;
            }
            if reached {
                max_x = max_x.max(it.current_x);
                break;
            }
        } else {
            let (buffer, at) = position(src, it)?;
            skip = if buffer && at >= to { Move::PosMatchOrZv } else { move_in_line(src, cx, it, to, -1, POS)? };
        }
        match skip {
            Move::PosMatchOrZv => {
                max_x = max_x.max(it.current_x);
                break;
            }
            Move::NewlineOrCr => {
                max_x = max_x.max(it.current_x);
                advance(src, it);
                it.continuation_lines_width = 0;
            }
            Move::LineTruncated => {
                max_x = cx.last_visible_x;
                it.continuation_lines_width = 0;
                next_line_start(src, it)?;
                if op.pos {
                    let at = charpos(src, it)?;
                    let at_end = src.slot(it.index)?.elem.is_none();
                    if at > to || (at == to && to == cx.zv && !cx.zv_after_newline && at_end) { break; }
                }
            }
            Move::LineContinued => {
                max_x = cx.last_visible_x;
                if element(src, it)?.is_some_and(|e| matches!(e.shape, Shape::Tab | Shape::PixelTab { .. })) {
                    it.continuation_lines_width += cx.last_visible_x;
                    if it.current_x != cx.last_visible_x && op.vpos && !op.x && !op.pos {
                        line_start_x = it.current_x + it.pixel_width - cx.last_visible_x;
                        advance(src, it);
                    }
                } else {
                    it.continuation_lines_width += it.current_x;
                }
            }
            Move::Undefined | Move::XReached => unreachable!("move_in_line result {skip:?}"),
        }
        it.current_x = line_start_x;
        it.wrap_prefix_width = 0;
        line_start_x = 0;
        it.hpos = 0;
        it.lnum_x = 0;
        it.lnum_hpos = 0;
        it.current_y += it.max_h.height();
        it.vpos += 1;
        it.max_h = Rise::default();
    }
    Ok(max_x)
}

/// reseat_at_next_visible_line_start through forward_to_next_line_start:
/// past the next newline the iterator delivers.
fn next_line_start<E: Elements>(src: &mut E, it: &mut It) -> R<()> {
    while let Some(e) = load(src, it)? {
        advance(src, it);
        if e.end_of_line() { break; }
    }
    it.loaded = false;
    Ok(())
}

/// move_it_vertically_backward (IT, 0)'s count of screen rows from the
/// origin, a line start, to START: whole rows are taken until the
/// iterator stands where a move may stop (not inside a string), then on
/// to START.
pub(super) fn screen_row<E: Elements>(src: &mut E, cx: &Ctx, start: i64) -> R<i64> {
    Ok(screen_position(src, cx, start)?.vpos)
}

/// [`screen_row`]'s iterator, stopped at START.
fn screen_position<E: Elements>(src: &mut E, cx: &Ctx, start: i64) -> R<It> {
    let mut probe = It::default();
    let mut to = start;
    loop {
        let op = Op { pos: to > 0, vpos: true, ..Op::default() };
        let next_row = probe.vpos + 1;
        move_to(src, cx, &mut probe, to, -1, -1, next_row, op)?;
        if charpos(src, &probe)? >= start { to = -1; }
        if valid_after_move(src, &probe)?.0 || after_string_newline(src, &probe, start)? { break; }
    }
    move_to(src, cx, &mut probe, start, -1, -1, -1, POS)?;
    Ok(probe)
}

/// The iterator is in a string at START, just after the string's newline.
fn after_string_newline<E: Elements>(src: &mut E, it: &It, start: i64) -> R<bool> {
    let Some(e) = element(src, it)? else { return Ok(false) };
    if it.prefix.is_some() || e.method != Method::String || charpos(src, it)? != start || it.index == 0 { return Ok(false); }
    let before = src.slot(it.index - 1)?.elem;
    Ok(before.is_some_and(|b| b.end_of_line() && b.object == e.object && b.strpos.zip(e.strpos).is_some_and(|(b, e)| b + 1 == e)))
}

/// move_it_by_lines from the origin: the start of screen row ROW.
pub(super) fn row_start<E: Elements>(src: &mut E, cx: &Ctx, row: i64) -> R<It> {
    let mut it = It::default();
    if row > 0 {
        move_to(src, cx, &mut it, -1, -1, -1, row, Op { vpos: true, ..Op::default() })?;
        let (valid, display) = valid_after_move(src, &it)?;
        if !valid {
            let at = charpos(src, &it)? + i64::from(display);
            move_to(src, cx, &mut it, at, -1, -1, -1, POS)?;
        }
    }
    it.current_y = 0;
    it.vpos = 0;
    Ok(it)
}

/// move_it_by_lines (IT, 0) from a line start before START: the start of
/// the screen line showing START.
pub(super) fn screen_line_start<E: Elements>(src: &mut E, cx: &Ctx, start: i64) -> R<It> {
    let row = screen_row(src, cx, start)?;
    row_start(src, cx, row)
}

/// The measurement window_text_pixel_size asks for, in columns and lines.
pub(super) struct Measure {
    pub start: i64,
    pub end: i64,
    pub begv: i64,
    /// The character at START is a newline.
    pub start_on_newline: bool,
    pub x_limit: Option<i64>,
    pub max_y: i64,
    pub ignore_line_at_end: bool,
}

/// window_text_pixel_size (xdisp.c:11742) from the screen line start IT,
/// before its limits and the mode lines: the widest x, the start x to
/// subtract from it, and the height.
pub(super) fn text_size<E: Elements>(src: &mut E, cx: &Ctx, mut it: It, m: &Measure) -> R<(i64, i64, i64)> {
    let start = m.start;
    it.current_x = 0;
    it.hpos = 0;
    it.lnum_x = 0;
    it.lnum_hpos = 0;
    it.wrap_prefix_width = 0;
    let mut start_x;
    if charpos(src, &it)? != start {
        let it1 = it;
        move_to(src, cx, &mut it, start, -1, -1, -1, POS)?;
        if charpos(src, &it)? > start && start > m.begv {
            let (it1pos, it1_x) = (charpos(src, &it1)?, it1.current_x);
            it = it1;
            if start - 1 > it1pos {
                move_to(src, cx, &mut it, start - 1, -1, -1, -1, POS)?;
            } else {
                move_in_line(src, cx, &mut it, start, it1_x + 1, Op { pos: true, x: true, ..Op::default() })?;
            }
            start_x = it.current_x;
            if charpos(src, &it)? == start - 1 { start_x += it.pixel_width; }
        } else {
            start_x = it.current_x;
        }
    } else {
        start_x = it.current_x;
    }
    let mut op = Op { pos: true, y: true, ..Op::default() };
    let mut to_x = -1;
    it.current_y = 0;
    if m.start_on_newline {
        it.current_x = 0;
        start_x = 0;
        it.wrap_prefix_width = 0;
    }
    let mut cx = *cx;
    if let Some(max_x) = m.x_limit {
        cx.last_visible_x = max_x;
        op.x = true;
        to_x = i64::MAX;
    }
    let it2 = it;
    let mut end = m.end;
    let mut x = move_to(src, &cx, &mut it, end, to_x, m.max_y, -1, op)?;
    let mut doff = 0;
    if charpos(src, &it)? > end {
        let end_y = it.current_y;
        end -= 1;
        it = it2;
        x = move_to(src, &cx, &mut it, end, to_x, m.max_y, -1, op)?;
        let at = charpos(src, &it)?;
        if at == end {
            x += it.pixel_width;
            if m.ignore_line_at_end {
                if it.current_y < end_y { doff = it.max_h.max(it.h).height(); }
            } else {
                it.max_h = it.max_h.max(it.h);
            }
        } else if at > end && wrap(&cx, &it) == Wrap::Truncate && it.current_x >= cx.last_visible_x {
            it.current_y += it.max_h.max(it.h).height();
        }
    }
    if it.current_y > 0 { start_x = 0; }
    let y = it.current_y + doff + if m.ignore_line_at_end { 0 } else { it.max_h.height() };
    Ok((x, start_x, y))
}

/// Elements the iterator can deliver again from another line start, as
/// reseat does, for motion backwards.
pub(super) trait Lines: Elements {
    /// Deliver from START, a line start.
    fn reseat(&mut self, start: i64) -> R<()>;
    /// Element INDEX as delivered, measured or not.
    fn delivered(&mut self, index: usize) -> R<Slot>;
    /// back_to_previous_visible_line_start: the start of the visible line
    /// holding POS - 1.
    fn line_start_before(&mut self, pos: i64) -> R<i64>;
    fn begv(&self) -> i64;
}

/// move_it_by_lines with DVPOS > 0 from IT: the start of the screen line
/// DVPOS below, or where the elements end.
fn lines_down<E: Elements>(src: &mut E, cx: &Ctx, it: &mut It, dvpos: i64) -> R<()> {
    let row = it.vpos + dvpos;
    move_to(src, cx, it, -1, -1, -1, row, Op { vpos: true, ..Op::default() })?;
    let (valid, display) = valid_after_move(src, it)?;
    if !valid {
        let at = charpos(src, it)? + i64::from(display);
        move_to(src, cx, it, at, -1, -1, -1, POS)?;
    }
    Ok(())
}

/// move_it_by_lines with DVPOS < 0 from POS: back over visible lines until
/// DVPOS screen lines lie between the line start reached and the screen
/// line showing POS, or BEGV is reached; the elements then deliver from
/// that line start and the iterator stands at the screen line's start,
/// its vpos the (negative) count of screen lines moved.
pub(super) fn lines_up<E: Lines>(src: &mut E, cx: &Ctx, pos: i64, dvpos: i64) -> R<It> {
    let begv = src.begv();
    let mut origin = src.line_start_before(pos)?;
    src.reseat(origin)?;
    let at = screen_position(src, cx, pos)?;
    let (mut rows, mut plain) = (at.vpos, buffer_text(src, &at)?);
    while rows < -dvpos && origin > begv {
        let below = origin;
        origin = src.line_start_before(below)?;
        src.reseat(origin)?;
        // Over the buffer's own text, a line ended by its newline adds its
        // rows to those below it, and only it needs walking. Anything else
        // from here to POS, or a newline the display table shows otherwise
        // or not at all, and only a walk to POS can say.
        if plain {
            let at = screen_position(src, cx, below)?;
            plain = buffer_text(src, &at)? && at.current_x == 0 && at.continuation_lines_width == 0
                && charpos(src, &at)? == below && after_newline(src, &at, below)?;
            if plain {
                rows += at.vpos;
                continue;
            }
            src.reseat(origin)?;
        }
        rows = screen_row(src, cx, pos)?;
    }
    let moved = rows.min(-dvpos);
    let mut it = row_start(src, cx, rows - moved)?;
    it.vpos = -moved;
    Ok(it)
}

/// Whether all IT has passed, and what it stands at, is the buffer's own
/// text: no string, display table glyph, stretch or display property. Only
/// there does a walk count no row past where it stops, and a reseat deliver
/// what the walk delivered.
fn buffer_text<E: Lines>(src: &mut E, it: &It) -> R<bool> {
    let own = |slot: Slot| slot.elem.is_none_or(|e| e.method == Method::Buffer && !e.from_display && e.ellipsis.is_none());
    for index in 0..it.index {
        if !own(src.delivered(index)?) { return Ok(false); }
    }
    Ok(it.prefix.is_none() && src.delivered(it.index).is_ok_and(own))
}

/// Whether the last element IT passed is the buffer's newline just before
/// BELOW: what `Source' delivers next then depends on nothing before it.
fn after_newline<E: Lines>(src: &mut E, it: &It, below: i64) -> R<bool> {
    let Some(last) = it.index.checked_sub(1) else { return Ok(false) };
    Ok(src.delivered(last)?.elem.is_some_and(|e| e.charpos == below - 1 && matches!(e.shape, Shape::Glyph { c: 10, .. })))
}

/// Fvertical_motion (indent.c:2207) on a displayed window, from point PT:
/// the position LINES screen lines down (up when negative), at x TO_X of
/// that line when given, and the screen lines moved. The elements deliver
/// from whatever line start the motion needed; a window that does not
/// start at PT's line start makes no difference here.
pub(super) fn vertical_motion<E: Lines>(src: &mut E, cx: &Ctx, pt: i64, lines: i64, to_x: Option<i64>) -> R<(i64, i64)> {
    // reseat_at_previous_visible_line_start, then on to PT.
    let origin = src.line_start_before(pt)?;
    src.reseat(origin)?;
    let mut it = It::default();
    move_to(src, cx, &mut it, pt, -1, -1, -1, POS)?;
    let mut overshot = false;
    if charpos(src, &it)? > pt {
        // Past PT, as on a truncated line whose PT lies beyond the right
        // margin: back a line unless still on PT's own screen line.
        if it.vpos > 0 || it.current_x >= cx.last_visible_x {
            let at = charpos(src, &it)?;
            it = lines_up(src, cx, at, -1)?;
        }
        overshot = true;
    }
    if lines <= 0 {
        it.vpos = 0;
        it.current_y = 0;
        if lines < 0 && charpos(src, &it)? > src.begv() {
            let at = charpos(src, &it)?;
            it = lines_up(src, cx, at, lines)?;
        } else if lines == 0 {
            let at = charpos(src, &it)?;
            it = lines_up(src, cx, at, 0)?;
        }
    } else if overshot {
        it.vpos = 0;
        it.current_y = 0;
        lines_down(src, cx, &mut it, lines)?;
    } else if pt < cx.zv {
        if charpos(src, &it)? < pt {
            (it.vpos, it.current_y) = (0, 0);
            lines_down(src, cx, &mut it, 1)?;
        }
        while charpos(src, &it)? == pt {
            (it.vpos, it.current_y) = (0, 0);
            lines_down(src, cx, &mut it, 1)?;
        }
        if lines > 1 { lines_down(src, cx, &mut it, lines - 1)?; }
    } else {
        (it.vpos, it.current_y) = (0, 0);
        lines_down(src, cx, &mut it, lines)?;
        if charpos(src, &it)? == pt { it.vpos = 0; }
    }
    if let Some(x) = to_x {
        move_in_line(src, cx, &mut it, cx.zv, x, Op { x: true, ..Op::default() })?;
    }
    Ok((charpos(src, &it)?, it.vpos))
}

/// Fwindow_end's motion (window.c:1782) from IT at a window's start:
/// move_it_vertically down the text area's height DY, then, when the row
/// reached starts inside the window, move_it_past_eol over it.
pub(super) fn window_end<E: Lines>(src: &mut E, cx: &Ctx, mut it: It, dy: i64) -> R<i64> {
    move_to(src, cx, &mut it, cx.zv, -1, dy, -1, Op { pos: true, y: true, ..Op::default() })?;
    let at = charpos(src, &it)?;
    // At a ZV after no newline, move_it_vertically moves back to its
    // screen line's start: below the window, where text hidden to ZV
    // begins; inside it, the move past its line's end comes back to ZV.
    if at == cx.zv {
        if it.current_y < dy || cx.zv_after_newline || at <= src.begv() { return Ok(at); }
        return src.line_start_before(at);
    }
    if it.current_y >= dy { return Ok(at); }
    if move_in_line(src, cx, &mut it, cx.zv, 0, POS)? == Move::NewlineOrCr { advance(src, &mut it); }
    charpos(src, &it)
}
