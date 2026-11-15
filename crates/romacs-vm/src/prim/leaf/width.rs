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
        let view = self.vm.str_view_from(string, from).expect("string-width takes a string");
        let multibyte = view.multibyte;
        let chars: Vec<u32> = if multibyte { view.chars().take(to - from).collect() }
            else { view.bytes[..to - from].iter().map(|&b| u32::from(b)).collect() };
        let display = self.display_table()?;
        let rule = self.vm.width_rule().map_err(Fault::Error)?;
        let emoji = if multibyte { self.emoji_runs(&chars)? } else { vec![] };
        let (mut width, mut i, mut runs) = (0i64, 0, emoji.into_iter().peekable());
        while i < chars.len() {
            if let Some(&(start, end)) = runs.peek() && start == i {
                // compose-gstring-and-emoji-for-terminal: the composition is 2 columns.
                width += 2;
                i = end;
                runs.next();
                continue;
            }
            width += self.char_width_in(chars[i], display, &rule)?;
            i += 1;
        }
        Value::integer(width).map_err(Fault::Error)
    }
    /// The runs of CHARS lisp_string_width counts as one composition
    /// (find_automatic_composition): those a rule of `composition-function-table'
    /// whose function is `compose-gstring-and-emoji' matches, a rule for the
    /// character PREV-CHARS after the run's start, its regexp anchored at the
    /// start (nil: that character alone). Emoji, ZWJ, flag and VS-16
    /// sequences (composite.el, emoji-zwj.el); other compositions keep their
    /// characters' widths, as compose-gstring-for-terminal gives them.
    fn emoji_runs(&mut self, chars: &[u32]) -> std::result::Result<Vec<(usize, usize)>, Fault> {
        let table_symbol = self.intern("composition-function-table");
        let Some(table) = self.variable_value(table_symbol)? else { return Ok(vec![]) };
        if !matches!(self.vm.object_if(table), Some(crate::data::Object::CharTable(_))) { return Ok(vec![]); }
        let emoji = self.intern("compose-gstring-and-emoji");
        let (mut runs, mut i) = (vec![], 0);
        'start: while i < chars.len() {
            for k in 0..=3usize.min(chars.len() - 1 - i) {
                let c = chars[i + k];
                if c < 0x80 { continue; }
                let mut rules = self.vm.char_table_get(table, c as usize).map_err(Fault::Error)?;
                while let Some((item, rest)) = self.pair(rules) {
                    rules = rest;
                    let Ok(rule) = self.vector_values(item) else { continue };
                    if rule.len() != 3 || rule[2] != emoji || rule[1].integer_value() != Some(k as i64) { continue; }
                    let end = if rule[0] == Value::NIL {
                        (k == 0).then_some(i + 1)
                    } else {
                        let pattern = format!("\\`{}", self.string_text(rule[0])?);
                        let text: String = chars[i..].iter().map_while(|&c| char::from_u32(c)).collect();
                        let (pattern, text) = (self.string(&pattern), self.string(&text));
                        self.vm.string_match_groups(pattern, text, crate::regexp::Translation::None, None)?
                            .and_then(|groups| groups.first().copied().flatten())
                            .and_then(|(from, to)| (from == 0 && to > k).then_some(i + to))
                    };
                    if let Some(end) = end {
                        runs.push((i, end));
                        i = end;
                        continue 'start;
                    }
                }
            }
            i += 1;
        }
        Ok(runs)
    }
}
