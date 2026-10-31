//! treesit.c: tree-sitter parsers, nodes and queries as Lisp objects over
//! `crate::treesit`, with GNU's lazy parsing, positions in characters,
//! signals and query semantics. Primitives that call Lisp (notifiers,
//! `#pred?', function predicates) suspend in a `Task` and resume in
//! `prims/treesit.rs`.
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};
use crate::treesit::{self as ts, native, Linecol};
use std::cell::RefCell;
use std::rc::Rc;

type R<T> = std::result::Result<T, Fault>;
type Out = super::super::Out;

/// treesit.c's recursion limit for searches and sparse trees.
pub(crate) const RECURSION_LIMIT: i64 = 1000;

/// The languages loaded this session, by symbol name (GNU caches them for
/// the session, never closing a grammar).
static LANGUAGES: std::sync::Mutex<Vec<(String, native::Language, Option<String>)>> = std::sync::Mutex::new(Vec::new());

fn fixnum(n: i64) -> Value {
    Value::integer(n).expect("fixnum")
}
fn flag(b: bool) -> Value {
    if b { Value::TRUE } else { Value::NIL }
}

/// The 1-based bytes and characters of a buffer's accessible region.
pub(crate) struct Region {
    pub begv_byte: i64,
    pub zv_byte: i64,
}

impl Leaf<'_> {
    pub(crate) fn ts(&self, v: Value) -> Option<&ts::Object> {
        match self.vm.object_if(v) {
            Some(data::Object::Treesit(t)) => Some(t),
            _ => None,
        }
    }
    fn ts_mut(&mut self, v: Value) -> &mut ts::Object {
        let index = v.index(0).expect("checked treesit object") - 1;
        match &mut self.vm.objects[index] {
            data::Object::Treesit(t) => t,
            _ => unreachable!("checked treesit object"),
        }
    }
    pub(crate) fn ts_parser(&self, v: Value) -> R<&ts::Parser> {
        match self.ts(v) {
            Some(ts::Object::Parser(p)) => Ok(p),
            _ => Err(Fault::Wrong("treesit-parser-p", v)),
        }
    }
    fn ts_parser_mut(&mut self, v: Value) -> &mut ts::Parser {
        match self.ts_mut(v) {
            ts::Object::Parser(p) => p,
            _ => unreachable!("checked parser"),
        }
    }
    pub(crate) fn ts_node(&self, v: Value) -> R<&ts::Node> {
        match self.ts(v) {
            Some(ts::Object::Node(n)) => Ok(n),
            _ => Err(Fault::Wrong("treesit-node-p", v)),
        }
    }
    pub(crate) fn ts_query(&self, v: Value) -> R<&ts::Query> {
        match self.ts(v) {
            Some(ts::Object::Query(q)) => Ok(q),
            _ => Err(Fault::Wrong("treesit-compiled-query-p", v)),
        }
    }
    pub(crate) fn ts_task(&self, v: Value) -> &ts::Task {
        match self.ts(v) {
            Some(ts::Object::Task(t)) => t,
            _ => unreachable!("a treesit task"),
        }
    }
    pub(crate) fn ts_task_mut(&mut self, v: Value) -> &mut ts::Task {
        match self.ts_mut(v) {
            ts::Object::Task(t) => t,
            _ => unreachable!("a treesit task"),
        }
    }
    pub(crate) fn ts_allocate(&mut self, object: ts::Object) -> Value {
        self.vm.allocate_object(data::Object::Treesit(Box::new(object)))
    }
    pub(crate) fn ts_list(&mut self, values: &[Value]) -> Value {
        values.iter().rev().fold(Value::NIL, |tail, &v| self.cons(v, tail))
    }
    /// The condition CONDITION with DATA.
    pub(crate) fn ts_signal(&mut self, condition: &str, data: &[Value]) -> Fault {
        let condition = self.intern(condition);
        let data = self.ts_list(data);
        Fault::Signal(condition, data)
    }
    fn ts_message(&mut self, condition: &str, message: &str, rest: &[Value]) -> Fault {
        let mut data = vec![self.string(message)];
        data.extend_from_slice(rest);
        self.ts_signal(condition, &data)
    }
    fn ts_wrong(&mut self, predicate: Value, value: Value) -> Fault {
        self.ts_signal("wrong-type-argument", &[predicate, value])
    }
    /// A string, multibyte or unibyte.
    pub(crate) fn ts_is_string(&self, v: Value) -> bool {
        matches!(self.kind(v), Kind::String | Kind::Bytes)
    }
    fn ts_symbol(&self, v: Value) -> R<()> {
        if self.kind(v) == Kind::Symbol { Ok(()) } else { Err(Fault::Wrong("symbolp", v)) }
    }
    fn ts_value(&mut self, name: &str) -> R<Value> {
        let symbol = self.intern(name);
        Ok(self.variable_value(symbol)?.unwrap_or(Value::NIL))
    }
    fn ts_list_values(&self, mut list: Value) -> Vec<Value> {
        let mut out = Vec::new();
        while let Some((car, cdr)) = self.pair(list) {
            out.push(car);
            list = cdr;
        }
        out
    }
    /// assq that ignores non-cons elements.
    fn ts_assq(&self, key: Value, list: Value) -> Option<Value> {
        self.ts_list_values(list).into_iter().find(|&e| self.pair(e).is_some_and(|(k, _)| k == key))
    }

    // Buffers.

    fn ts_buffer_text(&self, buffer: Value) -> Option<&crate::text::Text> {
        match self.vm.object_if(buffer) {
            Some(data::Object::Buffer(b)) => Some(b),
            _ => None,
        }
    }
    fn ts_buffer_text_mut(&mut self, buffer: Value) -> Option<&mut crate::text::Text> {
        let index = buffer.index(0).ok()? - 1;
        match &mut self.vm.objects[index] {
            data::Object::Buffer(b) => Some(b),
            _ => None,
        }
    }
    fn ts_base(&self, buffer: Value) -> Value {
        match self.vm.object_if(buffer) {
            Some(data::Object::Buffer(b)) if b.base != Value::NIL => b.base,
            _ => buffer,
        }
    }
    /// BUFFER, or the current buffer when nil, checked.
    fn ts_buffer_arg(&mut self, buffer: Value) -> R<Value> {
        let buffer = if buffer == Value::NIL { self.current_buffer()? } else { buffer };
        match self.vm.object_if(buffer) {
            Some(data::Object::Buffer(_)) => Ok(buffer),
            // A unibyte buffer's text is not a rope: VM2 parses multibyte
            // buffers only (VM2-TREESIT-RUNTIME).
            Some(data::Object::ByteBuffer(_)) => Err(Fault::Error("Tree-sitter needs a multibyte buffer".into())),
            _ => Err(Fault::Wrong("bufferp", buffer)),
        }
    }
    pub(crate) fn ts_live(&self, buffer: Value) -> bool {
        self.lifecycle_live_p(buffer)
    }
    fn ts_region(&self, buffer: Value) -> Region {
        let text = self.ts_buffer_text(buffer).expect("a multibyte buffer");
        let a = text.accessible();
        Region { begv_byte: a.start.0 as i64 + 1, zv_byte: a.end.0 as i64 + 1 }
    }
    /// BUF_BYTEPOS_TO_CHARPOS for a 1-based byte.
    fn ts_char(&self, buffer: Value, byte: i64) -> i64 {
        let text = self.ts_buffer_text(buffer).expect("a multibyte buffer");
        let byte = (byte - 1).clamp(0, text.len_bytes() as i64) as usize;
        text.char_of_byte(ByteOffset(byte)).map_or(1, |c| c.0 as i64 + 1)
    }
    /// BUF_CHARPOS_TO_BYTEPOS for a 1-based character.
    fn ts_byte(&self, buffer: Value, pos: i64) -> i64 {
        let text = self.ts_buffer_text(buffer).expect("a multibyte buffer");
        let pos = (pos - 1).clamp(0, text.len_chars() as i64) as usize;
        text.byte_of_char(CharOffset(pos)).map_or(1, |b| b.0 as i64 + 1)
    }
    /// treesit_check_position: POS, an integer or marker, within BUFFER's
    /// accessible region.
    fn ts_position(&mut self, pos: Value, buffer: Value) -> R<i64> {
        let n = match self.kind(pos) {
            Kind::Integer => pos.integer_value().unwrap_or(0),
            Kind::Marker => self.marker_position(pos).flatten().ok_or(Fault::Error("Marker does not point anywhere".into()))?,
            _ => return Err(Fault::Wrong("integer-or-marker-p", pos)),
        };
        let region = self.ts_region(buffer);
        let (begv, zv) = (self.ts_char(buffer, region.begv_byte), self.ts_char(buffer, region.zv_byte));
        if n < begv || n > zv { return Err(Fault::Range1(pos)); }
        Ok(n)
    }
    /// treesit_check_buffer_size.
    fn ts_check_size(&mut self, buffer: Value) -> R<()> {
        let bytes = self.ts_buffer_text(buffer).map_or(0, |t| t.len_bytes());
        if bytes > u32::MAX as usize {
            return Err(self.ts_message("treesit-buffer-too-large", "Buffer size cannot be larger than 4GB", &[fixnum(bytes as i64)]));
        }
        Ok(())
    }
    /// The linecol caches (BEGV, POINT, ZV) of BUFFER.
    fn ts_linecols(&self, buffer: Value) -> [Linecol; 3] {
        self.ts_buffer_text(buffer).map_or([ts::EMPTY_LINECOL; 3], |t| t.ts_linecols)
    }

    // Languages.

    /// resolve_language_symbol: LANGUAGE through `treesit-language-remap-alist'.
    fn ts_remap(&mut self, language: Value) -> R<Value> {
        let alist = self.ts_value("treesit-language-remap-alist")?;
        Ok(self.ts_assq(language, alist).and_then(|e| self.pair(e)).map_or(language, |(_, to)| to))
    }
    /// treesit_load_language: LANGUAGE's grammar and file, or the signal
    /// data of `treesit-load-language-error'.
    pub(crate) fn ts_load(&mut self, language: Value) -> R<std::result::Result<(native::Language, Option<String>), Value>> {
        let name = self.symbol_name(language)?;
        if let Some((_, lang, file)) = LANGUAGES.lock().unwrap().iter().find(|(n, ..)| *n == name) {
            return Ok(Ok((*lang, file.clone())));
        }
        let extra = self.ts_value("treesit-extra-load-path")?;
        if extra != Value::NIL && self.pair(extra).is_none() {
            return Err(Fault::Wrong("listp", extra));
        }
        let mut base = format!("libtree-sitter-{name}").into_bytes();
        let mut c_name = format!("tree_sitter_{}", name.replace('-', "_")).into_bytes();
        let overrides = self.ts_value("treesit-load-name-override-list")?;
        for entry in self.ts_list_values(overrides) {
            let Some((lang, rest)) = self.pair(entry) else { return Err(Fault::Wrong("listp", entry)) };
            self.ts_symbol(lang)?;
            if lang == language {
                let items = self.ts_list_values(rest);
                let (library, function) = (items.first().copied().unwrap_or(Value::NIL), items.get(1).copied().unwrap_or(Value::NIL));
                base = self.ts_string_bytes(library)?;
                c_name = self.ts_string_bytes(function)?;
                break;
            }
        }
        let mut stems = Vec::new();
        for dir in self.ts_list_values(extra) {
            stems.push(self.ts_expand(&base, dir)?);
        }
        let user = self.ts_value("user-emacs-directory")?;
        let mut sub = b"tree-sitter/".to_vec();
        sub.extend_from_slice(&base);
        stems.push(self.ts_expand(&sub, user)?);
        stems.push(base);
        let suffixes = self.ts_value("dynamic-library-suffixes")?;
        let suffixes: Vec<Vec<u8>> = self.ts_list_values(suffixes).into_iter().filter_map(|s| self.string_bytes(s).ok()).collect();
        let mut errors = Vec::new();
        let mut opened = None;
        'stems: for stem in &stems {
            // GNU conses each suffix's names before the last's.
            for suffix in suffixes.iter().rev() {
                let mut plain = stem.clone();
                plain.extend_from_slice(suffix);
                let mut candidates = vec![plain.clone()];
                for version in (native::MIN_ABI_VERSION..=native::ABI_VERSION).rev() {
                    candidates.push([&plain[..], format!(".{version}.0").as_bytes()].concat());
                }
                candidates.push([&plain[..], b".0"].concat());
                candidates.push([&plain[..], b".0.0"].concat());
                for candidate in candidates {
                    match native::open_library(&candidate) {
                        Ok(handle) => {
                            opened = Some((handle, candidate));
                            break 'stems;
                        }
                        Err(message) => errors.push(message),
                    }
                }
            }
        }
        let Some((handle, library)) = opened else {
            let mut data = vec![self.intern("not-found")];
            for message in errors {
                data.push(self.make_string(message.into_bytes(), false));
            }
            return Ok(Err(self.ts_list(&data)));
        };
        let (lang, file) = match native::language_symbol(handle, &c_name) {
            Ok(found) => found,
            Err(message) => {
                let data = [self.intern("symbol-error"), self.make_string(message.into_bytes(), false)];
                return Ok(Err(self.ts_list(&data)));
            }
        };
        if !lang.compatible() {
            let library = String::from_utf8_lossy(&library);
            let message = format!("{library}'s ABI version is {}, but supported versions are {}-{}", lang.abi_version(), native::MIN_ABI_VERSION, native::ABI_VERSION);
            let data = [self.intern("language-grammar-version-mismatch"), self.string(&message)];
            return Ok(Err(self.ts_list(&data)));
        }
        LANGUAGES.lock().unwrap().push((name, lang, file.clone()));
        Ok(Ok((lang, file)))
    }
    fn ts_string_bytes(&self, v: Value) -> R<Vec<u8>> {
        if !self.ts_is_string(v) { return Err(Fault::Wrong("stringp", v)); }
        self.string_bytes(v)
    }
    /// Fexpand_file_name of NAME (bytes) under DIR, as host bytes.
    fn ts_expand(&mut self, name: &[u8], dir: Value) -> R<Vec<u8>> {
        let name = self.make_string(name.to_vec(), false);
        let expanded = self.vm.expand_file_name_legacy(name, dir).map_err(Fault::Error)?;
        self.file_name_bytes(expanded)
    }
    /// The grammar of LANGUAGE or its load error.
    fn ts_language(&mut self, language: Value) -> R<native::Language> {
        match self.ts_load(language)? {
            Ok((lang, _)) => Ok(lang),
            Err(data) => {
                let condition = self.intern("treesit-load-language-error");
                Err(Fault::Signal(condition, data))
            }
        }
    }
}

