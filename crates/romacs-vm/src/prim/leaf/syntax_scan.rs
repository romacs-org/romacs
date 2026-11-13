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
    /// The text and, when the scan honours them, a reader of its
    /// `syntax-table` properties.
    Multibyte(&'a crate::text::Text, Option<crate::properties::KeyReader<'a>>),
    Unibyte(&'a crate::bytes::ByteBuffer),
}

#[cfg(test)]
thread_local! {
    /// The syntax reads of every `View` on this thread.
    static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The characters every `View` on this thread passed without a read.
    static PASSED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The current buffer as `crate::syntax_scan::Text`. Every read is live: a
/// scan stopped for Lisp goes on where it stopped, so it reads no position
/// twice.
struct View<'a> {
    vm: &'a Vm,
    body: Body<'a>,
    table: crate::char_table::TableReader<'a>,
    properties: bool,
    /// The codes of the ASCII characters this scan has read, each `UNREAD`
    /// until then: an entry is a cons a program may alter without writing
    /// the table, so its code is read from it once a scan, during which no
    /// program runs.
    ascii: [std::cell::Cell<i32>; 128],
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

/// No `syntax-ppss` answer yet, as on every scan that never stops: one
/// map for all of them.
static UNANSWERED: BTreeMap<i64, i64> = BTreeMap::new();

/// What `View::ascii` holds for a character not read yet: no syntax code,
/// which is a class and flags in the low 24 bits.
const UNREAD: i32 = i32::MIN;

/// SYNTAX_WITH_FLAGS of C in TABLE: the car of its (CODE . MATCH) entry.
fn entry_code(vm: &Vm, table: &crate::char_table::TableReader<'_>, c: u32) -> i32 {
    cons_code(vm, table.get(c).unwrap_or(Value::NIL))
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
                Body::Multibyte(text, properties.then(|| text.properties.reader("syntax-table")))
            }
            data::Object::ByteBuffer(b) => Body::Unibyte(b),
            _ => unreachable!("buffer_index validates"),
        };
        Self {
            vm, body, table: vm.char_table_reader(table), properties,
            // One value throughout, which fills as one block: a closure
            // called for each slot, or a tag stored in each, cost more
            // than a short scan.
            ascii: [const { std::cell::Cell::new(UNREAD) }; 128],
            last: std::cell::Cell::new((usize::MAX, 0)),
        }
    }
    fn len(&self) -> usize {
        match self.body {
            Body::Multibyte(t, _) => t.len_bytes(),
            Body::Unibyte(b) => b.bytes.len(),
        }
    }
    fn property(&self, byte: usize) -> Option<Value> {
        match self.body {
            Body::Multibyte(_, ref reader) => reader.as_ref()?.get(ByteOffset(byte)),
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
        let c = self.char_at(at);
        if self.properties {
            if let Some(value) = self.property(at.byte) {
                if self.vm.is_syntax_table(value) { return cons_code(self.vm, self.vm.char_table_get(value, c as usize).unwrap_or(Value::NIL)); }
                if value.tag() == 3 { return cons_code(self.vm, value); }
            }
        }
        self.table_syntax(c)
    }
    fn table_syntax(&self, c: u32) -> i32 {
        let Some(memo) = self.ascii.get(c as usize) else { return entry_code(self.vm, &self.table, c) };
        let read = memo.get();
        if read != UNREAD {
            return read;
        }
        let code = entry_code(self.vm, &self.table, c);
        memo.set(code);
        code
    }
    fn pass_comment_plain(&self, at: Pos, limit: i64) -> (Pos, Option<i32>) {
        let Body::Multibyte(text, ref reader) = self.body else { return (at, None) };
        let len = text.len_bytes();
        let (mut pos, mut last) = (at, None);
        while pos.char < limit && pos.byte < len {
            // Only where the table alone gives the syntax.
            let unset_until = reader.as_ref().map_or(usize::MAX, |reader| reader.unset_until(ByteOffset(pos.byte)));
            let room = (unset_until.saturating_sub(pos.byte)).min(usize::try_from(limit - pos.char).unwrap_or(usize::MAX));
            if room == 0 {
                break;
            }
            let (count, whole) = {
                let piece = text.chunk_from(pos.byte);
                let bytes = &piece[..piece.len().min(room)];
                let count = bytes.iter().position(|&byte| {
                    let plain = byte < 0x80 && scan::comment_plain(self.table_syntax(u32::from(byte)));
                    if plain { last = Some(byte); }
                    !plain
                });
                (count.unwrap_or(bytes.len()), count.is_none())
            };
            // ASCII: as many characters as bytes.
            pos = Pos { char: pos.char + i64::try_from(count).expect("a run inside the text"), byte: pos.byte + count };
            if !whole {
                break;
            }
        }
        #[cfg(test)]
        PASSED.with(|n| n.set(n.get() + usize::try_from(pos.char - at.char).unwrap_or(0)));
        (pos, last.map(|byte| self.table_syntax(u32::from(byte))))
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
    /// start's BEGV, ZV and point, then where the scan stopped
    /// (`MEMORY_STATE').
    Propertize(i64, i64, Value),
    Ppss(i64, Value),
}

enum Result_ { Comment(Pos, bool), Lists(Option<i64>), Parse(ParseState), Prefix(Pos) }

/// What a stopped scan goes on from.
enum Kept { Parse(Box<scan::Suspended>), Scan(Box<scan::Scan>) }

/// A parse the text's summaries serve (`crate::syntax_index`), and whether
/// the plain scanner checks it.
struct Served {
    context: Context,
    check: bool,
}


/// A scan's memory: the first run's BEGV, ZV and point, coerced FROM and
/// TO, its variables as flags, its initial parse state (`state_memory`),
/// then where it stopped (`suspended_memory` for a parse, `scan_memory` for
/// another scan); nothing when it stopped before reading.
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
    values.extend(forw_memory(suspended.forw.as_ref()));
    values.extend(state_memory(&suspended.state));
    values
}

