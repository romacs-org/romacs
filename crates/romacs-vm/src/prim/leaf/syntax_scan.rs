//! syntax.c's scanning primitives over the current buffer: the character
//! view `crate::syntax_scan` reads, the variables a scan obeys, argument
//! checks, and parse states as Lisp lists. The view borrows the VM
//! immutably, so nothing allocates or collects while the engine runs.
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};
use crate::syntax_scan::{self as scan, Halt, ParseState, Pos, Scanner};
use std::collections::BTreeMap;

enum Body<'a> {
    Multibyte(&'a crate::text::Text, &'a str),
    Unibyte(&'a crate::bytes::ByteBuffer),
}

/// The current buffer as `crate::syntax_scan::Text`.
struct View<'a> {
    vm: &'a Vm,
    body: Body<'a>,
    table: Value,
    properties: bool,
    ascii: [i32; 128],
}

/// SYNTAX_WITH_FLAGS of C in TABLE: the car of its (CODE . MATCH) entry.
fn entry_code(vm: &Vm, table: Value, c: u32) -> i32 {
    let entry = vm.char_table_get(table, c as usize).unwrap_or(Value::NIL);
    cons_code(vm, entry)
}
fn cons_code(vm: &Vm, entry: Value) -> i32 {
    if entry.tag() != 3 { return scan::SWHITESPACE; }
    vm.cons_pair(entry).ok().and_then(|(code, _)| code.integer_value()).map_or(0, |code| code as i32)
}

impl<'a> View<'a> {
    fn new(vm: &'a Vm, index: usize, table: Value, properties: bool) -> Self {
        let body = match &vm.objects[index] {
            data::Object::Buffer(b) => {
                let text: &crate::text::Text = b;
                Body::Multibyte(text, text.as_str())
            }
            data::Object::ByteBuffer(b) => Body::Unibyte(b),
            _ => unreachable!("buffer_index validates"),
        };
        let mut ascii = [0; 128];
        for (c, code) in ascii.iter_mut().enumerate() { *code = entry_code(vm, table, c as u32); }
        Self { vm, body, table, properties, ascii }
    }
    fn len(&self) -> usize {
        match self.body {
            Body::Multibyte(_, text) => text.len(),
            Body::Unibyte(b) => b.bytes.len(),
        }
    }
    fn property(&self, byte: usize) -> Option<Value> {
        match self.body {
            Body::Multibyte(t, _) => t.properties.get_property(ByteOffset(byte), "syntax-table"),
            Body::Unibyte(b) => b.properties.get(&ByteOffset(byte)).and_then(|p| p.get("syntax-table")).copied(),
        }
    }
}

impl scan::Text for View<'_> {
    fn char_at(&self, at: Pos) -> u32 {
        match self.body {
            Body::Multibyte(_, text) => text.get(at.byte..).and_then(|s| s.chars().next()).map_or(0, u32::from),
            Body::Unibyte(b) => b.bytes.get(at.byte).map_or(0, |&byte| {
                if byte < 128 { u32::from(byte) } else { u32::from(byte) + crate::lstring::BYTE8_OFFSET }
            }),
        }
    }
    fn inc(&self, at: Pos) -> Pos {
        let width = match self.body {
            Body::Multibyte(_, text) => text.get(at.byte..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8),
            Body::Unibyte(_) => 1,
        };
        Pos { char: at.char + 1, byte: at.byte + width }
    }
    fn dec(&self, at: Pos) -> Pos {
        let mut byte = at.byte.saturating_sub(1).min(self.len());
        if let Body::Multibyte(_, text) = self.body {
            while byte > 0 && !text.is_char_boundary(byte) { byte -= 1; }
        }
        Pos { char: at.char - 1, byte }
    }
    fn pos(&self, charpos: i64) -> Pos {
        match self.body {
            Body::Multibyte(t, text) => {
                let chars = t.len_chars() as i64;
                let c = charpos.clamp(1, chars + 1);
                let byte = t.byte_of_char(CharOffset(c as usize - 1)).map_or(text.len(), |b| b.0);
                Pos { char: c, byte }
            }
            Body::Unibyte(b) => {
                let c = charpos.clamp(1, b.bytes.len() as i64 + 1);
                Pos { char: c, byte: c as usize - 1 }
            }
        }
    }
    fn syntax(&self, at: Pos) -> i32 {
        let c = self.char_at(at);
        if self.properties {
            if let Some(value) = self.property(at.byte) {
                if self.vm.is_syntax_table(value) { return entry_code(self.vm, value, c); }
                if value.tag() == 3 { return cons_code(self.vm, value); }
            }
        }
        self.table_syntax(c)
    }
    fn table_syntax(&self, c: u32) -> i32 {
        if c < 128 { self.ascii[c as usize] } else { entry_code(self.vm, self.table, c) }
    }
}