// Parsers.
impl Leaf<'_> {
    /// treesit_check_parser: a parser that is not deleted.
    pub(crate) fn ts_check_parser(&mut self, parser: Value) -> R<()> {
        if self.ts_parser(parser)?.deleted {
            return Err(self.ts_signal("treesit-parser-deleted", &[parser]));
        }
        Ok(())
    }
    fn ts_registry(&self, base: Value) -> &ts::Registry {
        &self.ts_buffer_text(base).expect("a multibyte buffer").treesit
    }
    fn ts_registry_mut(&mut self, base: Value) -> &mut ts::Registry {
        &mut self.ts_buffer_text_mut(base).expect("a multibyte buffer").treesit
    }

    pub(crate) fn ts_parser_create(&mut self, language: Value, buffer: Value, no_reuse: Value, tag: Value) -> Out {
        self.ts_symbol(language)?;
        self.ts_symbol(tag)?;
        let buffer = self.ts_buffer_arg(buffer)?;
        let base = self.ts_base(buffer);
        if tag == Value::TRUE {
            let predicate = [self.intern("not"), Value::TRUE];
            let predicate = self.ts_list(&predicate);
            return Err(self.ts_wrong(predicate, Value::TRUE));
        }
        self.ts_check_size(base)?;
        let remapped = self.ts_remap(language)?;
        self.ts_symbol(remapped)?;
        if no_reuse == Value::NIL {
            let reuse = self.ts_registry(base).parsers.iter().map(|(p, _)| *p).find(|&p| {
                self.ts_parser(p).is_ok_and(|p| p.tag == tag && p.language == language && p.buffer == buffer)
            });
            if let Some(parser) = reuse {
                return Ok(parser);
            }
        }
        let lang = self.ts_language(remapped)?;
        let mut native = native::Parser::new();
        native.set_language(lang);
        let tracking = self.ts_value("treesit-languages-require-line-column-tracking")?;
        let tracks = self.ts_list_values(tracking).contains(&remapped);
        let region = self.ts_region(buffer);
        let linecols = tracks.then(|| {
            let text = self.ts_buffer_text(buffer).expect("a multibyte buffer");
            let beg = ts::linecol_of_pos(text, region.begv_byte, ts::BOB_LINECOL);
            (beg, ts::linecol_of_pos(text, region.zv_byte, beg))
        });
        let core = Rc::new(RefCell::new(ts::Core::new((region.begv_byte, region.zv_byte), linecols)));
        let parser = self.ts_allocate(ts::Object::Parser(ts::Parser {
            buffer,
            language,
            tag,
            embed_level: Value::NIL,
            notifiers: Value::NIL,
            last_set_ranges: Value::NIL,
            native: Rc::new(RefCell::new(native)),
            core: core.clone(),
            timestamp: 0,
            deleted: false,
            within_reparse: false,
        }));
        let text = self.ts_buffer_text_mut(base).expect("a multibyte buffer");
        if tracks && text.ts_linecols[0].bytepos == 0 {
            text.ts_linecols = [ts::BOB_LINECOL; 3];
        }
        text.treesit.parsers.insert(0, (parser, core));
        Ok(parser)
    }

    pub(crate) fn ts_parser_delete(&mut self, parser: Value) -> Out {
        self.ts_check_parser(parser)?;
        let base = self.ts_base(self.ts_parser(parser)?.buffer);
        if self.ts_buffer_text(base).is_some() {
            self.ts_registry_mut(base).parsers.retain(|(p, _)| *p != parser);
        }
        self.ts_parser_mut(parser).deleted = true;
        Ok(Value::NIL)
    }

    pub(crate) fn ts_parser_list(&mut self, buffer: Value, language: Value, tag: Value) -> Out {
        let buffer = self.ts_buffer_arg(buffer)?;
        let base = self.ts_base(buffer);
        let parsers: Vec<Value> = self.ts_registry(base).parsers.iter().map(|(p, _)| *p).filter(|&p| {
            self.ts_parser(p).is_ok_and(|p| (language == Value::NIL || p.language == language) && (tag == Value::TRUE || p.tag == tag) && p.buffer == buffer)
        }).collect();
        Ok(self.ts_list(&parsers))
    }

    pub(crate) fn ts_parser_field(&mut self, parser: Value, field: fn(&ts::Parser) -> Value) -> Out {
        self.ts_check_parser(parser)?;
        Ok(field(self.ts_parser(parser)?))
    }

    pub(crate) fn ts_parser_set_embed_level(&mut self, parser: Value, level: Value) -> Out {
        self.ts_check_parser(parser)?;
        if level != Value::NIL {
            match self.kind(level) {
                Kind::Integer if level.integer_value().unwrap_or(0) < 0 => return Err(Fault::Range1(level)),
                Kind::Integer | Kind::Bignum | Kind::Float => {}
                _ => return Err(Fault::Wrong("numberp", level)),
            }
        }
        self.ts_parser_mut(parser).embed_level = level;
        Ok(level)
    }

    pub(crate) fn ts_tracking(&mut self, buffer: Value) -> Out {
        let buffer = if buffer == Value::NIL { self.current_buffer()? } else { buffer };
        if !self.lifecycle_buffer_p(buffer) { return Err(Fault::Wrong("bufferp", buffer)); }
        Ok(flag(self.ts_linecols(buffer)[0].bytepos != 0))
    }

    pub(crate) fn ts_parser_tracking(&mut self, parser: Value) -> Out {
        Ok(flag(self.ts_parser(parser)?.core.borrow().linecols.is_some()))
    }

    pub(crate) fn ts_make_node(&mut self, parser: Value, node: native::Node) -> Value {
        let p = self.ts_parser(parser).ok().expect("a parser");
        let tree = p.core.borrow().tree.clone().expect("a parsed tree");
        let timestamp = p.timestamp;
        self.ts_allocate(ts::Object::Node(ts::Node { parser, node, tree, timestamp }))
    }

    /// treesit_sync_visible_region for PARSER, and its ranges' fix-up.
    fn ts_sync_visible(&mut self, parser: Value) -> R<()> {
        let p = self.ts_parser(parser)?;
        let (buffer, core) = (p.buffer, p.core.clone());
        if !self.ts_live(buffer) {
            return Ok(());
        }
        let region = self.ts_region(buffer);
        let linecols = (self.ts_linecols(buffer)[0].bytepos != 0).then(|| {
            let text = self.ts_buffer_text(buffer).expect("a multibyte buffer");
            let caches = text.ts_linecols;
            (ts::linecol_of_pos(text, region.begv_byte, caches[0]), ts::linecol_of_pos(text, region.zv_byte, caches[2]))
        });
        let had_tree = core.borrow().tree.is_some();
        core.borrow_mut().sync_visible(region.begv_byte, region.zv_byte, linecols);
        if !had_tree {
            return Ok(());
        }
        // Clip the ranges last set to the accessible region (GNU fixes the
        // character ranges, then sets the byte ones).
        let ranges = self.ts_parser(parser)?.last_set_ranges;
        if ranges == Value::NIL {
            return Ok(());
        }
        let (begv, zv) = (self.ts_char(buffer, region.begv_byte), self.ts_char(buffer, region.zv_byte));
        let (mut head, mut previous, mut tail) = (ranges, Value::NIL, ranges);
        while let Some((range, next)) = self.pair(tail) {
            let (b, e) = self.pair(range).map(|(b, e)| (b.integer_value().unwrap_or(0), e.integer_value().unwrap_or(0))).unwrap_or((0, 0));
            if e <= begv {
                head = self.pair(head).map_or(Value::NIL, |(_, rest)| rest);
            } else if b >= zv {
                if previous == Value::NIL { head = Value::NIL; } else { self.setcdr(previous, Value::NIL)?; }
                break;
            } else {
                if b < begv { self.setcar(range, fixnum(begv))?; }
                if e > zv { self.setcdr(range, fixnum(zv))?; }
            }
            previous = tail;
            tail = next;
        }
        if head == Value::NIL {
            let zero = self.cons(fixnum(begv), fixnum(begv));
            head = self.cons(zero, Value::NIL);
        }
        self.ts_parser_mut(parser).last_set_ranges = head;
        let bytes = self.ts_byte_ranges(parser, head);
        self.ts_parser(parser)?.native.borrow_mut().set_included_ranges(&bytes);
        Ok(())
    }
    /// treesit_make_ts_ranges: RANGES (characters) as offsets from the
    /// parser's visible beginning.
    fn ts_byte_ranges(&self, parser: Value, ranges: Value) -> Vec<(u32, u32)> {
        let p = self.ts_parser(parser).ok().expect("a parser");
        let (buffer, beg) = (p.buffer, p.core.borrow().visible.0);
        self.ts_list_values(ranges).into_iter().filter_map(|r| self.pair(r)).map(|(b, e)| {
            let b = self.ts_byte(buffer, b.integer_value().unwrap_or(1)) - beg;
            let e = self.ts_byte(buffer, e.integer_value().unwrap_or(1)) - beg;
            (b as u32, e as u32)
        }).collect()
    }

    /// treesit_ensure_parsed up to its notifiers: the changed ranges when it
    /// parsed, None when no parse was due. A parse leaves the parser
    /// `within_reparse' for its notifiers (`ts_reparse_done').
    pub(crate) fn ts_reparse(&mut self, parser: Value) -> R<Option<Value>> {
        if self.ts_parser(parser)?.within_reparse {
            return Ok(None);
        }
        self.ts_parser_mut(parser).within_reparse = true;
        let buffer = self.ts_parser(parser)?.buffer;
        self.ts_check_size(self.ts_base(buffer))?;
        self.ts_sync_visible(parser)?;
        let p = self.ts_parser(parser)?;
        let (core, native) = (p.core.clone(), p.native.clone());
        if !core.borrow().need_reparse {
            self.ts_parser_mut(parser).within_reparse = false;
            return Ok(None);
        }
        let (old, visible) = { let c = core.borrow(); (c.tree.clone(), c.visible) };
        let new = {
            let text = self.ts_buffer_text(buffer).filter(|_| self.ts_live(buffer)).map(|t| &**t);
            let mut reader = ts::Reader::new(text, ((visible.0 - 1) as usize, (visible.1 - 1) as usize));
            native.borrow_mut().parse(old.as_deref(), &mut reader)
        };
        let Some(new) = new else {
            return Err(self.ts_signal("treesit-parse-error", &[buffer]));
        };
        let new = Rc::new(new);
        let changed = old.as_ref().map(|old| new.changed_since(old));
        {
            let mut c = core.borrow_mut();
            c.tree = Some(new);
            c.need_reparse = false;
        }
        self.ts_parser_mut(parser).timestamp += 1;
        let ranges = match changed {
            Some(changed) => {
                let pairs: Vec<Value> = changed.into_iter().map(|(b, e)| {
                    let (b, e) = (self.ts_char(buffer, visible.0 + i64::from(b)), self.ts_char(buffer, visible.0 + i64::from(e)));
                    self.cons(fixnum(b), fixnum(e))
                }).collect();
                self.ts_list(&pairs)
            }
            None => {
                let region = self.ts_region(buffer);
                let whole = self.cons(fixnum(self.ts_char(buffer, region.begv_byte)), fixnum(self.ts_char(buffer, region.zv_byte)));
                self.cons(whole, Value::NIL)
            }
        };
        Ok(Some(ranges))
    }
    pub(crate) fn ts_reparse_done(&mut self, parser: Value) {
        if self.ts_parser(parser).is_ok() {
            self.ts_parser_mut(parser).within_reparse = false;
        }
    }
    /// The root node of PARSER's current tree; a parser never parsed (its
    /// notifier re-entered it) parses nothing here.
    pub(crate) fn ts_root(&mut self, parser: Value) -> Out {
        let tree = self.ts_parser(parser)?.core.borrow().tree.clone();
        match tree {
            Some(tree) => Ok(self.ts_make_node(parser, native::Node(tree.root()))),
            None => Err(self.ts_signal("treesit-parse-error", &[self.ts_parser(parser)?.buffer])),
        }
    }

    // Ranges.

    pub(crate) fn ts_set_included_ranges(&mut self, parser: Value, ranges: Value) -> Out {
        self.ts_check_parser(parser)?;
        if ranges != Value::NIL && self.pair(ranges).is_none() { return Err(Fault::Wrong("consp", ranges)); }
        let last = self.ts_parser(parser)?.last_set_ranges;
        if self.equal(last, ranges).unwrap_or(false) {
            return Ok(Value::NIL);
        }
        self.ts_check_ranges(ranges)?;
        let buffer = self.ts_parser(parser)?.buffer;
        self.ts_check_size(buffer)?;
        self.ts_sync_visible(parser)?;
        self.ts_parser_mut(parser).last_set_ranges = ranges;
        let bytes = self.ts_byte_ranges(parser, ranges);
        if !self.ts_parser(parser)?.native.borrow_mut().set_included_ranges(&bytes) {
            return Err(self.ts_message("treesit-range-invalid", "Something went wrong when setting ranges", &[ranges]));
        }
        self.ts_parser(parser)?.core.borrow_mut().need_reparse = true;
        Ok(Value::NIL)
    }
    /// treesit_check_range_argument, in the current buffer.
    fn ts_check_ranges(&mut self, ranges: Value) -> R<()> {
        let buffer = self.current_buffer()?;
        let region = self.ts_region(buffer);
        let (begv, zv) = (self.ts_char(buffer, region.begv_byte), self.ts_char(buffer, region.zv_byte));
        let mut last = begv;
        let mut tail = ranges;
        while tail != Value::NIL {
            let Some((range, next)) = self.pair(tail) else { return Err(Fault::Wrong("listp", ranges)) };
            let Some((b, e)) = self.pair(range) else { return Err(Fault::Wrong("consp", range)) };
            let b = b.integer_value().filter(|_| self.kind(b) == Kind::Integer).ok_or(Fault::Wrong("fixnump", b))?;
            let e = e.integer_value().filter(|_| self.kind(e) == Kind::Integer).ok_or(Fault::Wrong("fixnump", e))?;
            if !(last <= b && b <= e && e <= zv) {
                return Err(self.ts_message("treesit-range-invalid", "RANGE is either overlapping, out-of-order or out-of-range", &[ranges]));
            }
            last = e;
            tail = next;
        }
        Ok(())
    }
    pub(crate) fn ts_included_ranges(&mut self, parser: Value) -> Out {
        self.ts_check_parser(parser)?;
        self.ts_sync_visible(parser)?;
        Ok(self.ts_parser(parser)?.last_set_ranges)
    }

    // Notifiers.

    pub(crate) fn ts_notifiers(&mut self, parser: Value) -> Out {
        self.ts_check_parser(parser)?;
        let mut functions = self.ts_list_values(self.ts_parser(parser)?.notifiers);
        functions.reverse();
        Ok(self.ts_list(&functions))
    }
    pub(crate) fn ts_add_notifier(&mut self, parser: Value, function: Value) -> Out {
        self.ts_check_parser(parser)?;
        self.ts_symbol(function)?;
        let functions = self.ts_parser(parser)?.notifiers;
        if !self.ts_list_values(functions).contains(&function) {
            let functions = self.cons(function, functions);
            self.ts_parser_mut(parser).notifiers = functions;
        }
        Ok(Value::NIL)
    }
    pub(crate) fn ts_remove_notifier(&mut self, parser: Value, function: Value) -> Out {
        self.ts_check_parser(parser)?;
        self.ts_symbol(function)?;
        let functions: Vec<Value> = self.ts_list_values(self.ts_parser(parser)?.notifiers).into_iter().filter(|&f| f != function).collect();
        let functions = self.ts_list(&functions);
        self.ts_parser_mut(parser).notifiers = functions;
        Ok(Value::NIL)
    }
}

