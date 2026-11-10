//! syntax.c's scanning primitives over the current buffer: the character
//! view `crate::syntax_scan` reads, the variables a scan obeys, argument
//! checks, and parse states as Lisp lists. The view borrows the VM
//! immutably, so nothing allocates or collects while the engine runs.
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};
use crate::syntax_index::{Context, Decline, SyntaxIndex};
use crate::syntax_scan::{self as scan, Halt, ParseState, Pos, Scanner};
use std::collections::BTreeMap;

enum Body<'a> {
    /// The text, its str, and a reader of its `syntax-table' properties.
    Multibyte(&'a crate::text::Text, crate::properties::KeyReader<'a>),
    Unibyte(&'a crate::bytes::ByteBuffer),
}

#[cfg(test)]
thread_local! {
    /// The syntax reads of every `View` on this thread, replayed or live.
    static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The current buffer as `crate::syntax_scan::Text`.
///
/// A rerun after a mid-scan Lisp call replays the syntax the stopped run
/// read, in order, then reads live: GNU's one scan read those positions
/// before the call, so their syntax is what it was then, even where the
/// call changed `syntax-table' properties since. A parse records nothing:
/// it goes on where it stopped instead.
struct View<'a> {
    vm: &'a Vm,
    body: Body<'a>,
    table: Value,
    properties: bool,
    /// Memo of the ASCII codes this scan has read: a scan reads a few,
    /// and each read walks the table and its parents.
    ascii: [std::cell::Cell<Option<i32>>; 128],
    replay: Vec<i32>,
    read: std::cell::RefCell<Vec<i32>>,
    /// Whether the scan can stop to call Lisp, which reruns it from `read'.
    record: bool,
    /// The last character read, as (byte, code): a scan reads a
    /// character, its syntax and its width at one position in turn.
    last: std::cell::Cell<(usize, u32)>,
}

/// The text of the multibyte buffer at INDEX.
fn buffer_text(vm: &Vm, index: usize) -> &crate::text::Text {
    match &vm.objects[index] {
        data::Object::Buffer(b) => b,
        _ => unreachable!("a multibyte buffer"),
    }
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
                Body::Multibyte(text, text.properties.reader("syntax-table"))
            }
            data::Object::ByteBuffer(b) => Body::Unibyte(b),
            _ => unreachable!("buffer_index validates"),
        };
        let ascii = std::array::from_fn(|_| std::cell::Cell::new(None));
        Self { vm, body, table, properties, ascii, replay: Vec::new(), read: Default::default(), record: true, last: std::cell::Cell::new((usize::MAX, 0)) }
    }
    fn len(&self) -> usize {
        match self.body {
            Body::Multibyte(t, _) => t.len_bytes(),
            Body::Unibyte(b) => b.bytes.len(),
        }
    }
    fn property(&self, byte: usize) -> Option<Value> {
        match self.body {
            Body::Multibyte(_, ref reader) => reader.get(ByteOffset(byte)),
            Body::Unibyte(b) => b.properties.get(&ByteOffset(byte)).and_then(|p| p.get("syntax-table")).copied(),
        }
    }
}

impl scan::Text for View<'_> {
    fn char_at(&self, at: Pos) -> u32 {
        match self.body {
            Body::Multibyte(t, _) => {
                let (byte, code) = self.last.get();
                if byte == at.byte { return code; }
                let code = t.character_at(ByteOffset(at.byte)).ok().flatten().map_or(0, |c| c as u32);
                self.last.set((at.byte, code));
                code
            }
            Body::Unibyte(b) => b.bytes.get(at.byte).map_or(0, |&byte| {
                if byte < 128 { u32::from(byte) } else { u32::from(byte) + crate::lstring::BYTE8_OFFSET }
            }),
        }
    }
    fn inc(&self, at: Pos) -> Pos {
        let width = match self.body {
            // A placeholder's UTF-8 width is its raw byte's internal width.
            Body::Multibyte(..) if at.byte >= self.len() => 1,
            Body::Multibyte(..) => crate::lstring::char_string(self.char_at(at), &mut [0; 5]),
            Body::Unibyte(_) => 1,
        };
        Pos { char: at.char + 1, byte: at.byte + width }
    }
    fn dec(&self, at: Pos) -> Pos {
        let mut byte = at.byte.saturating_sub(1).min(self.len());
        if let Body::Multibyte(t, _) = self.body && byte < self.len() {
            byte = t.char_start_before(byte + 1);
        }
        Pos { char: at.char - 1, byte }
    }
    fn pos(&self, charpos: i64) -> Pos {
        match self.body {
            Body::Multibyte(t, _) => {
                let chars = t.len_chars() as i64;
                let c = charpos.clamp(1, chars + 1);
                let byte = t.byte_of_char(CharOffset(c as usize - 1)).map_or(t.len_bytes(), |b| b.0);
                Pos { char: c, byte }
            }
            Body::Unibyte(b) => {
                let c = charpos.clamp(1, b.bytes.len() as i64 + 1);
                Pos { char: c, byte: c as usize - 1 }
            }
        }
    }
    fn syntax(&self, at: Pos) -> i32 {
        #[cfg(test)]
        READS.with(|n| n.set(n.get() + 1));
        if !self.record {
            return self.live_syntax(at);
        }
        let mut read = self.read.borrow_mut();
        let syntax = match self.replay.get(read.len()) {
            Some(&syntax) => syntax,
            None => self.live_syntax(at),
        };
        read.push(syntax);
        syntax
    }
    fn table_syntax(&self, c: u32) -> i32 {
        let Some(memo) = self.ascii.get(c as usize) else { return entry_code(self.vm, self.table, c) };
        memo.get().unwrap_or_else(|| { let code = entry_code(self.vm, self.table, c); memo.set(Some(code)); code })
    }
}