/// A scanning primitive: `crate::prims::syntax_scan` keeps it and its
/// arguments in the continuation registers between reruns.
#[derive(Clone, Copy)]
pub(crate) enum Op { ForwardComment, ScanLists, ScanSexps, ParsePartialSexp, BackwardPrefixChars }
impl Op {
    pub(crate) fn code(self) -> i64 { self as i64 }
    pub(crate) fn of(code: Value) -> Self {
        [Op::ForwardComment, Op::ScanLists, Op::ScanSexps, Op::ParsePartialSexp, Op::BackwardPrefixChars]
            [code.integer_value().expect("saved scan operation") as usize]
    }
}

/// What a scan asks for next.
pub(crate) enum Outcome {
    Done(Value),
    /// Call `internal--syntax-propertize' with the first position, then
    /// require `syntax-propertize--done' past the second.
    Propertize(i64, i64),
    Ppss(i64),
}

enum Result_ { Comment(Pos, bool), Lists(Option<i64>), Parse(ParseState), Prefix(Pos) }

impl Leaf<'_> {
    fn syntax_var(&mut self, name: &str) -> std::result::Result<Value, Fault> {
        let symbol = self.intern(name);
        Ok(self.variable_value(symbol)?.unwrap_or(Value::NIL))
    }
    fn fixnum(&self, v: Value) -> std::result::Result<i64, Fault> {
        v.integer_value().ok_or(Fault::Wrong("fixnump", v))
    }
    /// buffer.c fix_position.
    fn fix_position(&self, v: Value) -> std::result::Result<i64, Fault> {
        if let Some(n) = v.integer_value() { return Ok(n); }
        match self.marker_position(v) {
            Some(Some(n)) => return Ok(n),
            Some(None) => return Err(Fault::Error("Marker does not point anywhere".into())),
            None => {}
        }
        match self.bigint_value(v) {
            Some(n) if n.sign() == num_bigint::Sign::Minus => Ok(Value::MIN_FIXNUM),
            Some(_) => Ok(Value::MAX_FIXNUM),
            None => Err(Fault::Wrong("integer-or-marker-p", v)),
        }
    }
    /// Fcar and Fcdr together.
    fn syntax_car_cdr(&self, v: Value) -> std::result::Result<(Value, Value), Fault> {
        if v == Value::NIL { return Ok((Value::NIL, Value::NIL)); }
        self.pair(v).ok_or(Fault::Wrong("listp", v))
    }
    /// internalize_parse_state.
    fn internalize_parse_state(&self, external: Value) -> std::result::Result<ParseState, Fault> {
        let mut state = ParseState::default();
        if external == Value::NIL { return Ok(state); }
        let mut rest = external;
        let mut next = |leaf: &Self, skip: usize| -> std::result::Result<Value, Fault> {
            for _ in 0..skip { rest = leaf.syntax_car_cdr(rest)?.1; }
            Ok(leaf.syntax_car_cdr(rest)?.0)
        };
        let fixnum = |v: Value| v.integer_value();
        state.depth = fixnum(next(self, 0)?).unwrap_or(0);
        let instring = next(self, 3)?;
        state.instring = if instring == Value::NIL { -1 } else {
            fixnum(instring).filter(|c| (0..=i64::from(crate::lstring::MAX_CHAR)).contains(c))
                .map_or(scan::ST_STRING_STYLE, |c| c as i32)
        };
        let incomment = next(self, 1)?;
        state.incomment = if incomment == Value::NIL { 0 } else { fixnum(incomment).unwrap_or(-1) };
        state.quoted = next(self, 1)? != Value::NIL;
        let comstyle = next(self, 2)?;
        state.comstyle = if comstyle == Value::NIL { 0 } else {
            fixnum(comstyle).filter(|s| (0..=i64::from(scan::ST_COMMENT_STYLE)).contains(s))
                .map_or(scan::ST_COMMENT_STYLE, |s| s as i32)
        };
        state.comstr_start = fixnum(next(self, 1)?).unwrap_or(-1);
        let mut levels = next(self, 1)?;
        while levels != Value::NIL {
            let (start, tail) = self.syntax_car_cdr(levels)?;
            state.levelstarts.push(fixnum(start));
            levels = tail;
        }
        let prev = next(self, 1)?;
        state.prev_syntax = if prev == Value::NIL { scan::SMAX } else { fixnum(prev).map_or(scan::SMAX, |s| s as i32) };
        Ok(state)
    }
    /// Fparse_partial_sexp's value.
    fn externalize_parse_state(&mut self, state: &ParseState) -> Value {
        let int = |n: i64| Value::integer(n).expect("buffer position");
        let opt = |n: i64| if n < 0 { Value::NIL } else { int(n) };
        let mut levels = Value::NIL;
        for start in state.levelstarts.iter().rev() { levels = self.cons(int(start.unwrap_or(-1)), levels); }
        let syntax_table = self.intern("syntax-table");
        let items = [
            int(state.depth),
            opt(state.prevlevelstart),
            opt(state.thislevelstart),
            match state.instring { -1 => Value::NIL, scan::ST_STRING_STYLE => Value::TRUE, c => int(i64::from(c)) },
            match state.incomment { 0 => Value::NIL, n if n < 0 => Value::TRUE, n => int(n) },
            if state.quoted { Value::TRUE } else { Value::NIL },
            int(state.mindepth),
            match state.comstyle { 0 => Value::NIL, scan::ST_COMMENT_STYLE => syntax_table, s => int(i64::from(s)) },
            if state.incomment != 0 || state.instring >= 0 { int(state.comstr_start) } else { Value::NIL },
            levels,
            if state.prev_syntax == scan::SMAX { Value::NIL } else { int(i64::from(state.prev_syntax)) },
        ];
        items.iter().rev().fold(Value::NIL, |tail, &item| self.cons(item, tail))
    }
    fn scan_error(&mut self, message: &str, a: i64, b: i64) -> Fault {
        let condition = self.intern("scan-error");
        let text = self.string(message);
        let b = self.cons(Value::integer(b).expect("buffer position"), Value::NIL);
        let a = self.cons(Value::integer(a).expect("buffer position"), b);
        Fault::Signal(condition, self.cons(text, a))
    }
    /// The buffer's (BEGV, ZV, point) as engine positions.
    fn syntax_bounds(&self, index: usize) -> std::result::Result<(Pos, Pos, Pos), Fault> {
        match &self.vm.objects[index] {
            data::Object::Buffer(b) => {
                let at = |byte: ByteOffset| -> std::result::Result<Pos, Fault> {
                    Ok(Pos { char: b.char_of_byte(byte).map_err(Fault::Error)?.0 as i64 + 1, byte: byte.0 })
                };
                let range = b.accessible();
                Ok((at(range.start)?, at(range.end)?, at(b.point)?))
            }
            data::Object::ByteBuffer(b) => {
                let at = |byte: ByteOffset| Pos { char: byte.0 as i64 + 1, byte: byte.0 };
                Ok((at(b.restriction.start), at(b.restriction.end), at(b.point)))
            }
            _ => Err(Fault::Error("buffer required".into())),
        }
    }
    fn buffer_state_mut(&mut self, index: usize) -> &mut crate::bytes::BufferState {
        match &mut self.vm.objects[index] {
            data::Object::Buffer(b) => &mut b.state,
            data::Object::ByteBuffer(b) => &mut b.state,
            _ => unreachable!("buffer_index validates"),
        }
    }
    /// The current buffer's CHARS_MODIFF.
    pub(crate) fn syntax_chars_modiff(&mut self) -> std::result::Result<i64, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        Ok(self.buffer_state_mut(index).modification.chars_modiff as i64)
    }
    pub(crate) fn syntax_set_point(&mut self, charpos: i64) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match &mut self.vm.objects[index] {
            data::Object::Buffer(b) => b.point = b.byte_of_char(CharOffset(charpos as usize - 1)).map_err(Fault::Error)?,
            data::Object::ByteBuffer(b) => b.point = ByteOffset(charpos as usize - 1),
            _ => unreachable!("buffer_index validates"),
        }
        Ok(())
    }
    /// parse_sexp_propertize's check after `internal--syntax-propertize'.
    pub(crate) fn syntax_propertize_check(&mut self, charpos: i64, chars_modiff: i64) -> std::result::Result<(), Fault> {
        if self.syntax_chars_modiff()? != chars_modiff {
            return Err(Fault::Error("internal--syntax-propertize modified the buffer!".into()));
        }
        let done = self.syntax_var("syntax-propertize--done")?.integer_value().unwrap_or(-1);
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let zv = self.syntax_bounds(index)?.1.char;
        if done <= charpos && done < zv {
            return Err(Fault::Error("internal--syntax-propertize did not move syntax-propertize--done".into()));
        }
        Ok(())
    }

    /// Run scanning primitive OP on ARGS. ANSWERS is an alist of the
    /// defun starts `syntax-ppss' gave for earlier runs.
    pub(crate) fn syntax_scan(&mut self, op: Op, args: &[Value], answers: Value) -> std::result::Result<Outcome, Fault> {
        // Argument checks, in GNU's order.
        let mut parse = None;
        match op {
            Op::ForwardComment => { self.fixnum(args[0])?; }
            Op::ScanLists => { for &arg in &args[..3] { self.fixnum(arg)?; } }
            Op::ScanSexps => { for &arg in &args[..2] { self.fixnum(arg)?; } }
            Op::ParsePartialSexp => {
                let target = if args[2] == Value::NIL { i64::MIN } else { self.fixnum(args[2])? };
                let (to, from) = (self.fix_position(args[1])?, self.fix_position(args[0])?);
                if to < from { return Err(Fault::Error("End position is smaller than start position".into())); }
                let index = self.vm.buffer_index().map_err(Fault::Error)?;
                let (begv, zv, _) = self.syntax_bounds(index)?;
                if !(begv.char <= from && to <= zv.char) {
                    let buffer = self.vm.slots[self.vm.buffer_slot].value;
                    let data = self.cons(args[1], Value::NIL);
                    let data = self.cons(args[0], data);
                    let data = self.cons(buffer, data);
                    return Err(Fault::Signal(self.intern("args-out-of-range"), data));
                }
                let state = self.internalize_parse_state(args[4])?;
                let syntax_table = self.intern("syntax-table");
                let commentstop = if args[5] == Value::NIL { 0 } else if args[5] == syntax_table { -1 } else { 1 };
                parse = Some((from, to, target, args[3] != Value::NIL, commentstop, state));
            }
            Op::BackwardPrefixChars => {}
        }
        let flag = |leaf: &mut Self, name: &str| leaf.syntax_var(name).map(|v| v != Value::NIL);
        let properties = flag(self, "parse-sexp-lookup-properties")?;
        let options = scan::Options {
            ignore_comments: flag(self, "parse-sexp-ignore-comments")?,
            comment_end_can_be_escaped: flag(self, "comment-end-can-be-escaped")?,
            multibyte_syntax_as_symbol: flag(self, "multibyte-syntax-as-symbol")?,
            open_paren_in_column_0_is_defun_start: flag(self, "open-paren-in-column-0-is-defun-start")?,
            comment_use_syntax_ppss: flag(self, "comment-use-syntax-ppss")?,
            propertize_done: if properties {
                Some(self.syntax_var("syntax-propertize--done")?.integer_value().unwrap_or(-1))
            } else {
                None
            },
        };
        let mut ppss = BTreeMap::new();
        let mut tail = answers;
        while let Some((answer, rest)) = self.pair(tail) {
            let (pos, value) = self.pair(answer).expect("saved syntax-ppss answer");
            ppss.insert(pos.integer_value().expect("position"), value.integer_value().expect("position"));
            tail = rest;
        }
        let table = self.vm.current_syntax_table()?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let (begv, zv, point) = self.syntax_bounds(index)?;
        let epoch = self.vm.defun_start_epoch;
        let (cached, modiff) = {
            let state = self.buffer_state_mut(index);
            (state.defun_start.filter(|&(owner, _)| owner == epoch).map(|(_, cache)| cache), state.modification.modiff)
        };
        let view = View::new(self.vm, index, table, properties);
        let mut scanner = Scanner { text: &view, begv, zv, options, ppss: &ppss, defun_start: cached, modiff };
        let result = match op {
            Op::ForwardComment => {
                let count = args[0].integer_value().expect("checked");
                scanner.forward_comment(point, count).map(|(at, found)| Result_::Comment(at, found))
            }
            Op::ScanLists | Op::ScanSexps => {
                let int = |v: Value| v.integer_value().expect("checked");
                let depth = if let Op::ScanLists = op { int(args[2]) } else { 0 };
                scanner.scan_lists(int(args[0]), int(args[1]), depth, matches!(op, Op::ScanSexps)).map(Result_::Lists)
            }
            Op::ParsePartialSexp => {
                let (from, to, target, stopbefore, commentstop, mut state) = parse.expect("checked");
                let start = scan::Text::pos(&view, from);
                scanner.scan_sexps_forward(&mut state, start, to, target, stopbefore, commentstop)
                    .map(|()| Result_::Parse(state))
            }
            Op::BackwardPrefixChars => scanner.backward_prefix_chars(point).map(Result_::Prefix),
        };
        let written = scanner.defun_start.filter(|&cache| Some(cache) != cached);
        drop(view);
        let result = match result {
            Ok(result) => result,
            Err(Halt::ScanError(message, a, b)) => return Err(self.scan_error(message, a, b)),
            Err(Halt::Propertize(arg, charpos)) => return Ok(Outcome::Propertize(arg, charpos)),
            Err(Halt::Ppss(pos)) => return Ok(Outcome::Ppss(pos)),
        };
        if let Some(cache) = written {
            self.vm.defun_start_epoch += 1;
            let epoch = self.vm.defun_start_epoch;
            self.buffer_state_mut(index).defun_start = Some((epoch, cache));
        }
        let set_point = |leaf: &mut Self, at: Pos| match &mut leaf.vm.objects[index] {
            data::Object::Buffer(b) => b.point = ByteOffset(at.byte),
            data::Object::ByteBuffer(b) => b.point = ByteOffset(at.byte),
            _ => unreachable!("buffer_index validates"),
        };
        Ok(Outcome::Done(match result {
            Result_::Comment(at, found) => {
                set_point(self, at);
                if found { Value::TRUE } else { Value::NIL }
            }
            Result_::Lists(at) => at.map_or(Value::NIL, |at| Value::integer(at).expect("buffer position")),
            Result_::Parse(state) => {
                self.syntax_set_point(state.location)?;
                self.externalize_parse_state(&state)
            }
            Result_::Prefix(at) => {
                set_point(self, at);
                Value::NIL
            }
        }))
    }

    /// Fchar_syntax.
    pub fn char_syntax(&mut self, character: Value) -> super::super::Out {
        let mut c = self.syntax_character(character)?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        if matches!(self.vm.objects[index], data::Object::ByteBuffer(_)) && (128..256).contains(&c) {
            c += crate::lstring::BYTE8_OFFSET;
        }
        let table = self.vm.current_syntax_table()?;
        let code = entry_code(self.vm, table, c) & 0xff;
        let spec = scan::SYNTAX_CODE_SPEC.get(code as usize).copied().unwrap_or(b' ');
        Ok(Value::integer(i64::from(spec)).expect("character"))
    }
    /// Fmatching_paren.
    pub fn matching_paren(&mut self, character: Value) -> super::super::Out {
        let c = self.syntax_character(character)?;
        let table = self.vm.current_syntax_table()?;
        let entry = self.vm.char_table_get(table, c as usize).map_err(Fault::Error)?;
        Ok(match cons_code(self.vm, entry) & 0xff {
            scan::SOPEN | scan::SCLOSE => self.pair(entry).map_or(Value::NIL, |(_, matching)| matching),
            _ => Value::NIL,
        })
    }
    fn syntax_character(&self, v: Value) -> std::result::Result<u32, Fault> {
        v.integer_value().filter(|c| (0..=i64::from(crate::lstring::MAX_CHAR)).contains(c))
            .map(|c| c as u32).ok_or(Fault::Wrong("characterp", v))
    }
}
