//! The display elements of a window's buffer on a terminal frame, in the
//! order xdisp.c's get_next_display_element delivers them: at each stop
//! position the display property (handle_display_prop), then invisibility
//! (handle_invisible_prop), then overlay strings (load_overlay_strings),
//! then the characters, each classified as get_next_display_element and
//! term.c produce_glyphs do. The window's buffer is current throughout.
use super::*;
use super::display_motion::{Elem, Elements, Glyphless, Method, Shape, Slot, R};
use crate::coordinates::{ByteOffset, CharOffset};
use crate::data::Object;
use romacs_regex_engine::emacs_char::{cat_eq, UnicodeCategory as Category};
use crate::overlay_order::{display_order, OverlayEntry};

/// What a window shows of its buffer, fixed for one measurement.
pub(super) struct View {
    pub window: Value,
    pub buffer: Value,
    pub begv: i64,
    pub zv: i64,
    /// The column width in pixels, for `(N)' pixel display specs.
    pub column: i64,
    /// window_box_width (TEXT_AREA) and the margins, in columns, or in
    /// pixels when PIXELS.
    pub box_width: i64,
    pub left_margin: i64,
    pub right_margin: i64,
    /// Display widths are a window-system frame's pixels, as a host lays
    /// the window out, not a terminal's columns.
    pub pixels: bool,
    /// The x a row's line number takes (lnum_pixel_width), in those units.
    pub lnum: i64,
}

/// Buffer variables the iterator reads (init_iterator).
struct Look {
    ctl_arrow: bool,
    display_table: Option<Value>,
    nobreak: Value,
    nobreak_ascii: bool,
    raw_hex: bool,
    glyphless: Value,
    unibyte_via_language: bool,
}

pub(super) struct Source<'l, 'a> {
    leaf: &'l mut Leaf<'a>,
    view: View,
    look: Look,
    syms: Syms,
    slots: Vec<Slot>,
    /// Next buffer position to deliver, and its byte.
    next: i64,
    byte: usize,
    /// Characters before here need no stop handling.
    run_end: i64,
    stop_due: bool,
    /// The buffer position the iterator read from before the next element.
    pre: Option<i64>,
    ended: bool,
    prefixes: Vec<Vec<Elem>>,
    /// An `:align-to' being computed has counted the line number
    /// (calc_pixel_width_or_height's LNUM_ONCE).
    lnum_added: bool,
    /// A byte and the lines of the accessible text before it, from which
    /// `line_at' counts on.
    lines: (usize, i64),
    /// How many columns a character takes (width_rule.rs), read once.
    rule: crate::width_rule::WidthRule,
    /// The images the elements show (Shape::Image's AT), with their specs.
    images: Vec<(romacs_layout_input::InlineImage, Value)>,
}

#[derive(Clone, Copy)]
struct Syms {
    display: Value, invisible: Value, composition: Value, before: Value, after: Value, priority: Value,
    line_prefix: Value, wrap_prefix: Value, space: Value, when: Value, disable_eval: Value, margin: Value,
    left_margin: Value, right_margin: Value, left_fringe: Value, right_fringe: Value, image: Value,
    slice: Value, space_width: Value, height: Value, raise: Value, min_width: Value, width: Value,
    relative_width: Value, align_to: Value, text: Value, left: Value, right: Value, center: Value,
    scroll_bar: Value, plus: Value, minus: Value, zero_width: Value, thin_space: Value, empty_box: Value,
    hex_code: Value, escape_glyph: Value, nobreak_space: Value, nobreak_hyphen: Value,
}

fn fault(message: &str) -> Fault { Fault::Error(message.into()) }
fn unsupported(what: &str) -> Fault { Fault::Error(format!("VM2 window-text-pixel-size {what} is unsupported")) }

/// A replacing display specification (handle_single_display_spec).
/// A margin's string or space shows outside the text area.
#[derive(Clone, Copy)]
enum Replacement { String(Value, bool), Space(Value, bool), Nothing,
    /// An image spec on a window-system frame, with the `(slice ...)' plist
    /// tail beside it (else nil).
    Image(Value, bool, Value) }

/// One character of a buffer or string, before classification.
#[derive(Clone, Copy)]
struct Origin { charpos: i64, string: Value, strpos: Option<usize>, from_display: bool, text_area: bool, whitespace_under: bool }

impl<'l, 'a> Source<'l, 'a> {
    pub(super) fn new(leaf: &'l mut Leaf<'a>, view: View, start: i64) -> R<Self> {
        let syms = Syms::new(leaf);
        let look = Look::new(leaf)?;
        let byte = leaf.buffer_byte(view.buffer, start)?;
        let rule = leaf.vm.width_rule().map_err(Fault::Error)?;
        Ok(Source { leaf, view, look, syms, slots: vec![], next: start, byte, run_end: start, stop_due: true,
            pre: None, ended: false, prefixes: vec![], lnum_added: false, lines: (0, -1), rule, images: vec![] })
    }

    pub(super) fn begv(&self) -> i64 { self.view.begv }
    pub(super) fn window(&self) -> Value { self.view.window }
    /// The leaf the elements come from, for a host layout to measure them.
    pub(super) fn leaf(&mut self) -> &mut Leaf<'a> { self.leaf }
    /// C's columns (CHARACTER_WIDTH).
    pub(super) fn width(&self, c: u32) -> i64 { self.rule.width(c) as i64 }

    /// Deliver from START again, a line start.
    /// The image a `Shape::Image' shows, and its spec.
    pub(super) fn image(&self, at: u32) -> (romacs_layout_input::InlineImage, Value) {
        self.images[at as usize]
    }

    pub(super) fn reset(&mut self, start: i64) -> R<()> {
        self.slots.clear();
        self.prefixes.clear();
        self.images.clear();
        self.move_to(start)?;
        (self.run_end, self.stop_due, self.pre, self.ended) = (start, true, None, false);
        Ok(())
    }

    /// FETCH_BYTE's character at POS, which may lie past a narrowing.
    pub(super) fn char_after(&self, pos: i64) -> R<Option<u32>> {
        let byte = self.leaf.buffer_byte(self.view.buffer, pos)?;
        Ok((byte < self.text().as_str().len()).then(|| self.char_at(byte)))
    }
    /// The character before POS is a newline.
    pub(super) fn after_newline(&self, pos: i64) -> R<bool> {
        let byte = self.leaf.buffer_byte(self.view.buffer, pos)?;
        Ok(byte > 0 && self.text().as_str().as_bytes()[byte - 1] == b'\n')
    }