/// A stopped comment scan as six fixnums, nil for none.
fn forw_memory(forw: Option<&scan::Forw>) -> Vec<Value> {
    let fixnum = |n: i64| Value::integer(n).expect("fixnum");
    let Some(f) = forw else { return vec![Value::NIL] };
    let mut values = vec![label_code(&scan::ForwAt::ALL, &f.at)];
    values.extend([f.from.char, f.nesting, i64::from(f.syntax), i64::from(f.code), i64::from(f.prev_syntax)].map(fixnum));
    values
}

/// `forw_memory` read back.
fn memory_forw(values: &[Value], pos: impl Fn(i64) -> Pos) -> Option<scan::Forw> {
    if values[0] == Value::NIL { return None; }
    let int = |i: usize| values[i].integer_value().expect("saved comment scan");
    let syntax = |i: usize| i32::try_from(int(i)).expect("saved syntax");
    let at = scan::ForwAt::ALL[usize::try_from(int(0)).expect("saved label")];
    Some(scan::Forw { at, from: pos(int(1)), nesting: int(2), syntax: syntax(3), code: syntax(4), prev_syntax: syntax(5) })
}

/// A stopped `scan-lists`, `scan-sexps` or `forward-comment` as fixnums:
/// its label, its locals (positions as characters), then its comment scan.
fn scan_memory(s: &scan::Scan) -> Vec<Value> {
    let mut values = vec![label_code(&scan::ScanAt::ALL, &s.at)];
    values.extend([s.from.char, s.count, s.depth, s.min_depth, i64::from(s.mathexit), s.last_good, i64::from(s.c),
        i64::from(s.syntax), i64::from(s.code), i64::from(s.comstyle), i64::from(s.comnested), s.mark.char]
        .map(|n| Value::integer(n).expect("fixnum")));
    values.extend(forw_memory(s.forw.as_ref()));
    values
}

