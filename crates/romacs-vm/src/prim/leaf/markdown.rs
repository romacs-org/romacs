//! Markdown in buffers: a region parsed as one document, or a buffer by the
//! blocks its index (`crate::markdown::Index`) says hold a region; the runs
//! written as `face` properties, blocks, links and code returned as Lisp
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
    /// (`with-silent-modifications`), else this signals. Returns (START .
    /// CODES): where the writes start, and the code blocks written, each
    /// (LANGUAGE BODY-START BODY-END), LANGUAGE a string or nil.
    pub(crate) fn markdown_fontify(&mut self, beg: Value, end: Value, faces: Value, from: Value) -> R<Value> {
        let (beg, end) = self.validate_region(beg, end)?;
        let buffer = self.markdown_silent_buffer()?;
        let text = self.markdown_text(beg, end)?;
        let parsed = markdown::spans(&text);
        let from_byte = match from.integer_value() {
            Some(position) if position > beg => byte_of_char(&text, usize::try_from(position.min(end) - beg).unwrap_or(0)),
            _ => 0,
        };
        let restart = markdown::restart(&parsed.tops, from_byte);
        let (start, codes) = self.markdown_write_runs(&text, &parsed, restart..text.len(), beg, faces, buffer)?;
        Ok(self.cons(start, codes))
    }

    /// Fontify the blocks of the current buffer, a Markdown document, that
    /// hold BEG..END, as `markdown_fontify` does a region: the buffer's
    /// block index (`crate::markdown::Index`) says which, and resyncs what
    /// changed since; the text it found changed outside those blocks loses
    /// its `fontified`, so redisplay asks for it again. Returns (START END
    /// . CODES): the blocks written, and their code blocks.
    pub(crate) fn markdown_fontify_buffer(&mut self, beg: Value, end: Value, faces: Value) -> R<Value> {
        let (beg, end) = self.validate_region(beg, end)?;
        let buffer = self.markdown_silent_buffer()?;
        let shape = self.buffer_shape()?;
        let to_offset = |p: i64| usize::try_from(p - 1).unwrap_or(0);
        let ((blocks, text, parsed), changed) = self.markdown_synced(buffer, |index, read| {
            let blocks = index.blocks(to_offset(beg), to_offset(end));
            let text = read(blocks.clone());
            let parsed = markdown::spans_with(&text, &|label| index.defines(label));
            (blocks, text, parsed)
        })?;
        let lo = i64::try_from(blocks.start).map_err(|_| Fault::Error("position out of range".into()))? + 1;
        let hi = lo + i64::try_from(text.chars().count()).map_err(|_| Fault::Error("position out of range".into()))?;
        // Narrowed, only the accessible text is written.
        let write_from = byte_of_char(&text, usize::try_from((shape.begv - lo).max(0)).unwrap_or(0));
        let write_to = byte_of_char(&text, usize::try_from((shape.zv.min(hi) - lo).max(0)).unwrap_or(0));
        let (start, codes) = self.markdown_write_runs(&text, &parsed, write_from..write_to.max(write_from), lo, faces, buffer)?;
        if let Some(changed) = changed {
            let fontified = self.intern("fontified");
            let (cf, ct) = (i64::try_from(changed.start).unwrap_or(0) + 1, i64::try_from(changed.end).unwrap_or(0) + 1);
            for (a, b) in [(cf, lo.min(ct)), (hi.max(cf), ct)] {
                let (a, b) = (a.max(shape.begv), b.min(shape.zv));
                if a < b {
                    let (a, b) = (Value::integer(a).map_err(Fault::Error)?, Value::integer(b).map_err(Fault::Error)?);
                    self.markdown_write(&PropertyEdit::Remove(vec![fontified]), a, b, buffer)?;
                }
            }
        }
        let end_value = Value::integer(hi.min(shape.zv)).map_err(Fault::Error)?;
        let tail = self.cons(end_value, codes);
        Ok(self.cons(start, tail))
    }

    /// F over BUFFER's Markdown index, synced to the text, and a reader of
    /// its text by character offsets; with the span the sync read again,
    /// when the index was already built.
    fn markdown_synced<T>(&mut self, buffer: Value, f: impl FnOnce(&markdown::Index, &dyn Fn(std::ops::Range<usize>) -> String) -> T)
        -> R<(T, Option<std::ops::Range<usize>>)> {
        let Some(data::Object::Buffer(body)) = self.vm.object_if(buffer) else {
            return Err(Fault::Error("romacs-markdown: not a multibyte buffer".into()));
        };
        let read = |r: std::ops::Range<usize>| -> String {
            let a = body.byte_of_char(crate::coordinates::CharOffset(r.start));
            let b = body.byte_of_char(crate::coordinates::CharOffset(r.end));
            match (a, b) {
                (Ok(a), Ok(b)) => body.slice(a..b).map(std::borrow::Cow::into_owned).unwrap_or_default(),
                _ => String::new(),
            }
        };
        let mut index = body.markdown.borrow_mut();
        let was_built = index.built();
        let changed = index.sync(body.len_chars(), &mut |r| read(r)).filter(|_| was_built);
        Ok((f(&index, &read), changed))
    }

    /// The top-level block of the current buffer, a Markdown document,
    /// holding POS, as (START . END).
    pub(crate) fn markdown_block_at(&mut self, pos: Value) -> R<Value> {
        let pos = pos.integer_value().ok_or(Fault::Wrong("integer-or-marker-p", pos))?;
        let buffer = self.current_buffer()?;
        let offset = usize::try_from(pos - 1).unwrap_or(0);
        let (blocks, _) = self.markdown_synced(buffer, |index, _| index.blocks(offset, offset))?;
        let (a, b) = (at(1, blocks.start)?, at(1, blocks.end)?);
        Ok(self.cons(a, b))
    }

    /// The links of BEG..END of the current buffer, one Markdown document:
    /// each (START END . DESTINATION), DESTINATION empty for a reference
    /// whose definition is outside the region.
    pub(crate) fn markdown_links(&mut self, beg: Value, end: Value) -> R<Value> {
        let (beg, end) = self.validate_region(beg, end)?;
        let text = self.markdown_text(beg, end)?;
        let parsed = markdown::spans(&text);
        let positions = Positions::new(&text, parsed.links.iter().flat_map(|(r, _)| [r.start, r.end]).collect(), beg);
        let mut list = Value::NIL;
        for (range, dest) in parsed.links.iter().rev() {
            let dest = self.make_string(dest.clone().into_bytes(), true);
            let tail = self.cons(positions.at(range.end)?, dest);
            let entry = self.cons(positions.at(range.start)?, tail);
            list = self.cons(entry, list);
        }
        Ok(list)
    }

    /// SOURCE, a pipe table, with its columns aligned, characters as wide
    /// as `string-width` counts them; nil when SOURCE holds no table.
    pub(crate) fn markdown_table_align(&mut self, source: Value) -> R<Value> {
        let view = self.string_view(source).map_err(|_| Fault::Wrong("stringp", source))?;
        let text: String = view.chars().filter_map(char::from_u32).collect();
        let rule = self.vm.width_rule().map_err(Fault::Error)?;
        Ok(match markdown::align(&text, &|c| rule.width(u32::from(c))) {
            Some(aligned) => self.make_string(aligned.into_bytes(), true),
            None => Value::NIL,
        })
    }

    /// The current buffer, once faces may be written to it silently: its
    /// modification hooks inhibited and its undo off, else an error.
    fn markdown_silent_buffer(&mut self) -> R<Value> {
        let buffer = self.current_buffer()?;
        let inhibit = self.intern("inhibit-modification-hooks");
        if self.variable_value(inhibit)?.is_none_or(|v| v == Value::NIL) || self.vm.keeps_undo(buffer)? {
            return Err(Fault::Error("romacs-markdown writes faces silently: call it inside `with-silent-modifications'".into()));
        }
        Ok(buffer)
    }

    /// Write PARSED's faces, TEXT beginning at position BASE, over its bytes
    /// WRITE: the `face` there removed, then each run's put. Returns the
    /// position the writes start at, and the code blocks whose text ends
    /// in WRITE, as Lisp lists (LANGUAGE BODY-START BODY-END).
    fn markdown_write_runs(&mut self, text: &str, parsed: &markdown::Parsed, write: std::ops::Range<usize>, base: i64, faces: Value, buffer: Value) -> R<(Value, Value)> {
        let runs = markdown::runs(&parsed.spans);
        let runs: Vec<(usize, usize, &[Kind])> = runs
            .iter()
            .filter(|r| r.end > write.start && r.start < write.end)
            .map(|r| (r.start.max(write.start), r.end.min(write.end), r.kinds.as_slice()))
            .collect();
        let codes: Vec<&markdown::Code> = parsed.codes.iter().filter(|c| c.body.end > write.start && c.body.start < write.end).collect();
        let mut offsets: Vec<usize> = vec![write.start, write.end];
        offsets.extend(runs.iter().flat_map(|r| [r.0, r.1]));
        offsets.extend(codes.iter().flat_map(|c| [c.body.start, c.body.end]));
        let positions = Positions::new(text, offsets, base);
        let face = self.intern("face");
        let table = self.markdown_face_table(faces);
        let start = positions.at(write.start)?;
        if write.start < write.end {
            self.markdown_write(&PropertyEdit::Remove(vec![face]), start, positions.at(write.end)?, buffer)?;
        }
        let mut values: HashMap<Vec<Kind>, Value> = HashMap::new();
        for &(a, b, kinds) in &runs {
            let value = if let Some(&value) = values.get(kinds) {
                value
            } else {
                let value = self.markdown_face_value(&table, kinds);
                values.insert(kinds.to_vec(), value);
                value
            };
            if value != Value::NIL {
                self.markdown_write(&PropertyEdit::Put(face, value), positions.at(a)?, positions.at(b)?, buffer)?;
            }
        }
        let mut list = Value::NIL;
        for code in codes.iter().rev() {
            let language = match &code.language {
                Some(name) => self.make_string(name.clone().into_bytes(), true),
                None => Value::NIL,
            };
            let entry = [language, positions.at(code.body.start)?, positions.at(code.body.end)?];
            let entry = entry.iter().rev().fold(Value::NIL, |l, &v| self.cons(v, l));
            list = self.cons(entry, list);
        }
        Ok((start, list))
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

/// Byte offsets of a text, each as the position it is at: the text begins
/// at BASE.
struct Positions {
    offsets: Vec<usize>,
    chars: Vec<usize>,
    base: i64,
}

impl Positions {
    fn new(text: &str, mut offsets: Vec<usize>, base: i64) -> Positions {
        offsets.sort_unstable();
        offsets.dedup();
        let chars = char_offsets(text, &offsets);
        Positions { offsets, chars, base }
    }

    fn at(&self, offset: usize) -> R<Value> {
        let i = self.offsets.binary_search(&offset).map_err(|_| Fault::Error("romacs-markdown: lost offset".into()))?;
        at(self.base, self.chars[i])
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
