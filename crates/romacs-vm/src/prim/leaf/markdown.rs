//! Markdown in buffers: a region parsed as one document (`crate::markdown`),
//! its runs written as `face` properties and its blocks returned as Lisp
//! values. Positions here are the buffer's; the module's are byte offsets
//! into the region's text.
use super::{Fault, Leaf};
use crate::buffer::PropertyEdit;
use crate::data;
use crate::markdown::{self, BlockKind, Kind};
use crate::Value;
use std::collections::HashMap;

type R<T> = std::result::Result<T, Fault>;

impl Leaf<'_> {
    /// The current buffer's text from character position BEG to END.
    fn markdown_text(&mut self, beg: i64, end: i64) -> R<String> {
        let buffer = self.current_buffer()?;
        let char_offset = |p: i64| usize::try_from(p - 1).map(crate::coordinates::CharOffset).map_err(|_| Fault::Error("position out of range".into()));
        match self.vm.object_if(buffer) {
            Some(data::Object::Buffer(text)) => {
                let a = text.byte_of_char(char_offset(beg)?).map_err(Fault::Error)?;
                let b = text.byte_of_char(char_offset(end)?).map_err(Fault::Error)?;
                Ok(text.slice(a..b).map_err(Fault::Error)?.into_owned())
            }
            _ => Err(Fault::Error("romacs-markdown: not a multibyte buffer".into())),
        }
    }

    /// Fontify BEG..END of the current buffer, one Markdown document: its
    /// `face` properties become its runs' faces, each kind's from FACES (an
    /// alist of kind symbols to faces; a kind absent or nil gets none).
    /// With FROM, the text before it is unchanged since the last call, and
    /// only the blocks from the one before FROM's are written again. The
    /// writes record no undo and run no hooks: the caller binds them off
    /// (`with-silent-modifications`), else this signals. Returns the
    /// position the writes start at.
    pub(crate) fn markdown_fontify(&mut self, beg: Value, end: Value, faces: Value, from: Value) -> R<Value> {
        let (beg, end) = self.validate_region(beg, end)?;
        let buffer = self.current_buffer()?;
        let inhibit = self.intern("inhibit-modification-hooks");
        if self.variable_value(inhibit)?.is_none_or(|v| v == Value::NIL) || self.vm.keeps_undo(buffer)? {
            return Err(Fault::Error("romacs-markdown--fontify writes faces silently: call it inside `with-silent-modifications'".into()));
        }
        let text = self.markdown_text(beg, end)?;
        let parsed = markdown::spans(&text);
        let from_byte = match from.integer_value() {
            Some(position) if position > beg => byte_of_char(&text, usize::try_from(position.min(end) - beg).unwrap_or(0)),
            _ => 0,
        };
        let restart = markdown::restart(&parsed.tops, from_byte);
        let runs = markdown::runs(&parsed.spans);
        let table = self.markdown_face_table(faces);
        // Where the writes start, and each run's ends, as positions.
        let mut offsets = vec![restart];
        for run in runs.iter().filter(|r| r.end > restart) {
            offsets.push(run.start.max(restart));
            offsets.push(run.end);
        }
        let chars = char_offsets(&text, &offsets);
        let position = |i: usize| at(beg, chars[i]);
        let face = self.intern("face");
        let start_value = position(0)?;
        let end_value = Value::integer(end).map_err(Fault::Error)?;
        self.markdown_write(&PropertyEdit::Remove(vec![face]), start_value, end_value, buffer)?;
        let mut values: HashMap<Vec<Kind>, Value> = HashMap::new();
        for (i, run) in runs.iter().filter(|r| r.end > restart).enumerate() {
            let value = if let Some(&value) = values.get(&run.kinds) {
                value
            } else {
                let value = self.markdown_face_value(&table, &run.kinds);
                values.insert(run.kinds.clone(), value);
                value
            };
            if value != Value::NIL {
                self.markdown_write(&PropertyEdit::Put(face, value), position(1 + 2 * i)?, position(2 + 2 * i)?, buffer)?;
            }
        }
        Ok(start_value)
    }

    /// One property write that must run no Lisp: the caller inhibited it.
    fn markdown_write(&mut self, edit: &PropertyEdit, start: Value, end: Value, buffer: Value) -> R<()> {
        match self.property_begin(edit, start, end, buffer)? {
            super::text_properties::PropertyBegin::Done(_) => Ok(()),
            super::text_properties::PropertyBegin::Hooks { .. } => {
                Err(Fault::Error("romacs-markdown--fontify: a face write would run hooks".into()))
            }
        }
    }

    /// Each kind's face, by [`Kind::index`], from FACES, an alist.
    fn markdown_face_table(&mut self, faces: Value) -> Vec<Value> {
        let names: Vec<Value> = Kind::ALL.iter().map(|k| self.intern(k.name())).collect();
        let mut table = vec![Value::NIL; Kind::ALL.len()];
        let mut tail = faces;
        while let Some((entry, rest)) = self.pair(tail) {
            if let Some((key, face)) = self.pair(entry)
                && let Some(i) = names.iter().position(|&n| n == key)
                && table[i] == Value::NIL
            {
                table[i] = face;
            }
            tail = rest;
        }
        table
    }

    /// The `face` value for KINDS: their faces innermost first, one face
    /// alone, nil for none.
    fn markdown_face_value(&mut self, table: &[Value], kinds: &[Kind]) -> Value {
        let faces: Vec<Value> = kinds.iter().map(|k| table[k.index()]).filter(|&f| f != Value::NIL).collect();
        match faces.as_slice() {
            [] => Value::NIL,
            [face] => *face,
            _ => faces.iter().rev().fold(Value::NIL, |list, &face| self.cons(face, list)),
        }
    }

    /// SOURCE, a pipe table, as a grid at most WIDTH columns wide, its
    /// kinds' faces from FACES as `markdown_fontify` takes them, and
    /// characters as wide as `string-width` counts them; nil when SOURCE
    /// holds no table.
    pub(crate) fn markdown_table_grid(&mut self, source: Value, width: Value, faces: Value) -> R<Value> {
        let view = self.string_view(source).map_err(|_| Fault::Wrong("stringp", source))?;
        let text: String = view.chars().filter_map(char::from_u32).collect();
        let width = width.integer_value().ok_or(Fault::Wrong("fixnump", width))?;
        let rule = self.vm.width_rule().map_err(Fault::Error)?;
        let columns = |c: char| rule.width(u32::from(c));
        let Some(grid) = markdown::grid(&text, usize::try_from(width).unwrap_or(0), &columns) else {
            return Ok(Value::NIL);
        };
        let offsets: Vec<usize> = grid.runs.iter().flat_map(|r| [r.start, r.end]).collect();
        let chars = char_offsets(&grid.text, &offsets);
        let string = self.make_string(grid.text.into_bytes(), true);
        let table = self.markdown_face_table(faces);
        let face = self.intern("face");
        let mut values: HashMap<Vec<Kind>, Value> = HashMap::new();
        for (i, run) in grid.runs.iter().enumerate() {
            let value = if let Some(&value) = values.get(&run.kinds) {
                value
            } else {
                let value = self.markdown_face_value(&table, &run.kinds);
                values.insert(run.kinds.clone(), value);
                value
            };
            if value != Value::NIL {
                let (start, end) = (at(0, chars[2 * i])?, at(0, chars[2 * i + 1])?);
                self.vm.property_edit_now(&PropertyEdit::Put(face, value), start, end, string)?;
            }
        }
        Ok(string)
    }

    /// The blocks of BEG..END of the current buffer, one Markdown document:
    /// each (KIND START END . PLIST), in document order. KIND is
    /// `heading', `code', `table', `item', `quote', `rule' or `html'; PLIST
    /// has `:depth' (nesting in lists and quotes), and `:level' for a
    /// heading, `:language' and `:body' (START . END) for code, `:ordered'
    /// for an item.
    pub(crate) fn markdown_blocks(&mut self, beg: Value, end: Value) -> R<Value> {
        let (beg, end) = self.validate_region(beg, end)?;
        let text = self.markdown_text(beg, end)?;
        let blocks = markdown::blocks(&text);
        // Every offset a block names, ascending, converted at once.
        let mut offsets: Vec<usize> = blocks.iter().flat_map(|b| {
            let body = match &b.kind { BlockKind::Code { body, .. } => body.clone(), _ => b.range.clone() };
            [b.range.start, b.range.end, body.start, body.end]
        }).collect();
        offsets.sort_unstable();
        offsets.dedup();
        let chars = char_offsets(&text, &offsets);
        let position = |offset: usize| -> R<Value> {
            let i = offsets.binary_search(&offset).map_err(|_| Fault::Error("romacs-markdown: lost offset".into()))?;
            at(beg, chars[i])
        };
        let mut out = Vec::with_capacity(blocks.len());
        for block in &blocks {
            let depth = Value::integer(i64::from(block.depth)).map_err(Fault::Error)?;
            let mut plist = vec![self.intern(":depth"), depth];
            let kind = match &block.kind {
                BlockKind::Heading(level) => {
                    plist.extend([self.intern(":level"), Value::integer(i64::from(*level)).map_err(Fault::Error)?]);
                    "heading"
                }
                BlockKind::Code { language, body } => {
                    let language = match language {
                        Some(name) => self.make_string(name.clone().into_bytes(), true),
                        None => Value::NIL,
                    };
                    let body = self.cons(position(body.start)?, position(body.end)?);
                    plist.extend([self.intern(":language"), language, self.intern(":body"), body]);
                    "code"
                }
                BlockKind::Table => "table",
                BlockKind::Item { ordered } => {
                    plist.extend([self.intern(":ordered"), if *ordered { Value::TRUE } else { Value::NIL }]);
                    "item"
                }
                BlockKind::Quote => "quote",
                BlockKind::Rule => "rule",
                BlockKind::Html => "html",
            };
            let tail = plist.iter().rev().fold(Value::NIL, |list, &v| self.cons(v, list));
            let tail = self.cons(position(block.range.end)?, tail);
            let tail = self.cons(position(block.range.start)?, tail);
            let kind = self.intern(kind);
            out.push(self.cons(kind, tail));
        }
        Ok(out.iter().rev().fold(Value::NIL, |list, &v| self.cons(v, list)))
    }
}

/// The position CHARS characters after BEG.
fn at(beg: i64, chars: usize) -> R<Value> {
    let chars = i64::try_from(chars).map_err(|_| Fault::Error("position out of range".into()))?;
    Value::integer(beg + chars).map_err(Fault::Error)
}

/// The byte offset of character CHARS in TEXT.
fn byte_of_char(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i)
}

/// The character offset of each of OFFSETS (bytes, ascending) in TEXT.
fn char_offsets(text: &str, offsets: &[usize]) -> Vec<usize> {
    if text.is_ascii() {
        return offsets.to_vec();
    }
    let (mut byte, mut chars) = (0, 0);
    offsets.iter().map(|&offset| {
        chars += text[byte..offset].chars().count();
        byte = offset;
        chars
    }).collect()
}
