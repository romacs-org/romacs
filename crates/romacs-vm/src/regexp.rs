//! Lisp search boundary. Only owned text and integer offsets reach the engine.
use crate::{coordinates::*, data::Object, *};
use romacs_regex_engine::{self as regex, EmacsStr, MatchRegisters};
use std::ops::Range;
#[path = "regexp_syntax.rs"]
mod syntax;
pub(crate) use syntax::{parse_syntax_descriptor, VmSyntax};

#[derive(Default)]
pub(super) struct MatchState {
    // Character positions, string-relative or one-based buffer positions.
    // No heap indices: collection may move every VM object between calls.
    pub(crate) groups: Vec<Option<(usize, usize)>>,
    pub(crate) succeeded: bool,
}
romacs_snapshot::snapshot_struct!(MatchState { groups, succeeded });

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
            | Primitive::PosixLookingAt
            | Primitive::PosixStringMatch
            | Primitive::PosixSearchForward
            | Primitive::PosixSearchBackward
            | Primitive::SearchForward
            | Primitive::SearchBackward
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
    /// Whether a character is a byte: unibyte or all-ASCII text.
    bytewise: bool,
    /// Known (byte, characters starting before it) pairs: the start, the
    /// end and the last conversion. A conversion counts characters only
    /// from the nearest, so a match near the last costs the distance from
    /// it, not the length of the text.
    anchors: std::cell::Cell<[(usize, usize); 3]>,
}
/// Internal multibyte text starts each character with one byte that is not
/// a continuation byte (10xxxxxx), so counting those counts characters.
fn starts_character(byte: u8) -> bool {
    byte & 0xc0 != 0x80
}
fn characters_in(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| starts_character(b)).count()
}
impl Input {
    /// BYTES, which hold CHARS characters.
    fn new(bytes: Vec<u8>, chars: usize, multibyte: bool) -> Self {
        let bytewise = !multibyte || bytes.is_ascii();
        let end = (bytes.len(), chars);
        Input { bytes, multibyte, bytewise, anchors: std::cell::Cell::new([(0, 0), end, end]) }
    }
    fn len(&self) -> usize {
        self.anchors.get()[1].1
    }
    fn remember(&self, byte: usize, chars: usize) {
        let mut anchors = self.anchors.get();
        anchors[2] = (byte, chars);
        self.anchors.set(anchors);
    }
    /// The byte where character CHARACTER starts, or the end.
    fn byte(&self, character: usize) -> usize {
        if self.bytewise {
            return character;
        }
        let (at, chars) = self.anchors.get().into_iter().min_by_key(|&(_, chars)| chars.abs_diff(character)).unwrap();
        let byte = if character >= chars {
            self.bytes[at..].iter().enumerate().filter(|&(_, &b)| starts_character(b))
                .nth(character - chars).map_or(self.bytes.len(), |(i, _)| at + i)
        } else {
            self.bytes[..at].iter().enumerate().rev().filter(|&(_, &b)| starts_character(b))
                .nth(chars - character - 1).map_or(0, |(i, _)| i)
        };
        if byte < self.bytes.len() {
            self.remember(byte, character);
        }
        byte
    }
    /// The characters that start before BYTE.
    fn character(&self, byte: usize) -> usize {
        if self.bytewise {
            return byte;
        }
        let byte = byte.min(self.bytes.len());
        let (at, chars) = self.anchors.get().into_iter().min_by_key(|&(at, _)| at.abs_diff(byte)).unwrap();
        let counted = if byte >= at {
            chars + characters_in(&self.bytes[at..byte])
        } else {
            chars - characters_in(&self.bytes[byte..at])
        };
        self.remember(byte, counted);
        counted
    }
}

/// Bytes on either side of what a buffer search reaches that its window
/// also holds: an assertion reads at most the character before or after
/// a position (`\b', `^', `$', `\_<'), four bytes at most, so a window
/// that does not end at the accessible text's bound gives the engine no
/// false bound there. Edit-and-search loops move the gap about twice this
/// per step.
const SEARCH_MARGIN: usize = 16;

/// A run of the current buffer's accessible text for one search, read in
/// place: the engine gets its bytes and positions convert through the
/// buffer's own character index, so a search costs what it reads, not the
/// buffer.
struct BufferInput<'a> {
    bytes: &'a [u8],
    /// None for a unibyte buffer, whose characters are its bytes.
    text: Option<&'a crate::text::TextBody>,
    /// The byte of the text BYTES start at.
    start: usize,
    /// The one-based character position of the accessible text's start,
    /// from which [`Self::character`] counts.
    base_char: usize,
    /// Point, as a byte of BYTES.
    point: usize,
}
impl BufferInput<'_> {
    /// The characters of the accessible text before byte BYTE of BYTES.
    fn character(&self, byte: usize) -> SearchResult<usize> {
        let Some(t) = self.text else { return Ok(byte) };
        Ok(t.char_of_byte(ByteOffset(self.start + byte))?.0 + 1 - self.base_char)
    }
}

