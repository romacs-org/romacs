//! The current syntax table, read live for one engine call, as GNU's SYNTAX()
//! reads it; the borrow of the VM keeps it unchanged for that call.
use super::*;
use crate::char_table::CharTable;
use regex::{SyntaxClass, SyntaxLookup};

pub(crate) fn parse_syntax_descriptor(description: &crate::lstring::OwnedStr) -> std::result::Result<Option<(i64, Option<u32>)>, String> {
    let first = description.bytes.first().copied().unwrap_or(0);
    let class = SyntaxClass::from_syntax_spec_byte(first)
        .ok_or_else(|| format!("Invalid syntax description letter: {}", char::from(first)))?;
    if class == SyntaxClass::InheritStd { return Ok(None); }
    let mut chars = description.chars();
    chars.next();
    let matching = chars.next();
    let mut code = class.code();
    if matching != Some(0) {
        for c in chars.take_while(|&c| c != 0) {
            if let Some(bit) = match c {
                49..=52 => Some(c - 49 + 16), 112 => Some(20),
                98 => Some(21), 110 => Some(22), 99 => Some(23), _ => None,
            } { code |= 1 << bit; }
        }
    }
    Ok(Some((code, matching.filter(|&c| c != 32 && c != 0))))
}

// Reuse the maintained GNU-exported word/symbol membership ranges. The
// accompanying case mappings are unused here; no second Unicode data copy.
#[allow(dead_code)]
mod unicode_membership {
    include!("prims/capitalize-tables.in");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(vm: &mut Vm, source: &str) -> Value {
        vm.eval_source_raw(source).unwrap()
    }

    #[test]
    fn standard_syntax_is_a_native_character_table() {
        let mut vm = Vm::new();
        let value = run(
            &mut vm,
            "(let ((s (standard-syntax-table))) (list (char-table-p s) (char-table-subtype s) (get 'syntax-table 'char-table-extra-slots) (aref s 65) (aref s 40) (aref s 128) (char-table-parent s)))",
        );
        assert_eq!(
            vm.display(value).unwrap(),
            "(t syntax-table 0 (2) (4 . 41) (2) nil)"
        );
        let source = include_str!("../../../lisp/gnu/subr.el");
        let start = source.find("(defun make-syntax-table ").unwrap();
        let end = start + source[start..].find("(defun syntax-after ").unwrap();
        run(&mut vm, &source[start..end]);
        let value = run(
            &mut vm,
            "(let ((s (make-syntax-table))) (list (char-table-p s) (eq (char-table-parent s) (standard-syntax-table)) (aref s 65) (eq s (set-syntax-table s))))",
        );
        assert_eq!(vm.display(value).unwrap(), "(t t (2) t)");
    }