impl View<'_> {
    fn live_syntax(&self, at: Pos) -> i32 {
        use scan::Text as _;
        let c = self.char_at(at);
        if self.properties {
            if let Some(value) = self.property(at.byte) {
                if self.vm.is_syntax_table(value) { return entry_code(self.vm, value, c); }
                if value.tag() == 3 { return cons_code(self.vm, value); }
            }
        }
        self.table_syntax(c)
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
    /// Both stops carry the scan's memory for its rerun: a vector of the
    /// start's BEGV, ZV and point, then where a parse stopped, or the
    /// syntax another scan read so far (`MEMORY_STATE').
    Propertize(i64, i64, Value),
    Ppss(i64, Value),
}

enum Result_ { Comment(Pos, bool), Lists(Option<i64>), Parse(ParseState), Prefix(Pos) }

/// A parse the text's summaries serve (`crate::syntax_index`), and whether
/// the plain scanner checks it.
struct Served {
    context: Context,
    check: bool,
}


/// A scan's memory: the first run's BEGV, ZV and point, coerced FROM and
/// TO, its variables as flags, its initial parse state (`state_memory`),
/// then, for a parse, where it stopped (`suspended_memory`; nothing when it
/// stopped before reading), else the syntax it read.
const MEMORY_STATE: usize = 6;

/// The index of ITEM in ALL, as a fixnum.
fn label_code<T: PartialEq>(all: &[T], item: &T) -> Value {
    let index = all.iter().position(|label| label == item).expect("a listed label");
    Value::integer(i64::try_from(index).expect("label index")).expect("fixnum")
}

/// A stopped parse as fixnums: its label, its locals (positions as
/// characters), its levels, its comment scan if it stopped in one, then its
/// parse state (`state_memory`).
fn suspended_memory(suspended: &scan::Suspended) -> Vec<Value> {
    let fixnum = |n: i64| Value::integer(n).expect("fixnum");
    let count = |n: usize| fixnum(i64::try_from(n).expect("count"));
    let l = &suspended.locals;
    let mut values = vec![label_code(&scan::At::ALL, &suspended.at)];
    values.extend([l.from.char, l.prev_from.char, i64::from(l.prev_from_syntax), i64::from(l.prev_prev_from_syntax),
        l.depth, l.mindepth, l.maxdepth, i64::from(l.in_symbol), l.symbol_prev, i64::from(l.spent)].map(fixnum));
    values.push(count(l.levels.len()));
    values.extend(l.levels.iter().flat_map(|level| [fixnum(level.last), fixnum(level.prev)]));
    match &suspended.forw {
        None => values.push(Value::NIL),
        Some(f) => {
            values.push(label_code(&scan::ForwAt::ALL, &f.at));
            values.extend([f.from.char, f.nesting, i64::from(f.syntax), i64::from(f.code), i64::from(f.prev_syntax)].map(fixnum));
        }
    }
    values.extend(state_memory(&suspended.state));
    values
}