    /// back_to_previous_visible_line_start: the start of the line holding
    /// POS - 1, passing back over invisible newlines and newlines a
    /// display property replaces.
    pub(super) fn visible_line_start(&mut self, pos: i64) -> R<i64> {
        let begv = self.view.begv;
        let mut p = pos;
        while p > begv {
            p = self.line_start_before(p)?;
            if p <= begv { break; }
            let invisible = self.property(p - 1, self.syms.invisible)?.0;
            if self.leaf.invisible_kind(invisible)? != 0 { continue; }
            let (spec, overlay) = self.property(p - 1, self.syms.display)?;
            if spec != Value::NIL && self.display_spec(spec, false, false)?.is_some() {
                let start = match self.overlay_span(overlay)? {
                    Some((start, _)) => start,
                    None => {
                        let (at, display) = (Value::integer(p).map_err(Fault::Error)?, self.syms.display);
                        self.leaf.vm.previous_property_change_values(at, Some(display), self.view.buffer, Value::NIL)?
                            .integer_value().unwrap_or(begv)
                    }
                };
                p = start.max(begv);
                continue;
            }
            break;
        }
        Ok(p)
    }
    /// The display POS shows, if it replaces the text there: where the
    /// text it replaces starts, and its string (nil for a space or an
    /// empty replacement), as pos_visible_p finds them.
    pub(super) fn replacing_display(&mut self, pos: i64) -> R<Option<(i64, Value)>> {
        if pos >= self.view.zv { return Ok(None); }
        let (spec, overlay) = self.property(pos, self.syms.display)?;
        if spec == Value::NIL { return Ok(None); }
        let string = match self.display_spec(spec, false, false)? {
            None => return Ok(None),
            Some(Replacement::String(string, _)) => string,
            Some(_) => Value::NIL,
        };
        let start = match self.overlay_span(overlay)? {
            Some((start, _)) => start,
            None => {
                let (at, display) = (Value::integer(pos + 1).map_err(Fault::Error)?, self.syms.display);
                self.leaf.vm.previous_property_change_values(at, Some(display), self.view.buffer, Value::NIL)?
                    .integer_value().unwrap_or(self.view.begv)
            }
        };
        Ok(Some((start.max(self.view.begv), string)))
    }
    /// Whether a row starts at POS: the character before it is a newline
    /// that is visible and not replaced, so no text before POS can reach
    /// POS's row.
    pub(super) fn starts_row(&mut self, pos: i64) -> R<bool> {
        if pos <= self.view.begv { return Ok(true); }
        if self.char_after(pos - 1)? != Some(0x0a) { return Ok(false); }
        let invisible = self.property(pos - 1, self.syms.invisible)?.0;
        Ok(self.leaf.invisible_kind(invisible)? == 0 && self.property(pos - 1, self.syms.display)?.0 == Value::NIL)
    }
    /// find_newline_no_quit backward from POS - 1: the line start before it.
    fn line_start_before(&self, pos: i64) -> R<i64> {
        let (low, at) = (self.leaf.buffer_byte(self.view.buffer, self.view.begv)?, self.leaf.buffer_byte(self.view.buffer, pos - 1)?);
        let bytes = self.text().as_str().as_bytes();
        let start = bytes[low..at].iter().rposition(|&b| b == b'\n').map_or(low, |i| low + i + 1);
        Ok(self.text().char_of_byte(ByteOffset(start)).map_err(Fault::Error)?.0 as i64 + 1)
    }

    fn text(&self) -> &crate::annotations::AnnotatedBuffer {
        match self.leaf.vm.shown_text(self.view.buffer) { Ok(b) => b, _ => unreachable!("checked text buffer") }
    }
    fn char_at(&self, byte: usize) -> u32 {
        // The flat text's scalar, then the raw-byte listing (empty for
        // pure Unicode text) decides its code.
        let text = self.text();
        text.as_str()[byte..].chars().next().map_or(0, |c| text.code_at(byte, c))
    }
    fn buffer_whitespace(&self, pos: i64) -> R<bool> {
        if pos >= self.view.zv { return Ok(false); }
        let byte = self.leaf.buffer_byte(self.view.buffer, pos)?;
        Ok(matches!(self.char_at(byte), 0x20 | 0x09))
    }
    fn property(&mut self, pos: i64, key: Value) -> R<(Value, Value)> {
        let byte = self.leaf.buffer_byte(self.view.buffer, pos)?;
        self.leaf.vm.window_char_property(self.view.buffer, byte, key, self.view.window).map_err(Fault::Error)
    }

    fn emit(&mut self, elem: Elem) {
        let pre = self.pre.take();
        self.slots.push(Slot { pre, elem: Some(elem) });
    }

    /// Generate until at least one more slot exists, or the text ends.
    fn generate(&mut self) -> R<()> {
        let before = self.slots.len();
        while self.slots.len() == before && !self.ended {
            if self.stop_due || self.next >= self.run_end {
                self.handle_stop()?;
                continue;
            }
            self.buffer_char()?;
        }
        Ok(())
    }

    fn buffer_char(&mut self) -> R<()> {
        let (pos, byte) = (self.next, self.byte);
        let c = self.char_at(byte);
        self.next += 1;
        self.byte += crate::lstring::char_string(c, &mut [0; 5]);
        if self.pre.is_none() { self.pre = Some(pos); }
        let origin = Origin { charpos: pos, string: Value::NIL, strpos: None, from_display: false, text_area: true,
            whitespace_under: matches!(c, 0x20 | 0x09) };
        self.character(c, origin)
    }

    fn move_to(&mut self, pos: i64) -> R<()> {
        self.next = pos;
        self.byte = self.leaf.buffer_byte(self.view.buffer, pos)?;
        Ok(())
    }

    /// handle_stop at the next buffer position.
    fn handle_stop(&mut self) -> R<()> {
        let p = self.next;
        self.stop_due = false;
        if self.pre.is_none() { self.pre = Some(p); }
        if p < self.view.zv {
            composition_refusal(self.property(p, self.syms.composition)?.0)?;
            let (spec, overlay) = self.property(p, self.syms.display)?;
            if spec != Value::NIL && let Some(replacement) = self.display_spec(spec, false, false)? {
                let end = match self.overlay_span(overlay)? {
                    Some((_, end)) => end.clamp(self.view.begv, self.view.zv),
                    None => self.display_end(p)?,
                };
                self.overlay_strings(p, p)?;
                let origin = Origin { charpos: p, string: Value::NIL, strpos: None, from_display: true, text_area: true,
                    whitespace_under: self.buffer_whitespace(p)? };
                self.replacement(replacement, origin)?;
                self.move_to(end)?;
                self.stop_due = true;
                return Ok(());
            }
            let (property, overlay) = self.property(p, self.syms.invisible)?;
            let invisible = self.leaf.invisible_kind(property)?;
            if invisible != 0 {
                let (end, ellipsis) = self.skip_invisible(p, invisible == 2)?;
                self.move_to(end)?;
                self.stop_due = true;
                if overlay == Value::NIL && self.has_overlay_strings(p)? {
                    self.overlay_strings(p, end)?;
                }
                if ellipsis { self.ellipsis(Origin { charpos: end, string: Value::NIL, strpos: None, from_display: false,
                    text_area: true, whitespace_under: self.buffer_whitespace(end)? }, true)?; }
                return Ok(());
            }
        }
        self.overlay_strings(p, p)?;
        if p >= self.view.zv {
            self.ended = true;
            return Ok(());
        }
        let zv_byte = self.leaf.buffer_byte(self.view.buffer, self.view.zv)?;
        let boundary = self.leaf.vm.next_property_boundary(self.view.buffer, ByteOffset(self.byte), ByteOffset(zv_byte))
            .map_err(Fault::Error)?;
        self.run_end = (self.text().char_of_byte(boundary).map_err(Fault::Error)?.0 as i64 + 1).max(p + 1);
        Ok(())
    }

    /// The span of OVERLAY, nil when a text property gave the value.
    fn overlay_span(&self, overlay: Value) -> R<Option<(i64, i64)>> {
        if overlay == Value::NIL { return Ok(None); }
        self.leaf.vm.overlay_positions(overlay).map_err(Fault::Error)
    }

    /// display_prop_end over the buffer.
    fn display_end(&mut self, p: i64) -> R<i64> {
        let (at, display, buffer) = (Value::integer(p).map_err(Fault::Error)?, self.syms.display, self.view.buffer);
        let end = self.leaf.vm.next_single_char_property_change_values(at, display, buffer, Value::NIL)?;
        Ok(end.integer_value().unwrap_or(self.view.zv))
    }

