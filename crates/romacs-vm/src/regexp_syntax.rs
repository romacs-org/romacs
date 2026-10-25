//! Syntax snapshot for one engine call. No VM value survives in this adapter.
use super::*;
use crate::char_table::CharTable;
use regex::{SyntaxClass, SyntaxLookup};
use std::collections::BTreeMap;

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
        let source = include_str!("../../../gnu-lisp/subr.el");
        let start = source.find("(defun make-syntax-table ").unwrap();
        let end = start + source[start..].find("(defun syntax-after ").unwrap();
        run(&mut vm, &source[start..end]);
        let value = run(
            &mut vm,
            "(let ((s (make-syntax-table))) (list (char-table-p s) (eq (char-table-parent s) (standard-syntax-table)) (aref s 65) (eq s (set-syntax-table s))))",
        );
        assert_eq!(vm.display(value).unwrap(), "(t t (2) t)");
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
        let before = vm.regexp_syntax(None).ok().unwrap();
        assert_eq!(before.char_syntax('β'), SyntaxClass::Word);
        assert_eq!(before.char_syntax('γ'), SyntaxClass::Punctuation);
        assert!(before.classes.len() < 1024, "wide ranges stay compressed");
        run(
            &mut vm,
            "(aset syntax-child 946 nil) (set-char-table-range syntax-parent '(128 . 4194303) '(3))",
        );
        let after = vm.regexp_syntax(None).ok().unwrap();
        assert_eq!(after.char_syntax('β'), SyntaxClass::Symbol);
        assert_eq!(
            before.char_syntax('β'),
            SyntaxClass::Word,
            "snapshot owns no live VM values"
        );
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
        assert_eq!(
            vm.regexp_syntax(None).ok().unwrap().char_syntax('β'),
            SyntaxClass::Open
        );
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