/// `suspended_memory` read back, POS giving a character's position.
fn memory_suspended(values: &[Value], pos: impl Fn(i64) -> Pos) -> scan::Suspended {
    let int = |i: usize| values[i].integer_value().expect("saved parse");
    let syntax = |i: usize| i32::try_from(int(i)).expect("saved syntax");
    let label = |i: usize| usize::try_from(int(i)).expect("saved label");
    let levels = usize::try_from(int(11)).expect("saved levels");
    let locals = scan::Locals {
        from: pos(int(1)), prev_from: pos(int(2)), prev_from_syntax: syntax(3), prev_prev_from_syntax: syntax(4),
        depth: int(5), mindepth: int(6), maxdepth: int(7), in_symbol: int(8) != 0, symbol_prev: int(9), spent: int(10) != 0,
        levels: (0..levels).map(|k| scan::Level { last: int(12 + 2 * k), prev: int(13 + 2 * k) }).collect(),
    };
    let at = 12 + 2 * levels;
    let (forw, state) = if values[at] == Value::NIL {
        (None, at + 1)
    } else {
        let forw = scan::Forw {
            at: scan::ForwAt::ALL[label(at)], from: pos(int(at + 1)), nesting: int(at + 2),
            syntax: syntax(at + 3), code: syntax(at + 4), prev_syntax: syntax(at + 5),
        };
        (Some(forw), at + 6)
    };
    scan::Suspended { at: scan::At::ALL[label(0)], locals, state: memory_state(&values[state..]), forw }
}

/// A parse state as eleven fixnums, its open-paren count, then the list.
fn state_memory(state: &ParseState) -> Vec<Value> {
    let fixnum = |n: i64| Value::integer(n).expect("fixnum");
    let mut values: Vec<Value> = [state.depth, i64::from(state.instring), state.incomment, i64::from(state.comstyle),
        i64::from(state.quoted), state.mindepth, state.thislevelstart, state.prevlevelstart, state.location,
        state.comstr_start, i64::from(state.prev_syntax), state.levelstarts.len() as i64].into_iter().map(fixnum).collect();
    values.extend(state.levelstarts.iter().map(|start| start.map_or(Value::NIL, fixnum)));
    values
}
fn memory_state(values: &[Value]) -> ParseState {
    let int = |i: usize| values[i].integer_value().expect("saved parse state");
    let count = int(11) as usize;
    ParseState {
        depth: int(0), instring: int(1) as i32, incomment: int(2), comstyle: int(3) as i32, quoted: int(4) != 0,
        mindepth: int(5), thislevelstart: int(6), prevlevelstart: int(7), location: int(8), comstr_start: int(9),
        prev_syntax: int(10) as i32,
        levelstarts: values[12..12 + count].iter().map(|v| v.integer_value()).collect(),
        ..ParseState::default()
    }
}