/// `scan_memory` read back, POS giving a character's position.
fn memory_scan(values: &[Value], pos: impl Fn(i64) -> Pos) -> scan::Scan {
    let int = |i: usize| values[i].integer_value().expect("saved scan");
    let syntax = |i: usize| i32::try_from(int(i)).expect("saved syntax");
    scan::Scan {
        at: scan::ScanAt::ALL[usize::try_from(int(0)).expect("saved label")],
        from: pos(int(1)), count: int(2), depth: int(3), min_depth: int(4), mathexit: int(5) != 0, last_good: int(6),
        c: u32::try_from(int(7)).expect("saved character"), syntax: syntax(8), code: syntax(9), comstyle: syntax(10),
        comnested: int(11) != 0, mark: pos(int(12)), forw: memory_forw(&values[13..], pos),
    }
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
    let forw = memory_forw(&values[at..], &pos);
    let state = at + if forw.is_some() { 6 } else { 1 };
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
    /// `forward-comment-function', which replaces `forward-comment' when set.
    pub(crate) fn forward_comment_function(&mut self) -> std::result::Result<Value, Fault> {
        let function = self.vm.forward_comment_function().map_err(Fault::Error)?;
        Ok(if function == Value::UNBOUND { Value::NIL } else { function })
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
        let syntax_table = self.vm.syntax_table_symbol();
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
                // The text's own ends are known without a search for them,
                // which would also displace the one position it remembers.
                let at = |byte: ByteOffset| -> std::result::Result<Pos, Fault> {
                    let before = if byte.0 == 0 { 0 } else if byte.0 == b.len_bytes() { b.len_chars() } else { b.char_of_byte(byte).map_err(Fault::Error)?.0 };
                    Ok(Pos { char: before as i64 + 1, byte: byte.0 })
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
    fn state_at(&self, index: usize) -> &crate::bytes::BufferState {
        match &self.vm.objects[index] {
            data::Object::Buffer(b) => &b.state,
            data::Object::ByteBuffer(b) => &b.state,
            _ => unreachable!("buffer_index validates"),
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
        Ok(self.state_at(index).modification.chars_modiff as i64)
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
    /// defun start `syntax-ppss` gave the call the last run stopped for.
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
                let syntax_table = self.vm.syntax_table_symbol();
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
                let syntax_table = self.vm.syntax_table_symbol();
                let commentstop = if args[5] == Value::NIL { 0 } else if args[5] == syntax_table { -1 } else { 1 };
                parse = Some((from, to, target, args[3] != Value::NIL, commentstop, state));
            }
            Op::BackwardPrefixChars => {}
        }
        let (flags, options) = self.syntax_options(memory.as_ref().map(|memory| int(memory[5])))?;
        let properties = options.propertize_done.is_some();
        let mut answered = BTreeMap::new();
        let mut tail = answers;
        while let Some((answer, rest)) = self.pair(tail) {
            let (pos, value) = self.pair(answer).expect("saved syntax-ppss answer");
            answered.insert(pos.integer_value().expect("position"), value.integer_value().expect("position"));
            tail = rest;
        }
        let ppss = if answers == Value::NIL { &UNANSWERED } else { &answered };
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let table = self.vm.buffer_syntax_table(index);
        // The scan reads the table through its flat view, kept between calls.
        self.vm.flatten_table(table);
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
            let state = self.state_at(index);
            (state.defun_start.filter(|&(owner, _)| owner == epoch).map(|(_, cache)| cache), state.modification.modiff)
        };
        let view = View::new(self.vm, index, table, properties);
        // A rerun scans in the first run's bounds, whatever the call did
        // to point or the restriction, and goes on where it stopped.
        let pos = |c: i64| scan::Text::pos(&view, c);
        let mut kept = None;
        let (begv, zv, point) = match &memory {
            None => (begv, zv, point),
            Some(memory) => {
                let tail = MEMORY_STATE + 12 + usize::try_from(int(memory[MEMORY_STATE + 11])).expect("saved levels");
                if tail < memory.len() {
                    kept = Some(if matches!(op, Op::ParsePartialSexp) {
                        Kept::Parse(Box::new(memory_suspended(&memory[tail..], pos)))
                    } else {
                        Kept::Scan(Box::new(memory_scan(&memory[tail..], pos)))
                    });
                }
                (pos(int(memory[0])), pos(int(memory[1])), pos(int(memory[2])))
            }
        };
        let mut scanner = Scanner { text: &view, begv, zv, options, ppss, defun_start: cached, modiff };
        let scan_stopped = |stopped: scan::Stopped<Box<scan::Scan>>| (stopped.halt, stopped.suspended.map(Kept::Scan));
        let parse_stopped = |stopped: scan::Stopped| (stopped.halt, stopped.suspended.map(Kept::Parse));
        let result: std::result::Result<Result_, (Halt, Option<Kept>)> = match (op, kept) {
            (Op::ForwardComment, Some(Kept::Scan(scan))) => scanner.comments(*scan).map(|(at, found)| Result_::Comment(at, found)).map_err(scan_stopped),
            (Op::ForwardComment, _) => {
                let count = args[0].integer_value().expect("checked");
                scanner.forward_comment(point, count).map(|(at, found)| Result_::Comment(at, found)).map_err(scan_stopped)
            }
            (Op::ScanLists | Op::ScanSexps, Some(Kept::Scan(scan))) => {
                scanner.lists(*scan, matches!(op, Op::ScanSexps)).map(Result_::Lists).map_err(scan_stopped)
            }
            (Op::ScanLists | Op::ScanSexps, _) => {
                let int = |v: Value| v.integer_value().expect("checked");
                let depth = if let Op::ScanLists = op { int(args[2]) } else { 0 };
                scanner.scan_lists(int(args[0]), int(args[1]), depth, matches!(op, Op::ScanSexps))
                    .map(Result_::Lists).map_err(scan_stopped)
            }
            (Op::ParsePartialSexp, kept) => {
                let (from, to, target, stopbefore, commentstop, mut state) = parse.clone().expect("checked");
                if let Some(Kept::Parse(suspended)) = kept {
                    scanner.resume(*suspended, to, target, stopbefore, commentstop).map(Result_::Parse).map_err(parse_stopped)
                } else {
                    scanner.parse(&mut state, pos(from), to, target, stopbefore, commentstop)
                        .map(|()| Result_::Parse(state)).map_err(parse_stopped)
                }
            }
            (Op::BackwardPrefixChars, _) => scanner.backward_prefix_chars(point).map(Result_::Prefix).map_err(|halt| (halt, None)),
        };
        let written = scanner.defun_start.filter(|&cache| Some(cache) != cached);
        let memory = |leaf: &mut Self, kept: Option<Kept>| {
            let fixnum = |n: i64| Value::integer(n).expect("fixnum");
            let (from, to, state) = match &parse {
                Some((from, to, _, _, _, state)) => (fixnum(*from), fixnum(*to), state.clone()),
                None => (Value::NIL, Value::NIL, ParseState::default()),
            };
            let mut values: Vec<Value> = vec![fixnum(begv.char), fixnum(zv.char), fixnum(point.char), from, to, fixnum(flags)];
            values.extend(state_memory(&state));
            match kept {
                Some(Kept::Parse(suspended)) => values.extend(suspended_memory(&suspended)),
                Some(Kept::Scan(scan)) => values.extend(scan_memory(&scan)),
                None => {}
            }
            leaf.vector(&values)
        };
        let result = match result {
            Ok(result) => result,
            Err((Halt::ScanError(message, a, b), _)) => return Err(self.scan_error(message, a, b)),
            Err((Halt::Propertize(arg, charpos), kept)) => return Ok(Outcome::Propertize(arg, charpos, memory(self, kept))),
            Err((Halt::Ppss(pos), kept)) => return Ok(Outcome::Ppss(pos, memory(self, kept))),
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
        // `crate::regexp::SCAN_VARIABLES': six flags, then `syntax-propertize--done'.
        let [flag_values @ .., done] = self.vm.scan_variables().map_err(Fault::Error)?;
        let set = |value: Value| value != Value::NIL && value != Value::UNBOUND;
        let flags = saved.unwrap_or_else(|| {
            flag_values.iter().enumerate().fold(0, |flags, (bit, &value)| if set(value) { flags | 1 << bit } else { flags })
        });
        let flag = |bit: usize| flags & (1 << bit) != 0;
        let options = scan::Options {
            ignore_comments: flag(1),
            comment_end_can_be_escaped: flag(2),
            multibyte_syntax_as_symbol: flag(3),
            open_paren_in_column_0_is_defun_start: flag(4),
            comment_use_syntax_ppss: flag(5),
            propertize_done: flag(0).then(|| done.integer_value().unwrap_or(-1)),
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
        let view = View::new(self.vm, index, table, options.propertize_done.is_some());
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
            ("log-overflows", int(stats.log_overflows)),
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

    /// The current buffer's syntax table, its flat view made current
    /// (`crate::char_table::flat`) for the readers this call makes of it.
    fn syntax_table_to_read(&mut self) -> std::result::Result<Value, Fault> {
        let table = self.vm.current_syntax_table()?;
        self.vm.flatten_table(table);
        Ok(table)
    }
    /// Fchar_syntax.
    pub fn char_syntax(&mut self, character: Value) -> super::super::Out {
        let mut c = self.syntax_character(character)?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        if matches!(self.vm.objects[index], data::Object::ByteBuffer(_)) && (128..256).contains(&c) {
            c += crate::lstring::BYTE8_OFFSET;
        }
        let table = self.syntax_table_to_read()?;
        let code = entry_code(self.vm, &self.vm.char_table_reader(table), c) & 0xff;
        let spec = scan::SYNTAX_CODE_SPEC.get(code as usize).copied().unwrap_or(b' ');
        Ok(Value::integer(i64::from(spec)).expect("character"))
    }
    /// Fmatching_paren.
    pub fn matching_paren(&mut self, character: Value) -> super::super::Out {
        let c = self.syntax_character(character)?;
        let table = self.syntax_table_to_read()?;
        let entry = self.vm.char_table_reader(table).get(c).map_err(Fault::Error)?;
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

    /// The syntax reads of SOURCE, which evaluates to the characters it scans.
    fn reads_per_character(source: &str) -> (usize, usize) {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let before = super::READS.with(std::cell::Cell::get);
        let chars = vm.eval_source_raw(source).unwrap().integer_value().unwrap();
        (super::READS.with(std::cell::Cell::get) - before, usize::try_from(chars).unwrap())
    }

    /// `scan-sexps` over a list that stops for `internal--syntax-propertize`
    /// at every chunk reads each character a constant number of times,
    /// however many stops it makes: what `electric-pair-mode` asks on every
    /// opener typed, after the edit left the rest of the buffer to propertize.
    #[test]
    fn a_propertized_list_scan_reads_each_character_a_few_times() {
        let source = format!(
            "(with-temp-buffer
               (insert \"(\")
               (dotimes (_ {lines}) (insert \"(defun f (a) \\\"doc (\\\" (list a ?\\\\( 'b)) ; c\\n\"))
               (insert \")\")
               (set (make-local-variable 'parse-sexp-lookup-properties) t)
               (fset 'internal--syntax-propertize
                     (lambda (pos) (setq syntax-propertize--done (min (point-max) (+ pos 2000)))))
               (setq syntax-propertize--done 1)
               (if (eq (scan-sexps (point-min) 1) (point-max)) (- (point-max) (point-min))))",
            lines = 400_000 / 45);
        let (reads, chars) = reads_per_character(&source);
        assert!(chars > 350_000, "{chars}");
        assert!(reads <= 3 * chars, "{reads} syntax reads for {chars} characters");
    }

    /// So too `forward-comment` over many comments.
    #[test]
    fn a_propertized_comment_scan_reads_each_character_a_few_times() {
        let source = format!(
            "(with-temp-buffer
               (let ((st (make-char-table 'syntax-table)))
                 (set-char-table-parent st (standard-syntax-table))
                 (modify-syntax-entry ?\\; \"<\" st) (modify-syntax-entry ?\\n \">\" st) (set-syntax-table st))
               (dotimes (_ {lines}) (insert \"  ;; a comment (with \\\"text\\\" in it)\\n\\n\"))
               (set (make-local-variable 'parse-sexp-lookup-properties) t)
               (fset 'internal--syntax-propertize
                     (lambda (pos) (setq syntax-propertize--done (min (point-max) (+ pos 2000)))))
               (setq syntax-propertize--done 1)
               (goto-char (point-min))
               (forward-comment (buffer-size))
               (if (eobp) (- (point-max) (point-min))))",
            lines = 400_000 / 40);
        let (reads, chars) = reads_per_character(&source);
        assert!(chars > 350_000, "{chars}");
        assert!(reads <= 3 * chars, "{reads} syntax reads for {chars} characters");
    }

    /// And a backward `scan-lists` that stops for `syntax-ppss` at every
    /// comment `back_comment` cannot decode from behind (one line in 200
    /// here): what `electric-pair-mode` asks on every closer typed.
    #[test]
    fn a_backward_list_scan_through_syntax_ppss_reads_each_character_a_few_times() {
        let source = format!(
            "(with-temp-buffer
               (let ((st (make-char-table 'syntax-table)))
                 (set-char-table-parent st (standard-syntax-table))
                 (modify-syntax-entry ?\\; \"<\" st) (modify-syntax-entry ?\\n \">\" st) (set-syntax-table st))
               (insert \"(\\n\")
               (dotimes (i {lines})
                 (insert (if (eq (% i 200) 0) \"(f a \\\"s\\\") ; odd \\\"\\n\" \"(f a \\\"s\\\") ; plain\\n\")))
               (insert \")\")
               (setq parse-sexp-ignore-comments t comment-use-syntax-ppss t stops 0)
               (fset 'syntax-ppss (lambda (pos) (setq stops (1+ stops)) (list 0 nil nil nil t nil 0 nil (- pos 7) nil)))
               (if (and (eq (scan-lists (point-max) -1 0) (point-min)) (> stops 100)) (- (point-max) (point-min))))",
            lines = 400_000 / 18);
        let (reads, chars) = reads_per_character(&source);
        assert!(chars > 350_000, "{chars}");
        assert!(reads <= 12 * chars, "{reads} syntax reads for {chars} characters");
    }

    /// The body of a comment is passed, not read a character at a time:
    /// cc-mode scans the same long comment several times on every key.
    /// So too by `forward-comment`, and up to a `syntax-table` property,
    /// which is read.
    #[test]
    fn a_comment_body_is_passed_without_reads() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let setup = "(progn (set-buffer (get-buffer-create \"comment\")) (erase-buffer)
               (let ((st (make-char-table 'syntax-table)))
                 (set-char-table-parent st (standard-syntax-table))
                 (modify-syntax-entry ?/ \". 14\" st) (modify-syntax-entry ?* \". 23\" st) (set-syntax-table st))
               (insert \"a /* \" (make-string 20000 ?x) \" */ b\")
               (setq syntax-propertize--done most-positive-fixnum))";
        vm.eval_source_raw(setup).unwrap();
        let counted = |vm: &mut Vm, source: &str, expected: &str| {
            let before = (super::READS.with(std::cell::Cell::get), super::PASSED.with(std::cell::Cell::get));
            let value = vm.eval_source_raw(source).unwrap();
            assert_eq!(vm.display(value).unwrap(), expected, "{source}");
            (super::READS.with(std::cell::Cell::get) - before.0, super::PASSED.with(std::cell::Cell::get) - before.1)
        };
        for source in [
            "(nth 4 (parse-partial-sexp 1 (point-max)))",
            "(let ((parse-sexp-lookup-properties t)) (nth 4 (parse-partial-sexp 1 (point-max))))",
            "(let ((s (parse-partial-sexp 1 10))) (parse-partial-sexp 10 (point-max) nil nil s 'syntax-table) (- (point) 20009))",
            "(let ((parse-sexp-lookup-properties t)) (goto-char 3) (forward-comment 1) (- (point) 20009))",
            "(progn (goto-char 3) (forward-comment 1) (- (point) 20009))",
        ] {
            // A parse to a position, not to a stop, may be answered from
            // the text's summaries instead of scanned at all.
            let scanned = source.starts_with("(let ((s") || source.contains("forward-comment");
            let (reads, passed) = counted(&mut vm, source, if scanned { "0" } else { "nil" });
            assert!(reads <= 40 && (!scanned || passed >= 19_990), "{source}: {reads} syntax reads, {passed} characters passed");
        }
        // A property in the middle is read; the text around it is passed.
        vm.eval_source_raw("(put-text-property 9000 9001 'syntax-table (string-to-syntax \"w\"))").unwrap();
        let (reads, passed) = counted(&mut vm, "(let ((parse-sexp-lookup-properties t)) (nth 4 (parse-partial-sexp 1 (point-max))))", "nil");
        assert!(reads <= 40, "{reads} syntax reads, {passed} characters passed");
        let (reads, passed) = counted(&mut vm, "(let ((parse-sexp-lookup-properties t)) (goto-char 3) (forward-comment 1) (- (point) 20009))", "0");
        assert!(reads <= 40 && passed >= 19_980, "{reads} syntax reads, {passed} characters passed");
    }
}