pub(super) struct VmSyntax {
    classes: BTreeMap<u32, SyntaxClass>,
    properties: BTreeMap<usize, SyntaxClass>,
    id: usize,
}
impl SyntaxLookup for VmSyntax {
    fn char_syntax(&self, c: char) -> SyntaxClass {
        self.classes
            .range(..=c as u32)
            .next_back()
            .map(|(_, class)| *class)
            .unwrap_or_else(|| regex::standard_syntax_class_for_char(c))
    }
    fn char_syntax_at(&self, c: char, pos: usize) -> SyntaxClass {
        self.properties
            .get(&pos)
            .copied()
            .unwrap_or_else(|| self.char_syntax(c))
    }
    fn emacs_char_syntax(&self, c: regex::emacs_char::EmacsChar) -> SyntaxClass {
        self.classes
            .range(..=c.code())
            .next_back()
            .map(|(_, class)| *class)
            .unwrap_or_else(|| regex::standard_syntax_class_for_code(c.code()))
    }
    fn emacs_char_syntax_at(&self, c: regex::emacs_char::EmacsChar, pos: usize) -> SyntaxClass {
        self.properties
            .get(&pos)
            .copied()
            .unwrap_or_else(|| self.emacs_char_syntax(c))
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
        !self.properties.is_empty()
    }
}
impl Vm {
    pub(crate) fn word_motion_syntax(
        &mut self,
    ) -> std::result::Result<impl SyntaxLookup + use<>, crate::prim::Fault> {
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
        let slot = self.names["#:vm2-standard-syntax"];
        if self.slots[slot].value == Value::NIL {
            let purpose = self.intern("syntax-table");
            // This symbol is interned during primitive registration. Defer its
            // predefined metadata with the table to keep VM startup heap-free.
            let purpose_slot = purpose.index(7).unwrap() - 1;
            if self.slots[purpose_slot].plist == Value::NIL {
                let property = self.intern("char-table-extra-slots");
                let tail = self.regexp_syntax_entry(0, Value::NIL);
                let plist = Value::reference(self.conses.len(), 3);
                self.conses.push((property, tail));
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
        self.slots[slot].value
    }
    fn regexp_current_syntax(&mut self) -> SearchResult<Value> {
        let i = self.buffer_index()?;
        let table = match &self.objects[i] {
            Object::Buffer(t) => t.syntax_table,
            Object::ByteBuffer(t) => t.syntax_table,
            _ => unreachable!("buffer_index validates"),
        };
        Ok(if table == Value::NIL {
            self.regexp_standard_syntax()
        } else {
            table
        })
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
            if let Object::Vector(values) = &mut self.objects[index] { values[code as usize] = entry; }
            return entry;
        }
        let value = Value::reference(self.conses.len(), 3);
        self.conses.push((Value::integer(code).unwrap(), matching));
        value
    }
    fn regexp_syntax_table(&self, table: Value) -> SearchResult<&CharTable> {
        match self.object_if(table) {
            Some(Object::CharTable(t))
                if self
                    .names
                    .get("syntax-table")
                    .is_some_and(|&slot| t.purpose == Value::reference(slot + 1, 7)) =>
            {
                Ok(t)
            }
            _ => Err(Fault::Wrong("syntax-table-p", table)),
        }
    }
    pub(crate) fn regexp_table_class(&self, table: Value, code: u32) -> Option<SyntaxClass> {
        let entry = self.char_table_get(table, code as usize).ok()?;
        let class = self
            .cons_pair(entry)
            .ok()
            .and_then(|(n, _)| n.integer_value())
            .and_then(SyntaxClass::from_code)
            .unwrap_or(SyntaxClass::Whitespace);
        Some(class)
    }
    fn regexp_syntax_map(&self, table: Value) -> SearchResult<BTreeMap<u32, SyntaxClass>> {
        // Split only at native leaf boundaries, not at every character. Parent
        // boundaries matter even where the child stores nil and inherits.
        fn boundaries(
            node: &char_table::CharTableNodeHandle,
            out: &mut std::collections::BTreeSet<u32>,
        ) {
            let node = node.lock().unwrap();
            for (i, slot) in node.slots.iter().enumerate() {
                out.insert(
                    (node.min_char + i * char_table::CharTableNode::WIDTHS[node.depth]) as u32,
                );
                if let char_table::CharTableSlot::Node(child) = slot {
                    boundaries(child, out);
                }
            }
        }
        self.regexp_syntax_table(table)?;
        let mut points = std::collections::BTreeSet::new();
        let mut current = table;
        let mut seen = HashSet::new();
        while let Some(Object::CharTable(t)) = self.object_if(current) {
            if !seen.insert(current.0) {
                return Err(Fault::Text("error", "Cyclic syntax table parent".into()));
            }
            t.validate_mapping()?;
            boundaries(&t.root, &mut points);
            current = t.parent;
        }
        let mut previous = None;
        Ok(points
            .into_iter()
            .map(|code| {
                (
                    code,
                    self.regexp_table_class(table, code)
                        .unwrap_or(SyntaxClass::Whitespace),
                )
            })
            .filter(|(_, class)| previous.replace(*class) != Some(*class))
            .collect())
    }
    fn regexp_property_class(&self, value: Value, code: u32) -> Option<SyntaxClass> {
        if self.regexp_syntax_table(value).is_ok() {
            return self.regexp_table_class(value, code);
        }
        let (class, _) = self.cons_pair(value).ok()?;
        SyntaxClass::from_code(class.as_integer().ok()?)
    }
    pub(super) fn regexp_syntax(&mut self, subject: Option<Value>) -> SearchResult<VmSyntax> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        self.regexp_standard_syntax();
        let current = self.regexp_current_syntax()?;
        let classes = self.regexp_syntax_map(current)?;
        let mut properties = BTreeMap::new();
        if self.slots[self.names["parse-sexp-lookup-properties"]].value != Value::NIL {
            let object = subject.unwrap_or(self.slots[self.buffer_slot].value);
            let mut record = |offset, code, value: Option<Value>| {
                if let Some(class) = value.and_then(|v| self.regexp_property_class(v, code)) {
                    properties.insert(offset, class);
                }
            };
            match self.object_if(object) {
                Some(Object::Buffer(t)) => {
                    let r = t.accessible();
                    for (offset, c) in t.as_str()[r.start.0..r.end.0].char_indices() {
                        record(
                            offset,
                            c as u32,
                            t.properties
                                .get_property(ByteOffset(r.start.0 + offset), "syntax-table"),
                        );
                    }
                }
                Some(Object::String(s) | Object::RawString(s)) => {
                    let mut offset = 0;
                    for c in s.view().chars() {
                        record(
                            offset,
                            c,
                            s.properties
                                .get_property(ByteOffset(offset), "syntax-table"),
                        );
                        offset += if s.is_multibyte() {
                            crate::lstring::string_char(&s.bytes()[offset..]).1
                        } else {
                            1
                        };
                    }
                }
                Some(Object::ByteBuffer(t)) => {
                    let r = &t.restriction;
                    for (offset, &c) in t.bytes[r.start.0..r.end.0].iter().enumerate() {
                        record(
                            offset,
                            c as u32,
                            t.properties
                                .get(&ByteOffset(r.start.0 + offset))
                                .and_then(|p| p.get("syntax-table"))
                                .copied(),
                        );
                    }
                }
                _ => (),
            }
        }
        // These snapshots are not cached. A fresh plain-Rust identity prevents
        // the engine's TLS caches from confusing separate VMs or table epochs.
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        Ok(VmSyntax {
            classes,
            properties,
            id,
        })
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
                copy.default = Value::NIL;
                if source == standard {
                    copy.parent = standard;
                }
                Ok(self.allocate_object(Object::CharTable(Box::new(copy))))
            }
            SetSyntaxTable => {
                self.regexp_syntax_table(args[0])?;
                let i = self.buffer_index()?;
                match &mut self.objects[i] {
                    Object::Buffer(t) => t.syntax_table = args[0],
                    Object::ByteBuffer(t) => t.syntax_table = args[0],
                    _ => unreachable!("buffer_index validates"),
                }
                Ok(args[0])
            }
            ModifySyntaxEntry => {
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
                let Object::CharTable(table) = &mut self.objects[table.index(0)? - 1] else {
                    unreachable!()
                };
                table.set_range(from as usize, to as usize, entry)?;
                Ok(Value::NIL)
            }
            _ => unreachable!("syntax dispatch"),
        }
    }
}