    /// handle_invisible_prop over buffer text: the first visible position
    /// at or after P, checked where properties or overlays may change, and
    /// whether any of the skipped text asks for an ellipsis.
    fn skip_invisible(&mut self, p: i64, mut ellipsis: bool) -> R<(i64, bool)> {
        let zv_byte = self.leaf.buffer_byte(self.view.buffer, self.view.zv)?;
        let mut pos = p;
        loop {
            let byte = self.leaf.buffer_byte(self.view.buffer, pos)?;
            let next = self.leaf.vm.next_property_boundary(self.view.buffer, ByteOffset(byte), ByteOffset(zv_byte))
                .map_err(Fault::Error)?;
            pos = (self.text().char_of_byte(next).map_err(Fault::Error)?.0 as i64 + 1).max(pos + 1);
            if pos >= self.view.zv { return Ok((self.view.zv, ellipsis)); }
            let property = self.property(pos, self.syms.invisible)?.0;
            match self.leaf.invisible_kind(property)? {
                0 => return Ok((pos, ellipsis)),
                2 => ellipsis = true,
                _ => {}
            }
        }
    }

    /// The overlays at P that apply to the window, with their start and end.
    fn overlays_at(&mut self, p: i64) -> R<Vec<(Value, i64, i64)>> {
        let lo = self.leaf.buffer_byte(self.view.buffer, (p - 1).max(self.view.begv))?;
        let hi = self.leaf.buffer_byte(self.view.buffer, (p + 1).min(self.view.zv))?;
        let mut found = vec![];
        self.leaf.vm.visit_overlays_in(self.view.buffer, ByteOffset(lo)..ByteOffset(hi), |overlay, _| found.push(overlay))
            .map_err(Fault::Error)?;
        let mut out = vec![];
        for overlay in found {
            if !self.leaf.vm.overlay_applies_to(overlay, self.view.window) { continue; }
            if let Some((start, end)) = self.leaf.vm.overlay_positions(overlay).map_err(Fault::Error)?
                && (start == p || end == p) && !out.iter().any(|(o, _, _)| *o == overlay) {
                out.push((overlay, start, end));
            }
        }
        Ok(out)
    }

    /// load_overlay_strings at P, overlays in ITREE_DESCENDING order: by
    /// decreasing start, then by when the itree last inserted each, which
    /// move-overlay and insertion before a front-advance start change and
    /// VM2 does not record. So two overlays showing strings here must not
    /// share a start. An overlay over invisible text shows both strings at
    /// either end.
    fn overlay_string_list(&mut self, p: i64) -> R<Vec<Value>> {
        let mut overlays = self.overlays_at(p)?;
        overlays.sort_by_key(|&(_, start, _)| std::cmp::Reverse(start));
        let mut entries = vec![];
        for (overlay, start, end) in overlays {
            let property = self.leaf.vm.overlay_property(overlay, self.syms.invisible).map_err(Fault::Error)?;
            let invisible = self.leaf.invisible_kind(property)? != 0;
            let priority = self.leaf.vm.overlay_property(overlay, self.syms.priority).map_err(Fault::Error)?
                .integer_value().unwrap_or(0);
            for (key, after, shows) in [(self.syms.before, false, start == p || (end == p && invisible)),
                (self.syms.after, true, end == p || (start == p && invisible))] {
                let string = self.leaf.vm.overlay_property(overlay, key).map_err(Fault::Error)?;
                if shows && self.nonempty_string(string) { entries.push(OverlayEntry { overlay, string, priority, after, start }); }
            }
        }
        if entries.iter().any(|a| entries.iter().any(|b| a.overlay != b.overlay && a.start == b.start)) {
            return Err(unsupported("of strings from overlays that share a start"));
        }
        Ok(display_order(entries))
    }
    fn nonempty_string(&self, v: Value) -> bool {
        self.leaf.vm.str_view(v).is_some_and(|s| s.chars > 0)
    }
    fn has_overlay_strings(&mut self, p: i64) -> R<bool> { Ok(!self.overlay_string_list(p)?.is_empty()) }

    /// The overlay strings at P, delivered at buffer position CHARPOS.
    fn overlay_strings(&mut self, p: i64, charpos: i64) -> R<()> {
        let under = self.buffer_whitespace(charpos)?;
        for string in self.overlay_string_list(p)? {
            self.string(string, Origin { charpos, string, strpos: None, from_display: false, text_area: true, whitespace_under: under })?;
        }
        Ok(())
    }

    /// A string's characters with its own invisible and, unless it came
    /// from a display property, display properties.
    fn string(&mut self, string: Value, origin: Origin) -> R<()> {
        let (chars, unibyte): (Vec<(usize, u32)>, bool) = {
            let view = self.leaf.vm.str_view(string).ok_or_else(|| fault("display string required"))?;
            if !view.is_multibyte() { (view.bytes().iter().enumerate().map(|(i, &b)| (i, unibyte_char(b))).collect(), true) }
            else {
                let (bytes, mut at, mut out) = (view.bytes(), 0, vec![]);
                while at < bytes.len() {
                    let (c, width) = crate::lstring::string_char(&bytes[at..]);
                    out.push((at, c));
                    at += width;
                }
                (out, false)
            }
        };
        if self.look.unibyte_via_language && unibyte && chars.iter().any(|&(_, c)| c > 0x7f) {
            return Err(unsupported("of a unibyte string with `unibyte-display-via-language-environment'"));
        }
        let end_byte = self.leaf.vm.str_view(string).map_or(0, |v| v.bytes().len());
        let mut i = 0;
        while i < chars.len() {
            let here = Origin { strpos: Some(i), ..origin };
            composition_refusal(self.leaf.vm.property_textget(string, i as i64, self.syms.composition)?)?;
            let spec = self.leaf.vm.property_textget(string, i as i64, self.syms.display)?;
            if spec != Value::NIL && let Some(replacement) = self.display_spec(spec, true, origin.from_display)? {
                let end = self.string_run_end(string, i, self.syms.display, spec, &chars, end_byte)?;
                let under = here.whitespace_under || matches!(chars[i].1, 0x20 | 0x09);
                self.replacement(replacement, Origin { from_display: true, whitespace_under: under, ..here })?;
                i = end.max(i + 1);
                continue;
            }
            let property = self.leaf.vm.property_textget(string, i as i64, self.syms.invisible)?;
            let invisible = self.leaf.invisible_kind(property)?;
            let boundary = self.leaf.vm.next_property_boundary(string, ByteOffset(chars[i].0), ByteOffset(end_byte))
                .map_err(Fault::Error)?.0;
            let run = char_at_byte(&chars, boundary).max(i + 1);
            if invisible != 0 {
                let (end, ellipsis) = self.skip_invisible_string(string, run, &chars, invisible == 2)?;
                if ellipsis { self.ellipsis(Origin { strpos: Some(end.min(chars.len().saturating_sub(1))), ..origin }, false)?; }
                i = end;
                continue;
            }
            for (k, &(_, c)) in chars.iter().enumerate().take(run).skip(i) {
                self.character(c, Origin { strpos: Some(k), whitespace_under: origin.whitespace_under || matches!(c, 0x20 | 0x09), ..origin })?;
            }
            i = run;
        }
        Ok(())
    }

    /// Where STRING's run of PROP, whose value at character AT is VALUE,
    /// ends (GNU's next-single-property-change): its property boundaries
    /// walked by byte, each mapped through CHARS, (byte, char) in order,
    /// without decoding the string again.
    fn string_run_end(&mut self, string: Value, at: usize, prop: Value, value: Value, chars: &[(usize, u32)], end_byte: usize) -> R<usize> {
        let mut byte = chars[at].0;
        loop {
            let next = self.leaf.vm.next_property_boundary(string, ByteOffset(byte), ByteOffset(end_byte)).map_err(Fault::Error)?.0;
            if next >= end_byte { return Ok(chars.len()) }
            let k = char_at_byte(chars, next);
            if self.leaf.vm.property_textget(string, k as i64, prop)? != value { return Ok(k) }
            byte = next;
        }
    }