/// treesit_cursor_helper_1: move CURSOR down to TARGET, whose bytes are
/// START..END, within LIMIT levels; false leaves CURSOR where it was.
fn cursor_to(cursor: &mut native::Cursor, target: native::Node, start: u32, end: u32, limit: i64) -> bool {
    if limit <= 0 {
        return false;
    }
    let mut node = cursor.node();
    if node.same(target) {
        return true;
    }
    if !((start != end && cursor.first_child_for_byte(start) != -1) || cursor.first_child()) {
        return false;
    }
    node = cursor.node();
    while node.start() <= end {
        if node.end() >= end && cursor_to(cursor, target, start, end, limit - 1) {
            return true;
        }
        if !cursor.next_sibling() {
            break;
        }
        node = cursor.node();
    }
    cursor.parent();
    false
}

/// treesit_cursor_helper: a cursor on NODE made from TREE's root, so its
/// parents and siblings are reachable; None past the recursion limit.
pub(crate) fn cursor_on(tree: &native::Tree, node: native::Node) -> Option<native::Cursor> {
    let mut cursor = native::Cursor::new(native::Node(tree.root()));
    cursor_to(&mut cursor, node, node.start(), node.end(), RECURSION_LIMIT).then_some(cursor)
}

// Nodes.
impl Leaf<'_> {
    pub(crate) fn ts_node_uptodate(&self, node: Value) -> bool {
        let n = self.ts_node(node).ok().expect("a node");
        self.ts_parser(n.parser).is_ok_and(|p| p.timestamp == n.timestamp)
    }
    pub(crate) fn ts_node_buffer_live(&self, node: Value) -> bool {
        let n = self.ts_node(node).ok().expect("a node");
        self.ts_parser(n.parser).is_ok_and(|p| self.ts_live(p.buffer))
    }
    /// treesit_check_node: an up-to-date node of a live buffer.
    pub(crate) fn ts_check_node(&mut self, node: Value) -> R<ts::Node> {
        let n = self.ts_node(node)?.clone();
        if !self.ts_node_uptodate(node) {
            return Err(self.ts_signal("treesit-node-outdated", &[node]));
        }
        if !self.ts_node_buffer_live(node) {
            return Err(self.ts_signal("treesit-node-buffer-killed", &[node]));
        }
        Ok(n)
    }
    fn ts_node_buffer(&self, n: &ts::Node) -> (Value, i64) {
        let p = self.ts_parser(n.parser).ok().expect("a parser");
        (p.buffer, p.core.borrow().visible.0)
    }
    /// A node's tree offset as a buffer position.
    pub(crate) fn ts_offset_pos(&self, n: &ts::Node, offset: u32) -> i64 {
        let (buffer, beg) = self.ts_node_buffer(n);
        self.ts_char(buffer, beg + i64::from(offset))
    }
    fn ts_wrap(&mut self, n: &ts::Node, found: Option<native::Node>) -> Value {
        match found {
            Some(node) => self.ts_allocate(ts::Object::Node(ts::Node { node, ..n.clone() })),
            None => Value::NIL,
        }
    }

    pub(crate) fn ts_node_type(&mut self, node: Value) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let n = self.ts_check_node(node)?;
        let kind = n.node.kind().unwrap_or(b"");
        Ok(self.ts_text_value(kind))
    }
    /// build_string: unibyte when ASCII.
    pub(crate) fn ts_text_value(&mut self, bytes: &[u8]) -> Value {
        self.make_string(bytes.to_vec(), !bytes.is_ascii())
    }
    pub(crate) fn ts_node_bound(&mut self, node: Value, end: bool) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let n = self.ts_check_node(node)?;
        let offset = if end { n.node.end() } else { n.node.start() };
        Ok(fixnum(self.ts_offset_pos(&n, offset)))
    }
    pub(crate) fn ts_node_string(&mut self, node: Value) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let n = self.ts_check_node(node)?;
        let sexp = n.node.sexp();
        Ok(self.ts_text_value(&sexp))
    }
    pub(crate) fn ts_node_parent(&mut self, node: Value) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let n = self.ts_check_node(node)?;
        let Some(mut cursor) = cursor_on(&n.tree, n.node) else { return Ok(Value::NIL) };
        let parent = cursor.parent().then(|| cursor.node());
        Ok(self.ts_wrap(&n, parent))
    }
    /// A child index N (negative from the end) of COUNT children, or None.
    fn ts_index(&self, n: Value, count: u32) -> R<Option<u32>> {
        if !matches!(self.kind(n), Kind::Integer | Kind::Bignum) { return Err(Fault::Wrong("integerp", n)); }
        let mut index = n.integer_value().unwrap_or(i64::MAX);
        if index < 0 { index += i64::from(count); }
        if index < 0 { return Ok(None); }
        if index > i64::from(u32::MAX) { return Err(Fault::Range1(n)); }
        Ok(Some(index as u32))
    }
    pub(crate) fn ts_node_child(&mut self, node: Value, n: Value, named: Value) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let node_data = self.ts_check_node(node)?;
        let named = named != Value::NIL;
        let Some(index) = self.ts_index(n, node_data.node.child_count(named))? else { return Ok(Value::NIL) };
        let child = node_data.node.child(index, named);
        Ok(self.ts_wrap(&node_data, child))
    }
    pub(crate) fn ts_node_check(&mut self, node: Value, property: Value) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        self.ts_node(node)?;
        self.ts_symbol(property)?;
        let name = self.symbol_name(property)?;
        if name == "outdated" {
            return Ok(flag(!self.ts_node_uptodate(node)));
        }
        let n = self.ts_check_node(node)?;
        let result = match name.as_str() {
            "named" => n.node.is_named(),
            "missing" => n.node.is_missing(),
            "extra" => n.node.is_extra(),
            "has-error" => n.node.has_error(),
            "live" => {
                let p = self.ts_parser(n.parser)?;
                !p.deleted && self.ts_live(p.buffer)
            }
            _ => {
                let message = self.string("Expecting `named', `missing', `extra', `outdated', `has-error', or `live', but got");
                let error = self.intern("error");
                let data = self.ts_list(&[message, property]);
                return Err(Fault::Signal(error, data));
            }
        };
        Ok(flag(result))
    }
    pub(crate) fn ts_node_field_name_for_child(&mut self, node: Value, n: Value) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let node_data = self.ts_check_node(node)?;
        let Some(index) = self.ts_index(n, node_data.node.child_count(false))? else { return Ok(Value::NIL) };
        Ok(node_data.node.field_name_for_child(index).map_or(Value::NIL, |name| self.ts_text_value(name)))
    }
    pub(crate) fn ts_node_child_count(&mut self, node: Value, named: Value) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let n = self.ts_check_node(node)?;
        Ok(fixnum(i64::from(n.node.child_count(named != Value::NIL))))
    }
    pub(crate) fn ts_node_child_by_field_name(&mut self, node: Value, field: Value) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let n = self.ts_check_node(node)?;
        let field = self.ts_string_bytes(field)?;
        let child = n.node.child_by_field_name(&field);
        Ok(self.ts_wrap(&n, child))
    }
    pub(crate) fn ts_node_sibling(&mut self, node: Value, named: Value, next: bool) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let n = self.ts_check_node(node)?;
        let sibling = match (next, named != Value::NIL) {
            (true, false) => n.node.next_sibling(),
            (true, true) => n.node.next_named_sibling(),
            (false, false) => n.node.prev_sibling(),
            (false, true) => n.node.prev_named_sibling(),
        };
        Ok(self.ts_wrap(&n, sibling))
    }
    pub(crate) fn ts_node_first_child_for_pos(&mut self, node: Value, pos: Value, named: Value) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let n = self.ts_check_node(node)?;
        let (buffer, beg) = self.ts_node_buffer(&n);
        let pos = self.ts_position(pos, buffer)?;
        let target = (self.ts_byte(buffer, pos) - beg) as u32;
        let mut cursor = native::Cursor::new(n.node);
        let named = named != Value::NIL;
        // treesit_cursor_first_child_for_byte (bug#60127).
        let found = (|| {
            if cursor.first_child_for_byte(target) == -1 && !cursor.first_child() { return false; }
            while cursor.node().end() <= target {
                if !cursor.next_sibling() { return false; }
            }
            while named && !cursor.node().is_named() {
                if !cursor.next_sibling() { return false; }
            }
            true
        })();
        let child = found.then(|| cursor.node());
        Ok(self.ts_wrap(&n, child))
    }
    pub(crate) fn ts_node_descendant_for_range(&mut self, node: Value, beg: Value, end: Value, named: Value) -> Out {
        if node == Value::NIL { return Ok(Value::NIL); }
        let n = self.ts_check_node(node)?;
        let (buffer, visible) = self.ts_node_buffer(&n);
        let (beg, end) = (self.ts_position(beg, buffer)?, self.ts_position(end, buffer)?);
        let (beg, end) = (self.ts_byte(buffer, beg) - visible, self.ts_byte(buffer, end) - visible);
        let found = n.node.descendant_for_range(beg as u32, end as u32, named != Value::NIL);
        Ok(self.ts_wrap(&n, found))
    }
    pub(crate) fn ts_node_eq(&mut self, a: Value, b: Value) -> Out {
        if a == Value::NIL || b == Value::NIL { return Ok(Value::NIL); }
        let (a, b) = (self.ts_node(a)?.node, self.ts_node(b)?.node);
        Ok(flag(a.same(b)))
    }
    pub(crate) fn ts_node_parser(&mut self, node: Value) -> Out {
        Ok(self.ts_node(node)?.parser)
    }
    /// treesit-subtree-stat: (MAX-DEPTH MAX-WIDTH COUNT) of NODE's subtree.
    pub(crate) fn ts_subtree_stat(&mut self, node: Value) -> Out {
        let n = self.ts_node(node)?.clone();
        let mut cursor = native::Cursor::new(n.node);
        let (mut max_depth, mut max_width, mut count, mut depth) = (1i64, 0i64, 0i64, 0i64);
        loop {
            count += 1;
            while cursor.first_child() {
                depth += 1;
                count += 1;
                let mut width = 1;
                while cursor.next_sibling() { width += 1; }
                max_width = max_width.max(width);
                cursor.parent();
                cursor.first_child();
            }
            max_depth = max_depth.max(depth);
            while !cursor.next_sibling() {
                if cursor.parent() {
                    depth -= 1;
                } else {
                    return Ok(self.ts_list(&[fixnum(max_depth), fixnum(max_width), fixnum(count)]));
                }
            }
        }
    }
}

