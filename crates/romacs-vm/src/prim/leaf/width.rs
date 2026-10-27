//! character.c's column widths: CHARACTER_WIDTH (buffer.h) over
//! `char-width-table', `tab-width' and `ctl-arrow', and char_width's
//! display-table glyph sum.
use super::*;

/// disptab.h DISP_TABLE_EXTRA_SLOTS (BOX_DOUBLE_UP_LEFT + 1).
const DISP_TABLE_EXTRA_SLOTS: usize = 18;

impl Leaf<'_> {
    /// buffer_display_table (indent.c): the buffer's `buffer-display-table',
    /// else `standard-display-table', when it is a display table.
    fn display_table(&mut self) -> std::result::Result<Option<Value>, Fault> {
        for name in ["buffer-display-table", "standard-display-table"] {
            let symbol = self.intern(name);
            let Some(table) = self.variable_value(symbol)? else { continue };
            let purpose = self.intern("display-table");
            if let Some(crate::data::Object::CharTable(t)) = self.vm.object_if(table)
                && t.purpose == purpose && t.extras.len() == DISP_TABLE_EXTRA_SLOTS {
                return Ok(Some(table));
            }
        }
        Ok(None)
    }
    /// CHARACTER_WIDTH.
    fn character_width(&mut self, c: u32) -> std::result::Result<i64, Fault> {
        Ok(match c {
            0x20..0x7f => 1,
            0x80.. => {
                let table = self.intern("char-width-table");
                let table = self.variable_value(table)?.unwrap_or(Value::NIL);
                let width = self.vm.char_table_get(table, c as usize).map_err(Fault::Error)?;
                width.integer_value().filter(|w| (0..=1000).contains(w)).unwrap_or(1000)
            }
            0x09 => self.tab_width()? as i64,
            0x0a => 0,
            _ => {
                let ctl_arrow = self.intern("ctl-arrow");
                if self.variable_value(ctl_arrow)?.is_some_and(|v| v != Value::NIL) { 2 } else { 4 }
            }
        })
    }
    /// char_width: a display-table vector entry is the sum of its glyphs'
    /// widths (a glyph code's character, or a character).
    fn char_width_in(&mut self, c: u32, display: Option<Value>) -> std::result::Result<i64, Fault> {
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
                    if let Some(c) = c { width += self.character_width(c as u32)?; }
                }
                return Ok(width);
            }
        }
        self.character_width(c)
    }
    pub(crate) fn char_width(&mut self, c: u32) -> super::super::Out {
        let display = self.display_table()?;
        Value::integer(self.char_width_in(c, display)?).map_err(Fault::Error)
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
        let view = self.vm.str_view(string).expect("string-width takes a string");
        let chars: Vec<u32> = if view.multibyte { view.chars().skip(from).take(to - from).collect() }
            else { view.bytes[from..to].iter().map(|&b| u32::from(b)).collect() };
        let display = self.display_table()?;
        let mut width = 0i64;
        for c in chars { width += self.char_width_in(c, display)?; }
        Value::integer(width).map_err(Fault::Error)
    }
}