    fn skip_invisible_string(&mut self, string: Value, mut at: usize, chars: &[(usize, u32)], mut ellipsis: bool) -> R<(usize, bool)> {
        let end_byte = self.leaf.vm.str_view(string).map_or(0, |v| v.bytes().len());
        while at < chars.len() {
            let property = self.leaf.vm.property_textget(string, at as i64, self.syms.invisible)?;
            match self.leaf.invisible_kind(property)? {
                0 => break,
                2 => ellipsis = true,
                _ => {}
            }
            let boundary = self.leaf.vm.next_property_boundary(string, ByteOffset(chars[at].0), ByteOffset(end_byte))
                .map_err(Fault::Error)?.0;
            at = char_at_byte(&chars, boundary).max(at + 1);
        }
        Ok((at, ellipsis))
    }

    /// setup_for_ellipsis: the display table's ellipsis vector, else `...'.
    fn ellipsis(&mut self, origin: Origin, buffer: bool) -> R<()> {
        let codes = match self.look.display_table.map(|t| self.extra(t, 4)).filter(|v| self.leaf.vector_values(*v).is_ok()) {
            Some(vector) => self.leaf.vector_values(vector).unwrap_or_default(),
            None => vec![Value::integer(0x2e).map_err(Fault::Error)?; 3],
        };
        let glyphs: Vec<(u32, Option<Value>)> = codes.iter().map(|&g| (self.glyph_char(g), None)).collect();
        let first = self.slots.len();
        self.vector(&glyphs, origin, buffer, true)?;
        // Each glyph in its code's face (GLYPH_CODE_FACE), by `face-id'.
        for (slot, code) in self.slots[first..].iter_mut().zip(codes) {
            let id = match self.leaf.pair(code) { Some((_, face)) => face.integer_value(), None => code.integer_value().map(|g| g >> 22) };
            if let Some(e) = slot.elem.as_mut() && let Some(id) = id.filter(|&id| id > 0) { e.ellipsis = Some(self.leaf.face_named_by_id(id)); }
        }
        Ok(())
    }

    fn extra(&self, table: Value, slot: usize) -> Value {
        match self.leaf.vm.object_if(table) {
            Some(Object::CharTable(t)) => t.extras.get(slot).copied().unwrap_or(Value::NIL),
            _ => Value::NIL,
        }
    }
    /// GLYPH_CODE_CHAR, or a space for an invalid glyph code.
    fn glyph_char(&self, glyph: Value) -> u32 {
        match self.leaf.pair(glyph) {
            Some((c, face)) if c.integer_value().is_some_and(|c| (0..=0x3fffff).contains(&c)) && face.integer_value().is_some_and(|f| f >= 0) =>
                c.integer_value().unwrap() as u32,
            Some(_) => 0x20,
            None => glyph.integer_value().filter(|&g| g >= 0).map_or(0x20, |g| (g & 0x3fffff) as u32),
        }
    }
    /// A glyph from the display table's slot, if it holds a glyph code.
    fn table_glyph(&self, slot: usize) -> Option<u32> {
        let table = self.look.display_table?;
        let glyph = self.extra(table, slot);
        let valid = match self.leaf.pair(glyph) {
            Some((c, face)) => c.integer_value().is_some_and(|c| (0..=0x3fffff).contains(&c)) && face.integer_value().is_some_and(|f| f >= 0),
            None => glyph.integer_value().is_some_and(|g| g >= 0),
        };
        valid.then(|| self.glyph_char(glyph))
    }

    /// Glyphs from a display vector, each its own element: an ELLIPSIS, or
    /// the glyphs the character at ORIGIN shows as.
    /// Each glyph comes with its own face, if any.
    fn vector(&mut self, glyphs: &[(u32, Option<Value>)], origin: Origin, buffer: bool, ellipsis: bool) -> R<()> {
        for (k, &(g, face)) in glyphs.iter().enumerate() {
            let shape = self.glyph_shape(g)?;
            let method = Method::Vector { last: k + 1 == glyphs.len(), buffer };
            self.emit(Elem { shape, method, charpos: origin.charpos, whitespace: matches!(g, 0x20 | 0x09) || origin.whitespace_under,
                text_area: origin.text_area, strpos: origin.strpos, from_display: origin.from_display, object: origin.string, ellipsis: ellipsis.then_some(Value::NIL),
                face, unnumbered: None });
        }
        Ok(())
    }
    /// GLYPH_CODE_FACE: the face a glyph code names by `face-id', if any.
    fn glyph_face(&mut self, glyph: Value) -> Option<Value> {
        let id = match self.leaf.pair(glyph) { Some((_, face)) => face.integer_value(), None => glyph.integer_value().map(|g| g >> 22) };
        id.filter(|&id| id > 0).map(|id| self.leaf.face_named_by_id(id))
    }
    /// produce_glyphs for a glyph character.
    fn glyph_shape(&mut self, c: u32) -> R<Shape> {
        Ok(match c {
            0x09 => Shape::Tab,
            0x0a => Shape::Glyph { c, width: 0 },
            0x20..0x7f => Shape::Glyph { c, width: 1 },
            _ if romacs_regex_engine::emacs_char::char_byte8_p(c) => Shape::Glyph { c, width: 1 },
            _ => Shape::Glyph { c, width: self.width(c) },
        })
    }

    /// get_next_display_element for character C: a display table entry,
    /// a glyphless method, a `nobreak-char-display' glyph, an escape or
    /// control sequence, or the character itself.
    fn character(&mut self, c: u32, origin: Origin) -> R<()> {
        let buffer = origin.string == Value::NIL;
        let plain = if buffer { Method::Buffer } else { Method::String };
        if let Some(table) = self.look.display_table {
            let entry = self.leaf.vm.char_table_get(table, c as usize).map_err(Fault::Error)?;
            if let Ok(items) = self.leaf.vector_values(entry) {
                let glyphs: Vec<(u32, Option<Value>)> = items.into_iter().map(|g| (self.glyph_char(g), self.glyph_face(g))).collect();
                return self.vector(&glyphs, origin, buffer, false);
            }
        }
        // A joiner continuing an emoji or variation sequence is part of the
        // one glyph GNU's composition makes of it (composite.el,
        // emoji-zwj.el), not glyphless; the host draws the sequence.
        if let Some(joiner) = char::from_u32(c).filter(|&j| romacs_layout_input::cluster::is_composition_joiner(j))
            && self.previous_char(origin.string).is_some_and(|prev| romacs_layout_input::cluster::joins(prev, joiner))
            && self.joins_next(joiner, origin)? {
            self.emit(Elem { shape: Shape::Glyph { c, width: 0 }, method: plain, charpos: origin.charpos, whitespace: origin.whitespace_under,
                text_area: origin.text_area, strpos: origin.strpos, from_display: origin.from_display, object: origin.string, ellipsis: None, face: None, unnumbered: None });
            return Ok(());
        }
        if let Some((width, how)) = self.glyphless(c)? {
            if let Some(how) = how.filter(|_| width > 0) {
                self.emit(Elem { shape: Shape::Glyphless { width, c, how }, method: plain, charpos: origin.charpos,
                    whitespace: origin.whitespace_under, text_area: origin.text_area, strpos: origin.strpos, from_display: origin.from_display, object: origin.string, ellipsis: None, face: None, unnumbered: None });
            }
            return Ok(());
        }
        let ascii_control = (c < 0x20 && c != 0x0a && c != 0x09) || c == 0x7f;
        let nonascii = c >= 0x80;
        let category = romacs_regex_engine::emacs_char::char_general_category(c);
        let nobreak = nonascii && self.look.nobreak != Value::NIL;
        let space = nobreak && category.is_some_and(|k| cat_eq(k, Category::SpaceSeparator));
        let hyphen = nobreak && matches!(c, 0xad | 0x2010 | 0x2011);
        let escape = self.table_glyph(2).unwrap_or(0x5c);
        // get_next_display_element: an escape or control sequence shows in
        // the escape or control glyph's own face when its display table
        // slot gives one, else in `escape-glyph'; a no-break character in
        // `nobreak-space' or `nobreak-hyphen'; merged over the text's.
        let slot_face = |source: &mut Self, slot: usize| {
            let glyph = source.look.display_table.map(|t| source.extra(t, slot)).unwrap_or(Value::NIL);
            source.glyph_face(glyph).unwrap_or(source.syms.escape_glyph)
        };
        let glyphs: Option<(Vec<u32>, Value)> = if ascii_control && self.look.ctl_arrow {
            Some((vec![self.table_glyph(3).unwrap_or(0x5e), c ^ 0x40], slot_face(self, 3)))
        } else if (space || hyphen) && self.look.nobreak == Value::TRUE {
            Some((vec![if !self.look.nobreak_ascii { c } else if space { 0x20 } else { 0x2d }],
                  if space { self.syms.nobreak_space } else { self.syms.nobreak_hyphen }))
        } else if space || hyphen {
            Some((vec![escape, if space { 0x20 } else { 0x2d }], slot_face(self, 2)))
        } else if ascii_control || (nonascii && (romacs_regex_engine::emacs_char::char_byte8_p(c) || !printable(category))) {
            let code = if romacs_regex_engine::emacs_char::char_byte8_p(c) { c & 0xff } else { c };
            let digits = if self.look.raw_hex { format!("x{code:02x}") } else { format!("{code:03o}") };
            Some((std::iter::once(escape).chain(digits.bytes().map(u32::from)).collect(), slot_face(self, 2)))
        } else {
            None
        };
        if let Some((glyphs, face)) = glyphs {
            let glyphs: Vec<(u32, Option<Value>)> = glyphs.into_iter().map(|g| (g, Some(face))).collect();
            return self.vector(&glyphs, origin, buffer, false);
        }
        let shape = match c {
            0x09 => Shape::Tab,
            0x0a => Shape::Glyph { c, width: 0 },
            _ => self.glyph_shape(c)?,
        };
        self.emit(Elem { shape, method: plain, charpos: origin.charpos, whitespace: matches!(c, 0x20 | 0x09) || origin.whitespace_under,
            text_area: origin.text_area, strpos: origin.strpos, from_display: origin.from_display, object: origin.string, ellipsis: None, face: None, unnumbered: None });
        Ok(())
    }