/// treesit_query_string_string: STRING as a query string literal.
fn query_string(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![b'"'];
    for &c in bytes {
        match c {
            0 => out.extend_from_slice(b"\\0"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'"' | b'\\' => out.extend_from_slice(&[b'\\', c]),
            _ => out.push(c),
        }
    }
    out.push(b'"');
    out
}

/// A query's text: its bytes and whether a multibyte string supplied any.
#[derive(Default)]
pub(crate) struct Expanded {
    pub bytes: Vec<u8>,
    pub multibyte: bool,
}

// Queries.
impl Leaf<'_> {
    /// treesit-pattern-expand of PATTERN, appended to OUT.
    fn ts_expand_pattern(&mut self, pattern: Value, out: &mut Expanded) -> R<()> {
        if self.kind(pattern) == Kind::Symbol {
            let name = self.symbol_name(pattern)?;
            let keyword = match name.as_str() {
                ":anchor" => Some("."),
                ":?" => Some("?"),
                ":*" => Some("*"),
                ":+" => Some("+"),
                ":equal" | ":eq?" => Some("#eq?"),
                ":match" | ":match?" => Some("#match?"),
                ":pred" | ":pred?" => Some("#pred?"),
                _ => None,
            };
            if let Some(text) = keyword {
                out.bytes.extend_from_slice(text.as_bytes());
                return Ok(());
            }
        }
        let vector = self.kind(pattern) == Kind::Vector;
        if vector || self.pair(pattern).is_some() {
            out.bytes.push(if vector { b'[' } else { b'(' });
            let items = if vector { self.vector_values(pattern)? } else { self.ts_proper_list(pattern)? };
            for (i, item) in items.into_iter().enumerate() {
                if i > 0 { out.bytes.push(b' '); }
                self.ts_expand_pattern(item, out)?;
            }
            out.bytes.push(if vector { b']' } else { b')' });
            return Ok(());
        }
        if self.ts_is_string(pattern) {
            let view = self.string_view(pattern)?;
            out.multibyte |= view.is_multibyte();
            let quoted = query_string(view.bytes());
            out.bytes.extend_from_slice(&quoted);
            return Ok(());
        }
        let printed = self.print_string(pattern, true, Value::TRUE, Value::NIL)?;
        let view = self.string_view(printed)?;
        out.multibyte |= view.is_multibyte();
        out.bytes.extend_from_slice(view.bytes());
        Ok(())
    }
    fn ts_proper_list(&self, list: Value) -> R<Vec<Value>> {
        let mut out = Vec::new();
        let mut tail = list;
        while tail != Value::NIL {
            let Some((car, cdr)) = self.pair(tail) else { return Err(Fault::Wrong("listp", tail)) };
            out.push(car);
            tail = cdr;
        }
        Ok(out)
    }
    fn ts_expanded_value(&mut self, expanded: Expanded) -> Value {
        let multibyte = expanded.multibyte && !expanded.bytes.is_ascii();
        self.make_string(expanded.bytes, multibyte)
    }
    pub(crate) fn ts_pattern_expand(&mut self, pattern: Value) -> Out {
        let mut out = Expanded::default();
        self.ts_expand_pattern(pattern, &mut out)?;
        Ok(self.ts_expanded_value(out))
    }
    /// treesit-query-expand: the patterns of QUERY joined by spaces.
    pub(crate) fn ts_query_expand_bytes(&mut self, query: Value) -> R<Expanded> {
        let mut out = Expanded::default();
        let items = if self.kind(query) == Kind::Vector { self.vector_values(query)? } else { self.ts_proper_list(query)? };
        for (i, item) in items.into_iter().enumerate() {
            if i > 0 { out.bytes.push(b' '); }
            self.ts_expand_pattern(item, &mut out)?;
        }
        Ok(out)
    }
    pub(crate) fn ts_query_expand(&mut self, query: Value) -> Out {
        let out = self.ts_query_expand_bytes(query)?;
        Ok(self.ts_expanded_value(out))
    }

    pub(crate) fn ts_query_p(&self, object: Value) -> bool {
        matches!(self.ts(object), Some(ts::Object::Query(_))) || self.pair(object).is_some() || self.ts_is_string(object)
    }

    /// SOURCE (a string, or a sexp expanded) compiled for LANGUAGE, or the
    /// `treesit-query-error' it signals.
    fn ts_compile(&mut self, language: native::Language, source: Value) -> R<Rc<ts::Compiled>> {
        let (text, source) = if self.pair(source).is_some() {
            let expanded = self.ts_query_expand_bytes(source)?;
            let bytes = expanded.bytes.clone();
            (bytes, self.ts_expanded_value(expanded))
        } else {
            (self.string_bytes(source)?, source)
        };
        match native::Query::new(language, &text) {
            Ok(query) => Ok(Rc::new(ts::Compiled::new(query))),
            Err((offset, message)) => {
                let data = [self.string(message), fixnum(i64::from(offset) + 1), source, self.string("Debug the query with `treesit-query-validate'")];
                Err(self.ts_signal("treesit-query-error", &data))
            }
        }
    }
    /// treesit_ensure_query_compiled for the compiled query QUERY.
    pub(crate) fn ts_ensure_compiled(&mut self, query: Value) -> R<Rc<ts::Compiled>> {
        let q = self.ts_query(query)?;
        if let Some(compiled) = &q.compiled {
            return Ok(compiled.clone());
        }
        let (language, source) = (q.language, q.source);
        let remapped = self.ts_remap(language)?;
        if self.kind(remapped) != Kind::Symbol {
            let message = self.string("Invalid language symbol");
            return Err(self.ts_signal("treesit-query-error", &[message, remapped]));
        }
        let lang = self.ts_language(remapped)?;
        let compiled = self.ts_compile(lang, source)?;
        if let ts::Object::Query(q) = self.ts_mut(query) {
            q.compiled = Some(compiled.clone());
        }
        Ok(compiled)
    }
    pub(crate) fn ts_query_compile(&mut self, language: Value, query: Value, eager: Value) -> Out {
        if !self.ts_query_p(query) {
            let predicate = self.intern("treesit-query-p");
            return Err(self.ts_wrong(predicate, query));
        }
        self.ts_symbol(language)?;
        let compiled = if matches!(self.ts(query), Some(ts::Object::Query(_))) {
            query
        } else {
            self.ts_allocate(ts::Object::Query(ts::Query { language, source: query, compiled: None }))
        };
        if eager != Value::NIL {
            self.ts_ensure_compiled(compiled)?;
        }
        Ok(compiled)
    }
    pub(crate) fn ts_query_field(&mut self, query: Value, language: bool) -> Out {
        let q = self.ts_query(query)?;
        Ok(if language { q.language } else { q.source })
    }
    pub(crate) fn ts_query_eager(&mut self, query: Value) -> Out {
        Ok(flag(self.ts_query(query)?.compiled.is_some()))
    }
}

