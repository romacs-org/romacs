//! GNU's fringes (fringe.c) as a window's snapshot publishes them: the
//! bitmap each logical indicator shows on each side, from the buffer's
//! `fringe-indicator-alist', `indicate-empty-lines' and
//! `indicate-buffer-boundaries', and the bitmaps those symbols name.
//! Bitmap symbols carry GNU's `fringe' property, their number; a bitmap
//! `define-fringe-bitmap' defined keeps its definition beside it.
use crate::{display::FaceTable, *};
use romacs_layout_input::{BitmapAlign, FaceHandle, FringeBitmap, Indicators};

/// The property holding a defined bitmap's (BITS WIDTH ALIGN PERIOD),
/// BITS a list of most-significant-aligned rows.
pub(crate) const DEFINITION: &str = "romacs--fringe-bitmap";
/// The property holding the face `set-fringe-bitmap-face' gave a bitmap.
pub(crate) const FACE: &str = "romacs--fringe-face";
/// GNU's standard bitmaps, numbers 1 to 24 in fringe.el's order
/// (fringe.c standard_bitmaps): eight pixels wide, the leftmost in bit 7.
const STANDARD: [(&[u16], BitmapAlign, u8); 24] = {
    use BitmapAlign::{Bottom, Center, Top};
    [
        (&[0x3c, 0x7e, 0xc3, 0xc3, 0x0c, 0x18, 0x18, 0x00, 0x18, 0x18], Center, 0), // question-mark
        (&[0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x18], Center, 0), // exclamation-mark
        (&[0x18, 0x30, 0x60, 0xfc, 0xfc, 0x60, 0x30, 0x18], Center, 0), // left-arrow
        (&[0x18, 0x0c, 0x06, 0x3f, 0x3f, 0x06, 0x0c, 0x18], Center, 0), // right-arrow
        (&[0x18, 0x3c, 0x7e, 0xff, 0x18, 0x18, 0x18, 0x18], Top, 0),    // up-arrow
        (&[0x18, 0x18, 0x18, 0x18, 0xff, 0x7e, 0x3c, 0x18], Bottom, 0), // down-arrow
        (&[0x3c, 0x7c, 0xc0, 0xe4, 0xfc, 0x7c, 0x3c, 0x7c], Center, 0), // left-curly-arrow
        (&[0x3c, 0x3e, 0x03, 0x27, 0x3f, 0x3e, 0x3c, 0x3e], Center, 0), // right-curly-arrow
        (&[0x3c, 0x7e, 0xff, 0xff, 0xff, 0xff, 0x7e, 0x3c], Center, 0), // large-circle
        (&[0x03, 0x0f, 0x1f, 0x3f, 0x3f, 0x1f, 0x0f, 0x03], Center, 0), // left-triangle
        (&[0xc0, 0xf0, 0xf8, 0xfc, 0xfc, 0xf8, 0xf0, 0xc0], Center, 0), // right-triangle
        (&[0xfc, 0xfc, 0xc0, 0xc0, 0xc0, 0xc0, 0xc0, 0x00], Top, 0),    // top-left-angle
        (&[0x3f, 0x3f, 0x03, 0x03, 0x03, 0x03, 0x03, 0x00], Top, 0),    // top-right-angle
        (&[0x00, 0xc0, 0xc0, 0xc0, 0xc0, 0xc0, 0xfc, 0xfc], Bottom, 0), // bottom-left-angle
        (&[0x00, 0x03, 0x03, 0x03, 0x03, 0x03, 0x3f, 0x3f], Bottom, 0), // bottom-right-angle
        (&[0xfc, 0xfc, 0xc0, 0xc0, 0xc0, 0xc0, 0xc0, 0xc0, 0xfc, 0xfc], Center, 0), // left-bracket
        (&[0x3f, 0x3f, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x3f, 0x3f], Center, 0), // right-bracket
        (&[0xfe; 13], Center, 0), // filled-rectangle
        (&[0xfe, 0x82, 0x82, 0x82, 0x82, 0x82, 0x82, 0x82, 0x82, 0x82, 0x82, 0x82, 0xfe], Center, 0), // hollow-rectangle
        (&[0x7e, 0x7e, 0x7e, 0x7e, 0x7e, 0x7e], Center, 0), // filled-square
        (&[0x7e, 0x42, 0x42, 0x42, 0x42, 0x7e], Center, 0), // hollow-square
        (&[0xc0; 13], Center, 0), // vertical-bar
        (&[0xfe, 0xfe], Bottom, 0), // horizontal-bar
        (&[0x00, 0x3c, 0x00], Top, 3), // empty-line, one period of GNU's 72 rows
    ]
};
/// The numbers above the standard bitmaps `define-fringe-bitmap' gives.
pub(crate) const FIRST_USER_BITMAP: i64 = STANDARD.len() as i64 + 1;