impl Leaf<'_> {
    fn syntax_var(&mut self, name: &str) -> std::result::Result<Value, Fault> {
        let symbol = self.intern(name);
        Ok(self.variable_value(symbol)?.unwrap_or(Value::NIL))
    }
    fn fixnum(&self, v: Value) -> std::result::Result<i64, Fault> {
        v.integer_value().ok_or(Fault::Wrong("fixnump", v))
    }
    /// buffer.c fix_position.
    pub(super) fn fix_position(&self, v: Value) -> std::result::Result<i64, Fault> {
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
        match self.vm.objects.entry_mut(index) {
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
        match self.vm.objects.entry_mut(index) {
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
    /// MEMORY is nil on a scan's first run, else what its last stop returned.
    pub(crate) fn syntax_scan(&mut self, op: Op, args: &[Value], answers: Value, memory: Value) -> std::result::Result<Outcome, Fault> {
        // A rerun takes the inputs GNU's scan read once from the first run:
        // its start, coerced positions, initial parse state and variables.
        let memory = if memory == Value::NIL { None } else { Some(self.vector_values(memory)?) };
        let int = |v: Value| v.integer_value().expect("saved scan input");
        // Argument checks, in GNU's order.
        let mut parse = None;
        match op {
            Op::ParsePartialSexp if memory.is_some() => {
                let memory = memory.as_ref().expect("rerun");
                let target = if args[2] == Value::NIL { i64::MIN } else { int(args[2]) };
                let syntax_table = self.intern("syntax-table");
                let commentstop = if args[5] == Value::NIL { 0 } else if args[5] == syntax_table { -1 } else { 1 };
                let state = memory_state(&memory[MEMORY_STATE..]);
                parse = Some((int(memory[3]), int(memory[4]), target, args[3] != Value::NIL, commentstop, state));
            }
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
        let (flags, options) = self.syntax_options(memory.as_ref().map(|memory| int(memory[5])))?;
        let properties = options.propertize_done.is_some();
        let mut ppss = BTreeMap::new();
        let mut tail = answers;
        while let Some((answer, rest)) = self.pair(tail) {
            let (pos, value) = self.pair(answer).expect("saved syntax-ppss answer");
            ppss.insert(pos.integer_value().expect("position"), value.integer_value().expect("position"));
            tail = rest;
        }
        let table = self.vm.current_syntax_table()?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        if let (Op::ParsePartialSexp, None, Some((from, to, target, stopbefore, commentstop, state))) = (op, &memory, &parse) {
            let plain = *target != i64::MIN || *stopbefore || *commentstop != 0;
            if let Some(served) = self.served(index, table, *from, *to, plain, options)? {
                let mut state = state.clone();
                if self.summarized(index, table, &served, options, (*from, *to), &mut state)? {
                    self.syntax_set_point(state.location)?;
                    return Ok(Outcome::Done(self.externalize_parse_state(&state)));
                }
            }
        }
        let (begv, zv, point) = self.syntax_bounds(index)?;
        let epoch = self.vm.defun_start_epoch;
        let (cached, modiff) = {
            let state = self.buffer_state_mut(index);
            (state.defun_start.filter(|&(owner, _)| owner == epoch).map(|(_, cache)| cache), state.modification.modiff)
        };
        let mut view = View::new(self.vm, index, table, properties);
        // Only a stop for Lisp reads the syntax a scan recorded, and a
        // stopped parse goes on where it stopped instead.
        view.record = !matches!(op, Op::ParsePartialSexp) && (properties || options.comment_use_syntax_ppss);
        // A rerun scans from the first run's start, whatever the call did
        // to point or the restriction.
        let mut suspended = None;
        let (begv, zv, point) = match &memory {
            None => (begv, zv, point),
            Some(memory) => {
                let tail = MEMORY_STATE + 12 + int(memory[MEMORY_STATE + 11]) as usize;
                if matches!(op, Op::ParsePartialSexp) {
                    if tail < memory.len() {
                        suspended = Some(memory_suspended(&memory[tail..], |c| scan::Text::pos(&view, c)));
                    }
                } else {
                    view.replay = memory[tail..].iter().map(|&s| int(s) as i32).collect();
                }
                let at = |i: usize| scan::Text::pos(&view, memory[i].integer_value().expect("saved position"));
                (at(0), at(1), at(2))
            }
        };
        let mut scanner = Scanner { text: &view, begv, zv, options, ppss: &ppss, defun_start: cached, modiff };
        let result: std::result::Result<Result_, scan::Stopped> = match op {
            Op::ForwardComment => {
                let count = args[0].integer_value().expect("checked");
                scanner.forward_comment(point, count).map(|(at, found)| Result_::Comment(at, found)).map_err(Into::into)
            }
            Op::ScanLists | Op::ScanSexps => {
                let int = |v: Value| v.integer_value().expect("checked");
                let depth = if let Op::ScanLists = op { int(args[2]) } else { 0 };
                scanner.scan_lists(int(args[0]), int(args[1]), depth, matches!(op, Op::ScanSexps))
                    .map(Result_::Lists).map_err(Into::into)
            }
            Op::ParsePartialSexp => {
                let (from, to, target, stopbefore, commentstop, mut state) = parse.clone().expect("checked");
                if let Some(suspended) = suspended {
                    scanner.resume(suspended, to, target, stopbefore, commentstop).map(Result_::Parse)
                } else {
                    let start = scan::Text::pos(&view, from);
                    scanner.parse(&mut state, start, to, target, stopbefore, commentstop).map(|()| Result_::Parse(state))
                }
            }
            Op::BackwardPrefixChars => scanner.backward_prefix_chars(point).map(Result_::Prefix).map_err(Into::into),
        };
        let written = scanner.defun_start.filter(|&cache| Some(cache) != cached);
        let read = view.read.take();
        drop(view);
        let memory = |leaf: &mut Self, suspended: Option<Box<scan::Suspended>>| {
            let fixnum = |n: i64| Value::integer(n).expect("fixnum");
            let (from, to, state) = match &parse {
                Some((from, to, _, _, _, state)) => (fixnum(*from), fixnum(*to), state.clone()),
                None => (Value::NIL, Value::NIL, ParseState::default()),
            };
            let mut values: Vec<Value> = vec![fixnum(begv.char), fixnum(zv.char), fixnum(point.char), from, to, fixnum(flags)];
            values.extend(state_memory(&state));
            match suspended {
                Some(suspended) => values.extend(suspended_memory(&suspended)),
                None => values.extend(read.iter().map(|&s| fixnum(i64::from(s)))),
            }
            leaf.vector(&values)
        };
        let result = match result {
            Ok(result) => result,
            Err(scan::Stopped { halt: Halt::ScanError(message, a, b), .. }) => return Err(self.scan_error(message, a, b)),
            Err(scan::Stopped { halt: Halt::Propertize(arg, charpos), suspended }) => {
                return Ok(Outcome::Propertize(arg, charpos, memory(self, suspended)));
            }
            Err(scan::Stopped { halt: Halt::Ppss(pos), suspended }) => return Ok(Outcome::Ppss(pos, memory(self, suspended))),
        };
        if let Some(cache) = written {
            self.vm.defun_start_epoch += 1;
            let epoch = self.vm.defun_start_epoch;
            self.buffer_state_mut(index).defun_start = Some((epoch, cache));
        }
        let set_point = |leaf: &mut Self, at: Pos| match leaf.vm.objects.entry_mut(index) {
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

    /// Whether the summaries of the current buffer's text serve a parse
    /// from FROM to TO, PLAIN when its arguments ask for a stop: a long
    /// enough span of a multibyte text whose `syntax-table' properties, if
    /// honoured, are propertized past TO, so that no scan of it would stop
    /// for Lisp. Counts the reason when they do not; when they do, gives
    /// the summaries the property changes since they last served.
    fn served(&mut self, index: usize, table: Value, from: i64, to: i64, plain: bool, options: scan::Options)
        -> std::result::Result<Option<Served>, Fault> {
        if !matches!(self.vm.objects[index], data::Object::Buffer(_)) { return Ok(None); }
        let decline = if !SyntaxIndex::pays(to - from) {
            Some(Decline::Short)
        } else if plain {
            Some(Decline::Arguments)
        } else if self.syntax_var("romacs-inhibit-syntax-summaries")? != Value::NIL {
            Some(Decline::Inhibited)
        } else if let Some(done) = options.propertize_done.filter(|&done| done <= to)
            && done < self.syntax_bounds(index)?.1.char {
            // update_forward stops for Lisp at `syntax-propertize--done' and
            // past it, short of ZV, and forw_comment updates at TO itself.
            Some(Decline::Properties)
        } else {
            None
        };
        let text: &crate::text::Text = buffer_text(self.vm, index);
        if let Some(decline) = decline {
            text.syntax.borrow_mut().stats.declined[decline as usize] += 1;
            return Ok(None);
        }
        // Summaries hold no Values: only taking property changes needs the
        // entry, and so the barrier.
        if text.properties.has_syntax_changes() {
            let data::Object::Buffer(buffer) = self.vm.objects.entry_mut(index) else { unreachable!("checked above") };
            buffer.sync_syntax_properties();
        }
        let check = self.syntax_var("romacs-syntax-summaries-check")? != Value::NIL;
        let context = Context {
            table: self.vm.char_table_serial(table),
            writes: crate::char_table::writes(),
            escaped_ends: options.comment_end_can_be_escaped,
            properties: options.propertize_done.is_some(),
        };
        Ok(Some(Served { context, check }))
    }

    /// The variables a scan obeys, from SAVED (a rerun's flags) or read
    /// now, as flags and as the engine's options.
    fn syntax_options(&mut self, saved: Option<i64>) -> std::result::Result<(i64, scan::Options), Fault> {
        const FLAGS: [&str; 6] = ["parse-sexp-lookup-properties", "parse-sexp-ignore-comments", "comment-end-can-be-escaped",
            "multibyte-syntax-as-symbol", "open-paren-in-column-0-is-defun-start", "comment-use-syntax-ppss"];
        let flags = match saved {
            Some(flags) => flags,
            None => {
                let mut flags = 0;
                for (bit, name) in FLAGS.iter().enumerate() {
                    if self.syntax_var(name)? != Value::NIL { flags |= 1 << bit; }
                }
                flags
            }
        };
        let flag = |bit: usize| flags & (1 << bit) != 0;
        let options = scan::Options {
            ignore_comments: flag(1),
            comment_end_can_be_escaped: flag(2),
            multibyte_syntax_as_symbol: flag(3),
            open_paren_in_column_0_is_defun_start: flag(4),
            comment_use_syntax_ppss: flag(5),
            propertize_done: if flag(0) {
                Some(self.syntax_var("syntax-propertize--done")?.integer_value().unwrap_or(-1))
            } else {
                None
            },
        };
        Ok((flags, options))
    }

    /// The summaries' parse of SPAN (character positions) of the buffer at
    /// INDEX under TABLE, from STATE into STATE: false, STATE as it was,
    /// where they do not serve it. Under `romacs-syntax-summaries-check' the
    /// scanner parses too, and a difference signals.
    fn summarized(&mut self, index: usize, table: Value, served: &Served, options: scan::Options,
                  (from, to): (i64, i64), state: &mut ParseState) -> std::result::Result<bool, Fault> {
        let (begv, zv, _) = self.syntax_bounds(index)?;
        let mut view = View::new(self.vm, index, table, options.propertize_done.is_some());
        view.record = false;
        let Body::Multibyte(text, _) = &view.body else { return Ok(false) };
        let ppss = BTreeMap::new();
        let mut scanner = Scanner { text: &view, begv, zv, options, ppss: &ppss, defun_start: None, modiff: 0 };
        let stopped = |_| Fault::Error("syntax summaries: a served parse stopped for Lisp".into());
        let start = state.clone();
        let summarized = text.syntax.borrow_mut()
            .parse(&mut scanner, text.len_chars(), served.context.clone(), from, to, state).map_err(stopped)?;
        if summarized && served.check {
            let mut plain = start;
            scanner.scan_sexps_forward(&mut plain, scan::Text::pos(&view, from), to, i64::MIN, false, 0).map_err(stopped)?;
            if plain != *state {
                return Err(Fault::Error(format!("romacs-syntax-summaries-check: {from}..{to}: summaries {state:?}, scanner {plain:?}")));
            }
        }
        Ok(summarized)
    }

    /// `romacs-syntax-summaries-stats' of the current buffer.
    pub(crate) fn syntax_summaries_stats(&mut self) -> super::super::Out {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::Buffer(_) = &self.vm.objects[index] else { return Ok(Value::NIL) };
        let (stats, leaves, bytes) = {
            let summaries = buffer_text(self.vm, index).syntax.borrow();
            (summaries.stats, summaries.leaves(), summaries.storage_bytes())
        };
        let int = |n: u64| Value::integer(n as i64).expect("count");
        let mut declined = Value::NIL;
        for decline in Decline::ALL.into_iter().rev() {
            let name = self.intern(decline.name());
            let pair = self.cons(name, int(stats.declined[decline as usize]));
            declined = self.cons(pair, declined);
        }
        let items = [
            ("served", int(stats.served)), ("declined", declined), ("leaves", int(leaves as u64)),
            ("leaf-scans", int(stats.leaf_scans)), ("plain-leaves", int(stats.plain_leaves)),
            ("composed", int(stats.composed)), ("context-switches", int(stats.switches)),
            ("bytes", int(bytes as u64)), ("char-table-writes", int(crate::char_table::writes())),
        ];
        let mut plist = Value::NIL;
        for (key, value) in items.into_iter().rev() {
            let key = self.intern(&format!(":{key}"));
            plist = self.cons(value, plist);
            plist = self.cons(key, plist);
        }
        Ok(plist)
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

#[cfg(test)]
mod tests {
    use crate::Vm;

    /// A parse that stops for `internal--syntax-propertize` at every chunk
    /// reads each character once, as GNU's one scan does, however many
    /// stops it makes: a stop resumes the scan rather than restarting it.
    #[test]
    fn a_propertized_parse_reads_each_character_once() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let span: usize = 400_000;
        let source = format!(
            "(with-temp-buffer
               (dotimes (_ {lines}) (insert \"(defun f (a) \\\"doc (\\\" (list a ?\\\\( 'b)) ; c\\n\"))
               (set (make-local-variable 'parse-sexp-lookup-properties) t)
               (fset 'internal--syntax-propertize
                     (lambda (pos) (setq syntax-propertize--done (min (point-max) (+ pos 2000)))))
               (setq syntax-propertize--done 1)
               (parse-partial-sexp (point-min) (point-max))
               (- (point-max) (point-min)))",
            lines = span / 45);
        let before = super::READS.with(std::cell::Cell::get);
        let chars = vm.eval_source_raw(&source).unwrap().integer_value().unwrap();
        let reads = super::READS.with(std::cell::Cell::get) - before;
        assert!(chars > 350_000, "{chars}");
        assert!(reads <= 2 * usize::try_from(chars).unwrap(), "{reads} syntax reads for {chars} characters");
    }
}