/// What a suspendable primitive asks next: its value, or a Lisp call whose
/// value resumes TASK.
pub(crate) enum Next {
    Done(Value),
    Call { f: Value, args: Vec<Value>, task: Value },
}

/// One predicate's verdict on a match.
enum Verdict {
    Pass,
    Fail,
    /// `#pred?': call F with ARGS.
    Call(Value, Vec<Value>),
}

// The task values of treesit-query-capture.
const CAPTURE_PARSER: usize = 0;
const CAPTURE_FN: usize = 1;
const CAPTURE_FLAGS: usize = 3;

// treesit-query-capture.
impl Leaf<'_> {
    fn ts_capture_state(&self, task: Value) -> std::cell::RefMut<'_, ts::Capture> {
        std::cell::RefMut::map(self.ts_task(task).state.borrow_mut(), |s| match s {
            ts::TaskState::Capture(c) => c,
            _ => unreachable!("a capture task"),
        })
    }
    /// treesit-query-capture on the up-to-date NODE, until its first
    /// `#pred?' call.
    pub(crate) fn ts_capture(&mut self, node: Value, query: Value, beg: Value, end: Value, node_only: Value, grouped: Value) -> R<Next> {
        let n = self.ts_check_node(node)?;
        let (buffer, visible_beg) = self.ts_node_buffer(&n);
        let beg = if beg == Value::NIL { None } else { Some(self.ts_position(beg, buffer)?) };
        let end = if end == Value::NIL { None } else { Some(self.ts_position(end, buffer)?) };
        let compiled = if matches!(self.ts(query), Some(ts::Object::Query(_))) {
            self.ts_ensure_compiled(query)?
        } else {
            let language = self.ts_parser(n.parser)?.native.borrow().language();
            self.ts_compile(language, query)?
        };
        let range = match (beg, end) {
            (Some(b), Some(e)) => {
                let (b, e) = (self.ts_byte(buffer, b), self.ts_byte(buffer, e));
                if b == visible_beg && e == visible_beg {
                    return Ok(Next::Done(Value::NIL));
                }
                ((b - visible_beg) as u32, (e - visible_beg) as u32)
            }
            _ => (0, u32::MAX),
        };
        let found = compiled.query.matches(n.node, range);
        let mut names: Vec<Option<Value>> = Vec::new();
        let mut values = vec![n.parser, Value::NIL, Value::NIL, fixnum(i64::from(node_only != Value::NIL) | i64::from(grouped != Value::NIL) << 1)];
        let mut matches = Vec::with_capacity(found.len());
        for m in found {
            let start = values.len();
            for &(node, index) in &m.captures {
                let index = index as usize;
                if names.len() <= index { names.resize(index + 1, None); }
                let name = match names[index] {
                    Some(name) => name,
                    None => {
                        let name = String::from_utf8_lossy(compiled.query.capture_name(index as u32)).into_owned();
                        let name = self.intern(&name);
                        names[index] = Some(name);
                        name
                    }
                };
                let node = self.ts_allocate(ts::Object::Node(ts::Node { node, ..n.clone() }));
                values.push(name);
                values.push(node);
            }
            matches.push((m.pattern, start, m.captures.len()));
        }
        let timestamp = self.ts_parser(n.parser)?.timestamp;
        let state = ts::Capture { compiled, matches, at: 0, predicate: 0, passed: Vec::new(), timestamp };
        let task = self.ts_allocate(ts::Object::Task(ts::Task { values, state: Rc::new(RefCell::new(ts::TaskState::Capture(state))) }));
        self.ts_capture_run(task)
    }

    /// Resume TASK with VALUE, the result of its `#pred?' call.
    pub(crate) fn ts_capture_resume(&mut self, task: Value, value: Value) -> R<Next> {
        let parser = self.ts_task(task).values[CAPTURE_PARSER];
        let timestamp = self.ts_capture_state(task).timestamp;
        if self.ts_parser(parser)?.timestamp != timestamp {
            let f = self.ts_task(task).values[CAPTURE_FN];
            return Err(self.ts_signal("treesit-query-error", &[f]));
        }
        {
            let mut state = self.ts_capture_state(task);
            if value == Value::NIL {
                state.at += 1;
                state.predicate = 0;
            } else {
                state.predicate += 1;
            }
        }
        self.ts_capture_run(task)
    }

    /// NODE's buffer and the 0-based byte it starts at.
    fn ts_node_span(&self, node: Value) -> (Value, usize) {
        let n = self.ts_node(node).ok().expect("a node");
        let (buffer, beg) = self.ts_node_buffer(n);
        (buffer, (beg - 1 + i64::from(n.node.start())) as usize)
    }
    /// The text NODE spans in its buffer, GNU's internal bytes.
    fn ts_node_text(&self, node: Value) -> Vec<u8> {
        let n = self.ts_node(node).ok().expect("a node");
        let (buffer, beg) = self.ts_node_buffer(n);
        let text = self.ts_buffer_text(buffer).expect("a multibyte buffer");
        let (s, e) = ((beg - 1 + i64::from(n.node.start())) as usize, (beg - 1 + i64::from(n.node.end())) as usize);
        let (s, e) = (s.min(text.len_bytes()), e.min(text.len_bytes()));
        text.internal(ByteOffset(s)..ByteOffset(e.max(s))).map(|c| c.into_owned()).unwrap_or_default()
    }

    /// treesit_predicate_capture_name_to_node: the last capture named by
    /// INDEX among the match's PAIRS.
    fn ts_capture_node(&mut self, compiled: &ts::Compiled, pairs: &[Value], index: u32) -> std::result::Result<Value, Vec<Value>> {
        let name = String::from_utf8_lossy(compiled.query.capture_name(index)).into_owned();
        let name = self.intern(&name);
        match pairs.chunks(2).rev().find(|pair| pair[0] == name) {
            Some(pair) => Ok(pair[1]),
            None => Err(vec![self.string("Cannot find captured node"), name, self.string("A predicate can only refer to captured nodes in the same pattern")]),
        }
    }

    /// treesit_eval_predicates for one predicate: its verdict, or the data
    /// of the `treesit-query-error' GNU's validation signals. A regexp
    /// search's own error propagates unchanged.
    fn ts_predicate(&mut self, compiled: &ts::Compiled, predicate: &ts::Predicate, pairs: &[Value]) -> R<std::result::Result<Verdict, Vec<Value>>> {
        let args = &predicate.args;
        let count = fixnum(args.len() as i64);
        macro_rules! node {
            ($index:expr) => {
                match self.ts_capture_node(compiled, pairs, $index) {
                    Ok(node) => node,
                    Err(data) => return Ok(Err(data)),
                }
            };
        }
        Ok(match predicate.name.as_slice() {
            b"eq?" => {
                if args.len() != 2 {
                    return Ok(Err(vec![self.string("Predicate `equal' requires two arguments but got"), count]));
                }
                let mut texts = Vec::new();
                for arg in args {
                    texts.push(match arg {
                        ts::Arg::String(s) => s.clone(),
                        ts::Arg::Capture(i) => {
                            let node = node!(*i);
                            self.ts_node_text(node)
                        }
                    });
                }
                Ok(if texts[0] == texts[1] { Verdict::Pass } else { Verdict::Fail })
            }
            b"match?" => {
                if args.len() != 2 {
                    return Ok(Err(vec![self.string("Predicate `match?' requires two arguments but got"), count]));
                }
                let (regexp, capture) = match (&args[0], &args[1]) {
                    (ts::Arg::String(r), ts::Arg::Capture(c)) | (ts::Arg::Capture(c), ts::Arg::String(r)) => (r, *c),
                    _ => return Ok(Err(vec![self.string("Predicate `match?' takes a regexp and a node capture (order doesn't matter), but got"), count])),
                };
                let node = node!(capture);
                let text = self.ts_node_text(node);
                let span = self.ts_node_span(node);
                let regexp = self.ts_text_value(regexp);
                let multibyte = !text.is_ascii();
                Ok(if self.vm.bytes_search(regexp, &text, multibyte, Some(span))? { Verdict::Pass } else { Verdict::Fail })
            }
            b"pred?" => {
                if args.len() < 2 {
                    return Ok(Err(vec![self.string("Predicate `pred' requires at least two arguments, but only got"), count]));
                }
                let f = match &args[0] {
                    ts::Arg::String(name) => self.intern(&String::from_utf8_lossy(name)),
                    // Fintern of a capture's symbol.
                    ts::Arg::Capture(i) => {
                        let name = String::from_utf8_lossy(compiled.query.capture_name(*i)).into_owned();
                        let symbol = self.intern(&name);
                        return Err(Fault::Wrong("stringp", symbol));
                    }
                };
                let mut nodes = Vec::new();
                for arg in &args[1..] {
                    match arg {
                        ts::Arg::Capture(i) => nodes.push(node!(*i)),
                        ts::Arg::String(s) => {
                            let s = self.ts_text_value(s);
                            return Ok(Err(vec![self.string("Cannot find captured node"), s, self.string("A predicate can only refer to captured nodes in the same pattern")]));
                        }
                    }
                }
                Ok(Verdict::Call(f, nodes))
            }
            name => {
                let name = self.ts_text_value(name);
                Err(vec![self.string("Invalid predicate"), name, self.string("Currently Emacs only supports `equal', `match', and `pred' predicates")])
            }
        })
    }

    /// Run TASK's predicates from where it stopped.
    fn ts_capture_run(&mut self, task: Value) -> R<Next> {
        loop {
            let (compiled, at, predicate, matches_len) = {
                let s = self.ts_capture_state(task);
                (s.compiled.clone(), s.at, s.predicate, s.matches.len())
            };
            if at >= matches_len {
                return self.ts_capture_finish(task);
            }
            let (pattern, start, count) = self.ts_capture_state(task).matches[at];
            let predicates = &compiled.predicates[pattern as usize];
            if predicate >= predicates.len() {
                let mut s = self.ts_capture_state(task);
                s.passed.push(at);
                s.at += 1;
                s.predicate = 0;
                continue;
            }
            let pairs: Vec<Value> = self.ts_task(task).values[start..start + 2 * count].to_vec();
            match self.ts_predicate(&compiled, &predicates[predicate], &pairs)? {
                Ok(Verdict::Pass) => self.ts_capture_state(task).predicate += 1,
                Ok(Verdict::Fail) => {
                    let mut s = self.ts_capture_state(task);
                    s.at += 1;
                    s.predicate = 0;
                }
                Ok(Verdict::Call(f, args)) => {
                    self.ts_task_mut(task).values[CAPTURE_FN] = f;
                    let parser = self.ts_task(task).values[CAPTURE_PARSER];
                    let timestamp = self.ts_parser(parser)?.timestamp;
                    self.ts_capture_state(task).timestamp = timestamp;
                    let buffer = self.ts_parser(parser)?.buffer;
                    if self.current_buffer()? != buffer {
                        // GNU runs the predicates in the parser's buffer.
                        let quote = self.intern("quote");
                        let args = self.ts_list(&args);
                        let (f, args) = (self.ts_list(&[quote, f]), self.ts_list(&[quote, args]));
                        let call = [self.intern("apply"), f, args];
                        let call = self.ts_list(&call);
                        let switch = [self.intern("set-buffer"), buffer];
                        let switch = self.ts_list(&switch);
                        let form = [self.intern("save-current-buffer"), switch, call];
                        let form = self.ts_list(&form);
                        let eval = self.intern("eval");
                        return Ok(Next::Call { f: eval, args: vec![form, Value::TRUE], task });
                    }
                    return Ok(Next::Call { f, args, task });
                }
                Err(data) => {
                    let condition = self.intern("treesit-query-error");
                    let data = self.ts_list(&data);
                    return Err(Fault::Signal(condition, data));
                }
            }
        }
    }
    fn ts_capture_finish(&mut self, task: Value) -> R<Next> {
        let flags = self.ts_task(task).values[CAPTURE_FLAGS].integer_value().unwrap_or(0);
        let (node_only, grouped) = (flags & 1 != 0, flags & 2 != 0);
        let (matches, passed) = {
            let s = self.ts_capture_state(task);
            (s.matches.clone(), s.passed.clone())
        };
        let values = self.ts_task(task).values.clone();
        let mut out = Vec::new();
        for i in passed {
            let (_, start, count) = matches[i];
            let mut items = Vec::with_capacity(count);
            for pair in values[start..start + 2 * count].chunks(2) {
                items.push(if node_only { pair[1] } else { self.cons(pair[0], pair[1]) });
            }
            if grouped {
                out.push(self.ts_list(&items));
            } else {
                out.extend(items);
            }
        }
        Ok(Next::Done(self.ts_list(&out)))
    }
}

