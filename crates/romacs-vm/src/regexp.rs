//! Lisp search boundary. Only owned text and integer offsets reach the engine.
use crate::{coordinates::*, data::Object, variable_runtime::Places, *};
use romacs_regex::{self as regex, EmacsStr, MatchRegisters};
use std::borrow::Cow;
use std::ops::Range;
#[path = "regexp_syntax.rs"]
mod syntax;
pub(crate) use syntax::{parse_syntax_descriptor, VmSyntax};

#[cfg(test)]
#[path = "regexp_entry_tests.rs"]
mod entry_tests;

#[cfg(test)]
thread_local! {
    /// The buffer positions this thread's searches converted between bytes
    /// and characters, and the strings they copied to read.
    static CONVERTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static COPIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
fn note(counter: &'static std::thread::LocalKey<std::cell::Cell<usize>>, n: usize) {
    counter.with(|c| c.set(c.get() + n));
}

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

struct Input<'a> {
    bytes: Cow<'a, [u8]>,
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
impl<'a> Input<'a> {
    /// BYTES, which hold CHARS characters.
    fn new(bytes: impl Into<Cow<'a, [u8]>>, chars: usize, multibyte: bool) -> Self {
        let bytes = bytes.into();
        // Multibyte text with a byte to each character is ASCII.
        let bytewise = !multibyte || chars == bytes.len();
        let end = (bytes.len(), chars);
        Input { bytes, multibyte, bytewise, anchors: std::cell::Cell::new([(0, 0), end, end]) }
    }
}
impl Input<'_> {
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
/// place, so a search costs what it reads, not the buffer.
struct BufferInput<'a> {
    bytes: &'a [u8],
    /// Point, as a byte of BYTES.
    point: usize,
}

/// The current buffer as a search finds it: its object, which nothing moves
/// while the search runs (a collection waits for the engine), the bytes
/// of its accessible text and of point, and whether it is multibyte.
struct SearchBuffer {
    index: usize,
    accessible: Range<usize>,
    point: usize,
    multibyte: bool,
}

/// A register as an offset: None for a group that took no part.
fn register(offset: i64) -> Option<usize> {
    usize::try_from(offset).ok()
}