    /// Whether a ZWJ at ORIGIN joins the buffer character after it, as a
    /// sequence needs (a trailing ZWJ joins nothing and stays glyphless);
    /// another joiner, or one in a string, needs nothing after it.
    fn joins_next(&mut self, joiner: char, origin: Origin) -> R<bool> {
        if joiner != '\u{200D}' || origin.string != Value::NIL { return Ok(true); }
        if self.next >= self.view.zv { return Ok(false); }
        Ok(char::from_u32(self.char_at(self.byte)).is_some_and(|next| romacs_layout_input::cluster::joins(joiner, next)))
    }

    /// The character the last element showed, when it is a character of
    /// OBJECT (the buffer's text, or the same string).
    fn previous_char(&self, object: Value) -> Option<char> {
        let elem = self.slots.last()?.elem.as_ref()?;
        let Shape::Glyph { c, .. } = elem.shape else { return None };
        (elem.object == object && elem.ellipsis.is_none()).then(|| char::from_u32(c)).flatten()
    }

    /// lookup_glyphless_char_display and term.c produce_glyphless_glyph:
    /// the columns a glyphless character takes and how it shows, 0 (and
    /// no look) for `zero-width', or None when C displays normally.
    fn glyphless(&mut self, c: u32) -> R<Option<(i64, Option<Glyphless>)>> {
        let table = self.look.glyphless;
        let has_extra = matches!(self.leaf.vm.object_if(table), Some(Object::CharTable(t)) if !t.extras.is_empty());
        if !has_extra { return Ok(None); }
        let mut method = self.leaf.vm.char_table_get(table, c as usize).map_err(Fault::Error)?;
        // (GRAPHICAL . TEXT): a window-system frame's, else a terminal's.
        if let Some((graphical, text)) = self.leaf.pair(method) { method = if self.view.pixels { graphical } else { text }; }
        let s = self.syms;
        Ok(if method == Value::NIL { None }
            else if method == s.zero_width { Some((0, None)) }
            else if method == s.thin_space { Some((1, Some(Glyphless::ThinSpace))) }
            else if method == s.empty_box { Some((self.width(c).clamp(1, 4) + 2, Some(Glyphless::EmptyBox))) }
            else if method == s.hex_code { Some((if c < 0x10000 { 6 } else { 8 }, Some(Glyphless::HexCode))) }
            else if let Some(view) = self.leaf.vm.str_view(method) {
                let width = if view.chars == 1 { 1 } else { view.bytes().iter().take_while(|b| b.is_ascii()).take(6).count() as i64 + 2 };
                Some((width, Some(Glyphless::Acronym(method))))
            } else { None })
    }