/// A thing predicate's verdict on a node, or the function it must call
/// first (guarded when GNU checks the call left the tree alone).
enum Thing {
    Yes,
    No,
    Call { f: Value, guarded: bool },
}

// The task values of a walk.
const WALK_PARSER: usize = 0;
const WALK_PREDICATE: usize = 1;
const WALK_PROCESS: usize = 2;
const WALK_PENDING: usize = 3;
/// A sparse tree's root cons, then each level's parent cons.
const WALK_PARENTS: usize = 4;

/// treesit_traverse_sibling_helper (with ts_tree_cursor_goto_previous_sibling).
fn sibling(cursor: &mut native::Cursor, forward: bool, named: bool) -> bool {
    loop {
        let moved = if forward { cursor.next_sibling() } else { cursor.previous_sibling() };
        if !moved { return false; }
        if !named || cursor.node().is_named() { return true; }
    }
}

/// treesit_traverse_child_helper.
fn child(cursor: &mut native::Cursor, forward: bool, named: bool) -> bool {
    if !cursor.first_child() {
        return false;
    }
    if !forward {
        while cursor.next_sibling() {}
    }
    if !named || cursor.node().is_named() || sibling(cursor, forward, true) {
        return true;
    }
    cursor.parent();
    false
}

impl Leaf<'_> {
    fn ts_walk(&self, task: Value) -> std::cell::RefMut<'_, ts::Walk> {
        std::cell::RefMut::map(self.ts_task(task).state.borrow_mut(), |s| match s {
            ts::TaskState::Walk(w) => w,
            _ => unreachable!("a walk task"),
        })
    }
    /// treesit_traverse_get_predicate: THING's definition for LANGUAGE in
    /// `treesit-thing-settings', or nil.
    fn ts_thing_definition(&mut self, thing: Value, language: Value) -> R<Value> {
        let settings = self.ts_value("treesit-thing-settings")?;
        let Some(entry) = self.ts_assq(language, settings) else { return Ok(Value::NIL) };
        let definitions = self.pair(entry).map_or(Value::NIL, |(_, d)| d);
        let Some(entry) = self.ts_assq(thing, definitions) else { return Ok(Value::NIL) };
        Ok(self.pair(entry).and_then(|(_, rest)| self.pair(rest)).map_or(Value::NIL, |(pred, _)| pred))
    }
    fn ts_thing_symbol(&mut self, pred: Value) -> R<bool> {
        let property = self.intern("treesit-thing-symbol");
        Ok(self.kind(pred) == Kind::Symbol && self.symbol_property(pred, property)? != Value::NIL)
    }
    /// A function predicate: a function, unless a symbol marked as a thing.
    fn ts_function_pred(&mut self, pred: Value) -> R<bool> {
        Ok(crate::prim::prims::vars::function_p(self, pred)? && !self.ts_thing_symbol(pred)?)
    }
    /// treesit_traverse_validate_predicate: the condition and data of what
    /// is wrong with PRED, if anything.
    fn ts_validate(&mut self, pred: Value, language: Value, level: usize) -> R<Option<(&'static str, Value)>> {
        let invalid = |cx: &mut Self, message: &str, pred: Option<Value>| {
            let mut data = vec![cx.string(message)];
            data.extend(pred);
            Some(("treesit-invalid-predicate", cx.ts_list(&data)))
        };
        if level > 99 {
            return Ok(invalid(self, "Predicate recursion level exceeded: it must not exceed 100 levels", None));
        }
        if self.ts_is_string(pred) || self.ts_function_pred(pred)? {
            return Ok(None);
        }
        if self.kind(pred) == Kind::Symbol {
            let name = self.symbol_name(pred)?;
            if name == "named" || name == "anonymous" { return Ok(None); }
            let definition = self.ts_thing_definition(pred, language)?;
            if definition == Value::NIL {
                let data = [self.string("Cannot find the definition of the predicate in `treesit-thing-settings'"), pred];
                return Ok(Some(("treesit-predicate-not-found", self.ts_list(&data))));
            }
            return self.ts_validate(definition, language, level + 1);
        }
        if let Some((car, cdr)) = self.pair(pred) {
            let head = (self.kind(car) == Kind::Symbol).then(|| self.symbol_name(car)).transpose()?;
            match head.as_deref() {
                Some("not") => {
                    let Some((inner, rest)) = self.pair(cdr) else { return Ok(invalid(self, "Invalid `not' predicate", Some(pred))) };
                    if rest != Value::NIL { return Ok(invalid(self, "`not' can only have one argument", Some(pred))); }
                    return self.ts_validate(inner, language, level + 1);
                }
                Some("or" | "and") => {
                    if self.pair(cdr).is_none() {
                        return Ok(invalid(self, "`or' or `and' must have a list of patterns as arguments ", Some(pred)));
                    }
                    for item in self.ts_list_values(cdr) {
                        if let Some(error) = self.ts_validate(item, language, level + 1)? { return Ok(Some(error)); }
                    }
                    return Ok(None);
                }
                _ if self.ts_is_string(car) && crate::prim::prims::vars::function_p(self, cdr)? => return Ok(None),
                _ => {}
            }
        }
        Ok(invalid(self, "Invalid predicate, see `treesit-thing-settings' for valid forms of predicate", Some(pred)))
    }
    /// A node type against the regexp PATTERN (fast_c_string_match).
    fn ts_type_match(&mut self, pattern: Value, node: native::Node) -> R<bool> {
        let Some(kind) = node.kind() else { return Ok(false) };
        self.vm.bytes_search(pattern, kind, !kind.is_ascii(), None)
    }
    /// treesit_traverse_match_predicate on NODE, its function predicates
    /// answered by RESULTS in call order from USED on.
    fn ts_thing_match(&mut self, pred: Value, node: native::Node, parser: Value, named: bool, results: &[bool], used: &mut usize) -> R<Thing> {
        if named && !node.is_named() {
            return Ok(Thing::No);
        }
        let verdict = |b: bool| if b { Thing::Yes } else { Thing::No };
        if self.ts_is_string(pred) {
            return Ok(verdict(self.ts_type_match(pred, node)?));
        }
        let function = |used: &mut usize, f: Value, guarded: bool| match results.get(*used) {
            Some(&r) => { *used += 1; verdict(r) }
            None => Thing::Call { f, guarded },
        };
        if self.ts_function_pred(pred)? {
            return Ok(function(used, pred, true));
        }
        if self.kind(pred) == Kind::Symbol {
            return Ok(match self.symbol_name(pred)?.as_str() {
                "named" => verdict(node.is_named()),
                "anonymous" => verdict(!node.is_named()),
                _ => {
                    let language = self.ts_parser(parser)?.language;
                    let definition = self.ts_thing_definition(pred, language)?;
                    return self.ts_thing_match(definition, node, parser, named, results, used);
                }
            });
        }
        let Some((car, cdr)) = self.pair(pred) else { return Ok(Thing::No) };
        let head = if self.kind(car) == Kind::Symbol { self.symbol_name(car)? } else { String::new() };
        match head.as_str() {
            "not" => {
                let inner = self.pair(cdr).map_or(Value::NIL, |(i, _)| i);
                Ok(match self.ts_thing_match(inner, node, parser, named, results, used)? {
                    Thing::Yes => Thing::No,
                    Thing::No => Thing::Yes,
                    call => call,
                })
            }
            "or" | "and" => {
                let or = head == "or";
                for item in self.ts_list_values(cdr) {
                    match self.ts_thing_match(item, node, parser, named, results, used)? {
                        Thing::Yes if or => return Ok(Thing::Yes),
                        Thing::No if !or => return Ok(Thing::No),
                        Thing::Call { f, guarded } => return Ok(Thing::Call { f, guarded }),
                        _ => {}
                    }
                }
                Ok(verdict(!or))
            }
            _ if self.ts_is_string(car) && crate::prim::prims::vars::function_p(self, cdr)? => {
                if !self.ts_type_match(car, node)? {
                    return Ok(Thing::No);
                }
                Ok(function(used, cdr, false))
            }
            _ => Ok(Thing::No),
        }
    }
    /// Validate PREDICATE for NODE's language: Ok(false) when a missing
    /// thing makes the primitive return nil (MISSING_OK).
    fn ts_walk_validate(&mut self, node: Value, predicate: Value, missing_ok: bool) -> R<bool> {
        let parser = self.ts_node(node)?.parser;
        let language = self.ts_parser(parser)?.language;
        match self.ts_validate(predicate, language, 0)? {
            None => Ok(true),
            Some(("treesit-predicate-not-found", _)) if missing_ok => Ok(false),
            // GNU never defines this error symbol, so its xsignal1 is
            // signal's "Invalid error symbol".
            Some(("treesit-predicate-not-found", _)) => {
                let condition = self.intern("treesit-predicate-not-found");
                Err(self.ts_message("error", "Invalid error symbol", &[condition]))
            }
            // xsignal1 (ERR, DATA): DATA is the condition's one datum.
            Some((condition, data)) => Err(self.ts_signal(condition, &[data])),
        }
    }
    /// A walk task over CURSOR of NODE's parser.
    fn ts_walk_task(&mut self, node: Value, predicate: Value, process: Value, cursor: native::Cursor, tree: Rc<native::Tree>, kind: ts::WalkKind) -> Value {
        let parser = self.ts_node(node).ok().expect("a node").parser;
        let values = vec![parser, predicate, process, Value::NIL];
        let walk = ts::Walk { cursor, tree, results: Vec::new(), pending: None, kind };
        self.ts_allocate(ts::Object::Task(ts::Task { values, state: Rc::new(RefCell::new(ts::TaskState::Walk(walk))) }))
    }

    pub(crate) fn ts_node_match_p(&mut self, node: Value, predicate: Value, ignore_missing: Value) -> R<Next> {
        if node == Value::NIL { return Ok(Next::Done(Value::NIL)); }
        self.ts_node(node)?;
        if !self.ts_walk_validate(node, predicate, ignore_missing != Value::NIL)? {
            return Ok(Next::Done(Value::NIL));
        }
        let n = self.ts_node(node)?.clone();
        let task = self.ts_walk_task(node, predicate, Value::NIL, native::Cursor::new(n.node), n.tree.clone(), ts::WalkKind::Match);
        self.ts_walk_run(task)
    }
    /// The parser's current tree and a cursor on NODE in it.
    fn ts_helper_cursor(&mut self, node: Value) -> R<Option<(native::Cursor, Rc<native::Tree>)>> {
        let n = self.ts_node(node)?.clone();
        let Some(tree) = self.ts_parser(n.parser)?.core.borrow().tree.clone() else { return Ok(None) };
        Ok(cursor_on(&tree, n.node).map(|c| (c, tree)))
    }
    pub(crate) fn ts_search_subtree(&mut self, node: Value, predicate: Value, backward: Value, all: Value, depth: Value) -> R<Next> {
        self.ts_node(node)?;
        self.ts_symbol(all)?;
        self.ts_symbol(backward)?;
        let limit = self.ts_depth(depth)?;
        if !self.ts_walk_validate(node, predicate, true)? {
            return Ok(Next::Done(Value::NIL));
        }
        let Some((cursor, tree)) = self.ts_helper_cursor(node)? else { return Ok(Next::Done(Value::NIL)) };
        let kind = ts::WalkKind::Subtree { forward: backward == Value::NIL, named: all == Value::NIL, stack: vec![ts::Dfs::Enter { limit }] };
        let task = self.ts_walk_task(node, predicate, Value::NIL, cursor, tree, kind);
        self.ts_walk_run(task)
    }
    pub(crate) fn ts_search_forward(&mut self, start: Value, predicate: Value, backward: Value, all: Value) -> R<Next> {
        self.ts_node(start)?;
        self.ts_symbol(all)?;
        self.ts_symbol(backward)?;
        if !self.ts_walk_validate(start, predicate, true)? {
            return Ok(Next::Done(Value::NIL));
        }
        let Some((cursor, tree)) = self.ts_helper_cursor(start)? else { return Ok(Next::Done(Value::NIL)) };
        let kind = ts::WalkKind::Forward { forward: backward == Value::NIL, named: all == Value::NIL, phase: ts::ForwardPhase::Initial };
        let task = self.ts_walk_task(start, predicate, Value::NIL, cursor, tree, kind);
        self.ts_walk_run(task)
    }
    pub(crate) fn ts_induce_sparse_tree(&mut self, root: Value, predicate: Value, process: Value, depth: Value) -> R<Next> {
        self.ts_node(root)?;
        if process != Value::NIL && !crate::prim::prims::vars::function_p(self, process)? {
            return Err(Fault::Wrong("functionp", process));
        }
        let limit = self.ts_depth(depth)?;
        if !self.ts_walk_validate(root, predicate, true)? {
            return Ok(Next::Done(Value::NIL));
        }
        let n = self.ts_node(root)?.clone();
        let kind = ts::WalkKind::Sparse { stack: vec![ts::SparseFrame { limit, matched: false, children: false }], processing: false };
        let task = self.ts_walk_task(root, predicate, process, native::Cursor::new(n.node), n.tree.clone(), kind);
        let top = self.cons(Value::NIL, Value::NIL);
        self.ts_task_mut(task).values.push(top);
        self.ts_task_mut(task).values.push(top);
        self.ts_walk_run(task)
    }
    fn ts_depth(&self, depth: Value) -> R<i64> {
        if depth == Value::NIL { return Ok(RECURSION_LIMIT); }
        if self.kind(depth) != Kind::Integer { return Err(Fault::Wrong("fixnump", depth)); }
        Ok(depth.integer_value().unwrap_or(0))
    }

    /// Resume TASK with VALUE, its pending call's result.
    pub(crate) fn ts_walk_resume(&mut self, task: Value, value: Value, guarded: bool, timestamp: u64) -> R<Next> {
        let parser = self.ts_task(task).values[WALK_PARSER];
        if guarded && self.ts_parser(parser)?.timestamp != timestamp {
            let f = self.ts_task(task).values[WALK_PENDING];
            return Err(self.ts_signal("treesit-buffer_changed", &[f]));
        }
        let processing = matches!(self.ts_walk(task).kind, ts::WalkKind::Sparse { processing: true, .. });
        if processing {
            self.ts_sparse_link(task, value);
        } else {
            self.ts_walk(task).results.push(value != Value::NIL);
        }
        self.ts_walk_run(task)
    }

    /// Match the cursor's node: Ok(Some(verdict)), or the call to make.
    fn ts_walk_match(&mut self, task: Value, named: bool) -> R<std::result::Result<bool, Next>> {
        let (node, results) = {
            let w = self.ts_walk(task);
            (w.cursor.node(), w.results.clone())
        };
        let (parser, predicate) = (self.ts_task(task).values[WALK_PARSER], self.ts_task(task).values[WALK_PREDICATE]);
        let mut used = 0;
        match self.ts_thing_match(predicate, node, parser, named, &results, &mut used)? {
            Thing::Yes => { self.ts_walk(task).results.clear(); Ok(Ok(true)) }
            Thing::No => { self.ts_walk(task).results.clear(); Ok(Ok(false)) }
            Thing::Call { f, guarded } => {
                let lisp_node = self.ts_walk_node(task, node);
                Ok(Err(self.ts_walk_call(task, f, lisp_node, guarded)))
            }
        }
    }
    fn ts_walk_node(&mut self, task: Value, node: native::Node) -> Value {
        let parser = self.ts_task(task).values[WALK_PARSER];
        let tree = self.ts_walk(task).tree.clone();
        let timestamp = self.ts_parser(parser).map_or(0, |p| p.timestamp);
        self.ts_allocate(ts::Object::Node(ts::Node { parser, node, tree, timestamp }))
    }
    fn ts_walk_call(&mut self, task: Value, f: Value, node: Value, guarded: bool) -> Next {
        self.ts_task_mut(task).values[WALK_PENDING] = f;
        let parser = self.ts_task(task).values[WALK_PARSER];
        let timestamp = self.ts_parser(parser).map_or(0, |p| p.timestamp);
        self.ts_walk_pending(task, guarded, timestamp);
        Next::Call { f, args: vec![node], task }
    }
    fn ts_walk_pending(&mut self, task: Value, guarded: bool, timestamp: u64) {
        let mut w = self.ts_walk(task);
        w.pending = Some((guarded, timestamp));
    }
    pub(crate) fn ts_walk_pending_of(&self, task: Value) -> (bool, u64) {
        self.ts_walk(task).pending.take().unwrap_or((false, 0))
    }
    fn ts_walk_found(&mut self, task: Value) -> Next {
        let node = self.ts_walk(task).cursor.node();
        Next::Done(self.ts_walk_node(task, node))
    }

    /// Run TASK's walk until it finishes or calls Lisp.
    fn ts_walk_run(&mut self, task: Value) -> R<Next> {
        enum Kind { Match, Subtree(bool, bool), Forward(bool, bool), Sparse }
        let kind = match &self.ts_walk(task).kind {
            ts::WalkKind::Match => Kind::Match,
            ts::WalkKind::Subtree { forward, named, .. } => Kind::Subtree(*forward, *named),
            ts::WalkKind::Forward { forward, named, .. } => Kind::Forward(*forward, *named),
            ts::WalkKind::Sparse { .. } => Kind::Sparse,
        };
        match kind {
            Kind::Match => Ok(match self.ts_walk_match(task, false)? {
                Ok(found) => Next::Done(flag(found)),
                Err(call) => call,
            }),
            Kind::Subtree(forward, named) => self.ts_subtree_run(task, forward, named),
            Kind::Forward(forward, named) => self.ts_forward_run(task, forward, named),
            Kind::Sparse => self.ts_sparse_run(task),
        }
    }
    fn ts_dfs_stack(&self, task: Value) -> std::cell::RefMut<'_, Vec<ts::Dfs>> {
        std::cell::RefMut::map(self.ts_walk(task), |w| match &mut w.kind {
            ts::WalkKind::Subtree { stack, .. } => stack,
            _ => unreachable!("a subtree walk"),
        })
    }
    /// treesit_search_dfs, iterative: each level is entered (its node
    /// matched, then its first child) or iterating its children.
    fn ts_subtree_run(&mut self, task: Value, forward: bool, named: bool) -> R<Next> {
        loop {
            let Some(&top) = self.ts_dfs_stack(task).last() else { return Ok(Next::Done(Value::NIL)) };
            match top {
                ts::Dfs::Enter { limit } => {
                    match self.ts_walk_match(task, named)? {
                        Ok(true) => return Ok(self.ts_walk_found(task)),
                        Ok(false) => {}
                        Err(call) => return Ok(call),
                    }
                    let entered = limit != 0 && child(&mut self.ts_walk(task).cursor, forward, named);
                    let mut stack = self.ts_dfs_stack(task);
                    stack.pop();
                    if entered {
                        stack.push(ts::Dfs::Children { limit });
                        stack.push(ts::Dfs::Enter { limit: limit - 1 });
                    }
                }
                ts::Dfs::Children { limit } => {
                    let more = sibling(&mut self.ts_walk(task).cursor, forward, false);
                    if more {
                        self.ts_dfs_stack(task).push(ts::Dfs::Enter { limit: limit - 1 });
                    } else {
                        self.ts_walk(task).cursor.parent();
                        self.ts_dfs_stack(task).pop();
                    }
                }
            }
        }
    }
    fn ts_forward_phase(&self, task: Value, phase: Option<ts::ForwardPhase>) -> ts::ForwardPhase {
        let mut w = self.ts_walk(task);
        let ts::WalkKind::Forward { phase: current, .. } = &mut w.kind else { unreachable!("a forward walk") };
        if let Some(phase) = phase { *current = phase; }
        *current
    }
    /// treesit_search_forward: leaves first, then upwards.
    fn ts_forward_run(&mut self, task: Value, forward: bool, named: bool) -> R<Next> {
        loop {
            match self.ts_forward_phase(task, None) {
                ts::ForwardPhase::Initial => {}
                ts::ForwardPhase::Leaf | ts::ForwardPhase::Parent => match self.ts_walk_match(task, named)? {
                    Ok(true) => return Ok(self.ts_walk_found(task)),
                    Ok(false) => {}
                    Err(call) => return Ok(call),
                },
            }
            let mut w = self.ts_walk(task);
            if sibling(&mut w.cursor, forward, named) {
                while child(&mut w.cursor, forward, false) {}
                drop(w);
                self.ts_forward_phase(task, Some(ts::ForwardPhase::Leaf));
            } else if w.cursor.parent() {
                drop(w);
                self.ts_forward_phase(task, Some(ts::ForwardPhase::Parent));
            } else {
                return Ok(Next::Done(Value::NIL));
            }
        }
    }
    fn ts_sparse_stack(&self, task: Value) -> std::cell::RefMut<'_, Vec<ts::SparseFrame>> {
        std::cell::RefMut::map(self.ts_walk(task), |w| match &mut w.kind {
            ts::WalkKind::Sparse { stack, .. } => stack,
            _ => unreachable!("a sparse walk"),
        })
    }
    fn ts_set_processing(&self, task: Value, on: bool) {
        if let ts::WalkKind::Sparse { processing, .. } = &mut self.ts_walk(task).kind { *processing = on; }
    }
    /// Add the matched VALUE as a child of the current level's parent, and
    /// make it the parent of the levels below (treesit_build_sparse_tree).
    fn ts_sparse_link(&mut self, task: Value, value: Value) {
        self.ts_set_processing(task, false);
        let depth = self.ts_sparse_stack(task).len();
        let parent = self.ts_task(task).values[WALK_PARENTS + depth];
        let this = self.cons(value, Value::NIL);
        let siblings = self.pair(parent).map_or(Value::NIL, |(_, d)| d);
        let linked = self.cons(this, siblings);
        let _ = self.setcdr(parent, linked);
        self.ts_task_mut(task).values[WALK_PARENTS + depth] = this;
        if let Some(top) = self.ts_sparse_stack(task).last_mut() { top.matched = true; }
        self.ts_sparse_descend(task);
    }
    /// Past a level's own match: into its first child, or done.
    fn ts_sparse_descend(&mut self, task: Value) {
        let top = *self.ts_sparse_stack(task).last().expect("a level");
        let entered = top.limit > 0 && self.ts_walk(task).cursor.first_child();
        if let Some(t) = self.ts_sparse_stack(task).last_mut() { t.children = true; }
        if entered {
            self.ts_sparse_push(task, top.limit - 1);
        } else {
            self.ts_sparse_finish(task, false);
        }
    }
    fn ts_sparse_push(&mut self, task: Value, limit: i64) {
        let depth = self.ts_sparse_stack(task).len();
        let parent = self.ts_task(task).values[WALK_PARENTS + depth];
        let values = &mut self.ts_task_mut(task).values;
        values.truncate(WALK_PARENTS + depth + 1);
        values.push(parent);
        self.ts_sparse_stack(task).push(ts::SparseFrame { limit, matched: false, children: false });
    }
    /// End the current level, its children reversed when it matched.
    fn ts_sparse_finish(&mut self, task: Value, back_up: bool) {
        if back_up { self.ts_walk(task).cursor.parent(); }
        let depth = self.ts_sparse_stack(task).len();
        let frame = self.ts_sparse_stack(task).pop().expect("a level");
        if frame.matched {
            let this = self.ts_task(task).values[WALK_PARENTS + depth];
            self.ts_nreverse_cdr(this);
        }
    }
    fn ts_nreverse_cdr(&mut self, cell: Value) {
        let children = self.ts_list_values(self.pair(cell).map_or(Value::NIL, |(_, d)| d));
        let reversed: Vec<Value> = children.into_iter().rev().collect();
        let list = self.ts_list(&reversed);
        let _ = self.setcdr(cell, list);
    }
    /// treesit_build_sparse_tree, iterative.
    fn ts_sparse_run(&mut self, task: Value) -> R<Next> {
        loop {
            let Some(&top) = self.ts_sparse_stack(task).last() else {
                let root = self.ts_task(task).values[WALK_PARENTS];
                self.ts_nreverse_cdr(root);
                let empty = self.pair(root).is_none_or(|(_, d)| d == Value::NIL);
                return Ok(Next::Done(if empty { Value::NIL } else { root }));
            };
            if top.children {
                let more = self.ts_walk(task).cursor.next_sibling();
                if more {
                    self.ts_sparse_push(task, top.limit - 1);
                } else {
                    self.ts_sparse_finish(task, true);
                }
                continue;
            }
            match self.ts_walk_match(task, false)? {
                Ok(true) => {
                    let node = self.ts_walk(task).cursor.node();
                    let lisp_node = self.ts_walk_node(task, node);
                    let process = self.ts_task(task).values[WALK_PROCESS];
                    if process != Value::NIL {
                        self.ts_set_processing(task, true);
                        return Ok(self.ts_walk_call(task, process, lisp_node, true));
                    }
                    self.ts_sparse_link(task, lisp_node);
                }
                Ok(false) => self.ts_sparse_descend(task),
                Err(call) => return Ok(call),
            }
        }
    }
}