/// REGISTERS, byte offsets into a text, as the positions AT gives those
/// bytes, in place: what Lisp is to see of a match.
fn place_registers(registers: &mut MatchRegisters, at: impl Fn(usize) -> SearchResult<usize>) -> SearchResult<()> {
    for offsets in [&mut registers.start[..], &mut registers.end[..]] {
        for offset in offsets {
            if let Some(byte) = register(*offset) {
                *offset = i64::try_from(at(byte)?).map_err(|e| Fault::Internal(e.to_string()))?;
            }
        }
    }
    Ok(())
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
    /// The variables every search or syntax scan reads, found by name at
    /// the first and kept: each reads the variable itself, as GNU's reads
    /// its C variable, whatever its name is interned as later.
    slots: Option<SearchSlots>,
    /// What the last search learned of its syntax table.
    syntax: syntax::SyntaxMemo,
    /// The last literal search's string, quoted: its bytes are kept for
    /// the next.
    quoted: Vec<u8>,
}
struct SearchSlots {
    preserve_match_data: usize,
    match_buffer: usize,
    standard_syntax: usize,
    /// The symbol `syntax-table`, a syntax table's purpose.
    syntax_table: usize,
    /// `case-fold-search`, often buffer-local.
    case_fold: Places<1>,
    /// `parse-sexp-lookup-properties`, which `syntax-propertize` sets
    /// buffer-locally.
    lookup_properties: Places<1>,
    scan: Places<{ SCAN_VARIABLES.len() }>,
    /// `forward-comment-function`, which replaces `forward-comment`.
    forward_comment: Places<1>,
}
/// The variables a syntax scan obeys, in the order [`Vm::scan_variables`]
/// gives their values: six flags, then how far `syntax-propertize` has
/// gone.
pub(crate) const SCAN_VARIABLES: [&str; 7] = ["parse-sexp-lookup-properties", "parse-sexp-ignore-comments",
    "comment-end-can-be-escaped", "multibyte-syntax-as-symbol", "open-paren-in-column-0-is-defun-start",
    "comment-use-syntax-ppss", "syntax-propertize--done"];
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
            .flat_map(|(source, v)| v.iter().enumerate().map(move |(i, e)| (e.used.get(), source, i)))
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
    /// The cache's clock at the last use: a hit moves it without the cache.
    used: std::cell::Cell<u64>,
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
        let input = Input::new(view.bytes(), view.chars, view.is_multibyte());
        let groups = register_spans(&registers, |byte| Ok(input.character(byte)));
        Ok(Some(groups.map_err(|fault| self.regexp_fault(fault))?))
    }

    fn string_matcher_search(&mut self, matcher: &mut StringMatcher, i: usize, string: Value)
        -> std::result::Result<Option<MatchRegisters>, prim::Fault> {
        if matcher.patterns[i].is_none() {
            let compiled = self.regexp_input(matcher.sources[i])
                .and_then(|source| self.regexp_pattern(&source, false, matcher.key, || matcher.translation.clone(), &matcher.syntax))
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
    fn regexp_pattern(&mut self, pattern: &Input<'_>, posix: bool, key: Translation,
                      translation: impl FnOnce() -> Option<regex::CaseTranslation>, source: &syntax::SyntaxSource) -> SearchResult<std::rc::Rc<std::cell::RefCell<regex::CompiledPattern>>> {
        let table = source.table();
        let cache = &mut self.regexp_cache;
        cache.clock += 1;
        let clock = cache.clock;
        if let Some(entry) = cache.patterns.get(&*pattern.bytes).and_then(|v| v.iter().find(|e| e.multibyte == pattern.multibyte
            && e.posix == posix && e.translation == key && e.syntax.is_none_or(|t| t == table))) {
            entry.used.set(clock);
            return Ok(entry.compiled.clone());
        }
        let mut compiled = regex::regex_compile_lisp_with_translation(
            &EmacsStr::new(&pattern.bytes, pattern.multibyte), posix, translation(),
        ).map_err(|e| Fault::Text("invalid-regexp", e.to_string()))?;
        compiled.rebind_syntax_table(&syntax::VmSyntax::new(self, source.clone()));
        let syntax = compiled.used_syntax().then_some(table);
        let compiled = std::rc::Rc::new(std::cell::RefCell::new(compiled));
        let cache = &mut self.regexp_cache;
        if cache.len >= PATTERN_CACHE_SIZE {
            cache.evict();
        }
        cache.patterns.entry(pattern.bytes.to_vec()).or_default().push(CachedPattern {
            multibyte: pattern.multibyte, posix, translation: key, syntax, compiled: compiled.clone(), used: std::cell::Cell::new(clock) });
        cache.len += 1;
        Ok(compiled)
    }

    /// syntax.c's clear_regexp_cache after `modify-syntax-entry'. A
    /// collection, which moves the tables, drops the patterns kept for one.
    pub(crate) fn clear_regexp_cache(&mut self, only_tables: bool) {
        self.regexp_cache.syntax.forget();
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
            let Object::Buffer(b) = vm.objects.entry_mut(index) else { return Err(Fault::Internal("multibyte buffer required".into())) };
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
            let compiled = vm.regexp_pattern(&pattern, false, Translation::None, || None, &source)?;
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
        -> std::result::Result<Option<(Input<'static>, usize, MatchRegisters)>, prim::Fault> {
        let run = |vm: &mut Self| -> SearchResult<Option<(Input<'static>, usize, MatchRegisters)>> {
            let pattern = vm.regexp_input(pattern)?;
            let input = vm.regexp_input(string)?;
            let source = vm.regexp_syntax(Some(string))?;
            let compiled = vm.regexp_pattern(&pattern, false, key, move || translation, &source)?;
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
        // A leaf reaches this search, and the matcher answers inspections.
        let found = if crate::host::inspect_enabled() { self.with_site_leaf(run) } else { run(self) };
        found.map_err(|fault| self.regexp_fault(fault))
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
            self.conses.set_car(cell.index(3).map_err(prim::Fault::Error)?, Value::NIL);
        }
        Ok(())
    }
    /// The string V's text, copied: what outlives a borrow of the VM.
    fn regexp_input(&self, v: Value) -> SearchResult<Input<'static>> {
        #[cfg(test)]
        note(&COPIED, 1);
        let view = self.str_view(v).ok_or(Fault::Wrong("stringp", v))?;
        Ok(Input::new(view.bytes().to_vec(), view.chars, view.is_multibyte()))
    }
    /// The string V's text where it lies, for a search that borrows the VM.
    fn regexp_subject(&self, v: Value) -> SearchResult<Input<'_>> {
        let view = self.str_view(v).ok_or(Fault::Wrong("stringp", v))?;
        Ok(Input::new(view.bytes(), view.chars, view.is_multibyte()))
    }
    /// The current buffer, as a search needs it before it reads any text:
    /// no position is converted until Lisp is to see one.
    fn regexp_search_buffer(&mut self) -> SearchResult<SearchBuffer> {
        let index = self.buffer_index()?;
        Ok(match &self.objects[index] {
            Object::Buffer(t) => {
                let r = t.accessible();
                SearchBuffer { index, accessible: r.start.0..r.end.0, point: t.point.0, multibyte: true }
            }
            Object::ByteBuffer(t) => {
                SearchBuffer { index, accessible: t.restriction.start.0..t.restriction.end.0, point: t.point.0, multibyte: false }
            }
            _ => unreachable!("buffer_index validates"),
        })
    }
    /// The one-based character position of byte BYTE in the buffer at
    /// object INDEX.
    fn regexp_buffer_char(&self, index: usize, byte: usize) -> SearchResult<usize> {
        Ok(match &self.objects[index] {
            Object::Buffer(t) => {
                #[cfg(test)]
                note(&CONVERTED, 1);
                t.char_of_byte(ByteOffset(byte))?.0 + 1
            }
            _ => byte + 1,
        })
    }
    /// The byte a search of BUFFER going FORWARD (or back) stops at for
    /// BOUND, a position or a marker: an error when it lies on the wrong
    /// side of point, else clipped to the accessible text.
    fn regexp_bound(&self, buffer: &SearchBuffer, bound: Value, forward: bool) -> SearchResult<usize> {
        let limit = if let Some(Object::Marker(_)) = self.object_if(bound) {
            let position = self.marker_position_value(bound)?;
            if position == Value::NIL {
                return Err(Fault::Text("error", "Marker does not point anywhere".into()));
            }
            position.as_integer()?
        } else {
            bound.as_integer().map_err(|_| Fault::Wrong("integer-or-marker-p", bound))?
        };
        let point = i64::try_from(self.regexp_buffer_char(buffer.index, buffer.point)?).map_err(|e| Fault::Internal(e.to_string()))?;
        if if forward { limit < point } else { limit > point } {
            return Err(Fault::Text("error", "Invalid search bound (wrong side of point)".into()));
        }
        // The characters before the bound; none before a bound below 1.
        let before = usize::try_from(limit - 1).unwrap_or(0);
        let byte = match &self.objects[buffer.index] {
            Object::Buffer(t) if before < t.len_chars() => {
                #[cfg(test)]
                note(&CONVERTED, 1);
                t.byte_of_char(CharOffset(before))?.0
            }
            Object::Buffer(t) => t.len_bytes(),
            _ => before,
        };
        Ok(byte.clamp(buffer.accessible.start, buffer.accessible.end))
    }
    /// A run of the buffer at object INDEX that holds the bytes NEED and
    /// lies within its accessible bytes, readable contiguously: for a
    /// multibyte buffer, the window its text gives (`search_window').
    fn regexp_buffer_window(&mut self, index: usize, need: Range<usize>) -> Range<usize> {
        match self.objects.entry_mut(index) {
            Object::Buffer(t) => {
                let r = t.accessible();
                t.search_window(need, r.start.0..r.end.0)
            }
            Object::ByteBuffer(t) => t.restriction.start.0..t.restriction.end.0,
            _ => unreachable!("buffer_index validates"),
        }
    }
    /// WINDOW of the buffer at object INDEX, for one search.
    fn regexp_buffer_input(&self, index: usize, window: Range<usize>) -> BufferInput<'_> {
        match &self.objects[index] {
            Object::Buffer(t) => BufferInput { bytes: t.search_bytes(window.clone()), point: t.point.0 - window.start },
            Object::ByteBuffer(t) => BufferInput { bytes: &t.bytes[window.clone()], point: t.point.0 - window.start },
            _ => unreachable!("buffer_index validates"),
        }
    }
    fn regexp_int(&self, v: Value) -> SearchResult<i64> {
        v.as_integer().map_err(|_| Fault::Wrong("fixnump", v))
    }
    #[inline]
    fn search_slots(&mut self) -> &mut SearchSlots {
        if self.regexp_cache.slots.is_none() {
            self.regexp_cache.slots = Some(SearchSlots {
                preserve_match_data: self.slot("inhibit-changing-match-data"),
                match_buffer: self.slot("#:vm2-match-buffer"),
                standard_syntax: self.slot("#:vm2-standard-syntax"),
                syntax_table: self.slot("syntax-table"),
                case_fold: Places::new([self.slot("case-fold-search")]),
                lookup_properties: Places::new([self.slot("parse-sexp-lookup-properties")]),
                scan: Places::new(SCAN_VARIABLES.map(|name| self.slot(name))),
                forward_comment: Places::new([self.slot("forward-comment-function")]),
            });
        }
        self.regexp_cache.slots.as_mut().expect("found above")
    }
    /// The values in the current buffer of the variables AT names among
    /// the search slots, read where they were last found while that holds.
    fn search_values<const N: usize>(&mut self, at: fn(&mut SearchSlots) -> &mut Places<N>) -> Result<[Value; N]> {
        let mut places = *at(self.search_slots());
        if let Some(values) = self.read_places(&places) {
            return Ok(values);
        }
        let values = self.find_places(&mut places)?;
        *at(self.search_slots()) = places;
        Ok(values)
    }
    /// The values in the current buffer of [`SCAN_VARIABLES`], in order.
    pub(crate) fn scan_variables(&mut self) -> Result<[Value; SCAN_VARIABLES.len()]> {
        self.search_values(|slots| &mut slots.scan)
    }
    /// `forward-comment-function` in the current buffer.
    pub(crate) fn forward_comment_function(&mut self) -> Result<Value> {
        self.search_values(|slots| &mut slots.forward_comment).map(|[function]| function)
    }
    /// The symbol `syntax-table`.
    pub(crate) fn syntax_table_symbol(&mut self) -> Value {
        Value::reference(self.search_slots().syntax_table + 1, 7)
    }
    /// Whether V is a string: what a search checks of its arguments, in
    /// their order, before it reads either.
    #[inline]
    fn regexp_stringp(&self, v: Value) -> bool {
        Self::image_string(v) || matches!(self.object_if(v), Some(Object::String(_) | Object::RawString(_) | Object::Bytes(_)))
    }
    #[inline]
    fn regexp_preserve(&mut self) -> bool {
        let slot = self.search_slots().preserve_match_data;
        self.slots[slot].value != Value::NIL
    }
    /// The current buffer's binding: case-fold-search is often buffer-local.
    fn regexp_case_fold(&mut self) -> SearchResult<bool> {
        let [fold] = self.search_values(|slots| &mut slots.case_fold)?;
        Ok(fold != Value::NIL)
    }
    /// PATTERN for a search of MULTIBYTE text, for the longest match when
    /// POSIX, case-folded with the standard table when FOLDED, from the
    /// pattern cache.
    fn regexp_compiled(
        &mut self,
        pattern: &Input<'_>,
        multibyte: bool,
        posix: bool,
        folded: bool,
        source: &syntax::SyntaxSource,
    ) -> SearchResult<std::rc::Rc<std::cell::RefCell<regex::CompiledPattern>>> {
        // The translation is built only for a pattern the cache lacks: the
        // standard one is a 256-entry table.
        let key = if folded { Translation::Standard } else { Translation::None };
        let compiled = self.regexp_pattern(pattern, posix, key, || folded.then(regex::CaseTranslation::standard), source)?;
        compiled.borrow_mut().set_target_multibyte(multibyte);
        Ok(compiled)
    }
    /// `regexp_compiled` of the string PATTERN, which a cached pattern
    /// answers from the string's own bytes; only a miss copies them.
    fn regexp_compiled_string(
        &mut self,
        pattern: Value,
        multibyte: bool,
        posix: bool,
        folded: bool,
        source: &syntax::SyntaxSource,
    ) -> SearchResult<std::rc::Rc<std::cell::RefCell<regex::CompiledPattern>>> {
        let key = if folded { Translation::Standard } else { Translation::None };
        let table = source.table();
        self.regexp_cache.clock += 1;
        let clock = self.regexp_cache.clock;
        let hit = self.str_view(pattern).and_then(|view| {
            let entry = self.regexp_cache.patterns.get(view.bytes())?.iter().find(|e| e.multibyte == view.is_multibyte()
                && e.posix == posix && e.translation == key && e.syntax.is_none_or(|t| t == table))?;
            entry.used.set(clock);
            Some(entry.compiled.clone())
        });
        let compiled = if let Some(compiled) = hit {
            compiled
        } else {
            let pattern = self.regexp_input(pattern)?;
            self.regexp_pattern(&pattern, posix, key, || folded.then(regex::CaseTranslation::standard), source)?
        };
        compiled.borrow_mut().set_target_multibyte(multibyte);
        Ok(compiled)
    }
    /// REGISTERS, positions already (`place_registers`), as the match data,
    /// of the current buffer when BUFFER: into the groups GNU's one
    /// `search_regs` is, which a match as wide as the last allocates
    /// nothing to fill.
    fn regexp_save(&mut self, registers: &MatchRegisters, buffer: bool) {
        let (starts, ends) = (&registers.start[..], &registers.end[..]);
        let groups = &mut self.regexp.groups;
        groups.clear();
        for (i, &start) in starts.iter().enumerate() {
            groups.push(match (register(start), register(ends[i])) {
                (Some(start), Some(end)) => Some((start, end)),
                _ => None,
            });
        }
        self.regexp.succeeded = true;
        let slot = self.search_slots().match_buffer;
        self.slots[slot].value = if buffer {
            self.slots[self.buffer_slot].value
        } else {
            Value::NIL
        };
    }
    /// The compiled pattern that matches exactly STRING: `search_command`
    /// with RE false is the same search over STRING quoted, which matches
    /// where the literal does, with the same folding.
    fn regexp_compiled_literal(&mut self, string: Value, multibyte: bool, posix: bool, folded: bool, source: &syntax::SyntaxSource)
        -> SearchResult<std::rc::Rc<std::cell::RefCell<regex::CompiledPattern>>> {
        let mut quoted = std::mem::take(&mut self.regexp_cache.quoted);
        quoted.clear();
        let view = self.str_view(string).ok_or(Fault::Wrong("stringp", string))?;
        for &byte in view.bytes() {
            if b"[*.\\?+^$".contains(&byte) {
                quoted.push(b'\\');
            }
            quoted.push(byte);
        }
        let (chars, pattern_multibyte) = (view.chars + quoted.len() - view.bytes().len(), view.is_multibyte());
        let compiled = self.regexp_compiled(&Input::new(&quoted[..], chars, pattern_multibyte), multibyte, posix, folded, source);
        self.regexp_cache.quoted = quoted;
        compiled
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
                    data::unibyte_object(t.bytes[aa..bb].to_vec())
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
                                bias: anchors::Bias::Left, last_position: 0
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
            self.conses.set_car(index, values.get(i).copied().unwrap_or(Value::NIL));
            previous = Some(index);
            tail = next;
            i += 1;
        }
        if i < values.len() {
            let rest = self.list_value(&values[i..]);
            self.conses.set_cdr(previous.expect("nonempty reuse"), rest);
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
            return self.regexp_string_match(&args, posix);
        }
        self.regexp_buffer_search(p, posix, &args)
    }
    /// `string-match` of PATTERN in STRING from START, unless INHIBIT
    /// leaving the match data; for the longest match when POSIX.
    fn regexp_string_match(&mut self, &[pattern, string, start, inhibit]: &[Value; 4], posix: bool) -> SearchResult<Value> {
        if !self.regexp_stringp(pattern) {
            return Err(Fault::Wrong("stringp", pattern));
        }
        let (chars, multibyte) = {
            let view = self.str_view(string).ok_or(Fault::Wrong("stringp", string))?;
            (view.chars, view.is_multibyte())
        };
        let from = if start == Value::NIL { 0 } else { self.regexp_int(start)? };
        let from = if from < 0 { i64::try_from(chars).map_or(from, |chars| chars + from) } else { from };
        let Some(from) = usize::try_from(from).ok().filter(|&from| from <= chars) else {
            return Err(Fault::Signal("args-out-of-range", vec![string, start]));
        };
        let folded = self.regexp_case_fold()?;
        let source = self.regexp_syntax(Some(string))?;
        let compiled = self.regexp_compiled_string(pattern, multibyte, posix, folded, &source)?;
        let save = inhibit == Value::NIL && !self.regexp_preserve();
        let armed = self.arm_regexp_quit();
        // The string is read where it lies: nothing moves or changes it
        // while the search borrows the VM.
        let found = {
            let input = self.regexp_subject(string)?;
            let compiled = compiled.borrow();
            let syntax = syntax::VmSyntax::new(self, source);
            let byte = input.byte(from);
            // GNU at_dot compares the text pointer with buffer point; a
            // standalone string cannot contain that buffer address.
            let found = regex::re_search(&compiled, &input.bytes, byte, (input.bytes.len() - byte).cast_signed(), &syntax, usize::MAX);
            match found {
                Some((at, mut registers)) => {
                    // The match data's positions are characters of the string.
                    if save && !input.bytewise {
                        place_registers(&mut registers, |byte| Ok(input.character(byte)))?;
                    }
                    Some((input.character(at), registers))
                }
                None => None,
            }
        };
        self.regexp_quit(armed)?;
        if regex::take_matcher_overflow() {
            return Err(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into()));
        }
        let Some((at, registers)) = found else { return Ok(Value::NIL) };
        if save {
            self.regexp_save(&registers, false);
        }
        Ok(Value::integer(i64::try_from(at).map_err(|e| Fault::Internal(e.to_string()))?)?)
    }
    /// `looking-at' (P) of PATTERN at point, and the searches from point
    /// for it, as a regexp or as a literal, to BOUND, REPEAT times: for the
    /// longest match when POSIX. BOUND is `looking-at''s INHIBIT-MODIFY.
    /// Positions stay bytes until Lisp is to see them: the match data, a
    /// search's value, and a bound it gave.
    fn regexp_buffer_search(&mut self, p: Primitive, posix: bool, &[pattern, bound, noerror, repeat]: &[Value; 4]) -> SearchResult<Value> {
        use Primitive::{LookingAt, ReSearchBackward, SearchBackward, SearchForward};
        let looking = p == LookingAt;
        let count = if !looking && repeat != Value::NIL { self.regexp_int(repeat)? } else { 1 };
        let count = if matches!(p, ReSearchBackward | SearchBackward) { -count } else { count };
        if !self.regexp_stringp(pattern) {
            return Err(Fault::Wrong("stringp", pattern));
        }
        let buffer = self.regexp_search_buffer()?;
        let SearchBuffer { index, point: origin, multibyte, .. } = buffer;
        let (low, high) = (buffer.accessible.start, buffer.accessible.end);
        let limit = if looking || bound == Value::NIL {
            if looking || count > 0 { high } else { low }
        } else {
            self.regexp_bound(&buffer, bound, count > 0)?
        };
        if count == 0 {
            let point = self.regexp_buffer_char(index, origin)?;
            return Ok(Value::integer(i64::try_from(point).map_err(|e| Fault::Internal(e.to_string()))?)?);
        }
        let folded = self.regexp_case_fold()?;
        let source = self.regexp_syntax_of(index, None)?;
        let compiled = if matches!(p, SearchForward | SearchBackward) {
            self.regexp_compiled_literal(pattern, multibyte, posix, folded, &source)?
        } else {
            self.regexp_compiled_string(pattern, multibyte, posix, folded, &source)?
        };
        let save = (!looking || bound == Value::NIL) && !self.regexp_preserve();
        // What the engine can read: the candidates and the matches, which
        // end at the bound forward and at point backward (GNU's STOP), and
        // a margin for the characters assertions look at on either side.
        let reach = if looking {
            origin..high
        } else if count > 0 {
            origin..limit
        } else {
            limit..origin
        };
        let need = reach.start.saturating_sub(SEARCH_MARGIN).max(low)..(reach.end + SEARCH_MARGIN).min(high);
        let window = self.regexp_buffer_window(index, need);
        let armed = self.arm_regexp_quit();
        // Match data is saved once, from the last success, after the engine.
        let (mut matched, mut overflow, mut failed) = (None, false, false);
        let mut point = origin - window.start;
        let limit = limit - window.start;
        {
            let compiled = compiled.borrow();
            let text = self.regexp_buffer_input(index, window.clone());
            let syntax = syntax::VmSyntax::new(self, source.rebased(window.start));
            for _ in 0..count.unsigned_abs() {
                let found = if looking {
                    regex::re_match(&compiled, text.bytes, point, text.bytes.len(), &syntax, text.point)
                } else {
                    regex::re_search(&compiled, text.bytes, point, limit.cast_signed() - point.cast_signed(), &syntax, text.point)
                };
                // An overflow still keeps an earlier iteration's match data.
                overflow = regex::take_matcher_overflow();
                let Some((_, registers)) = found.filter(|_| !overflow) else {
                    failed = true;
                    break;
                };
                point = register(if count > 0 { registers.end[0] } else { registers.start[0] }).expect("a match has its whole span");
                matched = Some(registers);
            }
        }
        let character = |vm: &Self, byte: usize| vm.regexp_buffer_char(index, window.start + byte);
        if save && let Some(registers) = &mut matched {
            place_registers(registers, |byte| character(self, byte))?;
        }
        // A search's value is where it left point: the end of its last
        // match, which the match data has as a character already.
        let value = if failed || looking {
            None
        } else if save && let Some(registers) = &matched {
            register(if count > 0 { registers.end[0] } else { registers.start[0] })
        } else {
            Some(character(self, point)?)
        };
        if failed && noerror != Value::NIL && noerror != Value::TRUE {
            point = limit;
        }
        self.regexp_quit(armed)?;
        if save && let Some(registers) = &matched {
            self.regexp_save(registers, true);
        }
        if overflow {
            return Err(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into()));
        }
        if looking {
            return Ok(if failed { Value::NIL } else { Value::TRUE });
        }
        if failed {
            if noerror == Value::NIL {
                return Err(Fault::Signal("search-failed", vec![pattern]));
            }
            if noerror == Value::TRUE {
                return Ok(Value::NIL);
            }
        }
        match self.objects.entry_mut(index) {
            Object::Buffer(t) => t.point = ByteOffset(point + window.start),
            Object::ByteBuffer(t) => t.point = ByteOffset(point + window.start),
            _ => unreachable!("buffer_index validates"),
        }
        match value {
            Some(position) => Ok(Value::integer(i64::try_from(position).map_err(|e| Fault::Internal(e.to_string()))?)?),
            None => Ok(Value::NIL),
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

/// What the matcher's quit probe reads while a search runs on this thread
/// (`Vm::arm_regexp_quit`): one cell, set and cleared once a search.
#[derive(Clone, Copy)]
struct Armed {
    /// The quit signal of the VM whose search runs, while a C-g may stop
    /// its matcher; null otherwise.
    quit: *const crate::QuitSignal,
    /// The user signals `debug-on-event' named when that search began
    /// (`Vm::debug_event_mask`): no Lisp runs inside a match to change it.
    debug_events: u8,
    /// For the search's extent, what an inspection reads: the matcher
    /// touches neither, and the search runs no Lisp.
    frames: Option<(*const crate::backtrace::Backtrace, *const Vec<String>)>,
    /// Whether this thread's matcher polls `regexp_quit_probe`: installed
    /// at the thread's first armed search, and left.
    probing: bool,
}
impl Armed {
    /// No search armed.
    const fn idle(probing: bool) -> Self {
        Self { quit: std::ptr::null(), debug_events: 0, frames: None, probing }
    }
}
thread_local! {
    static ARMED: std::cell::Cell<Armed> = const { std::cell::Cell::new(Armed::idle(false)) };
}

/// The matcher's quit probe (romacs_regex::host): a C-g the host
/// counted for the searching VM, or a debug event.
fn regexp_quit_probe() -> bool {
    let armed = ARMED.with(std::cell::Cell::get);
    // An inspection leaves the match alone: only a real quit stops it.
    if crate::host::inspect_pending() && let Some((backtrace, symbols)) = armed.frames {
        let generation = crate::host::inspect_generation();
        // SAFETY: set for the extent of one search by the VM that owns them
        // (`ArmedQuit`); nothing changes them while it matches.
        let json = unsafe { crate::inspect::snapshot_names(&*backtrace, &*symbols, "regexp") };
        crate::host::inspect_publish(generation, json);
    }
    // SAFETY: set only for the extent of one search by the VM that owns the
    // signal (`ArmedQuit`), which outlives the search.
    (!armed.quit.is_null() && unsafe { (*armed.quit).due() }) || crate::host::user_signal_waiting(armed.debug_events)
}

/// A search's armed quit probe, disarmed when dropped.
pub(crate) struct ArmedQuit(bool);
impl Drop for ArmedQuit {
    fn drop(&mut self) {
        if self.0 {
            ARMED.with(|armed| armed.set(Armed::idle(true)));
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
        let reached = self.native_quit_reached();
        let inhibit = self.slots[self.quit_slots[1]].value;
        let presses = reached && std::sync::Arc::strong_count(&self.quit) > 1 && (inhibit == Value::NIL || inhibit == Value::UNBOUND);
        let debug_events = if reached { self.debug_event_mask() } else { 0 };
        let inspected = crate::host::inspect_enabled();
        if !(presses || debug_events != 0 || inspected) {
            return ArmedQuit(false);
        }
        let armed = Armed {
            quit: if presses { std::sync::Arc::as_ptr(&self.quit) } else { std::ptr::null() },
            debug_events,
            frames: inspected.then_some((&raw const self.backtrace, &raw const self.symbol_names)),
            probing: true,
        };
        if !ARMED.with(|cell| cell.replace(armed)).probing {
            regex::host::set_quit_probe(Some(regexp_quit_probe));
        }
        ArmedQuit(true)
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