/// Which fringe a buffer's indicator setting puts an indicator in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side { Left = 0, Right = 1 }

impl Vm {
    /// WINDOW's margins and fringes as its snapshot publishes them, faces
    /// from TABLE: the face its fringes' background is painted in, the
    /// `vertical-border' face of a window with another left of it, and the
    /// bitmaps its buffer's indicators resolve to (in the `fringe' face
    /// unless their own), unless only the widths, which lay rows out, are
    /// WANTED.  Where the frame draws pane chrome (`romacs-pane-chrome',
    /// rlisp/romacs-chrome.el) the fringes are part of the pane: their
    /// background is the window's default face's, whatever the theme gives
    /// `fringe' (VM2-PANE-CHROME); else the `fringe' face's, as GNU's.
    pub(crate) fn window_chrome(&mut self, window: Value, table: &FaceTable, indicators: bool) -> Result<romacs_layout_input::WindowChrome> {
        let [left_fringe, right_fringe] = self.window_fringe_widths(window)?;
        let column = self.window_state.column;
        let w = self.window(window)?;
        let (left_margin, right_margin, outside_margins) = (w.left_margin * column, w.right_margin * column, w.fringes_outside_margins);
        let (x, frame, buffer, mini) = (w.x, w.frame, w.buffer, w.mini);
        let root_x = self.frame(frame).ok().and_then(|f| self.window(f.root).ok()).map_or(0, |r| r.x);
        let fringe_face = self.named(&table, "fringe").unwrap_or_default();
        let host = self.host_frame_peek();
        let pane = self.frame_parameter_of(host, "romacs-pane-chrome")?.is_some_and(|v| v != Value::NIL);
        let fringe_background = if pane { self.named(&table, "default").unwrap_or_default() } else { fringe_face };
        let [right_divider, bottom_divider] = self.window_dividers(window)?;
        // A frame with right dividers draws them instead (gui_draw_vertical_border).
        let dividers = self.frame(frame).map_or(0, |f| f.dividers[0]);
        let border = if x > root_x && dividers == 0 { self.named(&table, "vertical-border") } else { None };
        let indicators = if left_fringe + right_fringe == 0 || !indicators {
            Indicators::default()
        } else {
            self.fringe_indicators(buffer, mini, [left_fringe, right_fringe], fringe_face, table)?
        };
        // Only a side without a fringe shows a truncation or continuation glyph.
        let line_glyphs = if left_fringe == 0 || right_fringe == 0 { self.line_glyphs(buffer)? } else { [None; 2] };
        Ok(romacs_layout_input::WindowChrome { left_margin, right_margin, left_fringe, right_fringe, outside_margins,
                                               fringe_face: fringe_background, border, indicators, right_divider, bottom_divider, line_glyphs })
    }

    /// The truncation and continuation glyphs (slots 0 and 1) of BUFFER's
    /// display table, its `buffer-display-table' else `standard-display-table'
    /// (window_display_table), that are characters without a face.
    pub(crate) fn line_glyphs(&mut self, buffer: Value) -> Result<[Option<char>; 2]> {
        for name in ["buffer-display-table", "standard-display-table"] {
            let table = self.buffer_variable(buffer, name)?;
            if let Some(slots) = self.display_table_line_slots(table) {
                return Ok(slots.map(|glyph| self.plain_glyph(glyph)));
            }
        }
        Ok([None; 2])
    }