    /// handle_display_spec: the replacing specification in SPEC, if any.
    /// IN_STRING stops at the first one, as for a string's own property.
    /// In a string from a display property (NESTED) nothing replaces.
    fn display_spec(&mut self, spec: Value, in_string: bool, nested: bool) -> R<Option<Replacement>> {
        let s = self.syms;
        let mut spec = spec;
        let mut eval = true;
        if let Some((head, tail)) = self.leaf.pair(spec) && head == s.disable_eval {
            eval = false;
            spec = self.leaf.pair(tail).map_or(Value::NIL, |(first, _)| first);
        }
        let simple = |leaf: &Leaf<'_>, head: Value| {
            head != Value::NIL && ![s.image, s.space, s.when, s.slice, s.space_width, s.height, s.raise, s.left_fringe, s.right_fringe, s.min_width]
                .contains(&head) && !leaf.pair(head).is_some_and(|(h, _)| h == s.margin)
        };
        let items: Vec<Value> = match self.leaf.pair(spec) {
            Some((head, _)) if simple(self.leaf, head) => self.leaf.vm.list_values(spec).unwrap_or_default(),
            _ => self.leaf.vector_values(spec).unwrap_or_else(|_| vec![spec]),
        };
        let mut found = None;
        // `(slice X Y WIDTH HEIGHT)' beside an image picks its part (it->slice).
        let mut slice = Value::NIL;
        for item in items {
            if let Some((head, tail)) = self.leaf.pair(item) && head == s.slice { slice = tail; }
            if let Some(r) = self.single_display_spec(item, eval, nested)? {
                if found.is_none() { found = Some(r); }
                if in_string { break; }
            }
        }
        Ok(found.map(|r| match r { Replacement::Image(spec, text_area, _) => Replacement::Image(spec, text_area, slice), r => r }))
    }

    /// handle_single_display_spec. A nested string's spec matters only
    /// for `min-width', which GNU applies before it stops there; FORM in
    /// `(when FORM ...)' cannot change what a nested string shows.
    fn single_display_spec(&mut self, spec: Value, eval: bool, nested: bool) -> R<Option<Replacement>> {
        let s = self.syms;
        let mut spec = spec;
        if let Some((head, rest)) = self.leaf.pair(spec) && head == s.when {
            let Some((form, value)) = self.leaf.pair(rest) else { return Ok(None) };
            let form = if eval { form } else { Value::NIL };
            if form == Value::NIL { return Ok(None); }
            if form != Value::TRUE && !nested {
                return Err(unsupported("of a `(when FORM ...)' display specification"));
            }
            spec = value;
        }
        if let Some((head, rest)) = self.leaf.pair(spec) && head == s.min_width
            && self.leaf.pair(rest).is_some_and(|(w, _)| self.leaf.pair(w).is_some()) {
            return Err(unsupported("of a `(min-width ...)' display specification"));
        }
        if nested { return Ok(None); }
        if let Some((head, rest)) = self.leaf.pair(spec) {
            let has_arg = self.leaf.pair(rest).is_some();
            if has_arg && [s.height, s.space_width, s.raise].contains(&head) { return Ok(None); }
            if head == s.slice { return Ok(None); }
            if has_arg && (head == s.left_fringe || head == s.right_fringe) { return Ok(Some(Replacement::Nothing)); }
        }
        // `(margin LOCATION)': a nil or absent LOCATION is the text area.
        let (value, location) = match self.leaf.pair(spec) {
            Some((head, rest)) if let Some((h, tail)) = self.leaf.pair(head) && h == s.margin => {
                let location = self.leaf.pair(tail).map_or(Value::NIL, |(l, _)| l);
                if location == Value::NIL || location == s.left_margin || location == s.right_margin {
                    (self.leaf.pair(rest).map_or(rest, |(v, _)| v), location)
                } else { (spec, Value::NIL) }
            }
            _ => (spec, Value::NIL),
        };
        let text_area = location == Value::NIL;
        Ok(if self.leaf.vm.str_view(value).is_some() { Some(Replacement::String(value, text_area)) }
            else if self.leaf.pair(value).is_some_and(|(h, _)| h == s.space) { Some(Replacement::Space(value, text_area)) }
            // valid_image_p, on a window-system frame only: elsewhere the text shows.
            else if self.leaf.pair(value).is_some_and(|(h, _)| h == s.image) && self.leaf.vm.window_system_open()
                && self.leaf.image_spec(value)?.is_some() { Some(Replacement::Image(value, text_area, Value::NIL)) }
            else { None })
    }

    fn replacement(&mut self, replacement: Replacement, origin: Origin) -> R<()> {
        match replacement {
            Replacement::Nothing => Ok(()),
            Replacement::String(string, text_area) => {
                if self.leaf.vm.str_view(string).is_some_and(|v| v.chars == 0) { return Ok(()); }
                self.string(string, Origin { string, strpos: None, from_display: true, text_area, ..origin })
            }
            Replacement::Image(spec, text_area, slice) => {
                let Some(parsed) = self.leaf.image_spec(spec)? else { return Ok(()) };
                let window = self.view.window;
                let face = self.leaf.vm.remapped_default_face(window).map_err(Fault::Error)?;
                let Some(image) = self.leaf.image_glyph(&parsed, slice, face.as_ref())? else { return Ok(()) };
                let at = self.images.len() as u32;
                self.images.push((image, spec));
                let shape = Shape::Image { width: image.width, ascent: image.ascent, descent: image.descent, at };
                self.emit(Elem { shape, method: Method::Stretch, charpos: origin.charpos, whitespace: origin.whitespace_under,
                    text_area, strpos: origin.strpos, from_display: origin.from_display, object: origin.string, ellipsis: None, face: None, unnumbered: None });
                Ok(())
            }
            Replacement::Space(spec, text_area) => {
                let origin = Origin { text_area, ..origin };
                let shape = self.stretch_shape(spec, origin)?;
                // As computed before the row's line number is produced.
                let lnum = std::mem::replace(&mut self.view.lnum, 0);
                let unnumbered = if lnum == 0 { Ok(shape) } else { self.stretch_shape(spec, origin) };
                self.view.lnum = lnum;
                let unnumbered = Some(unnumbered?).filter(|&s| s != shape);
                self.emit(Elem { shape, method: Method::Stretch, charpos: origin.charpos, whitespace: origin.whitespace_under,
                    text_area: origin.text_area, strpos: origin.strpos, from_display: origin.from_display, object: origin.string, ellipsis: None, face: None, unnumbered });
                Ok(())
            }
        }
    }

    /// SPEC's stretch in a header or mode line (`Leaf::band_stretches'): a
    /// `(space ...)' as the text's stretch is, but that it reads no buffer
    /// character: a `:relative-width' counts a column, the band's space;
    /// None for another display.
    pub(super) fn band_stretch(&mut self, spec: Value) -> R<Option<romacs_layout_input::StretchWidth>> {
        use romacs_layout_input::StretchWidth;
        let s = self.syms;
        if self.leaf.pair(spec).is_none_or(|(car, _)| car != s.space) { return Ok(None); }
        let plist = self.leaf.pair(spec).map_or(Value::NIL, |(_, p)| p);
        let column = self.view.column;
        if self.plist_get(plist, s.width) == Value::NIL
            && let Some(factor) = self.leaf.number_value(self.plist_get(plist, s.relative_width)).filter(|&f| f > 0.0) {
            return Ok(Some(StretchWidth::Width { width: (factor * column as f64) as i64, zero_ok: false }));
        }
        let origin = Origin { charpos: self.view.begv, string: Value::NIL, strpos: None, from_display: true, text_area: true, whitespace_under: false };
        Ok(Some(match self.stretch_shape(spec, origin) {
            Ok(Shape::Stretch { width, zero_ok }) => StretchWidth::Width { width, zero_ok },
            Ok(Shape::Align { target }) => StretchWidth::AlignTo(target),
            // What the text's stretches refuse (a height of its own, physical
            // units) shows as one column, as GNU's default width is.
            Ok(_) | Err(_) => StretchWidth::Width { width: column, zero_ok: false },
        }))
    }

    /// produce_stretch_glyph's width, before it depends on the x position.
    /// In pixels, a height other than the face's is refused: the host
    /// makes a stretch as tall as its face.
    fn stretch_shape(&mut self, spec: Value, origin: Origin) -> R<Shape> {
        let s = self.syms;
        let plist = self.leaf.pair(spec).map_or(Value::NIL, |(_, p)| p);
        if self.view.pixels {
            for key in [":height", ":relative-height", ":ascent"] {
                let key = self.leaf.intern(key);
                if self.plist_get(plist, key) != Value::NIL { return Err(unsupported("of a space with its own height in a displayed window")); }
            }
        }
        let width = self.plist_get(plist, s.width);
        if width != Value::NIL && let Some(tem) = self.pixels(width, None)? {
            return Ok(Shape::Stretch { width: tem as i64, zero_ok: true });
        }
        let relative = self.plist_get(plist, s.relative_width);
        if let Some(factor) = self.leaf.number_value(relative).filter(|&f| f > 0.0) {
            let c = if origin.string == Value::NIL { self.char_at(self.leaf.buffer_byte(self.view.buffer, origin.charpos)?) }
                else { self.leaf.vm.str_view(origin.string).and_then(|v| v.chars().nth(origin.strpos.unwrap_or(0))).unwrap_or(0x20) };
            // In pixels, FACTOR times the host's glyph of C (Org's table
            // separators); a TAB's would depend on x.
            if self.view.pixels {
                if c == 0x09 || c == 0x0a { return Err(unsupported("of a space of relative width in a displayed window")); }
                return Ok(Shape::RelativeStretch { factor, c });
            }
            return Ok(match self.glyph_shape(c)? {
                Shape::Tab => Shape::TabStretch { factor },
                Shape::Glyph { width, .. } => Shape::Stretch { width: (factor * width as f64) as i64, zero_ok: false },
                _ => Shape::Stretch { width: factor as i64, zero_ok: false },
            });
        }
        let align = self.plist_get(plist, s.align_to);
        if align != Value::NIL {
            let mut anchor = -1;
            self.lnum_added = false;
            if let Some(tem) = self.pixels(align, Some(&mut anchor))? {
                let base = if anchor < 0 { 0 } else { anchor - self.view.left_margin };
                return Ok(Shape::Align { target: tem as i64 + base });
            }
        }
        // FRAME_COLUMN_WIDTH: a column, a terminal's one.
        Ok(Shape::Stretch { width: if self.view.pixels { self.view.column } else { 1 }, zero_ok: false })
    }

    fn plist_get(&self, mut plist: Value, key: Value) -> Value {
        for _ in 0..self.leaf.list_limit() {
            let Some((k, rest)) = self.leaf.pair(plist) else { break };
            let Some((v, rest)) = self.leaf.pair(rest) else { break };
            if k == key { return v; }
            plist = rest;
        }
        Value::NIL
    }

    /// calc_pixel_width_or_height for a width on a terminal frame, in
    /// columns, or with the view's PIXELS on a window-system frame, in
    /// pixels. ANCHOR receives an `:align-to' window element's offset.
    fn pixels(&mut self, prop: Value, mut anchor: Option<&mut i64>) -> R<Option<f64>> {
        let s = self.syms;
        let v = &self.view;
        // A number counts columns; `(N)', pixels.
        let (column, pixel) = if v.pixels { (v.column as f64, 1.0) } else { (1.0, v.column as f64) };
        if prop == Value::NIL { return Ok(Some(0.0)); }
        let mut prop = prop;
        if self.leaf.kind(prop) == super::Kind::Symbol {
            let name = self.leaf.symbol_name(prop)?;
            if ["in", "mm", "cm"].contains(&name.as_str()) { return Err(unsupported("of a display width in physical units")); }
            // On a window-system frame, the face's font's measures.
            if (prop == s.height || prop == s.width) && v.pixels { return Err(unsupported("of a font-relative display width in a displayed window")); }
            if prop == s.height || prop == s.width { return Ok(Some(1.0)); }
            if prop == s.text { return Ok(Some((v.box_width - v.lnum) as f64)); }
            // An alignment counts the line number once, from the left.
            let once = if anchor.is_some() && !self.lnum_added && (prop == s.left || prop == s.center) { v.lnum } else { 0 };
            let elements = [(s.left, v.left_margin + once), (s.right, v.left_margin + v.box_width), (s.center, v.left_margin + (once + v.box_width) / 2),
                (s.left_fringe, v.left_margin), (s.right_fringe, v.left_margin + v.box_width), (s.left_margin, 0),
                (s.right_margin, v.left_margin + v.box_width), (s.scroll_bar, v.left_margin + v.box_width + v.right_margin)];
            if let Some(a) = anchor.as_deref_mut() && *a < 0 && let Some(&(_, at)) = elements.iter().find(|(k, _)| *k == prop) {
                *a = at;
                self.lnum_added |= once != 0;
                return Ok(Some(0.0));
            }
            let widths = [(s.left_fringe, 0), (s.right_fringe, 0), (s.left_margin, v.left_margin), (s.right_margin, v.right_margin), (s.scroll_bar, 0)];
            if !anchor.as_deref().is_some_and(|a| *a < 0) && let Some(&(_, w)) = widths.iter().find(|(k, _)| *k == prop) {
                return Ok(Some(w as f64));
            }
            prop = self.leaf.variable_value(prop)?.unwrap_or(Value::NIL);
        }
        // While no window element anchors an alignment, a number counts
        // from the text area's edge, the line number included once.
        let once = |source: &mut Self, anchor: &Option<&mut i64>| -> f64 {
            if anchor.as_deref().is_some_and(|a| *a < 0) && !source.lnum_added { source.lnum_added = true; source.view.lnum as f64 } else { 0.0 }
        };
        if let Some(n) = self.leaf.number_value(prop) { return Ok(Some(n * column + once(self, &anchor))); }
        let Some((mut car, cdr)) = self.leaf.pair(prop) else { return Ok(None) };
        // An image spec: its own width, margins aside (img->width).
        if car == s.image && self.view.pixels && self.leaf.vm.window_system_open() && let Some(spec) = self.leaf.image_spec(prop)? {
            let env = self.leaf.image_env(Value::NIL)?;
            let (_, entry, _) = self.leaf.image_lookup_display(&spec, &env)?;
            return Ok(Some(f64::from(entry.facts.width)));
        }
        if self.leaf.kind(car) == super::Kind::Symbol {
            if car == s.plus || car == s.minus {
                let mut total = 0.0;
                let mut first = true;
                for item in self.leaf.vm.list_values(cdr).unwrap_or_default() {
                    let Some(px) = self.pixels(item, anchor.as_deref_mut())? else { return Ok(None) };
                    if first { total = if car == s.plus { px } else { -px }; first = false; } else { total += px; }
                }
                return Ok(Some(if car == s.minus { -total } else { total }));
            }
            car = self.leaf.variable_value(car)?.unwrap_or(Value::NIL);
        }
        let Some(pixels) = self.leaf.number_value(car) else { return Ok(None) };
        let units = pixels / pixel;
        // (NUM . UNIT): the line number counted before UNIT's measure.
        let offset = once(self, &anchor);
        if cdr == Value::NIL { return Ok(Some(units + offset)); }
        Ok(self.pixels(cdr, anchor)?.map(|factor| units * factor + offset))
    }

    /// handle_line_prefix's property: the element's string, else the
    /// buffer at its position, else the buffer's `line-prefix' or
    /// `wrap-prefix'.
    fn prefix_value(&mut self, slot: Slot, loaded: bool, wrap: bool) -> R<Value> {
        let key = if wrap { self.syms.wrap_prefix } else { self.syms.line_prefix };
        let elem = slot.elem;
        let at = match (loaded, slot.pre) { (false, Some(pre)) => pre, _ => elem.map_or(self.view.zv, |e| e.charpos) };
        let mut value = Value::NIL;
        let in_string = loaded || slot.pre.is_none();
        let stretch = in_string && elem.is_some_and(|e| e.method == Method::Stretch);
        if !stretch {
            if in_string && let Some(e) = elem && let Some(k) = e.strpos && e.object != Value::NIL {
                value = self.leaf.vm.property_textget(e.object, k as i64, key)?;
            }
            if value == Value::NIL && at < self.view.zv { value = self.property(at, key)?.0; }
        }
        if value == Value::NIL { value = self.leaf.variable_value(key)?.unwrap_or(Value::NIL); }
        Ok(value)
    }
}

