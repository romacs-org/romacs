//! Lisp search boundary. Only owned text and integer offsets reach the engine.
use crate::{coordinates::*, data::Object, *};
use romacs_regex_engine::{self as regex, EmacsStr, MatchRegisters};
#[path = "regexp_syntax.rs"]
mod syntax;

#[derive(Default)]
pub(super) struct MatchState {
    // Character positions, string-relative or one-based buffer positions.
    // No heap indices: collection may move every VM object between calls.
    pub(crate) groups: Vec<Option<(usize, usize)>>,
    pub(crate) succeeded: bool,
}

pub(super) fn is_regexp_primitive(p: Primitive) -> bool {
    matches!(
        p,
        Primitive::LookingAt
            | Primitive::StringMatch
            | Primitive::MatchString
            | Primitive::MatchBeginning
            | Primitive::MatchEnd
            | Primitive::MatchData
            | Primitive::ReSearchForward
            | Primitive::ReSearchBackward
            | Primitive::SyntaxTable
            | Primitive::StandardSyntaxTable
            | Primitive::CopySyntaxTable
            | Primitive::SetSyntaxTable
            | Primitive::ModifySyntaxEntry
    )
}

struct Input {
    bytes: Vec<u8>,
    multibyte: bool,
}
impl Input {
    fn len(&self) -> usize {
        if self.multibyte && !self.bytes.is_ascii() {
            self.character(self.bytes.len())
        } else {
            self.bytes.len()
        }
    }
    fn byte(&self, character: usize) -> usize {
        if self.multibyte && !self.bytes.is_ascii() {
            let mut byte = 0;
            for _ in 0..character {
                if byte == self.bytes.len() {
                    break;
                }
                byte += crate::lstring::string_char(&self.bytes[byte..]).1;
            }
            byte
        } else {
            character
        }
    }
    fn character(&self, byte: usize) -> usize {
        if self.multibyte && !self.bytes.is_ascii() {
            let mut at = 0;
            let mut count = 0;
            while at < byte {
                at += crate::lstring::string_char(&self.bytes[at..]).1;
                count += 1;
            }
            count
        } else {
            byte
        }
    }
}

