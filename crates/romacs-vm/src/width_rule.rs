//! CHARACTER_WIDTH (buffer.h, character.h): the one rule Romacs counts a
//! character's columns by, wherever it counts them: `char-width',
//! `string-width', `current-column', `format''s precision, the mode line,
//! the status bar, the mini-window and the display's glyphs.  A non-ASCII
//! character takes its `char-width-table' entry (international/
//! characters.el builds the table), a control character two columns under
//! `ctl-arrow' and four without, a tab `tab-width' columns or the next tab
//! stop.  The variables are read once per scan into this value, which
//! needs no VM, so a character costs a table lookup at most.
use crate::char_table::CharTableView;
use crate::data::Object;
use crate::{Result, Value, Vm};

#[derive(Clone)]
pub(crate) struct WidthRule {
    /// SANE_TAB_WIDTH.
    pub(crate) tab: usize,
    /// `char-width-table', then its parents.
    tables: Vec<CharTableView>,
    ctl_arrow: bool,
}

impl WidthRule {
    /// C's columns, a tab `tab-width' of them (char_width's count).
    pub(crate) fn width(&self, c: u32) -> usize {
        match c {
            0x20..=0x7e => 1,
            0x80.. => {
                let width = self.tables.iter().map(|t| t.get(c as usize)).find(|&w| w != Value::NIL);
                width.and_then(|w| w.integer_value()).map_or(1, |w| if (0..=1000).contains(&w) { w as usize } else { 1000 })
            }
            0x09 => self.tab,
            0x0a => 0,
            _ => if self.ctl_arrow { 2 } else { 4 },
        }
    }
    /// C's columns when it starts at COLUMN: a tab reaches the next stop
    /// (scan_for_column's count).
    pub(crate) fn columns(&self, c: u32, column: usize) -> usize {
        if c == 0x09 { self.tab - column % self.tab } else { self.width(c) }
    }
    /// Tab stops every TAB columns, `ctl-arrow' on, and every other
    /// non-ASCII character one column: character.c's defaults.
    #[cfg(test)]
    pub(crate) fn plain(tab: usize) -> WidthRule { WidthRule { tab, tables: vec![], ctl_arrow: true } }
    /// The columns of S's characters, tabs counted from column 0.
    pub(crate) fn string_columns(&self, s: &str) -> usize {
        s.chars().fold(0, |column, c| column + self.columns(c as u32, column))
    }
}

impl Vm {
    /// The current buffer's width rule.
    pub(crate) fn width_rule(&mut self) -> Result<WidthRule> {
        let read = |vm: &mut Vm, name: &str| { let slot = vm.slot(name); vm.variable_value(slot) };
        let (tab, ctl_arrow, table) = (read(self, "tab-width")?, read(self, "ctl-arrow")?, read(self, "char-width-table")?);
        Ok(self.rule_of(tab, ctl_arrow, table))
    }
    /// BUFFER's width rule, its own `tab-width' and `ctl-arrow'.
    pub(crate) fn buffer_width_rule(&mut self, buffer: Value) -> Result<WidthRule> {
        let (tab, ctl_arrow) = (self.buffer_variable(buffer, "tab-width")?, self.buffer_variable(buffer, "ctl-arrow")?);
        let table = self.buffer_variable(buffer, "char-width-table")?;
        Ok(self.rule_of(tab, ctl_arrow, table))
    }
    fn rule_of(&self, tab: Value, ctl_arrow: Value, mut table: Value) -> WidthRule {
        let tab = tab.integer_value().filter(|n| (1..=1000).contains(n)).unwrap_or(8) as usize;
        let ctl_arrow = ctl_arrow != Value::NIL && ctl_arrow != Value::UNBOUND;
        let mut tables = vec![];
        while let Some(Object::CharTable(t)) = self.object_if(table) {
            if tables.len() > 64 { break; }
            tables.push(t.view());
            table = t.parent();
        }
        WidthRule { tab, tables, ctl_arrow }
    }
}