// Languages, linecol caches and printing.
impl Leaf<'_> {
    pub(crate) fn ts_language_available_p(&mut self, language: Value, detail: Value) -> Out {
        self.ts_symbol(language)?;
        Ok(match (self.ts_load(language)?, detail == Value::NIL) {
            (Ok(_), true) => Value::TRUE,
            (Ok(_), false) => self.cons(Value::TRUE, Value::NIL),
            (Err(_), true) => Value::NIL,
            (Err(data), false) => self.cons(Value::NIL, data),
        })
    }
    pub(crate) fn ts_language_abi_version(&mut self, language: Value) -> Out {
        self.ts_symbol(language)?;
        Ok(match self.ts_load(language)? {
            Ok((lang, _)) => fixnum(i64::from(lang.abi_version())),
            Err(_) => Value::NIL,
        })
    }
    pub(crate) fn ts_grammar_location(&mut self, language: Value) -> Out {
        self.ts_symbol(language)?;
        Ok(match self.ts_load(language)? {
            Ok((_, Some(file))) => {
                // DECODE_FILE of the grammar's file name.
                let bytes = file.as_bytes();
                if bytes.is_ascii() { self.make_string(bytes.to_vec(), false) } else { self.make_string(crate::text::octets_as_multibyte(bytes).0, true) }
            }
            _ => Value::NIL,
        })
    }
    pub(crate) fn ts_linecol_at(&mut self, pos: Value) -> Out {
        let buffer = self.current_buffer()?;
        let pos = self.ts_position(pos, buffer)?;
        let byte = self.ts_byte(buffer, pos);
        let text = self.ts_buffer_text(buffer).expect("a multibyte buffer");
        let linecol = ts::linecol_of_pos(text, byte, text.ts_linecols[1]);
        Ok(self.cons(fixnum(linecol.line), fixnum(linecol.col)))
    }
    pub(crate) fn ts_linecol_cache_set(&mut self, line: Value, col: Value, bytepos: Value) -> Out {
        let mut fields = [0; 3];
        for (field, v) in fields.iter_mut().zip([line, col, bytepos]) {
            if self.kind(v) != Kind::Integer { return Err(Fault::Wrong("fixnump", v)); }
            *field = v.integer_value().unwrap_or(0);
        }
        let buffer = self.current_buffer()?;
        let text = self.ts_buffer_text_mut(buffer).ok_or(Fault::Error("Tree-sitter needs a multibyte buffer".into()))?;
        text.ts_linecols[1] = Linecol { line: fields[0], col: fields[1], bytepos: fields[2] };
        Ok(Value::NIL)
    }
    pub(crate) fn ts_linecol_cache(&mut self) -> Out {
        let buffer = self.current_buffer()?;
        let cache = self.ts_linecols(buffer)[1];
        let keys = [self.intern(":line"), self.intern(":col"), self.intern(":bytepos")];
        let plist = [keys[0], fixnum(cache.line), keys[1], fixnum(cache.col), keys[2], fixnum(cache.bytepos)];
        Ok(self.ts_list(&plist))
    }
}

impl Leaf<'_> {
    /// treesit-parse-string's `BVAR (buffer, ts_parser_list) = Qnil': the
    /// temporary buffer forgets PARSER, so its edits no longer reach it.
    pub(crate) fn ts_detach(&mut self, parser: Value) -> Out {
        let base = self.ts_base(self.ts_parser(parser)?.buffer);
        if self.ts_buffer_text(base).is_some() {
            self.ts_registry_mut(base).parsers.clear();
        }
        Ok(Value::NIL)
    }
}