    /// TABLE's truncation and continuation slots when it is a display table.
    pub(crate) fn display_table_line_slots(&mut self, table: Value) -> Option<[Value; 2]> {
        let purpose = self.display_symbol("display-table");
        let Some(crate::data::Object::CharTable(t)) = self.object_if(table) else { return None };
        (t.purpose == purpose && t.extras.len() >= 2).then(|| [t.extras[0], t.extras[1]])
    }

    /// GLYPH, a glyph code, as the character it shows when it has no face
    /// (GLYPH_CODE_CHAR, GLYPH_CODE_FACE 0).
    pub(crate) fn plain_glyph(&self, glyph: Value) -> Option<char> {
        glyph.integer_value().filter(|&code| code >= 0 && code >> 22 == 0).and_then(|code| char::from_u32(code as u32))
    }

    /// update_window_fringes's choices for BUFFER's rows: each logical
    /// indicator's bitmap on the side its setting puts it, and none where
    /// FRINGES has no width.
    fn fringe_indicators(&mut self, buffer: Value, mini: bool, fringes: [u32; 2], face: FaceHandle, table: &FaceTable) -> Result<Indicators> {
        let mut out = Indicators::default();
        for (name, slot) in [("truncation", &mut out.truncation), ("continuation", &mut out.continuation)] {
            for side in [Side::Left, Side::Right] {
                slot[side as usize] = self.logical_bitmap(buffer, name, side, face, table)?;
            }
        }
        // `indicate-buffer-boundaries': `left' or `right' puts every
        // boundary and arrow there; an alist places each by `t', `top',
        // `bottom', `up' and `down'; anything else, the boundaries left.
        let setting = if mini { Value::NIL } else { self.buffer_value(buffer, "indicate-buffer-boundaries")? };
        let (left, right) = (self.intern("left"), self.intern("right"));
        let side = |v: Value| if v == left { Some(Side::Left) } else if v == right { Some(Side::Right) } else { None };
        let [mut top, mut bottom, mut up, mut down] = [None; 4];
        if setting == left || setting == right {
            [top, bottom, up, down] = [side(setting); 4];
        } else if let Ok((first, _)) = self.cons_pair(setting) && self.cons_pair(first).is_ok() {
            let entry = |vm: &mut Self, key: &str| { let key = vm.intern(key); vm.assq_cdr(key, setting) };
            if let Some(all) = entry(self, "t")? { [top, bottom, up, down] = [side(all); 4]; }
            for (key, place) in [("top", &mut top), ("bottom", &mut bottom), ("up", &mut up), ("down", &mut down)] {
                if let Some(v) = entry(self, key)? { *place = side(v); }
            }
        } else if setting != Value::NIL {
            (top, bottom) = (Some(Side::Left), Some(Side::Left));
        }
        for (name, place, slot) in [("top", top, &mut out.top), ("bottom", bottom, &mut out.bottom),
                                    ("up", up, &mut out.up), ("down", down, &mut out.down)] {
            if let Some(s) = place { slot[s as usize] = self.logical_bitmap(buffer, name, s, face, table)?; }
        }
        if let Some(s) = top.filter(|&s| Some(s) == bottom) {
            out.top_bottom[s as usize] = self.logical_bitmap(buffer, "top-bottom", s, face, table)?;
        }
        // `indicate-empty-lines': the left fringe, or the right one when
        // it says `right' or the left has no width.
        let empty = self.buffer_value(buffer, "indicate-empty-lines")?;
        if empty != Value::NIL {
            let s = if empty == right || fringes[0] == 0 { Side::Right } else { Side::Left };
            out.empty_line[s as usize] = self.logical_bitmap(buffer, "empty-line", s, face, table)?;
        }
        for slot in [&mut out.truncation, &mut out.continuation, &mut out.top, &mut out.bottom,
                     &mut out.top_bottom, &mut out.up, &mut out.down, &mut out.empty_line] {
            for (bitmap, &width) in slot.iter_mut().zip(&fringes) {
                if width == 0 { *bitmap = None; }
            }
        }
        Ok(out)
    }

