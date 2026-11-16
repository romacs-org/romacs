//! character.c's column widths: CHARACTER_WIDTH (buffer.h) over
//! `char-width-table', `tab-width' and `ctl-arrow', and char_width's
//! display-table glyph sum.
use super::*;

/// disptab.h DISP_TABLE_EXTRA_SLOTS (BOX_DOUBLE_UP_LEFT + 1).
const DISP_TABLE_EXTRA_SLOTS: usize = 18;

impl Leaf<'_> {
    /// buffer_display_table (indent.c): the buffer's `buffer-display-table',
    /// else `standard-display-table', when it is a display table.
    pub(super) fn display_table(&mut self) -> std::result::Result<Option<Value>, Fault> {
        for name in ["buffer-display-table", "standard-display-table"] {
            let symbol = self.intern(name);
            let Some(table) = self.variable_value(symbol)? else { continue };
            let purpose = self.intern("display-table");
            if let Some(crate::data::Object::CharTable(t)) = self.vm.object_if(table)
                && t.purpose() == purpose && t.extras().len() == DISP_TABLE_EXTRA_SLOTS {
                return Ok(Some(table));
            }
        }
        Ok(None)
    }
    /// CHARACTER_WIDTH of one character, by the current buffer's rule
    /// (width_rule.rs); a scan reads the rule once instead.
    pub(super) fn character_width(&mut self, c: u32) -> std::result::Result<i64, Fault> {
        let rule = self.vm.width_rule().map_err(Fault::Error)?;
        Ok(rule.width(c) as i64)
    }
    /// char_width: a display-table vector entry is the sum of its glyphs'
    /// widths (a glyph code's character, or a character).
    fn char_width_in(&mut self, c: u32, display: Option<Value>, rule: &crate::width_rule::WidthRule) -> std::result::Result<i64, Fault> {
        if let Some(table) = display {
            let entry = self.vm.char_table_get(table, c as usize).map_err(Fault::Error)?;
            if let Ok(glyphs) = self.vector_values(entry) {
                let mut width = 0;
                for glyph in glyphs {
                    let c = match self.pair(glyph) {
                        Some((c, face)) if c.integer_value().is_some_and(|c| (0..=0x3fffff).contains(&c))
                            && face.integer_value().is_some_and(|f| f >= 0) => c.integer_value(),
                        Some(_) => None,
                        None => glyph.integer_value().filter(|&g| g >= 0).map(|g| g & 0x3fffff),
                    };
                    if let Some(c) = c { width += rule.width(c as u32) as i64; }
                }
                return Ok(width);
            }
        }
        Ok(rule.width(c) as i64)
    }
    pub(crate) fn char_width(&mut self, c: u32) -> super::super::Out {
        let display = self.display_table()?;
        let rule = self.vm.width_rule().map_err(Fault::Error)?;
        Value::integer(self.char_width_in(c, display, &rule)?).map_err(Fault::Error)
    }
    /// lisp_string_width over characters FROM..TO. A unibyte string's
    /// characters are its bytes, as GNU reads them.
    pub(crate) fn string_width(&mut self, string: Value, from: usize, to: usize) -> super::super::Out {
        if from < to {
            let composition = self.intern("composition");
            let end = Value::integer(to as i64).map_err(Fault::Error)?;
            let start = Value::integer(from as i64).map_err(Fault::Error)?;
            if self.vm.property_textget(string, from as i64, composition)? != Value::NIL
                || self.vm.next_single_property_change_values(start, composition, string, end)? != end {
                return Err(Fault::Error("string-width of composed text (the `composition' property) is not implemented".into()));
            }
        }
        let whole = self.vm.str_view(string).expect("string-width takes a string");
        // GNU's `multibyte': fewer characters than bytes, a string holding a
        // character beyond ASCII, the only kind find_automatic_composition reads.
        let multibyte = whole.multibyte && whole.chars < whole.bytes.len();
        let display = self.display_table()?;
        let rule = self.vm.width_rule().map_err(Fault::Error)?;
        let compositions = if multibyte && from < to { self.composition_table()? } else { None };
        // FROM..TO and the PREV-CHARS a rule looks ahead (3), decoded once.
        let mut text = self.characters(string, from, to - from + 3);
        let (mut width, mut i) = (0i64, 0);
        while from + i < to {
            if let Some(table) = compositions
                && let Some((end, emoji)) = self.composition_at(table, string, &text, from, i)? {
                // compose-gstring-and-emoji-for-terminal: two columns; another
                // function's run keeps its characters' widths, as
                // compose-gstring-for-terminal leaves them.
                if !emoji && end > text.len() {
                    let more = self.characters(string, from + text.len(), end - text.len());
                    text.extend(more);
                }
                width += if emoji { 2 } else {
                    let mut sum = 0;
                    for &(c, _) in &text[i..end] { sum += self.char_width_in(c, display, &rule)?; }
                    sum
                };
                i = end;
                continue;
            }
            width += self.char_width_in(text[i].0, display, &rule)?;
            i += 1;
        }
        Value::integer(width).map_err(Fault::Error)
    }
    /// Up to COUNT characters of STRING from character FROM, each with the
    /// byte it starts at; a unibyte string's characters are its bytes.
    fn characters(&self, string: Value, from: usize, count: usize) -> Vec<(u32, usize)> {
        let total = self.vm.str_view(string).expect("string-width takes a string").bytes.len();
        let view = self.vm.str_view_from(string, from).expect("string-width takes a string");
        let mut at = total - view.bytes.len();
        let mut out = Vec::with_capacity(count.min(view.chars));
        let mut bytes = view.bytes;
        while out.len() < count && !bytes.is_empty() {
            let (code, length) = if view.multibyte { crate::lstring::string_char(bytes) } else { (u32::from(bytes[0]), 1) };
            out.push((code, at));
            at += length;
            bytes = &bytes[length..];
        }
        out
    }
    /// `composition-function-table', when it is a char-table.
    fn composition_table(&mut self) -> std::result::Result<Option<Value>, Fault> {
        let symbol = self.intern("composition-function-table");
        let Some(table) = self.variable_value(symbol)? else { return Ok(None) };
        Ok(matches!(self.vm.object_if(table), Some(crate::data::Object::CharTable(_))).then_some(table))
    }
    /// find_automatic_composition at character FROM+START of STRING, whose
    /// characters from FROM are TEXT (a composition may end past TEXT and the
    /// width's TO): the first rule of TABLE for the character PREV-CHARS after
    /// START, its PREV-CHARS that, whose regexp matches at START (nil: that
    /// character alone) with a positive length. Its end, from FROM, and
    /// whether its function is `compose-gstring-and-emoji'.
    fn composition_at(&mut self, table: Value, string: Value, text: &[(u32, usize)], from: usize, start: usize)
        -> std::result::Result<Option<(usize, bool)>, Fault> {
        let emoji = self.intern("compose-gstring-and-emoji");
        for k in 0..=3usize.min(text.len() - 1 - start) {
            let mut rules = self.vm.char_table_get(table, text[start + k].0 as usize).map_err(Fault::Error)?;
            while let Some((item, rest)) = self.pair(rules) {
                rules = rest;
                let Ok(rule) = self.vector_values(item) else { continue };
                if rule.len() != 3 || rule[1].integer_value() != Some(k as i64) { continue; }
                let end = if rule[0] == Value::NIL { Some(start + 1) } else if self.string_view(rule[0]).is_ok() {
                    self.vm.string_looking_at(string, rule[0], from + start, text[start].1)?
                        .map(|end| end - from).filter(|&end| end > start)
                } else { None };
                if let Some(end) = end { return Ok(Some((end, rule[2] == emoji))); }
            }
        }
        Ok(None)
    }
}
