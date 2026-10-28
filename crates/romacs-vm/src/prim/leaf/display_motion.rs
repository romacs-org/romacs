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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Wrap { Truncate, Window, Word }

/// How the iterator delivers the element (`it->method`, and whether
/// `it->object` is the buffer).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Method { Buffer, Vector { last: bool, buffer: bool }, String, Stretch }

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Shape {
    /// A character glyph WIDTH columns wide; a newline is zero wide.
    Glyph { c: u32, width: i64 },
    /// A character the host lays out as one glyph WIDTH pixels wide in a
    /// row at least HEIGHT pixels tall; a newline is zero wide.
    Pixel { c: u32, width: i64, height: i64 },
    Tab,
    /// A TAB the host lays out as a stretch to the next tab stop in its
    /// pixels, SPACE being its face's space width, in a row at least
    /// HEIGHT pixels tall (romacs_layout_input::tab_advance).
    PixelTab { space: i64, height: i64 },
    Stretch { width: i64, zero_ok: bool },
    /// `:align-to', TARGET columns from the text area's left edge.
    Align { target: i64 },
    /// A glyphless character shown as WIDTH columns (term.c
    /// produce_glyphless_glyph).
    Glyphless { width: i64 },
    /// `:relative-width' FACTOR of a TAB, whose width depends on x.
    TabStretch { factor: f64 },
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
    /// max_ascent + max_descent of the row so far.
    pub max_h: i64,
    /// ascent + descent of the last element produced.
    pub h: i64,
    pub continuation_lines_width: i64,
    pub wrap_prefix_width: i64,
    pub pixel_width: i64,
    pub nglyphs: i64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Ctx {
    pub last_visible_x: i64,
    pub wrap: Wrap,
    pub tab_width: i64,
    pub zv: i64,
    pub zv_after_newline: bool,
    /// Whether a row may be continued or truncated. A host that lays out
    /// neither cannot show such a row, so meeting one is refused.
    pub breaks: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Move { Undefined, PosMatchOrZv, NewlineOrCr, LineTruncated, LineContinued, XReached }

#[derive(Clone, Copy, Default)]
pub(super) struct Op { pub pos: bool, pub x: bool, pub y: bool, pub vpos: bool }
const POS: Op = Op { pos: true, x: false, y: false, vpos: false };

/// The refusal for a row a host would have to continue or truncate.
fn unbroken() -> Fault {
    Fault::Error("VM2 window-text-pixel-size of a continued or truncated line in a displayed window is unsupported".into())
}

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

/// produce_glyphs (term.c) and produce_stretch_glyph.
fn produce(cx: &Ctx, it: &mut It, e: &Elem) {
    let from_prefix = it.prefix.is_some();
    let mut height = 1;
    let (width, glyphs) = match e.shape {
        Shape::Glyph { c: 10, .. } => (0, 0),
        Shape::Pixel { c, width, height: h } => { height = h; if c == 10 { (0, 0) } else { (width, 1) } }
        Shape::Glyph { width, .. } | Shape::Glyphless { width } => (width, width),
        Shape::Tab => { let spaces = tab_spaces(cx, it); (spaces, spaces) }
        Shape::PixelTab { space, height: h } => { height = h; (pixel_tab(cx, it, space), 1) }
        Shape::TabStretch { factor } => stretch(cx, it, e, (factor * tab_spaces(cx, it) as f64) as i64, false),
        Shape::Stretch { width, zero_ok } => stretch(cx, it, e, width, zero_ok),
        Shape::Align { target } => {
            let x = it.current_x + if it.align_visually { 0 } else { it.continuation_lines_width };
            stretch(cx, it, e, (target - x).max(0), true)
        }
    };
    it.pixel_width = width;
    it.nglyphs = glyphs;
    if e.text_area {
        it.current_x += width;
        if it.continuation_lines_width != 0 && from_prefix { it.wrap_prefix_width = it.current_x; }
    }
    it.h = height;
    it.max_h = it.max_h.max(height);
}

/// The spaces a TAB at the iterator's x takes to the next tab stop.
fn tab_spaces(cx: &Ctx, it: &It) -> i64 {
    let x = it.current_x + continued_width(it);
    (x + cx.tab_width) / cx.tab_width * cx.tab_width - x
}

/// The pixels a host's TAB at the iterator's x stretches, SPACE being its
/// face's space width.
pub(super) fn pixel_tab(cx: &Ctx, it: &It, space: i64) -> i64 {
    let x = it.current_x + continued_width(it);
    romacs_layout_input::tab_advance(x as f64, space as f64, cx.tab_width as u32) as i64
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
    if it.hpos == 0 {
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
                if !cx.breaks && new_x > cx.last_visible_x { return Err(unbroken()); }
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
                if new_x > 0 { it.hpos += 1; }
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
    if !cx.breaks && matches!(result, Move::LineContinued | Move::LineTruncated) {
        return Err(unbroken());
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

struct ContinueAt { i: i64, x: i64, new_x: i64, x0: i64, hpos_before: i64, max_h: i64 }

/// The branch of move_it_in_display_line_to for a glyph that does not fit
/// a continued line. A terminal has no fringe, so a newline after the last
/// glyph that fits ends this row rather than starting a continuation.
#[allow(clippy::too_many_arguments)]
fn continue_line<E: Elements>(src: &mut E, cx: &Ctx, it: &mut It, e: &Elem, at: ContinueAt,
    wrap_it: &mut Option<It>, atpos_it: &mut Option<It>, atx_it: &mut Option<It>, reached: &impl Fn(&Elem) -> bool) -> R<Move> {
    let line_wrap = wrap(cx, it);
    if it.hpos == 0 {
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
            let in_line = |it: &It| to_y >= it.current_y && to_y < it.current_y + it.max_h;
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
        it.current_y += it.max_h;
        it.vpos += 1;
        it.max_h = 0;
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
    Ok(probe.vpos)
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
                if it.current_y < end_y { doff = it.max_h.max(it.h); }
            } else {
                it.max_h = it.max_h.max(it.h);
            }
        } else if at > end && wrap(&cx, &it) == Wrap::Truncate && it.current_x >= cx.last_visible_x {
            it.current_y += it.max_h.max(it.h);
        }
    }
    if it.current_y > 0 { start_x = 0; }
    let y = it.current_y + doff + if m.ignore_line_at_end { 0 } else { it.max_h };
    Ok((x, start_x, y))
}