/// A unibyte string's byte as get_next_display_element shows it: a
/// non-ASCII byte is a raw-byte character (BYTE8_TO_CHAR).
/// The index in CHARS, (byte, char) in byte order, of the first character
/// at or after BYTE; CHARS' length past its end.
fn char_at_byte(chars: &[(usize, u32)], byte: usize) -> usize {
    chars.partition_point(|&(b, _)| b < byte)
}

fn unibyte_char(b: u8) -> u32 {
    if b.is_ascii() { u32::from(b) } else { 0x3fff00 + u32::from(b) }
}

/// Composed text, in a buffer or a string, has no terminal model here.
fn composition_refusal(property: Value) -> R<()> {
    if property == Value::NIL { Ok(()) } else { Err(unsupported("of composed text (the `composition' property)")) }
}

/// printablep (character.c): not a control, surrogate or unassigned
/// character. Codes beyond Unicode have no category.
fn printable(category: Option<i64>) -> bool {
    category.is_some_and(|k| ![Category::Control, Category::Surrogate, Category::Unassigned].into_iter().any(|c| cat_eq(k, c)))
}

impl Source<'_, '_> {
    /// The index of the accessible text's line holding POS.
    pub(super) fn line_of(&mut self, pos: i64) -> R<i64> {
        let begv = self.leaf.buffer_byte(self.view.buffer, self.view.begv)?;
        let at = self.leaf.buffer_byte(self.view.buffer, pos.clamp(self.view.begv, self.view.zv))?;
        let (from, line) = if self.lines.1 >= 0 && self.lines.0 <= at { self.lines } else { (begv, 0) };
        let line = line + self.text().as_str().as_bytes()[from..at].iter().filter(|&&c| c == b'\n').count() as i64;
        self.lines = (at, line);
        Ok(line)
    }
}