    /// The purpose's plist holds its own (0): a property put on
    /// `syntax-table' (gv.el's `gv-expander') leaves the whitespace
    /// descriptor every table shares as GNU's (0).
    #[test]
    fn a_property_on_syntax_table_leaves_whitespace_syntax_alone() {
        let mut vm = Vm::new();
        let value = run(
            &mut vm,
            "(progn (standard-syntax-table) (put 'syntax-table 'gv-expander 'x)
               (list (string-to-syntax \" \") (aref (standard-syntax-table) 32)
                     (symbol-plist 'syntax-table)))",
        );
        assert_eq!(vm.display(value).unwrap(), "((0) (0) (char-table-extra-slots 0 gv-expander x))");
    }

    #[test]
    fn native_syntax_entries_preserve_matching_flags_and_inheritance() {
        let mut vm = Vm::new();
        let value = run(
            &mut vm,
            r#"(let ((s (copy-syntax-table)))
          (modify-syntax-entry 65 "(β 1234bpnc" s)
          (let ((entry (aref s 65)))
            (modify-syntax-entry 65 "@" s)
            (list entry (aref s 65) (eq (char-table-parent s) (standard-syntax-table)))))"#,
        );
        assert_eq!(vm.display(value).unwrap(), "((16711684 . 946) (2) t)");
    }

    #[test]
    fn native_syntax_snapshots_follow_parent_ranges_and_mutations() {
        let mut vm = Vm::new();
        run(
            &mut vm,
            "(setq syntax-parent (copy-syntax-table) syntax-child (make-char-table 'syntax-table)) (set-char-table-parent syntax-child syntax-parent) (set-syntax-table syntax-child) (set-char-table-range syntax-parent '(128 . 4194303) '(1)) (aset syntax-child 946 '(2))",
        );
        let source = vm.regexp_syntax(None).ok().unwrap();
        let before = VmSyntax::new(&vm, source);
        assert_eq!(before.char_syntax('β'), SyntaxClass::Word);
        assert_eq!(before.char_syntax('γ'), SyntaxClass::Punctuation);
        run(
            &mut vm,
            "(aset syntax-child 946 nil) (set-char-table-range syntax-parent '(128 . 4194303) '(3))",
        );
        let source = vm.regexp_syntax(None).ok().unwrap();
        let after = VmSyntax::new(&vm, source);
        assert_eq!(after.char_syntax('β'), SyntaxClass::Symbol);
        let value = run(&mut vm, r#"(string-match "\\s_" "β")"#);
        assert_eq!(value, Value::integer(0).unwrap());
    }

    #[test]
    fn native_syntax_tables_and_entries_survive_collection() {
        let mut vm = Vm::new();
        run(
            &mut vm,
            "(make-vector 100 nil) (setq syntax-root (copy-syntax-table)) (modify-syntax-entry 946 \"(λ\" syntax-root) (set-syntax-table syntax-root)",
        );
        vm.collect(&mut []);
        let value = run(
            &mut vm,
            "(list (aref (syntax-table) 946) (aref (char-table-parent syntax-root) 40))",
        );
        assert_eq!(vm.display(value).unwrap(), "((4 . 955) (4 . 41))");
        let source = vm.regexp_syntax(None).ok().unwrap();
        assert_eq!(VmSyntax::new(&vm, source).char_syntax('β'), SyntaxClass::Open);
    }

    #[test]
    fn native_syntax_copy_defaults_properties_and_casing() {
        let mut vm = Vm::new();
        let value = run(
            &mut vm,
            r#"
          (setq syntax-original (make-char-table 'syntax-table))
          (set-char-table-parent syntax-original (standard-syntax-table))
          (set-char-table-range syntax-original nil '(1))
          (setq syntax-copy (copy-syntax-table syntax-original))
          (aset syntax-copy 45 '(2))
          (set-syntax-table syntax-copy)
          (list (aref syntax-copy 65) (aref syntax-original 65)
                (capitalize "one-two"))"#,
        );
        assert_eq!(vm.display(value).unwrap(), "((2) (1) \"One-two\")");
        let table = vm.slots[vm.names["syntax-copy"]].value;
        assert_eq!(vm.regexp_property_class(table, 45), Some(SyntaxClass::Word));
        let value = run(
            &mut vm,
            r#"
          (let ((s (copy-sequence "-")))
            (put-text-property 0 1 'syntax-table syntax-copy s)
            (let ((parse-sexp-lookup-properties t)) (string-match "\\w" s)))"#,
        );
        assert_eq!(value, Value::integer(0).unwrap());
    }
}

/// The table a search reads and, under `parse-sexp-lookup-properties',
/// the subject whose `syntax-table' properties override it.
#[derive(Clone)]
pub(crate) struct SyntaxSource {
    table: Value,
    subject: Option<PropertySubject>,
}
impl SyntaxSource {
    /// The syntax table the search reads.
    pub(crate) fn table(&self) -> Value {
        self.table
    }
    /// This source for an engine whose first byte is byte BASE of the
    /// subject.
    pub(crate) fn rebased(mut self, base: usize) -> Self {
        if let Some(subject) = &mut self.subject {
            subject.base = base;
        }
        self
    }
}

/// A string or buffer whose `syntax-table' properties a search reads, and
/// the offset in it of the engine's first byte.
#[derive(Clone, Copy)]
struct PropertySubject {
    object: Value,
    base: usize,
}

pub(crate) struct VmSyntax<'a> {
    vm: &'a Vm,
    table: crate::char_table::TableReader<'a>,
    /// The classes of the ASCII characters this call has read: an entry
    /// is a cons a program may alter without writing the table, so its
    /// class is read from it once a call, during which no program runs.
    ascii: [std::cell::Cell<Option<SyntaxClass>>; 128],
    subject: Option<PropertySubject>,
    /// The subject's `syntax-table` runs, for a text or string: a search
    /// reads the property at one position after another.
    reader: Option<crate::properties::KeyReader<'a>>,
    id: usize,
}
impl<'a> VmSyntax<'a> {
    pub(crate) fn new(vm: &'a Vm, source: SyntaxSource) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // A fresh identity per call keeps the engine's TLS caches from
        // confusing separate VMs or table contents.
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        // A source names a subject only when it has `syntax-table'
        // properties to read (`Vm::regexp_syntax_source').
        let subject = source.subject;
        let reader = subject.and_then(|s| match vm.object_if(s.object)? {
            Object::Buffer(t) => Some(t.properties.reader(crate::properties::Key::SYNTAX_TABLE)),
            Object::String(s) | Object::RawString(s) => Some(s.properties.reader(crate::properties::Key::SYNTAX_TABLE)),
            _ => None,
        });
        Self {
            vm,
            table: vm.char_table_reader(source.table),
            // A constant, not `from_fn': its closure, called out of line
            // for each slot, cost more than many searches.
            ascii: [const { std::cell::Cell::new(None) }; 128],
            subject,
            reader,
            id: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }
    fn class(&self, code: u32) -> SyntaxClass {
        let read = || self.table.get(code).map_or(SyntaxClass::Whitespace, |entry| self.vm.syntax_entry_class(entry));
        match self.ascii.get(code as usize) {
            Some(memo) => memo.get().unwrap_or_else(|| { let class = read(); memo.set(Some(class)); class }),
            None => read(),
        }
    }
    /// The class a `syntax-table' property gives CODE at POS, if any.
    fn property_class(&self, code: u32, pos: usize) -> Option<SyntaxClass> {
        let s = self.subject?;
        let value = match &self.reader {
            Some(reader) => reader.get(ByteOffset(s.base + pos))?,
            None => self.vm.syntax_property(s.object, s.base + pos)?,
        };
        self.vm.regexp_property_class(value, code)
    }
}
impl VmSyntax<'_> {
    /// `char_syntax_at` for a character code, raw bytes included.
    pub(crate) fn code_syntax_at(&self, code: u32, pos: usize) -> SyntaxClass {
        self.property_class(code, pos).unwrap_or_else(|| self.class(code))
    }
}
impl SyntaxLookup for VmSyntax<'_> {
    fn char_syntax(&self, c: char) -> SyntaxClass {
        self.class(c as u32)
    }
    fn char_syntax_at(&self, c: char, pos: usize) -> SyntaxClass {
        self.code_syntax_at(c as u32, pos)
    }
    fn emacs_char_syntax(&self, c: regex::emacs_char::EmacsChar) -> SyntaxClass {
        self.class(c.code())
    }
    fn emacs_char_syntax_at(&self, c: regex::emacs_char::EmacsChar, pos: usize) -> SyntaxClass {
        self.code_syntax_at(c.code(), pos)
    }
    fn char_has_category(&self, c: char, cat: u8) -> bool {
        regex::default_char_has_category(c, cat)
    }
    fn word_boundary_between(&self, a: char, b: char) -> bool {
        regex::DefaultSyntaxLookup.word_boundary_between(a, b)
    }
    fn cache_key(&self) -> regex::SyntaxCacheKey {
        regex::SyntaxCacheKey::Table {
            id: self.id,
            epoch: 0,
        }
    }
    fn table_write_tick(&self) -> u64 {
        0
    }
    fn position_dependent(&self) -> bool {
        self.subject.is_some()
    }
    fn plain_syntax_until(&self, pos: usize) -> usize {
        let Some(s) = self.subject else { return usize::MAX };
        self.vm.syntax_property_free_until(s.object, s.base + pos).map_or(usize::MAX, |end| end.saturating_sub(s.base).max(pos))
    }
}
impl Vm {
    /// The syntax source for word motion; read it through [`VmSyntax::new`].
    pub(crate) fn word_motion_syntax(
        &mut self,
    ) -> std::result::Result<SyntaxSource, crate::prim::Fault> {
        self.regexp_syntax(None).map_err(|fault| match fault {
            Fault::Wrong(predicate, value) => crate::prim::Fault::Wrong(predicate, value),
            Fault::Internal(text) => crate::prim::Fault::Error(text),
            Fault::Text(name, text) => {
                let condition = self.intern(name);
                let text = self.allocate_object(Object::String(Box::new(data::LispString::new(&text))));
                let data = self.list_value(&[text]);
                crate::prim::Fault::Signal(condition, data)
            }
            Fault::Signal(name, values) => {
                let condition = self.intern(name);
                let data = self.list_value(&values);
                crate::prim::Fault::Signal(condition, data)
            }
        })
    }

    fn regexp_standard_syntax(&mut self) -> Value {
        let slot = self.search_slots().standard_syntax;
        if self.slots[slot].value == Value::NIL {
            self.make_standard_syntax(slot);
        }
        self.slots[slot].value
    }
    /// The standard syntax table, made into SLOT at its first use.
    #[cold]
    fn make_standard_syntax(&mut self, slot: usize) {
        let purpose = self.intern("syntax-table");
        // This symbol is interned during primitive registration. Defer its
        // predefined metadata with the table to keep VM startup heap-free.
        let purpose_slot = purpose.index(7).unwrap() - 1;
        if self.slots[purpose_slot].plist == Value::NIL {
            let property = self.intern("char-table-extra-slots");
            // A fresh (0): the shared whitespace descriptor here would
            // let a later `put' on the symbol rewrite every whitespace
            // syntax entry (syntax.c syms_of_syntax's Fput).
            let tail = self.alloc_cons(Value::integer(0).unwrap(), Value::NIL);
            let plist = self.alloc_cons(property, tail);
            self.slots[purpose_slot].plist = plist;
        }
        let mut table = CharTable::new(purpose);
        let whitespace = self.regexp_syntax_entry(SyntaxClass::Whitespace.code(), Value::NIL);
        table.initialize(0, whitespace).expect("no extra slots");
        let word = self.regexp_syntax_entry(SyntaxClass::Word.code(), Value::NIL);
        let symbol = self.regexp_syntax_entry(SyntaxClass::Symbol.code(), Value::NIL);
        let punctuation = self.regexp_syntax_entry(SyntaxClass::Punctuation.code(), Value::NIL);
        // Populate the ordinary mutable table, not a lookup-time Unicode
        // override. Word/symbol membership is exact for the exported GNU
        // defaults; nonword Unicode classes are conservatively punctuation.
        // Full Unicode delimiter/whitespace syntax remains a separate port.
        for &(from, to, membership) in unicode_membership::WORD_RANGES {
            if to < 128 {
                continue;
            }
            let entry = match membership {
                1 => word,
                2 => symbol,
                _ => punctuation,
            };
            table
                .set_range(from.max(128) as usize, to as usize, entry)
                .expect("valid Unicode membership range");
        }
        for code in 0..128 {
            let matching = match code {
                40 => 41,
                41 => 40,
                91 => 93,
                93 => 91,
                123 => 125,
                125 => 123,
                _ => 0,
            };
            let entry = self.regexp_syntax_entry(
                regex::standard_syntax_class_for_code(code).code(),
                if matching == 0 {
                    Value::NIL
                } else {
                    Value::integer(matching).unwrap()
                },
            );
            table.set(code as usize, entry).expect("ASCII index");
        }
        self.slots[slot].value = self.allocate_object(Object::CharTable(Box::new(table)));
    }
    fn regexp_current_syntax(&mut self) -> SearchResult<Value> {
        let index = self.buffer_index()?;
        Ok(self.buffer_syntax_table(index))
    }
    /// The syntax table of the buffer at object INDEX, the standard one by
    /// default.
    pub(crate) fn buffer_syntax_table(&mut self, index: usize) -> Value {
        let table = match &self.objects[index] {
            Object::Buffer(t) => t.syntax_table,
            Object::ByteBuffer(t) => t.syntax_table,
            _ => unreachable!("buffer_index validates"),
        };
        if table == Value::NIL {
            self.regexp_standard_syntax()
        } else {
            table
        }
    }
    /// The current buffer's syntax table, the standard one by default.
    pub(crate) fn current_syntax_table(&mut self) -> std::result::Result<Value, crate::prim::Fault> {
        self.regexp_current_syntax().map_err(|_| crate::prim::Fault::Error("syntax table required".into()))
    }
    /// syntax-table-p.
    pub(crate) fn is_syntax_table(&self, value: Value) -> bool {
        self.regexp_syntax_table(value).is_ok()
    }
    pub(crate) fn regexp_syntax_entry(&mut self, code: i64, matching: Value) -> Value {
        if matching == Value::NIL && (0..16).contains(&code) {
            let slot = self.slot("#:vm2-syntax-code-objects");
            if self.slots[slot].value == Value::UNBOUND {
                self.slots[slot].value = self.allocate_object(Object::Vector(vec![Value::NIL; 16]));
            }
            let root = self.slots[slot].value;
            let index = root.index(0).unwrap() - 1;
            if let Object::Vector(values) = &self.objects[index] {
                if values[code as usize] != Value::NIL { return values[code as usize]; }
            }
            let entry = self.list_value(&[Value::integer(code).unwrap()]);
            if let Object::Vector(values) = self.objects.entry_mut(index) { values[code as usize] = entry; }
            return entry;
        }
        let value = self.alloc_cons(Value::integer(code).unwrap(), matching);
        value
    }
    fn regexp_syntax_table(&self, table: Value) -> SearchResult<&CharTable> {
        // The purpose symbol: the search slots have it once a search ran.
        let purpose = match &self.regexp_cache.slots {
            Some(slots) => Some(slots.syntax_table),
            None => self.names.get("syntax-table").copied(),
        };
        match self.object_if(table) {
            Some(Object::CharTable(t)) if purpose.is_some_and(|slot| t.purpose() == Value::reference(slot + 1, 7)) => Ok(t),
            _ => Err(Fault::Wrong("syntax-table-p", table)),
        }
    }
    /// The class of ENTRY, a syntax table's value for a character: its
    /// (CODE . MATCH) cons's, whitespace for anything else.
    pub(crate) fn syntax_entry_class(&self, entry: Value) -> SyntaxClass {
        self.cons_pair(entry).ok().and_then(|(code, _)| code.integer_value()).and_then(SyntaxClass::from_code)
            .unwrap_or(SyntaxClass::Whitespace)
    }
    pub(crate) fn regexp_table_class(&self, table: Value, code: u32) -> Option<SyntaxClass> {
        let entry = self.char_table_get(table, code as usize).ok()?;
        Some(self.syntax_entry_class(entry))
    }
    /// Whether OBJECT, a search subject, has any `syntax-table' property.
    fn has_syntax_properties(&self, object: Value) -> bool {
        match self.object_if(object) {
            Some(Object::Buffer(t)) => t.properties.has_key(crate::properties::Key::SYNTAX_TABLE),
            Some(Object::String(s) | Object::RawString(s)) => s.properties.has_key(crate::properties::Key::SYNTAX_TABLE),
            Some(Object::ByteBuffer(t)) => t.properties.values().any(|p| p.contains_key(crate::properties::Key::SYNTAX_TABLE)),
            _ => false,
        }
    }
    /// OBJECT's `syntax-table' property at byte AT.
    fn syntax_property(&self, object: Value, at: usize) -> Option<Value> {
        match self.object_if(object)? {
            Object::Buffer(t) => t.properties.get_property(ByteOffset(at), crate::properties::Key::SYNTAX_TABLE),
            Object::String(s) | Object::RawString(s) => s.properties.get_property(ByteOffset(at), crate::properties::Key::SYNTAX_TABLE),
            Object::ByteBuffer(t) => t.properties.get(&ByteOffset(at)).and_then(|p| p.get(crate::properties::Key::SYNTAX_TABLE)).copied(),
            _ => None,
        }
    }
    /// The first byte at or after AT with a `syntax-table' property in
    /// OBJECT, or None when none follows.
    fn syntax_property_free_until(&self, object: Value, at: usize) -> Option<usize> {
        let next = |p: &crate::properties::Properties, len: usize| {
            if p.get_property(ByteOffset(at), crate::properties::Key::SYNTAX_TABLE).is_some() { return Some(at); }
            let next = p.next_single_change(ByteOffset(at), crate::properties::Key::SYNTAX_TABLE, ByteOffset(len)).0;
            (next < len).then_some(next)
        };
        match self.object_if(object)? {
            Object::Buffer(t) => next(&t.properties, t.len_bytes()),
            Object::String(s) | Object::RawString(s) => next(&s.properties, s.bytes().len()),
            Object::ByteBuffer(t) => t.properties.range(ByteOffset(at)..)
                .find(|(_, p)| p.contains_key(crate::properties::Key::SYNTAX_TABLE)).map(|(b, _)| b.0),
            _ => None,
        }
    }
    fn regexp_property_class(&self, value: Value, code: u32) -> Option<SyntaxClass> {
        if self.regexp_syntax_table(value).is_ok() {
            return self.regexp_table_class(value, code);
        }
        let (class, _) = self.cons_pair(value).ok()?;
        SyntaxClass::from_code(class.as_integer().ok()?)
    }
    /// The current syntax table and SUBJECT's syntax-table properties (the
    /// current buffer's when None) for one engine call.
    pub(super) fn regexp_syntax(&mut self, subject: Option<Value>) -> SearchResult<SyntaxSource> {
        let index = self.buffer_index()?;
        self.regexp_syntax_of(index, subject)
    }
    /// [`Self::regexp_syntax`] with the current buffer at object INDEX.
    pub(super) fn regexp_syntax_of(&mut self, index: usize, subject: Option<Value>) -> SearchResult<SyntaxSource> {
        self.regexp_standard_syntax();
        let current = self.buffer_syntax_table(index);
        let start = |object: &Object| match object {
            Object::Buffer(t) => t.accessible().start.0,
            Object::ByteBuffer(t) => t.restriction.start.0,
            _ => 0,
        };
        let (object, base) = match subject {
            Some(subject) => (subject, self.object_if(subject).map_or(0, start)),
            None => (self.slots[self.buffer_slot].value, start(&self.objects[index])),
        };
        self.regexp_syntax_source(current, object, base)
    }
    /// The syntax a search of BUFFER's text from byte BASE reads, as if
    /// BUFFER were current and narrowed to start there (treesit.c's
    /// `#match?'): its table and, under `parse-sexp-lookup-properties', its
    /// properties.
    pub(super) fn regexp_buffer_syntax(&mut self, buffer: Value, base: usize) -> SearchResult<SyntaxSource> {
        let standard = self.regexp_standard_syntax();
        let table = match self.object_if(buffer) {
            Some(Object::Buffer(t)) if t.syntax_table != Value::NIL => t.syntax_table,
            _ => standard,
        };
        self.regexp_syntax_source(table, buffer, base)
    }
    fn regexp_syntax_source(&mut self, current: Value, object: Value, base: usize) -> SearchResult<SyntaxSource> {
        // The search reads the table's flat view (`char_table::flat`). A
        // table that already had one needs no check: a view is made only
        // of a table whose parents do not loop, and a buffer's table is
        // one that `set-syntax-table' took.
        if !self.flatten_table(current) {
            self.regexp_syntax_table(current)?;
            // A read follows parents, so a cycle must be refused here.
            if self.char_table_parents_cycle(current) {
                return Err(Fault::Text("error", "Cyclic syntax table parent".into()));
            }
        }
        // parse-sexp-lookup-properties: the subject's `syntax-table'
        // properties, read where the engine asks (GNU UPDATE_SYNTAX_TABLE).
        // A subject without them reads the table alone (`VmSyntax::new'),
        // so only one with them needs the variable, which every string
        // match would otherwise look up.
        if !self.has_syntax_properties(object) {
            return Ok(SyntaxSource { table: current, subject: None });
        }
        // The binding where the search runs: `syntax-propertize' sets it
        // buffer-locally, and a treesit `#match?' searches its parser's
        // buffer, which need not be current.
        let current_buffer = self.slots[self.buffer_slot].value;
        let lookup = match self.object_if(object) {
            Some(Object::Buffer(_) | Object::ByteBuffer(_)) if object != current_buffer => {
                let slot = self.search_slots().lookup_properties.symbol(0);
                self.value_in_buffer(slot, object)
            }
            _ => self.search_values(|slots| &mut slots.lookup_properties).map(|[lookup]| lookup),
        }
        .map_err(Fault::Internal)?;
        let subject = (lookup != Value::NIL).then_some(PropertySubject { object, base });
        Ok(SyntaxSource { table: current, subject })
    }
    pub(super) fn regexp_syntax_primitive(
        &mut self,
        p: Primitive,
        args: &[Value; 4],
    ) -> SearchResult<Value> {
        use Primitive::*;
        match p {
            StandardSyntaxTable => Ok(self.regexp_standard_syntax()),
            SyntaxTable => self.regexp_current_syntax(),
            CopySyntaxTable => {
                let source = if args[0] == Value::NIL {
                    self.regexp_current_syntax()?
                } else {
                    args[0]
                };
                let standard = self.regexp_standard_syntax();
                let mut copy = self.regexp_syntax_table(source)?.clone();
                copy.set_default(Value::NIL);
                if source == standard {
                    copy.set_parent(standard);
                }
                Ok(self.allocate_object(Object::CharTable(Box::new(copy))))
            }
            SetSyntaxTable => {
                self.regexp_syntax_table(args[0])?;
                let i = self.buffer_index()?;
                match self.objects.entry_mut(i) {
                    Object::Buffer(t) => t.syntax_table = args[0],
                    Object::ByteBuffer(t) => t.syntax_table = args[0],
                    _ => unreachable!("buffer_index validates"),
                }
                Ok(args[0])
            }
            ModifySyntaxEntry => {
                // Compiled patterns may have read the old classes.
                self.clear_regexp_cache(false);
                let (from, to) = if args[0].tag() == 3 {
                    let (a, b) = self.cons_pair(args[0])?;
                    (self.regexp_int(a)?, self.regexp_int(b)?)
                } else {
                    let c = self.regexp_int(args[0])?;
                    (c, c)
                };
                let description = self.str_view(args[1]).ok_or(Fault::Wrong("stringp", args[1]))?.to_owned();
                let parsed = parse_syntax_descriptor(&description).map_err(|message| Fault::Text("error", message))?;
                let table = if args[2] == Value::NIL {
                    self.regexp_current_syntax()?
                } else {
                    args[2]
                };
                self.regexp_syntax_table(table)?;
                if from < 0 || to > 0x3fffff || from > to {
                    return Err(Fault::Signal("args-out-of-range", vec![args[0]]));
                }
                let entry = parsed.map_or(Value::NIL, |(code, matching)| {
                    self.regexp_syntax_entry(code, matching.map_or(Value::NIL, |c| Value::integer(i64::from(c)).unwrap()))
                });
                let Object::CharTable(table) = self.object_mut(table)? else {
                    unreachable!()
                };
                table.set_range(from as usize, to as usize, entry)?;
                Ok(Value::NIL)
            }
            _ => unreachable!("syntax dispatch"),
        }
    }
}