    /// get_logical_fringe_bitmap (fringe.c:738) for the whole-row bitmap:
    /// INDICATOR's entry in BUFFER's `fringe-indicator-alist', else in its
    /// default value; an entry of nil shows none, t defers to the default.
    fn logical_bitmap(&mut self, buffer: Value, indicator: &str, side: Side, face: FaceHandle, table: &FaceTable) -> Result<Option<FringeBitmap>> {
        let key = self.intern(indicator);
        let slot = self.slot("fringe-indicator-alist");
        let local = self.value_in_buffer(slot, buffer)?;
        let default = self.default_value(slot)?;
        for alist in [local, default] {
            if alist == Value::UNBOUND { continue; }
            let Some(entry) = self.assq_cdr(key, alist)? else { continue };
            if entry == Value::NIL { return Ok(None); }
            let symbol = match self.cons_pair(entry) {
                Ok(_) => self.list_nth(entry, side as usize),
                Err(_) => Some(entry),
            };
            if let Some(symbol) = symbol.filter(|&s| s != Value::TRUE) {
                return self.fringe_bitmap(symbol, face, table);
            }
        }
        Ok(None)
    }

    /// The bitmap SYMBOL names (its `fringe' property), standard or
    /// defined, in the face `set-fringe-bitmap-face' gave it or FACE.
    pub(crate) fn fringe_bitmap(&mut self, symbol: Value, face: FaceHandle, table: &FaceTable) -> Result<Option<FringeBitmap>> {
        let Some(number) = self.property(symbol, "fringe")?.integer_value() else { return Ok(None) };
        let name = self.property(symbol, FACE)?;
        let face = if name == Value::NIL { face } else { self.display(name).ok().and_then(|n| self.named(&table, &n)).unwrap_or(face) };
        let number16 = u16::try_from(number).map_err(|e| e.to_string())?;
        let definition = self.property(symbol, DEFINITION)?;
        if definition == Value::NIL && (1..FIRST_USER_BITMAP).contains(&number) {
            let (bits, align, period) = STANDARD[number as usize - 1];
            return Ok(Some(FringeBitmap { number: number16, bits: bits.iter().map(|b| b << 8).collect(), width: 8, align, period, face }));
        }
        if definition == Value::NIL { return Ok(None); }
        let item = |vm: &Self, i| vm.list_nth(definition, i).unwrap_or(Value::NIL);
        let mut bits = vec![];
        let mut rows = item(self, 0);
        while let Ok((row, rest)) = self.cons_pair(rows) {
            bits.push(row.integer_value().unwrap_or(0) as u16);
            rows = rest;
        }
        let width = item(self, 1).integer_value().unwrap_or(8).clamp(1, 16) as u8;
        let align = match self.display(item(self, 2))?.as_str() { "top" => BitmapAlign::Top, "bottom" => BitmapAlign::Bottom, _ => BitmapAlign::Center };
        let period = if item(self, 3) != Value::NIL { bits.len().min(255) as u8 } else { 0 };
        Ok(Some(FringeBitmap { number: number16, bits, width, align, period, face }))
    }

    /// BUFFER's value of the variable NAME.
    fn buffer_value(&mut self, buffer: Value, name: &str) -> Result<Value> {
        let slot = self.slot(name);
        let value = self.value_in_buffer(slot, buffer)?;
        Ok(if value == Value::UNBOUND { Value::NIL } else { value })
    }

    /// The cdr of KEY's first entry in ALIST, None when it has none.
    fn assq_cdr(&self, key: Value, mut alist: Value) -> Result<Option<Value>> {
        while let Ok((entry, rest)) = self.cons_pair(alist) {
            if let Ok((k, v)) = self.cons_pair(entry) && k == key { return Ok(Some(v)); }
            alist = rest;
        }
        Ok(None)
    }

    /// SYMBOL's property NAME, nil when it has none or is no symbol.
    fn property(&mut self, symbol: Value, name: &str) -> Result<Value> {
        let key = self.intern(name);
        Ok(self.symbol_property(symbol, key).unwrap_or(Value::NIL))
    }

    /// The Nth element of LIST, None past its end.
    fn list_nth(&self, mut list: Value, n: usize) -> Option<Value> {
        for _ in 0..n { list = self.cons_pair(list).ok()?.1; }
        self.cons_pair(list).ok().map(|(car, _)| car)
    }
}