impl Elements for Source<'_, '_> {
    fn line_at(&mut self, index: usize) -> R<i64> {
        let slot = self.slot(index)?;
        // Where the element shows, past text the row start skips.
        let pos = slot.elem.map(|e| e.charpos).or(slot.pre).unwrap_or(self.view.zv);
        self.line_of(pos)
    }
    fn slot(&mut self, index: usize) -> R<Slot> {
        while self.slots.len() <= index && !self.ended { self.generate()?; }
        Ok(self.slots.get(index).copied().unwrap_or(Slot { pre: Some(self.next), elem: None }))
    }
    fn prefix(&mut self, index: usize, loaded: bool, wrap: bool) -> R<Option<(usize, bool)>> {
        let slot = self.slot(index)?;
        let value = self.prefix_value(slot, loaded, wrap)?;
        let charpos = match (loaded, slot.pre) { (false, Some(pre)) => pre, _ => slot.elem.map_or(self.next, |e| e.charpos) };
        let origin = Origin { charpos, string: value, strpos: None, from_display: false, text_area: true,
            whitespace_under: self.buffer_whitespace(charpos)? };
        let saved = (std::mem::take(&mut self.slots), self.pre.take());
        let outcome = if self.leaf.vm.str_view(value).is_some_and(|v| v.chars > 0) {
            self.string(value, origin).map(|()| false)
        } else if self.leaf.pair(value).is_some_and(|(h, _)| h == self.syms.space) {
            self.replacement(Replacement::Space(value, true), Origin { string: Value::NIL, ..origin }).map(|()| wrap)
        } else if self.leaf.pair(value).is_some_and(|(h, _)| h == self.syms.image) {
            Err(unsupported("of an image line prefix"))
        } else {
            Ok(false)
        };
        let produced = std::mem::replace(&mut self.slots, saved.0);
        self.pre = saved.1;
        let align = outcome?;
        let elems: Vec<Elem> = produced.into_iter().filter_map(|s| s.elem)
            .map(|e| if e.method == Method::Buffer { Elem { method: Method::String, ..e } } else { e }).collect();
        if elems.is_empty() { return Ok(None); }
        self.prefixes.push(elems);
        Ok(Some((self.prefixes.len() - 1, align)))
    }
    fn prefix_elem(&self, id: usize, k: usize) -> Option<Elem> {
        self.prefixes.get(id).and_then(|p| p.get(k)).copied()
    }
}

impl Leaf<'_> {
    /// Whether the current buffer's bytes FROM..TO hold a character GNU's
    /// iterator shows as other glyphs than itself (get_next_display_element):
    /// a control or escape sequence, a raw byte, a no-break character, a
    /// glyphless character; or whether a display table maps characters or
    /// sets the escape or control glyph.
    pub(crate) fn text_shows_glyphs(&mut self, from: usize, to: usize) -> R<bool> {
        let look = Look::new(self)?;
        if let Some(table) = look.display_table
            && let Some(Object::CharTable(t)) = self.vm.object_if(table)
            && (t.parent != Value::NIL || !t.all_nil() || t.extras.get(2..4).is_some_and(|s| s.iter().any(|&v| v != Value::NIL))) {
            return Ok(true);
        }
        let glyphless = match self.vm.object_if(look.glyphless) { Some(Object::CharTable(t)) if !t.extras.is_empty() => Some(look.glyphless), _ => None };
        let Ok(b) = self.vm.shown_text(self.vm.slots[self.vm.buffer_slot].value) else { return Ok(false) };
        let text = &b.as_str()[from.min(b.as_str().len())..to.min(b.as_str().len())];
        let mut special = vec![];
        for (at, ch) in text.char_indices() {
            let c = ch as u32;
            if (0x20..0x7f).contains(&c) || c == 0x0a || c == 0x09 { continue; }
            if c < 0x20 || c == 0x7f || b.is_raw_at(from + at) { return Ok(true); }
            special.push(c);
        }
        for c in special {
            let category = romacs_regex_engine::emacs_char::char_general_category(c);
            let nobreak = look.nobreak != Value::NIL
                && (category.is_some_and(|k| cat_eq(k, Category::SpaceSeparator)) || matches!(c, 0xad | 0x2010 | 0x2011));
            if nobreak || (0x80..0xa0).contains(&c) || !printable(category) { return Ok(true); }
            if let Some(table) = glyphless && self.vm.char_table_get(table, c as usize).map_err(Fault::Error)? != Value::NIL { return Ok(true); }
        }
        Ok(false)
    }
}

impl Look {
    fn new(leaf: &mut Leaf<'_>) -> R<Self> {
        let var = |leaf: &mut Leaf<'_>, name: &str| -> R<Value> {
            let symbol = leaf.intern(name);
            Ok(leaf.variable_value(symbol)?.unwrap_or(Value::NIL))
        };
        Ok(Look {
            ctl_arrow: var(leaf, "ctl-arrow")? != Value::NIL,
            display_table: leaf.display_table()?,
            nobreak: var(leaf, "nobreak-char-display")?,
            nobreak_ascii: var(leaf, "nobreak-char-ascii-display")? != Value::NIL,
            raw_hex: var(leaf, "display-raw-bytes-as-hex")? != Value::NIL,
            glyphless: var(leaf, "glyphless-char-display")?,
            unibyte_via_language: var(leaf, "unibyte-display-via-language-environment")? != Value::NIL,
        })
    }
}

impl Syms {
    fn new(leaf: &mut Leaf<'_>) -> Self {
        let mut i = |name: &str| leaf.intern(name);
        Syms {
            display: i("display"), invisible: i("invisible"), composition: i("composition"), before: i("before-string"),
            after: i("after-string"), priority: i("priority"), line_prefix: i("line-prefix"), wrap_prefix: i("wrap-prefix"),
            space: i("space"), when: i("when"), disable_eval: i("disable-eval"), margin: i("margin"), left_margin: i("left-margin"),
            right_margin: i("right-margin"), left_fringe: i("left-fringe"), right_fringe: i("right-fringe"), image: i("image"),
            slice: i("slice"), space_width: i("space-width"), height: i("height"), raise: i("raise"), min_width: i("min-width"),
            width: i(":width"), relative_width: i(":relative-width"), align_to: i(":align-to"), text: i("text"), left: i("left"),
            right: i("right"), center: i("center"), scroll_bar: i("scroll-bar"), plus: i("+"), minus: i("-"),
            zero_width: i("zero-width"), thin_space: i("thin-space"), empty_box: i("empty-box"), hex_code: i("hex-code"), escape_glyph: i("escape-glyph"), nobreak_space: i("nobreak-space"), nobreak_hyphen: i("nobreak-hyphen"),
        }
    }
}

impl Leaf<'_> {
    /// The byte of character position POS (1-based) in text buffer BUFFER.
    pub(super) fn buffer_byte(&self, buffer: Value, pos: i64) -> R<usize> {
        let Ok(b) = self.vm.shown_text(buffer) else { return Err(fault("text buffer required")) };
        Ok(b.byte_of_char(CharOffset((pos - 1).max(0) as usize)).map_err(Fault::Error)?.0)
    }
    pub(crate) fn number_value(&self, v: Value) -> Option<f64> {
        v.integer_value().map(|n| n as f64).or_else(|| self.float_value(v).ok())
    }
}

impl super::display_motion::Lines for Source<'_, '_> {
    fn reseat(&mut self, start: i64) -> R<()> { self.reset(start) }
    fn line_start_before(&mut self, pos: i64) -> R<i64> { self.visible_line_start(pos) }
    fn begv(&self) -> i64 { self.view.begv }
}