/// REGISTERS as group spans at the positions AT gives their bytes.
fn register_spans(registers: &MatchRegisters, at: impl Fn(usize) -> SearchResult<usize>)
    -> SearchResult<Vec<Option<(usize, usize)>>> {
    registers.start.iter().zip(&registers.end)
        .map(|(&a, &b)| (a >= 0 && b >= 0).then(|| Ok((at(a as usize)?, at(b as usize)?))).transpose())
        .collect()
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

/// What a cached pattern folds case with: nothing, the standard table,
/// or the completion translation over a buffer's case-canon table.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Translation { None, Standard, Completion(Value) }
romacs_snapshot::snapshot_enum!(Translation {
    Translation::None => (),
    Translation::Standard => (),
    Translation::Completion(f0) => (f0),
});

/// search.c's searchbufs: compiled patterns by source, so a pattern
/// searched again is not compiled again (compile_pattern). A pattern whose
/// compilation read the syntax table is kept for that table. GNU keeps
/// 20; one major mode's font-lock keywords and cc-mode's scanners use a
/// few hundred, so this keeps enough for a session's working set and drops
/// the least recently used beyond it.
#[derive(Default)]
pub(crate) struct PatternCache {
    patterns: HashMap<Vec<u8>, Vec<CachedPattern>>,
    len: usize,
    /// Uses so far: the clock `CachedPattern::used` reads.
    clock: u64,
}
impl PatternCache {
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn retain(&mut self, keep: impl Fn(&CachedPattern) -> bool) {
        self.patterns.retain(|_, v| { v.retain(&keep); !v.is_empty() });
        self.len = self.patterns.values().map(Vec::len).sum();
    }
    /// Drop the least recently used pattern.
    fn evict(&mut self) {
        let oldest = self.patterns.iter()
            .flat_map(|(source, v)| v.iter().enumerate().map(move |(i, e)| (e.used, source, i)))
            .min_by_key(|&(used, ..)| used)
            .map(|(_, source, i)| (source.clone(), i));
        if let Some((source, i)) = oldest {
            let v = self.patterns.get_mut(&source).expect("present");
            v.swap_remove(i);
            if v.is_empty() { self.patterns.remove(&source); }
            self.len -= 1;
        }
    }
}

struct CachedPattern {
    multibyte: bool,
    /// Compiled for the longest match (search.c `posix').
    posix: bool,
    translation: Translation,
    /// The syntax table the compiled fastmap read (GNU `used_syntax').
    syntax: Option<Value>,
    compiled: std::rc::Rc<std::cell::RefCell<regex::CompiledPattern>>,
    /// The cache's clock at the last use.
    used: u64,
}

/// The patterns [`PatternCache`] keeps.
const PATTERN_CACHE_SIZE: usize = 1024;

/// Patterns to test many strings, each taken from the pattern cache once:
/// no Lisp runs between the tests, so the patterns, the current buffer's
/// syntax table and the case translation hold throughout.
pub(crate) struct StringMatcher {
    sources: Vec<Value>,
    patterns: Vec<Option<std::rc::Rc<std::cell::RefCell<regex::CompiledPattern>>>>,
    key: Translation,
    translation: Option<regex::CaseTranslation>,
    syntax: syntax::SyntaxSource,
}

impl Vm {
    /// PATTERNS (strings) to compile once each with TRANSLATION and test
    /// strings with [`Self::string_matcher_matches`]. None while
    /// `parse-sexp-lookup-properties' makes each string's own `syntax-table'
    /// properties part of its match: those strings take
    /// [`Self::string_match_no_data`].
    pub(crate) fn string_matcher(&mut self, patterns: &[Value], key: Translation, translation: Option<regex::CaseTranslation>)
        -> std::result::Result<Option<StringMatcher>, prim::Fault> {
        if self.string_syntax_properties() { return Ok(None); }
        let syntax = self.regexp_syntax(None).map_err(|fault| self.regexp_fault(fault))?;
        Ok(Some(StringMatcher { sources: patterns.to_vec(), patterns: vec![None; patterns.len()], key, translation, syntax }))
    }

    /// Whether `parse-sexp-lookup-properties' makes a string's own
    /// `syntax-table' properties part of what a regexp matches in it.
    pub(crate) fn string_syntax_properties(&self) -> bool {
        self.slots[self.names["parse-sexp-lookup-properties"]].value != Value::NIL
    }

    /// Whether each ASCII character is a word constituent in the current
    /// syntax table, as `\<' and `\b' read a string.
    pub(crate) fn ascii_word_syntax(&mut self) -> std::result::Result<[bool; 128], prim::Fault> {
        let source = self.regexp_syntax(None).map_err(|fault| self.regexp_fault(fault))?;
        Ok(std::array::from_fn(|c| self.regexp_table_class(source.table(), c as u32) == Some(regex::SyntaxClass::Word)))
    }

    /// Whether STRING matches every pattern of MATCHER, in order.
    pub(crate) fn string_matcher_matches(&mut self, matcher: &mut StringMatcher, string: Value) -> std::result::Result<bool, prim::Fault> {
        for i in 0..matcher.sources.len() {
            if self.string_matcher_search(matcher, i, string)?.is_none() { return Ok(false); }
        }
        Ok(true)
    }

    /// The groups, in character positions, of the first match of
    /// MATCHER's pattern I in STRING; None when it does not match.
    pub(crate) fn string_matcher_groups(&mut self, matcher: &mut StringMatcher, i: usize, string: Value)
        -> std::result::Result<Option<Vec<Option<(usize, usize)>>>, prim::Fault> {
        let Some(registers) = self.string_matcher_search(matcher, i, string)? else { return Ok(None) };
        let view = self.str_view(string).ok_or(prim::Fault::Wrong("stringp", string))?;
        let input = Input::new(view.bytes().to_vec(), view.chars, view.is_multibyte());
        let groups = register_spans(&registers, |byte| Ok(input.character(byte)));
        Ok(Some(groups.map_err(|fault| self.regexp_fault(fault))?))
    }

    fn string_matcher_search(&mut self, matcher: &mut StringMatcher, i: usize, string: Value)
        -> std::result::Result<Option<MatchRegisters>, prim::Fault> {
        if matcher.patterns[i].is_none() {
            let compiled = self.regexp_input(matcher.sources[i])
                .and_then(|source| self.regexp_pattern(&source, false, matcher.key, matcher.translation.clone(), &matcher.syntax))
                .map_err(|fault| self.regexp_fault(fault))?;
            matcher.patterns[i] = Some(compiled);
        }
        let pattern = matcher.patterns[i].clone().unwrap();
        let mut pattern = pattern.borrow_mut();
        let found = {
            let view = self.str_view(string).ok_or(prim::Fault::Wrong("stringp", string))?;
            let (bytes, multibyte) = (view.bytes(), view.is_multibyte());
            let syntax = syntax::VmSyntax::new(self, matcher.syntax.clone());
            pattern.set_target_multibyte(multibyte);
            regex::re_search(&pattern, bytes, 0, bytes.len() as isize, &syntax, usize::MAX)
        };
        if regex::take_matcher_overflow() {
            return Err(self.regexp_fault(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into())));
        }
        Ok(found.map(|(_, registers)| registers))
    }

    /// PATTERN compiled, for the longest match when POSIX, with the
    /// TRANSLATION KEY names (built as TRANSLATION) for the syntax table
    /// SOURCE reads: the cached one, or a new one cached in place of the
    /// least recently used.
    fn regexp_pattern(&mut self, pattern: &Input, posix: bool, key: Translation, translation: Option<regex::CaseTranslation>,
                      source: &syntax::SyntaxSource) -> SearchResult<std::rc::Rc<std::cell::RefCell<regex::CompiledPattern>>> {
        let table = source.table();
        let cache = &mut self.regexp_cache;
        cache.clock += 1;
        let clock = cache.clock;
        if let Some(entry) = cache.patterns.get_mut(&pattern.bytes).and_then(|v| v.iter_mut().find(|e| e.multibyte == pattern.multibyte
            && e.posix == posix && e.translation == key && e.syntax.is_none_or(|t| t == table))) {
            entry.used = clock;
            return Ok(entry.compiled.clone());
        }
        let mut compiled = regex::regex_compile_lisp_with_translation(
            &EmacsStr::new(&pattern.bytes, pattern.multibyte), posix, translation,
        ).map_err(|e| Fault::Text("invalid-regexp", e.to_string()))?;
        compiled.rebind_syntax_table(&syntax::VmSyntax::new(self, source.clone()));
        let syntax = compiled.used_syntax().then_some(table);
        let compiled = std::rc::Rc::new(std::cell::RefCell::new(compiled));
        let cache = &mut self.regexp_cache;
        if cache.len >= PATTERN_CACHE_SIZE {
            cache.evict();
        }
        cache.patterns.entry(pattern.bytes.clone()).or_default().push(CachedPattern {
            multibyte: pattern.multibyte, posix, translation: key, syntax, compiled: compiled.clone(), used: clock });
        cache.len += 1;
        Ok(compiled)
    }

    /// syntax.c's clear_regexp_cache after `modify-syntax-entry'. A
    /// collection, which moves the tables, drops the patterns kept for one.
    pub(crate) fn clear_regexp_cache(&mut self, only_tables: bool) {
        self.regexp_cache.retain(|e| only_tables && e.syntax.is_none() && !matches!(e.translation, Translation::Completion(_)));
    }

    /// GNU fast_looking_at over the current buffer's accessible text, for
    /// each of PATTERNS: case-sensitive, current syntax, no match data. BODY
    /// gets that text, point's byte in it, and a matcher from a pattern's
    /// index and a byte of the text to the length of its match there.
    pub(crate) fn with_buffer_looking_at<T>(&mut self, patterns: &[Value],
        body: impl FnOnce(&str, usize, &mut dyn FnMut(usize, usize) -> Option<usize>) -> T) -> std::result::Result<T, prim::Fault> {
        let run = |vm: &mut Self| -> SearchResult<T> {
            let index = vm.buffer_index()?;
            if !matches!(vm.objects[index], Object::Buffer(_)) { return Err(Fault::Internal("multibyte buffer required".into())) }
            let patterns = patterns.iter().map(|&p| vm.regexp_input(p)).collect::<SearchResult<Vec<_>>>()?;
            let source = vm.regexp_syntax(None)?;
            let compiled = patterns.iter().map(|p| vm.regexp_compiled(p, true, false, false, &source)).collect::<SearchResult<Vec<_>>>()?;
            // Compiling may have collected: find the buffer again.
            let index = vm.buffer_index()?;
            let Object::Buffer(b) = &mut vm.objects[index] else { return Err(Fault::Internal("multibyte buffer required".into())) };
            let accessible = b.accessible();
            let range = accessible.start.0..accessible.end.0;
            // Without raw bytes the text is its internal form: both read
            // the accessible text in place, as one window.
            let raw = !b.raw_in(range.clone()).is_empty();
            if !raw {
                b.search_window(range.clone(), range.clone());
            }
            let Object::Buffer(b) = &vm.objects[index] else { unreachable!("checked above") };
            let (text, internal) = if raw {
                let mut text = b.slice(accessible.clone())?.into_owned();
                // BODY's text has, at a raw byte's two bytes, a character GNU's
                // bidi classes also count as strong left-to-right (U+00AA), so
                // the raw byte neither reads as its Latin-1 code nor moves an
                // offset; the matcher reads GNU's internal form.
                for &p in b.raw_in(range.clone()) {
                    let at = p as usize - range.start;
                    text.replace_range(at..at + 2, "\u{aa}");
                }
                (std::borrow::Cow::Owned(text), b.internal(accessible.clone())?)
            } else {
                let bytes = b.search_bytes(range.clone());
                let text = std::str::from_utf8(bytes).map_err(|e| Fault::Internal(e.to_string()))?;
                (std::borrow::Cow::Borrowed(text), std::borrow::Cow::Borrowed(bytes))
            };
            let point = b.point.0.clamp(accessible.start.0, accessible.end.0) - accessible.start.0;
            let syntax = syntax::VmSyntax::new(vm, source);
            let mut looks = |pattern: usize, at: usize| {
                let found = regex::re_match(&compiled[pattern].borrow(), &internal, at, internal.len(), &syntax, point);
                // An overflow is GNU's error; here it is no match.
                (!regex::take_matcher_overflow()).then_some(found).flatten().map(|(end, _)| end - at)
            };
            Ok(body(&text, point, &mut looks))
        };
        run(self).map_err(|fault| self.regexp_fault(fault))
    }

    /// Whether PATTERN matches somewhere in BYTES, GNU internal text:
    /// treesit.c's fast_c_string_match of a node type and its `#match?'
    /// search_buffer narrowed to a node. Case-sensitive, no match data.
    /// SPAN, a buffer and the byte BYTES start at, gives the search that
    /// buffer's syntax there; without it the current buffer's table holds.
    pub(crate) fn bytes_search(&mut self, pattern: Value, bytes: &[u8], multibyte: bool, span: Option<(Value, usize)>) -> std::result::Result<bool, prim::Fault> {
        let run = |vm: &mut Self| -> SearchResult<bool> {
            let pattern = vm.regexp_input(pattern)?;
            let source = match span {
                Some((buffer, base)) => vm.regexp_buffer_syntax(buffer, base)?,
                None => vm.regexp_syntax(None)?,
            };
            let compiled = vm.regexp_pattern(&pattern, false, Translation::None, None, &source)?;
            let mut compiled = compiled.borrow_mut();
            let syntax = syntax::VmSyntax::new(vm, source);
            compiled.set_target_multibyte(multibyte);
            let found = regex::re_search(&compiled, bytes, 0, bytes.len() as isize, &syntax, usize::MAX).is_some();
            if regex::take_matcher_overflow() {
                return Err(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into()));
            }
            Ok(found)
        };
        run(self).map_err(|fault| self.regexp_fault(fault))
    }

    /// GNU fast_string_match: case-sensitive, current syntax, no match-data writes.
    pub(crate) fn file_name_match(&mut self, pattern: Value, string: Value) -> std::result::Result<Option<usize>, prim::Fault> {
        self.string_match_no_data(pattern, string, Translation::None, None)
    }

    /// PATTERN against STRING with the translation KEY names (built as
    /// TRANSLATION), without touching the match data.
    pub(crate) fn string_match_no_data(&mut self, pattern: Value, string: Value, key: Translation,
                                       translation: Option<regex::CaseTranslation>) -> std::result::Result<Option<usize>, prim::Fault> {
        Ok(self.string_search(pattern, string, key, translation)?.map(|(_, at, _)| at))
    }

    /// [`Self::string_match_no_data`]'s match, as its groups in character positions.
    pub(crate) fn string_match_groups(&mut self, pattern: Value, string: Value, key: Translation,
                                      translation: Option<regex::CaseTranslation>)
        -> std::result::Result<Option<Vec<Option<(usize, usize)>>>, prim::Fault> {
        let groups = self.string_search(pattern, string, key, translation)?
            .map(|(input, _, registers)| register_spans(&registers, |byte| Ok(input.character(byte))))
            .transpose();
        groups.map_err(|fault| self.regexp_fault(fault))
    }

    fn string_search(&mut self, pattern: Value, string: Value, key: Translation, translation: Option<regex::CaseTranslation>)
        -> std::result::Result<Option<(Input, usize, MatchRegisters)>, prim::Fault> {
        let run = |vm: &mut Self| -> SearchResult<Option<(Input, usize, MatchRegisters)>> {
            let pattern = vm.regexp_input(pattern)?;
            let input = vm.regexp_input(string)?;
            let source = vm.regexp_syntax(Some(string))?;
            let compiled = vm.regexp_pattern(&pattern, false, key, translation, &source)?;
            let mut compiled = compiled.borrow_mut();
            let syntax = syntax::VmSyntax::new(vm, source);
            compiled.set_target_multibyte(input.multibyte);
            let armed = vm.arm_regexp_quit();
            let found = regex::re_search(&compiled, &input.bytes, 0, input.bytes.len() as isize, &syntax, usize::MAX);
            drop((syntax, compiled));
            vm.regexp_quit(armed)?;
            if regex::take_matcher_overflow() {
                return Err(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into()));
            }
            Ok(found.map(|(at, registers)| (input, at, registers)))
        };
        run(self).map_err(|fault| self.regexp_fault(fault))
    }

    fn regexp_fault(&mut self, fault: Fault) -> prim::Fault {
        match fault {
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
        }
    }

    /// search.c update_search_regs: OLDSTART..OLDEND became OLDSTART..NEWEND.
    /// Registers inside the old text go to OLDSTART, those after it shift
    /// with it, whatever the last match searched. A start at OLDSTART stays
    /// and an end at OLDEND moves, so a group the change touches encloses
    /// the new text (an empty change is an insertion at both).
    pub(crate) fn regexp_update_search_regs(&mut self, oldstart: i64, oldend: i64, newend: i64) {
        let change = newend - oldend;
        let start_moved = |at: i64| if at <= oldstart { at } else if at >= oldend { at + change } else { oldstart };
        let end_moved = |at: i64| if at >= oldend { at + change } else if at > oldstart { oldstart } else { at };
        for (start, end) in self.regexp.groups.iter_mut().flatten() {
            (*start, *end) = (start_moved(*start as i64) as usize, end_moved(*end as i64) as usize);
        }
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
        let mut seen = HashSet::default();
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
        Ok(Input::new(view.bytes().to_vec(), view.chars, view.is_multibyte()))
    }
    /// The current buffer's accessible bytes, the one-based character
    /// position of their start and their characters, and point's byte:
    /// what a search needs before it reads any text.
    fn regexp_buffer_frame(&self, index: usize) -> SearchResult<(Range<usize>, usize, usize, usize)> {
        Ok(match &self.objects[index] {
            Object::Buffer(t) => {
                let r = t.accessible();
                let base = t.char_of_byte(r.start)?.0;
                (r.start.0..r.end.0, base + 1, t.char_of_byte(r.end)?.0 - base, t.point.0)
            }
            Object::ByteBuffer(t) => {
                let r = t.restriction.start.0..t.restriction.end.0;
                (r.clone(), r.start + 1, r.len(), t.point.0)
            }
            _ => unreachable!("buffer_index validates"),
        })
    }
    /// A run of the buffer at object INDEX that holds the bytes NEED and
    /// lies within its accessible bytes, readable contiguously: for a
    /// multibyte buffer, the window its text gives (`search_window').
    fn regexp_buffer_window(&mut self, index: usize, need: Range<usize>) -> Range<usize> {
        match &mut self.objects[index] {
            Object::Buffer(t) => {
                let r = t.accessible();
                t.search_window(need, r.start.0..r.end.0)
            }
            Object::ByteBuffer(t) => t.restriction.start.0..t.restriction.end.0,
            _ => unreachable!("buffer_index validates"),
        }
    }
    /// WINDOW of the buffer at object INDEX, for one search, its
    /// characters counted from BASE_CHAR, the accessible text's first.
    fn regexp_buffer_input(&self, index: usize, window: Range<usize>, base_char: usize) -> BufferInput<'_> {
        match &self.objects[index] {
            Object::Buffer(t) => BufferInput {
                bytes: t.search_bytes(window.clone()),
                text: Some(t),
                start: window.start,
                base_char,
                point: t.point.0 - window.start,
            },
            Object::ByteBuffer(t) => BufferInput {
                bytes: &t.bytes[window.clone()],
                text: None,
                start: window.start,
                base_char,
                point: t.point.0 - window.start,
            },
            _ => unreachable!("buffer_index validates"),
        }
    }
    fn regexp_int(&self, v: Value) -> SearchResult<i64> {
        v.as_integer().map_err(|_| Fault::Wrong("fixnump", v))
    }
    fn regexp_preserve(&self) -> bool {
        self.slots[self.names["inhibit-changing-match-data"]].value != Value::NIL
    }
    /// The current buffer's binding: case-fold-search is often buffer-local.
    fn regexp_case_fold(&mut self) -> SearchResult<bool> {
        Ok(self.variable_value(self.names["case-fold-search"])? != Value::NIL)
    }
    /// PATTERN for a search of MULTIBYTE text, for the longest match when
    /// POSIX, case-folded with the standard table when FOLDED, from the
    /// pattern cache.
    fn regexp_compiled(
        &mut self,
        pattern: &Input,
        multibyte: bool,
        posix: bool,
        folded: bool,
        source: &syntax::SyntaxSource,
    ) -> SearchResult<std::rc::Rc<std::cell::RefCell<regex::CompiledPattern>>> {
        let (key, translation) = if folded {
            (Translation::Standard, Some(regex::CaseTranslation::standard()))
        } else {
            (Translation::None, None)
        };
        let compiled = self.regexp_pattern(pattern, posix, key, translation, source)?;
        compiled.borrow_mut().set_target_multibyte(multibyte);
        Ok(compiled)
    }
    /// GROUPS as the match data, of the current buffer when BUFFER.
    fn regexp_save(&mut self, groups: Vec<Option<(usize, usize)>>, buffer: bool) {
        self.regexp.groups = groups;
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
                    data::string_object(t.lisp_string(aa..bb)?)
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
        let mut seen = HashSet::default();
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
    fn regexp_primitive(&mut self, primitive: Primitive, start: usize, n: usize) -> SearchResult<Value> {
        use Primitive::*;
        // search.c: each posix- primitive is its plain search compiled for
        // the longest match.
        let (p, posix) = match primitive {
            PosixLookingAt => (LookingAt, true),
            PosixStringMatch => (StringMatch, true),
            PosixSearchForward => (ReSearchForward, true),
            PosixSearchBackward => (ReSearchBackward, true),
            p => (p, false),
        };
        let valid = match p {
            SyntaxTable | StandardSyntaxTable => n == 0,
            CopySyntaxTable => n <= 1,
            ModifySyntaxEntry => (2..=3).contains(&n),
            LookingAt | MatchString => (1..=2).contains(&n),
            StringMatch => (2..=4).contains(&n),
            ReSearchForward | ReSearchBackward | SearchForward | SearchBackward => (1..=4).contains(&n),
            MatchData => n <= 3,
            _ => n == 1,
        };
        if !valid {
            return Err(Fault::Signal(
                "wrong-number-of-arguments",
                vec![crate::prim::legacy_value(primitive).unwrap_or(Value::NIL), Value::integer(n as i64)?],
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
            let folded = self.regexp_case_fold()?;
            let source = self.regexp_syntax(Some(args[1]))?;
            let compiled = self.regexp_compiled(&pattern, input.multibyte, posix, folded, &source)?;
            let compiled = compiled.borrow();
            let syntax = syntax::VmSyntax::new(self, source);
            let byte = input.byte(pos as usize);
            let armed = self.arm_regexp_quit();
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
            drop((syntax, compiled));
            self.regexp_quit(armed)?;
            if regex::take_matcher_overflow() {
                return Err(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into()));
            }
            if let Some((at, registers)) = found {
                if !self.regexp_preserve() && args[3] == Value::NIL {
                    let groups = register_spans(&registers, |byte| Ok(input.character(byte)))?;
                    self.regexp_save(groups, false);
                }
                return Ok(Value::integer(input.character(at) as i64)?);
            }
            return Ok(Value::NIL);
        }
        let count = if matches!(p, ReSearchForward | ReSearchBackward | SearchForward | SearchBackward) && args[3] != Value::NIL {
            self.regexp_int(args[3])?
        } else {
            1
        };
        let count = if matches!(p, ReSearchBackward | SearchBackward) { -count } else { count };
        let mut pattern = self.regexp_input(args[0])?;
        // search_command with RE false: the same search over STRING quoted,
        // which matches exactly where the literal does, with the same folding.
        if matches!(p, SearchForward | SearchBackward) {
            let quoted: Vec<u8> = pattern.bytes.iter().flat_map(|&b| {
                let special = b"[*.\\?+^$".contains(&b);
                if special { vec![b'\\', b] } else { vec![b] }
            }).collect();
            let chars = pattern.len() + quoted.len() - pattern.bytes.len();
            pattern = Input::new(quoted, chars, pattern.multibyte);
        }
        let index = self.buffer_index()?;
        let (accessible, base_char, chars, point_byte) = self.regexp_buffer_frame(index)?;
        let multibyte = matches!(self.objects[index], Object::Buffer(_));
        let end_char = base_char + chars;
        let character_of = |vm: &Self, byte: usize| -> SearchResult<usize> {
            Ok(match &vm.objects[index] {
                Object::Buffer(t) => t.char_of_byte(ByteOffset(byte))?.0 + 1,
                _ => byte + 1,
            })
        };
        let original_char = character_of(self, point_byte)?;
        let bound = if p == LookingAt || args[1] == Value::NIL {
            if p == LookingAt || count > 0 {
                end_char
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
            limit.clamp(base_char as i64, end_char as i64) as usize
        };
        if count == 0 {
            return Ok(Value::integer(original_char as i64)?);
        }
        let limit_byte = match &self.objects[index] {
            Object::Buffer(t) => t.byte_of_char(CharOffset(bound - 1))?.0,
            _ => bound - 1,
        };
        let folded = self.regexp_case_fold()?;
        let source = self.regexp_syntax(None)?;
        let compiled = self.regexp_compiled(&pattern, multibyte, posix, folded, &source)?;
        let compiled = compiled.borrow();
        // What the engine can read: the candidates and the matches, which
        // end at the bound forward and at point backward (GNU's STOP), and
        // a margin for the characters assertions look at on either side.
        let reach = if p == LookingAt {
            point_byte..accessible.end
        } else if count > 0 {
            point_byte..limit_byte
        } else {
            limit_byte..point_byte
        };
        let need = reach.start.saturating_sub(SEARCH_MARGIN).max(accessible.start)
            ..(reach.end + SEARCH_MARGIN).min(accessible.end);
        // Compiling may have collected: find the buffer again.
        let index = self.buffer_index()?;
        let window = self.regexp_buffer_window(index, need);
        let text = self.regexp_buffer_input(index, window.clone(), base_char);
        let syntax = syntax::VmSyntax::new(self, source.rebased(window.start));
        // Match data is saved once, from the last success, after the engine.
        let (mut matched, mut overflow) = (None, false);
        let original = text.point;
        let limit = limit_byte - window.start;
        let mut point = original;
        let mut failed = false;
        let armed = self.arm_regexp_quit();
        for _ in 0..count.unsigned_abs() {
            let found = if p == LookingAt {
                regex::re_match(&compiled, text.bytes, point, text.bytes.len(), &syntax, original)
            } else {
                regex::re_search(&compiled, text.bytes, point, limit as isize - point as isize, &syntax, original)
            };
            // An overflow still keeps an earlier iteration's match data.
            overflow = regex::take_matcher_overflow();
            let Some((_, registers)) = found.filter(|_| !overflow) else {
                failed = true;
                break;
            };
            point = if count > 0 {
                registers.end[0]
            } else {
                registers.start[0]
            } as usize;
            matched = Some(registers);
        }
        let groups = matched
            .map(|registers| register_spans(&registers, |byte| Ok(text.character(byte)? + base_char)))
            .transpose()?;
        if failed && args[2] != Value::NIL && args[2] != Value::TRUE {
            point = limit;
        }
        let point_char = text.character(point)? + base_char;
        drop((syntax, text));
        self.regexp_quit(armed)?;
        if let Some(groups) = groups {
            if !self.regexp_preserve() && (p != LookingAt || args[1] == Value::NIL) {
                self.regexp_save(groups, true);
            }
        }
        if overflow {
            return Err(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into()));
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
        }
        // Recover the buffer after the engine, rather than retaining its index.
        let index = self.buffer_index()?;
        match &mut self.objects[index] {
            Object::Buffer(t) => t.point = ByteOffset(point + window.start),
            Object::ByteBuffer(t) => t.point = ByteOffset(point + window.start),
            _ => unreachable!("buffer_index validates"),
        }
        if failed {
            Ok(Value::NIL)
        } else {
            Ok(Value::integer(point_char as i64)?)
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

thread_local! {
    /// The quit signal of the VM whose search runs on this thread, while
    /// a C-g may stop its matcher (`Vm::arm_regexp_quit`).
    static REGEXP_QUIT: std::cell::Cell<*const crate::QuitSignal> = const { std::cell::Cell::new(std::ptr::null()) };
    /// The user signals `debug-on-event' named when that search began
    /// (`Vm::debug_event_mask`): no Lisp runs inside a match to change it.
    static REGEXP_DEBUG_EVENTS: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// The matcher's quit probe (romacs_regex_engine::host): a C-g the host
/// counted for the searching VM, or a debug event.
fn regexp_quit_probe() -> bool {
    // SAFETY: set only for the extent of one search by the VM that owns the
    // signal (`ArmedQuit`), which outlives the search.
    REGEXP_QUIT.with(|signal| { let signal = signal.get(); !signal.is_null() && unsafe { (*signal).due() } })
        || crate::host::user_signal_waiting(REGEXP_DEBUG_EVENTS.with(std::cell::Cell::get))
}

/// A search's armed quit probe, disarmed when dropped.
pub(crate) struct ArmedQuit(bool);
impl Drop for ArmedQuit {
    fn drop(&mut self) {
        if self.0 {
            REGEXP_QUIT.with(|signal| signal.set(std::ptr::null()));
            REGEXP_DEBUG_EVENTS.with(|events| events.set(0));
        }
    }
}

impl Vm {
    /// Let the matcher poll the host's C-g, or a debug event, during one
    /// search, as GNU's polls maybe_quit at its backtrack sites: only where
    /// the quit would be acted on, so an aborted search always ends in
    /// `quit'. A C-g only while `inhibit-quit' is nil; a debug event
    /// whatever it is, since handle_user_signal clears it.
    fn arm_regexp_quit(&self) -> ArmedQuit {
        let reached = self.quit.busy() && self.compile_depth == 0 && self.expanding.is_empty();
        let presses = reached && std::sync::Arc::strong_count(&self.quit) > 1 && self.quit_variables()[1] == Value::NIL;
        let debug = if reached { self.debug_event_mask() } else { 0 };
        let armed = presses || debug != 0;
        if armed {
            let quit = if presses { std::sync::Arc::as_ptr(&self.quit) } else { std::ptr::null() };
            REGEXP_QUIT.with(|signal| signal.set(quit));
            REGEXP_DEBUG_EVENTS.with(|events| events.set(debug));
            regex::host::set_quit_probe(Some(regexp_quit_probe));
        }
        ArmedQuit(armed)
    }
    /// After an ARMED search: the host's C-g or a debug event, if the
    /// matcher may have stopped for it, as `quit'.
    fn regexp_quit(&mut self, armed: ArmedQuit) -> std::result::Result<(), Fault> {
        let polled = armed.0;
        drop(armed);
        if !polled || !(self.quit.due() || crate::host::user_signal_waiting(self.debug_event_mask())) {
            return Ok(());
        }
        match self.maybe_quit() {
            Some(crate::threaded::Escape::Signal(..)) => Err(Fault::Signal("quit", Vec::new())),
            // A C-g `inhibit-quit' holds, which the matcher did not stop
            // for; the next safepoint has it.
            _ => Ok(()),
        }
    }
}