enum Fault {
    Signal(&'static str, Vec<Value>),
    Text(&'static str, String),
    Wrong(&'static str, Value),
    Internal(String),
}
impl From<String> for Fault {
    fn from(s: String) -> Self {
        Self::Internal(s)
    }
}
type SearchResult<T> = std::result::Result<T, Fault>;

impl Vm {
    /// GNU fast_string_match: case-sensitive, current syntax, no match-data writes.
    pub(crate) fn file_name_match(&mut self, pattern: Value, string: Value) -> std::result::Result<Option<usize>, prim::Fault> {
        let run = |vm: &mut Self| -> SearchResult<Option<usize>> {
            let pattern = vm.regexp_input(pattern)?;
            let input = vm.regexp_input(string)?;
            let syntax = vm.regexp_syntax(Some(string))?;
            let mut compiled = regex::regex_compile_lisp_with_translation(
                &EmacsStr::new(&pattern.bytes, pattern.multibyte), false, None,
            ).map_err(|e| Fault::Text("invalid-regexp", e.to_string()))?;
            compiled.set_target_multibyte(input.multibyte);
            compiled.rebind_syntax_table(&syntax);
            let found = regex::re_search(&compiled, &input.bytes, 0, input.bytes.len() as isize, &syntax, usize::MAX);
            if regex::take_matcher_overflow() {
                return Err(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into()));
            }
            Ok(found.map(|(at, _)| at))
        };
        run(self).map_err(|fault| match fault {
            Fault::Wrong(predicate, value) => prim::Fault::Wrong(predicate, value),
            Fault::Internal(text) => prim::Fault::Error(text),
            Fault::Text(name, text) => {
                let condition = self.intern(name);
                let text = self.allocate_object(Object::String(Box::new(data::LispString::new(&text))));
                let data = self.list_value(&[text]);
                prim::Fault::Signal(condition, data)
            }
            Fault::Signal(name, values) => {
                let condition = self.intern(name);
                let data = self.list_value(&values);
                prim::Fault::Signal(condition, data)
            }
        })
    }

    /// Bounded string-only Freplace_match. Unsupported domains refuse before
    /// allocation or mutation; string replacement leaves the registers intact.
    pub(crate) fn regexp_replace_match(&mut self, newtext: Value, fixedcase: Value, literal: Value, string: Value, subexp: Value) -> prim::Out {
        use prim::Fault;
        let replacement = self.str_view(newtext).ok_or(Fault::Wrong("stringp", newtext))?.to_owned();
        if string != Value::NIL && self.str_view(string).is_none() {
            return Err(Fault::Wrong("stringp", string));
        }
        if self.regexp.groups.is_empty() {
            return Err(Fault::Error("`replace-match' called before any match found".into()));
        }
        let sub = if subexp == Value::NIL { 0 } else {
            if matches!(self.object_if(subexp), Some(Object::Bignum(_))) {
                return Err(Fault::Error("VM2 replace-match unsupported: unrepresented subexpression register".into()));
            }
            let n = subexp.integer_value().ok_or(Fault::Wrong("integerp", subexp))?;
            if n < 0 || n as usize >= self.regexp.groups.len() {
                return Err(Fault::Error("VM2 replace-match unsupported: unrepresented subexpression register".into()));
            }
            n as usize
        };
        let refuse = |domain: &str| Fault::Error(format!("VM2 replace-match unsupported: {domain}"));
        if string == Value::NIL { return Err(refuse("buffer replacement")); }
        let input = self.str_view(string).unwrap().to_owned();
        let Some((a, b)) = self.regexp.groups[sub] else {
            let message = self.make_lisp_string(b"replace-match subexpression does not exist".to_vec(), false);
            let data = self.list_value(&[message, subexp]);
            let error = self.intern("error");
            return Err(Fault::Signal(error, data));
        };
        if (a as i64) < 0 || (b as i64) < 0 || b < a {
            return Err(refuse("signed or inverted match registers"));
        }
        if b > input.len() {
            return Err(Fault::Range(Value::integer(a as i64).map_err(Fault::Error)?, Value::integer(b as i64).map_err(Fault::Error)?));
        }
        if fixedcase == Value::NIL && !replacement.bytes().is_empty() {
            return Err(refuse("case inference with nonempty replacement"));
        }
        if literal == Value::NIL && replacement.bytes().contains(&b'\\') {
            return Err(refuse("replacement backslash directives"));
        }
        for value in [newtext, string] {
            if matches!(self.object_if(value), Some(Object::String(s) | Object::RawString(s)) if !s.properties.is_empty()) {
                return Err(refuse("text properties"));
            }
        }
        if input.is_multibyte() != replacement.is_multibyte() &&
            !(if input.is_multibyte() { replacement.bytes() } else { input.bytes() }).is_ascii() {
            return Err(refuse("mixed non-ASCII string encodings"));
        }
        let source = Input { bytes: input.bytes().to_vec(), multibyte: input.is_multibyte() };
        let mut bytes = source.bytes[..source.byte(a)].to_vec();
        bytes.extend_from_slice(replacement.bytes());
        bytes.extend_from_slice(&source.bytes[source.byte(b)..]);
        Ok(self.make_lisp_string(bytes, input.is_multibyte() || replacement.is_multibyte()))
    }

    pub(crate) fn regexp_translate_match_data(&mut self, delta: i64) {
        if self.regexp.succeeded {
            for (start, end) in self.regexp.groups.iter_mut().flatten() {
                // set-match-data retains signed offsets in usize slots. Recover
                // their signed meaning before adding, including negative ends.
                *start = (*start as i64).wrapping_add(delta).max(0) as usize;
                *end = (*end as i64).wrapping_add(delta).max(0) as usize;
            }
        }
    }

    pub(crate) fn regexp_set_match_data(&mut self, list: Value, reseat: Value) -> std::result::Result<Value, prim::Fault> {
        use prim::Fault;
        if list != Value::NIL && list.tag() != 3 {
            return Err(Fault::Wrong("listp", list));
        }
        let buffer_slot = self.names["#:vm2-match-buffer"];
        self.slots[buffer_slot].value = Value::NIL;
        // GNU validates the complete proper list before updating registers.
        // Only Rust memory is allocated below: retained cells cannot move.
        let mut cells = Vec::new();
        let mut tail = list;
        let mut seen = HashSet::new();
        while tail.tag() == 3 {
            if !seen.insert(tail.0) { return Err(Fault::Circular(list)); }
            let (value, next) = self.cons_pair(tail).map_err(Fault::Error)?;
            cells.push((tail, value));
            tail = next;
        }
        if tail != Value::NIL { return Err(Fault::Wrong("listp", tail)); }
        self.regexp.succeeded = true;
        let length = cells.len() / 2;
        self.regexp.groups.resize(self.regexp.groups.len().max(length), None);
        let mut i = 0;
        while i * 2 < cells.len() {
            let (cell, from) = cells[i * 2];
            if matches!(self.object_if(from), Some(Object::Buffer(_) | Object::ByteBuffer(_))) {
                self.slots[buffer_slot].value = from;
                break;
            }
            if i >= length { break; }
            if from == Value::NIL {
                self.regexp.groups[i] = None;
            } else {
                if let Some(Object::Marker(m)) = self.object_if(from) {
                    if m.buffer != Value::NIL { self.slots[buffer_slot].value = m.buffer; }
                }
                let a = self.regexp_match_position(from)?;
                self.regexp_reseat(cell, from, reseat)?;
                let (end_cell, end) = cells[i * 2 + 1];
                let b = self.regexp_match_position(end)?;
                self.regexp.groups[i] = (a >= 0).then_some((a as usize, b as usize));
                self.regexp_reseat(end_cell, end, reseat)?;
            }
            i += 1;
        }
        self.regexp.groups[i..].fill(None);
        Ok(Value::NIL)
    }
    fn regexp_match_position(&self, value: Value) -> std::result::Result<i64, prim::Fault> {
        if matches!(self.object_if(value), Some(Object::Marker(_))) {
            return Ok(self.marker_position_value(value).map_err(prim::Fault::Error)?.integer_value().unwrap_or(0));
        }
        value.as_integer().map_err(|_| prim::Fault::Wrong("integer-or-marker-p", value))
    }
    fn regexp_reseat(&mut self, cell: Value, value: Value, reseat: Value) -> std::result::Result<(), prim::Fault> {
        if reseat != Value::NIL && matches!(self.object_if(value), Some(Object::Marker(_))) {
            if Self::image_value(cell) { return Err(prim::Fault::Error("Attempt to modify read-only object".into())); }
            self.set_marker_object(value, Value::NIL, Value::NIL).map_err(prim::Fault::Error)?;
            self.conses[cell.index(3).map_err(prim::Fault::Error)?].0 = Value::NIL;
        }
        Ok(())
    }
    fn regexp_input(&self, v: Value) -> SearchResult<Input> {
        let view = self.str_view(v).ok_or(Fault::Wrong("stringp", v))?;
        Ok(Input {
            bytes: view.bytes().to_vec(),
            multibyte: view.is_multibyte(),
        })
    }
    fn regexp_int(&self, v: Value) -> SearchResult<i64> {
        v.as_integer().map_err(|_| Fault::Wrong("fixnump", v))
    }
    fn regexp_preserve(&self) -> bool {
        self.slots[self.names["inhibit-changing-match-data"]].value != Value::NIL
    }
    fn regexp_compile(
        &self,
        pattern: &Input,
        multibyte: bool,
        syntax: &syntax::VmSyntax,
    ) -> SearchResult<regex::CompiledPattern> {
        let folded = self.slots[self.names["case-fold-search"]].value != Value::NIL;
        let translation = folded.then(regex::CaseTranslation::standard);
        let mut compiled = regex::regex_compile_lisp_with_translation(
            &EmacsStr::new(&pattern.bytes, pattern.multibyte),
            false,
            translation,
        )
        .map_err(|e| Fault::Text("invalid-regexp", e.to_string()))?;
        compiled.set_target_multibyte(multibyte);
        compiled.rebind_syntax_table(syntax);
        Ok(compiled)
    }
    fn regexp_save(
        &mut self,
        input: &Input,
        registers: &MatchRegisters,
        base: usize,
        buffer: bool,
    ) {
        self.regexp.groups = registers
            .start
            .iter()
            .zip(&registers.end)
            .map(|(&a, &b)| {
                (a >= 0 && b >= 0).then(|| {
                    (
                        input.character(a as usize) + base,
                        input.character(b as usize) + base,
                    )
                })
            })
            .collect();
        self.regexp.succeeded = true;
        let slot = self.names["#:vm2-match-buffer"];
        self.slots[slot].value = if buffer {
            self.slots[self.buffer_slot].value
        } else {
            Value::NIL
        };
    }
    fn regexp_group(&self, v: Value) -> SearchResult<Option<(usize, usize)>> {
        let n = self.regexp_int(v)?;
        if n < 0 {
            return Err(Fault::Signal(
                "args-out-of-range",
                vec![v, Value::integer(0)?],
            ));
        }
        if !self.regexp.succeeded {
            return Err(Fault::Text(
                "error",
                "No match data, because no search succeeded".into(),
            ));
        }
        Ok(self.regexp.groups.get(n as usize).copied().flatten())
    }
    fn regexp_match_string(&mut self, group: Value, string: Value) -> SearchResult<Value> {
        let Some((a, b)) = self.regexp_group(group)? else {
            return Ok(Value::NIL);
        };
        // Copy the complete outgoing object (including annotations) before any
        // allocation. Never retain a borrowed string or a movable object index.
        let object = if string != Value::NIL {
            if let Some(Object::Vector(v)) = self.object_if(string) {
                if a > b || b > v.len() {
                    return Err(Fault::Signal(
                        "args-out-of-range",
                        vec![string, Value::integer(a as i64)?, Value::integer(b as i64)?],
                    ));
                }
                let copy = v[a..b].to_vec();
                return Ok(self.allocate_object(Object::Vector(copy)));
            }
            let input = self.regexp_input(string).map_err(|e| match e {
                Fault::Wrong(_, value) => Fault::Wrong("arrayp", value),
                other => other,
            })?;
            if b > input.len() || a > b {
                return Err(Fault::Signal(
                    "args-out-of-range",
                    vec![string, Value::integer(a as i64)?, Value::integer(b as i64)?],
                ));
            }
            let range = ByteOffset(input.byte(a))..ByteOffset(input.byte(b));
            let bytes = input.bytes[range.start.0..range.end.0].to_vec();
            if let Some(Object::String(s) | Object::RawString(s)) = self.object_if(string) {
                let mut out = data::LispString::from_bytes(bytes, input.multibyte);
                out.properties = s.properties.slice(range);
                if out.is_utf8() {
                    Object::String(Box::new(out))
                } else {
                    Object::RawString(Box::new(out))
                }
            } else {
                return Ok(self.make_lisp_string(bytes, input.multibyte));
            }
        } else {
            let index = self.buffer_index()?;
            let buffer = self.slots[self.buffer_slot].value;
            let fail = || {
                Fault::Signal(
                    "args-out-of-range",
                    vec![
                        buffer,
                        Value::integer(a as i64).expect("position fits"),
                        Value::integer(b as i64).expect("position fits"),
                    ],
                )
            };
            match &self.objects[index] {
                Object::Buffer(t) => {
                    let aa = a.checked_sub(1).ok_or_else(fail)?;
                    let bb = b.checked_sub(1).ok_or_else(fail)?;
                    let aa = t.byte_of_char(CharOffset(aa)).map_err(|_| fail())?;
                    let bb = t.byte_of_char(CharOffset(bb)).map_err(|_| fail())?;
                    if aa < t.accessible().start || bb > t.accessible().end || aa > bb {
                        return Err(fail());
                    }
                    let range = aa..bb;
                    let mut out = data::LispString::new(t.slice(range.clone())?);
                    out.properties = t.properties.slice(range);
                    Object::String(Box::new(out))
                }
                Object::ByteBuffer(t) => {
                    let aa = a.checked_sub(1).ok_or_else(fail)?;
                    let bb = b.checked_sub(1).ok_or_else(fail)?;
                    if aa < t.restriction.start.0 || bb > t.restriction.end.0 || aa > bb {
                        return Err(fail());
                    }
                    Object::Bytes(t.bytes[aa..bb].to_vec())
                }
                _ => unreachable!("buffer_index validates"),
            }
        };
        Ok(self.allocate_object(object))
    }
    /// `Fmatch_data (Qnil, Qnil, Qnil)' for `save_match_data_load' (autoload.rs).
    pub(super) fn regexp_match_data_saved(&mut self) -> Option<Value> {
        self.regexp_match_data(Value::NIL, Value::NIL, Value::NIL).ok()
    }
    fn regexp_match_data(
        &mut self,
        integers: Value,
        reuse: Value,
        reseat: Value,
    ) -> SearchResult<Value> {
        if !self.regexp.succeeded {
            return Ok(Value::NIL);
        }
        let groups = self.regexp.groups.clone();
        let len = groups
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |i| i + 1);
        let buffer_slot = self.names["#:vm2-match-buffer"];
        let buffer_search = self.slots[buffer_slot].value != Value::NIL;
        let binary_buffer = matches!(
            self.object_if(self.slots[buffer_slot].value),
            Some(Object::ByteBuffer(_))
        );
        let integer_positions = integers != Value::NIL || binary_buffer;
        let mut values = Vec::new();
        for group in &groups[..len] {
            for position in [group.map(|p| p.0), group.map(|p| p.1)] {
                let value = match position {
                    None => Value::NIL,
                    Some(p) if !buffer_search || integer_positions => Value::integer(p as i64)?,
                    Some(p) => {
                        let m =
                            self.allocate_object(Object::Marker(Box::new(annotations::Marker {
                                buffer: Value::NIL,
                                anchor: None,
                                bias: anchors::Bias::Left,
                            })));
                        self.set_marker_object(
                            m,
                            Value::integer(p as i64)?,
                            self.slots[buffer_slot].value,
                        )?
                    }
                };
                values.push(value);
            }
        }
        if buffer_search && integers != Value::NIL {
            values.push(self.slots[buffer_slot].value);
        }
        if reuse.tag() != 3 {
            return Ok(self.list_value(&values));
        }
        let mut tail = reuse;
        let mut previous = None;
        let mut i = 0;
        let mut seen = HashSet::new();
        while tail.tag() == 3 {
            if !seen.insert(tail.0) {
                return Err(Fault::Text(
                    "error",
                    "Circular match-data reuse list".into(),
                ));
            }
            if Self::image_value(tail) {
                return Err(Fault::Text(
                    "error",
                    "Attempt to modify read-only object".into(),
                ));
            }
            let index = tail.index(3)?;
            let (old, next) = self.conses[index];
            if reseat != Value::NIL && matches!(self.object_if(old), Some(Object::Marker(_))) {
                self.set_marker_object(old, Value::NIL, Value::NIL)?;
            }
            self.conses[index].0 = values.get(i).copied().unwrap_or(Value::NIL);
            previous = Some(index);
            tail = next;
            i += 1;
        }
        if i < values.len() {
            let rest = self.list_value(&values[i..]);
            self.conses[previous.expect("nonempty reuse")].1 = rest;
        }
        Ok(reuse)
    }
    fn regexp_primitive(&mut self, p: Primitive, start: usize, n: usize) -> SearchResult<Value> {
        use Primitive::*;
        let valid = match p {
            SyntaxTable | StandardSyntaxTable => n == 0,
            CopySyntaxTable => n <= 1,
            ModifySyntaxEntry => (2..=3).contains(&n),
            LookingAt | MatchString => (1..=2).contains(&n),
            StringMatch => (2..=4).contains(&n),
            ReSearchForward | ReSearchBackward => (1..=4).contains(&n),
            MatchData => n <= 3,
            _ => n == 1,
        };
        if !valid {
            return Err(Fault::Signal(
                "wrong-number-of-arguments",
                vec![Value::reference(p as usize, 6), Value::integer(n as i64)?],
            ));
        }
        let arg = |i| {
            if i < n {
                self.stack[start + i]
            } else {
                Value::NIL
            }
        };
        let args = [arg(0), arg(1), arg(2), arg(3)];
        if matches!(
            p,
            SyntaxTable
                | StandardSyntaxTable
                | CopySyntaxTable
                | SetSyntaxTable
                | ModifySyntaxEntry
        ) {
            return self.regexp_syntax_primitive(p, &args);
        }
        match p {
            MatchBeginning | MatchEnd => {
                return self
                    .regexp_group(args[0])?
                    .map(|(a, b)| Value::integer(if p == MatchBeginning { a } else { b } as i64))
                    .unwrap_or(Ok(Value::NIL))
                    .map_err(Into::into);
            }
            MatchString => return self.regexp_match_string(args[0], args[1]),
            MatchData => return self.regexp_match_data(args[0], args[1], args[2]),
            _ => (),
        }
        if p == StringMatch {
            let pattern = self.regexp_input(args[0])?;
            let input = self.regexp_input(args[1])?;
            let pos = if args[2] == Value::NIL {
                0
            } else {
                self.regexp_int(args[2])?
            };
            let pos = if pos < 0 {
                input.len() as i64 + pos
            } else {
                pos
            };
            if pos < 0 || pos as usize > input.len() {
                return Err(Fault::Signal("args-out-of-range", vec![args[1], args[2]]));
            }
            let syntax = self.regexp_syntax(Some(args[1]))?;
            let compiled = self.regexp_compile(&pattern, input.multibyte, &syntax)?;
            let byte = input.byte(pos as usize);
            let found = regex::re_search(
                &compiled,
                &input.bytes,
                byte,
                (input.bytes.len() - byte) as isize,
                &syntax,
                // GNU at_dot compares the text pointer with buffer point;
                // a standalone string cannot contain that buffer address.
                usize::MAX,
            );
            if regex::take_matcher_overflow() {
                return Err(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into()));
            }
            if let Some((at, registers)) = found {
                if !self.regexp_preserve() && args[3] == Value::NIL {
                    self.regexp_save(&input, &registers, 0, false);
                }
                return Ok(Value::integer(input.character(at) as i64)?);
            }
            return Ok(Value::NIL);
        }
        let count = if matches!(p, ReSearchForward | ReSearchBackward) && args[3] != Value::NIL {
            self.regexp_int(args[3])?
        } else {
            1
        };
        let count = if p == ReSearchBackward { -count } else { count };
        let pattern = self.regexp_input(args[0])?;
        let index = self.buffer_index()?;
        let (input, base_byte, base_char, original) = match &self.objects[index] {
            Object::Buffer(t) => {
                let r = t.accessible();
                (
                    Input {
                        bytes: t.bytes()[r.start.0..r.end.0].to_vec(),
                        multibyte: true,
                    },
                    r.start.0,
                    t.char_of_byte(r.start)?.0 + 1,
                    t.point.0 - r.start.0,
                )
            }
            Object::ByteBuffer(t) => {
                let r = &t.restriction;
                (
                    Input {
                        bytes: t.bytes[r.start.0..r.end.0].to_vec(),
                        multibyte: false,
                    },
                    r.start.0,
                    r.start.0 + 1,
                    t.point.0 - r.start.0,
                )
            }
            _ => unreachable!("buffer_index validates"),
        };
        let original_char = input.character(original) + base_char;
        let bound = if p == LookingAt || args[1] == Value::NIL {
            if p == LookingAt || count > 0 {
                input.len() + base_char
            } else {
                base_char
            }
        } else {
            let limit = if let Some(Object::Marker(_)) = self.object_if(args[1]) {
                let position = self.marker_position_value(args[1])?;
                if position == Value::NIL {
                    return Err(Fault::Text(
                        "error",
                        "Marker does not point anywhere".into(),
                    ));
                }
                position.as_integer()?
            } else {
                args[1]
                    .as_integer()
                    .map_err(|_| Fault::Wrong("integer-or-marker-p", args[1]))?
            };
            if if count > 0 {
                limit < original_char as i64
            } else {
                limit > original_char as i64
            } {
                return Err(Fault::Text(
                    "error",
                    "Invalid search bound (wrong side of point)".into(),
                ));
            }
            limit.clamp(base_char as i64, (input.len() + base_char) as i64) as usize
        };
        if count == 0 {
            return Ok(Value::integer(original_char as i64)?);
        }
        let syntax = self.regexp_syntax(None)?;
        let compiled = self.regexp_compile(&pattern, input.multibyte, &syntax)?;
        let limit = input.byte(bound - base_char);
        let mut point = original;
        let mut failed = false;
        for _ in 0..count.unsigned_abs() {
            let found = if p == LookingAt {
                regex::re_match(
                    &compiled,
                    &input.bytes,
                    point,
                    input.bytes.len(),
                    &syntax,
                    original,
                )
            } else {
                regex::re_search(
                    &compiled,
                    &input.bytes,
                    point,
                    limit as isize - point as isize,
                    &syntax,
                    original,
                )
            };
            if regex::take_matcher_overflow() {
                return Err(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into()));
            }
            let Some((_, registers)) = found else {
                failed = true;
                break;
            };
            point = if count > 0 {
                registers.end[0]
            } else {
                registers.start[0]
            } as usize;
            if !self.regexp_preserve() && (p != LookingAt || args[1] == Value::NIL) {
                self.regexp_save(&input, &registers, base_char, true);
            }
        }
        if p == LookingAt {
            return Ok(if failed { Value::NIL } else { Value::TRUE });
        }
        if failed {
            if args[2] == Value::NIL {
                return Err(Fault::Signal("search-failed", vec![self.stack[start]]));
            }
            if args[2] == Value::TRUE {
                return Ok(Value::NIL);
            }
            point = limit;
        }
        // Recover the buffer after the engine, rather than retaining its index.
        let index = self.buffer_index()?;
        match &mut self.objects[index] {
            Object::Buffer(t) => t.point = ByteOffset(point + base_byte),
            Object::ByteBuffer(t) => t.point = ByteOffset(point + base_byte),
            _ => unreachable!("buffer_index validates"),
        }
        if failed {
            Ok(Value::NIL)
        } else {
            Ok(Value::integer((input.character(point) + base_char) as i64)?)
        }
    }
    pub(super) fn regexp_call(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
        error: &mut Option<String>,
        signal: &mut Option<(Value, Value)>,
    ) -> Option<Value> {
        let fault = match self.regexp_primitive(p, start, n) {
            Ok(v) => return Some(v),
            Err(e) => e,
        };
        let (name, values) = match fault {
            Fault::Signal(name, values) => (name, values),
            Fault::Wrong(predicate, value) => {
                ("wrong-type-argument", vec![self.intern(predicate), value])
            }
            Fault::Text(name, text) => {
                let text =
                    self.allocate_object(Object::String(Box::new(data::LispString::new(&text))));
                (name, vec![text])
            }
            Fault::Internal(text) => {
                *error = Some(text);
                return None;
            }
        };
        *error = Some(name.into());
        let condition = self.intern(name);
        let data = self.list_value(&values);
        *signal = Some((condition, data));
        None
    }
    pub(super) fn regexp_primitive_result(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        let (mut error, mut signal) = (None, None);
        self.regexp_call(p, start, n, &mut error, &mut signal)
            .ok_or_else(|| error.unwrap_or_else(|| "regexp error".into()))
    }
}
