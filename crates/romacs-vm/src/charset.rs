//! Lazy GNU pre-Lisp charset registry, reused from vm2/charset-registry.
//! MAP definitions read pinned mapping data; decoder/encoder realization and
//! unification are separate operations, not simulated by this registry.
use super::*;
#[path = "charset_map.rs"]
pub(super) mod map;

impl Record {
    pub fn code_index(&self, code: u32) -> i32 {
        if self.linear { return code.wrapping_sub(self.min) as i32; }
        let mut index = 0u32;
        let mut stride = 1u32;
        for i in 0..4 {
            let byte = (code >> (8*i)) & 255;
            if byte < self.space[2*i] || byte > self.space[2*i+1] { return -1; }
            index = index.wrapping_add((byte-self.space[2*i]).wrapping_mul(stride));
            stride = stride.wrapping_mul(self.space[2*i+1]-self.space[2*i]+1);
        }
        index.wrapping_sub(self.index_offset as u32) as i32
    }
    /// GNU INDEX_TO_CODE_POINT.
    pub fn code_point(&self, index: u32) -> u32 {
        if self.linear { return index.wrapping_add(self.min); }
        let mut index = index.wrapping_add(self.index_offset as u32);
        let mut code = 0;
        for i in 0..4 {
            let n = self.space[2*i+1] - self.space[2*i] + 1;
            let byte = if i == 3 { index } else { index % n };
            code |= self.space[2*i].wrapping_add(byte) << (8*i);
            index /= n;
        }
        code
    }
    pub(super) fn tables(&self, entries: &[map::Entry]) -> Tables {
        let mut tables = Tables { encoder: Default::default(), decoder: Default::default() };
        for entry in entries {
            let from = self.code_index(entry.from);
            let to = if entry.from == entry.to { from } else { self.code_index(entry.to) };
            if from < 0 || to < 0 { continue; }
            for (k, index) in (from..=to).enumerate() {
                let c = entry.character + k as u32;
                tables.encoder.entry(c).or_insert(index as u32);
                tables.decoder.insert(index as u32, c);
            }
        }
        tables
    }
    /// CHARSET_FAST_MAP_REF.
    pub fn fast_map_has(&self, c: u32) -> bool {
        let c = c as usize;
        if c < 0x10000 { self.fast_map[c >> 10] & (1 << ((c >> 7) & 7)) != 0 }
        else { self.fast_map.get((c >> 15) + 62).is_some_and(|bits| bits & (1 << ((c >> 12) & 7)) != 0) }
    }
    pub(super) fn initialize_map(&mut self, entries: &[map::Entry]) {
        let Some(first) = entries.first() else { return };
        let mut min_char = first.character as i64;
        let mut max_char = min_char;
        let mut nonascii_min = 0x3fffff;
        for entry in entries {
            let from_index = self.code_index(entry.from);
            let to_index = self.code_index(entry.to);
            if from_index < 0 || to_index < 0 { continue; }
            let from_char = entry.character as i64;
            let to_char = from_char + i64::from(to_index - from_index);
            if to_char > max_char { max_char = to_char; }
            else if from_char < min_char { min_char = from_char; }
            if self.ascii_compatible {
                if from_char >= 128 { nonascii_min = nonascii_min.min(from_char); }
                else if to_char >= 128 { nonascii_min = 128; }
            }
            for c in from_char..=to_char {
                // GNU's fastmap covers the complete Emacs character domain.
                if c > 0x3fffff { break; }
                if c < 0x10000 { self.fast_map[c as usize >> 10] |= 1 << ((c >> 7) & 7); }
                else { self.fast_map[(c as usize >> 15) + 62] |= 1 << ((c >> 12) & 7); }
            }
        }
        self.min_char = if self.ascii_compatible { nonascii_min } else { min_char };
        self.max_char = max_char;
    }
}

pub(super) fn offset_fast_map(min_char: i64, max_char: i64) -> [u8; 190] {
    let mut map = [0u8; 190];
    let set = |map: &mut [u8; 190], c: i64| {
        if c < 0 { return; }
        if c < 0x10000 { map[c as usize >> 10] |= 1 << ((c >> 7) & 7); }
        else { map[(c as usize >> 15) + 62] |= 1 << ((c >> 12) & 7); }
    };
    let mut c = (min_char >> 7) << 7;
    while c < 0x10000 && c <= max_char { set(&mut map, c); c += 128; }
    c = (c >> 12) << 12;
    while c <= max_char { set(&mut map, c); c += 4096; }
    map
}

#[derive(Default)]
pub(super) struct Registry {
    pub initialized: bool,
    pub records: Vec<Record>,
    pub bindings: Vec<(Value, usize)>,
    pub ids: Vec<usize>,
    pub iso: Vec<((i64, bool, i64), usize)>,
    pub mule: Vec<(i64, usize, i64)>,
    pub ordered: Vec<usize>,
    pub iso_list: Vec<usize>,
    pub mule_list: Vec<usize>,
    pub ordered_tick: u64,
    /// Vcharset_non_preferred_head: the id starting the tail that the last
    /// set-charset-priority left unprioritized; None is GNU's nil.
    pub non_preferred_head: Option<usize>,
    pub jis_roman: Option<usize>,
    pub jis_1978: Option<usize>,
    pub jis_0208: Option<usize>,
    pub ksc5601: Option<usize>,
    /// Vchar_unify_table ranges (MIN-CHAR MAX-CHAR charset id); the unify
    /// maps themselves load when a decoder first needs them, as in GNU.
    pub unify_ranges: Vec<(i64, i64, usize)>,
}
romacs_snapshot::snapshot_struct!(Registry { initialized, records, bindings, ids, iso, mule, ordered, iso_list, mule_list, ordered_tick, non_preferred_head, jis_roman, jis_1978, jis_0208, ksc5601, unify_ranges });

#[allow(dead_code)] // Native definition metadata is retained even before encoders exist.
pub(super) struct Record {
    pub attrs: [Value; 10],
    pub space: [u32; 8],
    pub dimension: i64,
    pub min: u32,
    pub max: u32,
    pub offset: Option<u32>,
    pub iso_final: i64,
    pub iso_revision: i64,
    pub mule_id: i64,
    pub ascii_compatible: bool,
    pub supplementary: bool,
    pub invalid: u32,
    pub min_char: i64,
    pub max_char: i64,
    pub fast_map: [u8; 190],
    pub linear: bool,
    pub index_offset: i32,
    /// CHARSET_UNIFIED_P: set by unify-charset, never by definition.
    pub unified: bool,
    /// The encoder/deunifier and decoder load_charset builds from the map
    /// (MAP) or unify map (OFFSET) on first use; unify-charset clears it.
    pub tables: Option<std::rc::Rc<Tables>>,
}
romacs_snapshot::snapshot_struct!(Record { attrs, space, dimension, min, max, offset, iso_final, iso_revision, mule_id, ascii_compatible, supplementary, invalid, min_char, max_char, fast_map, linear, index_offset, unified, tables });

/// GNU's per-charset mapping tables, holding code indexes, never Lisp values.
pub(crate) struct Tables {
    /// load_charset (charset, 2): character to index, the first entry wins.
    pub encoder: std::collections::BTreeMap<u32, u32>,
    /// load_charset (charset, 1): index to character, the last entry wins.
    pub decoder: std::collections::BTreeMap<u32, u32>,
}
romacs_snapshot::snapshot_struct!(Tables { encoder, decoder });

impl Registry {
    pub fn native_record(&self, name: Value) -> Option<&Record> {
        let attrs = &self.records[self.lookup(name)?].attrs;
        Some(&self.records[self.ids[attrs[0].integer_value()? as usize]])
    }
    pub fn publish_iso(&mut self, key: (i64, bool, i64), id: usize) {
        if let Some(entry) = self.iso.iter_mut().find(|(k,_)| *k == key) { entry.1 = id; }
        else { self.iso.push((key, id)); }
        let find = |key| self.iso.iter().find(|(k,_)| *k == key).map(|(_,id)| *id);
        if find((1, false, 74)) == Some(id) { self.jis_roman = Some(id); }
        else if find((2, false, 64)) == Some(id) { self.jis_1978 = Some(id); }
        else if find((2, false, 66)) == Some(id) { self.jis_0208 = Some(id); }
        else if find((2, false, 67)) == Some(id) { self.ksc5601 = Some(id); }
    }
    pub fn trace(&self, pending: &mut Vec<Value>) {
        for (i, record) in self.records.iter().enumerate() {
            if self.ids.contains(&i) || self.bindings.iter().any(|(_,j)| *j == i) {
                pending.extend_from_slice(&record.attrs);
            }
        }
        pending.extend(self.bindings.iter().map(|(name, _)| *name));
    }
    pub fn rewrite(&mut self, rewrite: &impl Fn(&mut Value)) {
        for (i, record) in self.records.iter_mut().enumerate() {
            if self.ids.contains(&i) || self.bindings.iter().any(|(_,j)| *j == i) {
                for value in &mut record.attrs { rewrite(value); }
            } else { record.attrs = [Value::NIL; 10]; }
        }
        for (name, _) in &mut self.bindings {
            rewrite(name);
        }
    }
    pub fn lookup(&self, name: Value) -> Option<usize> {
        self.bindings
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, index)| *index)
    }
}

impl Vm {
    fn charset_cons(&mut self, car: Value, cdr: Value) -> Value {
        let value = self.alloc_cons(car, cdr);
        value
    }
    pub(super) fn ensure_charsets(&mut self) {
        if self.charsets.initialized {
            return;
        }
        self.charsets.initialized = true;
        let list_slot = self.slot("charset-list");
        self.specials.insert("charset-list".into());
        self.slots[list_slot].value = Value::NIL;
        // GNU charset.c:2436, the five pre-Lisp OFFSET definitions. No maps.
        for (id, name, dimension, space, final_char, mule_id, compatible, offset) in [
            (
                0,
                "ascii",
                1,
                [0, 127, 0, 0, 0, 0, 0, 0],
                Some(66),
                Some(0),
                true,
                0,
            ),
            (
                1,
                "iso-8859-1",
                1,
                [0, 255, 0, 0, 0, 0, 0, 0],
                None,
                None,
                true,
                0,
            ),
            (
                2,
                "unicode",
                3,
                [0, 255, 0, 255, 0, 16, 0, 0],
                None,
                None,
                true,
                0,
            ),
            (
                3,
                "emacs",
                3,
                [0, 255, 0, 255, 0, 63, 0, 0],
                None,
                None,
                true,
                0,
            ),
            (
                4,
                "eight-bit",
                1,
                [128, 255, 0, 0, 0, 0, 0, 0],
                None,
                None,
                false,
                4194176,
            ),
        ] {
            let name = self.intern(name);
            let space_value = self.allocate_object(data::Object::Vector(
                space
                    .iter()
                    .map(|v| Value::integer(*v as i64).unwrap())
                    .collect(),
            ));
            let props = [
                (":name", name),
                (":dimension", Value::integer(dimension).unwrap()),
                (":code-space", space_value),
                (
                    ":iso-final-char",
                    final_char
                        .map(|n| Value::integer(n).unwrap())
                        .unwrap_or(Value::NIL),
                ),
                (
                    ":emacs-mule-id",
                    mule_id
                        .map(|n| Value::integer(n).unwrap())
                        .unwrap_or(Value::NIL),
                ),
                (
                    ":ascii-compatible-p",
                    if compatible { Value::TRUE } else { Value::NIL },
                ),
                (":code-offset", Value::integer(offset as i64).unwrap()),
            ];
            let mut plist = Value::NIL;
            for (key, value) in props.into_iter().rev() {
                let key = self.intern(key);
                plist = self.charset_cons(value, plist);
                plist = self.charset_cons(key, plist);
            }
            let mut attrs = [Value::NIL; 10];
            attrs[0] = Value::integer(id).unwrap();
            attrs[1] = name;
            attrs[2] = plist;
            self.charsets.records.push(Record {
                attrs,
                min_char: offset as i64,
                max_char: offset as i64 + if id == 3 { 0x3fff7f } else { (space[1] | (space[3] << 8) | (space[5] << 16)) - space[0] } as i64,
                fast_map: [0; 190],
                linear: true,
                index_offset: 0,
                unified: false,
                tables: None,
                space,
                dimension,
                min: space[0],
                max: if id == 3 {
                    0x3fff7f
                } else {
                    space[1] | (space[3] << 8) | (space[5] << 16)
                },
                offset: Some(offset),
                iso_final: final_char.unwrap_or(-1),
                iso_revision: if id < 2 { -1 } else { 0 },
                mule_id: mule_id.unwrap_or(-1),
                ascii_compatible: compatible,
                supplementary: id >= 3,
                invalid: if id == 4 {
                    0
                } else if id == 3 {
                    0x3fff80
                } else {
                    1 + (space[1] | (space[3] << 8) | (space[5] << 16))
                },
            });
            let r = &mut self.charsets.records[id as usize];
            r.fast_map = offset_fast_map(r.min_char, r.max_char);
            if let Some(final_char) = final_char {
                self.charsets.publish_iso((dimension, space[1]-space[0]+1 == 96, final_char), id as usize);
                self.charsets.iso_list.push(id as usize);
            }
            if let Some(mule_id) = mule_id {
                self.charsets.mule.push((mule_id, id as usize, dimension + if mule_id < 160 { 1 } else { 2 }));
                self.charsets.mule_list.push(id as usize);
            }
            self.charsets.ordered.push(id as usize);
            self.charsets.ordered_tick += 1;
            self.charsets.ids.push(id as usize);
            self.charsets.bindings.push((name, id as usize));
            self.slots[list_slot].value = self.charset_cons(name, self.slots[list_slot].value);
        }
    }
}

#[cfg(test)]
mod definition_tests {
    use super::*;
    fn define(vm: &mut Vm, name: &str, offset: i64, final_char: &str, mule: &str, supplementary: &str) {
        vm.eval_source(&format!("(define-charset-internal '{name} nil [0 100 0 0 0 0 0 0] nil nil {final_char} nil {mule} nil {supplementary} nil {offset} nil nil nil nil nil)")).unwrap();
    }
    #[test]
    fn subset_bounds_use_nonnegative_fixnums_without_u32_truncation() {
        for (bound, expected) in [("0.0", "(wrong-type-argument wholenump 0.0)"), ("'(0 . 0)", "(wrong-type-argument wholenump (0 . 0))"), ("-1", "(wrong-type-argument wholenump -1)")] {
            let mut vm = Vm::new();
            let source = format!("(condition-case e (define-charset-internal 'invalid-subset nil [0 255 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil nil (list 'ascii {bound} 255 0) nil nil nil) (error e))");
            assert_eq!(vm.eval_source(&source).unwrap().display().unwrap(), expected);
            let name = vm.intern("invalid-subset");
            assert!(vm.charsets.lookup(name).is_none());
        }
        let mut vm = Vm::new();
        vm.eval_source("(define-charset-internal 'large-subset nil [0 255 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil nil '(ascii 0 4294967296 0) nil nil nil)").unwrap();
    }
    #[test]
    fn odd_map_retains_identity_but_does_not_publish_lists() {
        let mut vm = Vm::new();
        let result = vm.eval_source("(condition-case e (define-charset-internal 'odd-map nil [0 255 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil [65 256 66] nil nil nil nil) (error e))").unwrap();
        assert_eq!(result.display().unwrap(), "(error \"Invalid format operation %V\")");
        let name = vm.intern("odd-map");
        assert!(vm.charsets.lookup(name).is_some());
        assert_eq!(vm.charsets.ordered.len(), 5);
    }
    #[test]
    fn relative_map_search_uses_lisp_default_directory() {
        let mut vm = Vm::new();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        let source = format!("(let ((default-directory {:?}) (charset-map-path '(\"etc/charsets\"))) (define-charset-internal 'relative-map nil [0 255 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil \"8859-2\" nil nil nil nil))", root.to_str().unwrap());
        vm.eval_source(&source).unwrap();
        let name = vm.intern("relative-map");
        assert_eq!(vm.charsets.native_record(name).unwrap().max_char, 733);
    }
    #[test]
    fn superset_definition_copies_ids_offsets_bounds_and_union_fastmap() {
        let mut vm = Vm::new();
        define(&mut vm, "sup-parent", 70000, "nil", "nil", "nil");
        vm.eval_source("(define-charset-internal 'sup-child nil [0 255 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil nil nil '(ascii (sup-parent . -128)) nil nil)").unwrap();
        let parent = vm.intern("sup-parent");
        let child = vm.intern("sup-child");
        let p = vm.charsets.native_record(parent).unwrap();
        let c = vm.charsets.native_record(child).unwrap();
        assert_eq!((c.min_char, c.max_char), (0, 70100));
        let mut expected = p.fast_map;
        for (out, input) in expected.iter_mut().zip(vm.charsets.records[0].fast_map) { *out |= input; }
        assert_eq!(c.fast_map, expected, "SUPERSET merges every parent's fastmap");
        assert_eq!(vm.display(c.attrs[7]).unwrap(), "((0 . 0) (5 . -128))");
        assert_eq!(c.attrs[3], Value::NIL);
        assert_eq!(c.attrs[6], Value::NIL);
    }
    #[test]
    fn subset_definition_keeps_parent_id_bounds_offset_and_fastmap() {
        let mut vm = Vm::new();
        vm.eval_source("(define-charset-internal 'parent-map nil [0 255 0 0 0 0 0 0] nil nil nil nil nil t nil nil nil \"8859-2\" nil nil nil nil)").unwrap();
        vm.eval_source("(define-charset-internal 'right-map nil [32 127 0 0 0 0 0 0] nil nil 66 nil 130 nil nil nil nil nil '(parent-map 160 255 -128) nil nil nil)").unwrap();
        let parent = vm.intern("parent-map");
        let child = vm.intern("right-map");
        let p = vm.charsets.native_record(parent).unwrap();
        let c = vm.charsets.native_record(child).unwrap();
        assert_eq!((c.min_char, c.max_char), (p.min_char, p.max_char));
        assert_eq!(c.fast_map, p.fast_map);
        let subset = c.attrs[6];
        assert_eq!(vm.display(subset).unwrap(), "[5 160 255 -128]");
        assert_eq!(c.attrs[3], Value::NIL);
    }
    #[test]
    fn map_definition_reads_pinned_file_and_preserves_attributes() {
        let mut vm = Vm::new();
        vm.eval_source("(define-charset-internal 'map-probe nil [0 255 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil \"8859-2\" nil nil nil '(:probe yes))").unwrap();
        let name = vm.intern("map-probe");
        let r = vm.charsets.native_record(name).unwrap();
        assert_eq!((r.min_char, r.max_char), (0, 733));
        assert_eq!(r.offset, None);
        assert_ne!(r.attrs[3], Value::NIL, "MAP retains original mapping object");
        assert_eq!(vm.eval_source("(charset-plist 'map-probe)").unwrap().display().unwrap(), "(:probe yes)");
    }
    #[test]
    fn map_vector_wraps_fixnum_codes_to_unsigned() {
        let mut vm = Vm::new();
        vm.eval_source("(define-charset-internal 'wrapped-map nil [0 255 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil [4294967296 500 (4294967297 . 4294967299) 700] nil nil nil nil)").unwrap();
        let name = vm.intern("wrapped-map");
        let r = vm.charsets.native_record(name).unwrap();
        assert_eq!((r.min_char, r.max_char), (500, 702));
    }
    #[test]
    fn map_trimmed_sparse_range_subtracts_index_offset() {
        let mut vm = Vm::new();
        vm.eval_source("(define-charset-internal 'trimmed-map nil [1 2 3 4 0 0 0 0] 770 1026 nil nil nil nil nil nil nil [(770 . 1026) 700] nil nil nil nil)").unwrap();
        let name = vm.intern("trimmed-map");
        let r = vm.charsets.native_record(name).unwrap();
        assert_eq!(r.code_index(770), 0);
        assert_eq!(r.code_index(1025), 1);
        assert_eq!(r.code_index(1026), 2);
        assert_eq!((r.min_char, r.max_char), (700, 702));
    }
    #[test]
    fn map_sparse_ranges_use_dense_index_not_code_difference() {
        let mut vm = Vm::new();
        vm.eval_source("(define-charset-internal 'map-sparse nil [1 2 3 4 0 0 0 0] nil nil nil nil nil nil nil nil nil [(769 . 1026) 500] nil nil nil nil)").unwrap();
        let name = vm.intern("map-sparse");
        let r = vm.charsets.native_record(name).unwrap();
        assert_eq!((r.min_char, r.max_char), (500, 503), "range is four dense charset indices");
        assert_eq!(r.fast_map[0], 1 << 3, "MAP fastmap marks actual mapped range only");
    }
    #[test]
    fn map_missing_file_publishes_identity_before_error_not_lists() {
        let mut vm = Vm::new();
        let value = vm.eval_source("(condition-case e (define-charset-internal 'missing-map nil [0 255 0 0 0 0 0 0] nil nil 75 nil 129 nil nil nil nil \"romacs-no-such-map\" nil nil nil '(:saved yes)) (error e))").unwrap();
        assert_eq!(value.display().unwrap(), "(file-missing \"Loading charset map\" \"No such file or directory\" \"romacs-no-such-map\")");
        let name = vm.intern("missing-map");
        let r = vm.charsets.native_record(name).unwrap();
        assert_eq!((r.min_char, r.max_char), (0, 0));
        assert_eq!(vm.charsets.ids.len(), 6);
        assert_eq!(vm.charsets.ordered, [0, 1, 2, 3, 4]);
        assert_eq!(vm.charsets.iso_list, [0]);
        assert_eq!(vm.charsets.mule_list, [0]);
        assert_eq!(vm.eval_source("(charset-plist 'missing-map)").unwrap().display().unwrap(), "(:saved yes)");
    }
    #[test]
    fn map_rejects_invalid_character_but_ignores_outside_code_ranges() {
        let mut vm = Vm::new();
        let value = vm.eval_source("(condition-case e (define-charset-internal 'bad-map nil [0 255 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil [65 -1] nil nil nil nil) (error e))").unwrap();
        assert_eq!(value.display().unwrap(), "(wrong-type-argument wholenump -1)");
        vm.eval_source("(define-charset-internal 'filtered-map nil [0 255 0 0 0 0 0 0] 66 70 nil nil nil nil nil nil nil [1 256 (70 . 66) 300 256 400 80 4194304 66 500] nil nil nil nil)").unwrap();
        let name = vm.intern("filtered-map");
        let r = vm.charsets.native_record(name).unwrap();
        assert_eq!((r.min_char, r.max_char), (500, 500));
    }
    #[test]
    fn offset_fastmap_initial_bounds_and_seed_state() {
        let mut vm = Vm::new();
        assert!(!vm.charsets.initialized);
        assert!(vm.charsets.records.is_empty());
        define(&mut vm, "offset-native", 5000, "nil", "nil", "nil");
        let name = vm.intern("offset-native");
        let r = vm.charsets.native_record(name).unwrap();
        assert_eq!((r.min, r.max, r.min_char, r.max_char), (0, 100, 5000, 5100));
        assert_ne!(r.fast_map[4] & 1, 0, "OFFSET second-loop rounded-down fastmap bit");
        assert_ne!(r.fast_map[4] & 128, 0, "OFFSET first-loop fastmap bit");
        assert_eq!(vm.charsets.records[0].fast_map[0], 1);
        assert_eq!(vm.charsets.iso_list, [0]);
        assert_eq!(vm.charsets.mule, [(0, 0, 2)]);
        assert_eq!(vm.charsets.ordered, [0, 1, 2, 5, 3, 4]);
        assert_eq!(vm.charsets.ordered_tick, 6);
    }
    #[test]
    fn offset_sparse_min_max_index_normalization() {
        let mut vm = Vm::new();
        vm.eval_source("(define-charset-internal 'sparse nil [1 2 3 4 0 0 0 0] 770 1025 nil nil nil nil nil nil 1024 nil nil nil [9] nil)").unwrap();
        let name = vm.intern("sparse");
        let r = vm.charsets.native_record(name).unwrap();
        assert_eq!((r.dimension, r.min, r.max, r.index_offset), (2, 770, 1025, 1));
        assert_eq!((r.min_char, r.max_char, r.invalid, r.linear), (1024, 1025, 0, false));
        assert_ne!(r.attrs[8], Value::NIL, "unify-map attribute slot");
        assert_eq!(r.attrs[9], Value::NIL, "no deunifier realization");
        let records = vm.charsets.records.len();
        for n in 0..16 { define(&mut vm, "sparse", 5000+n, "nil", "nil", "nil"); }
        assert_eq!(vm.charsets.records.len(), records, "unshared redefinitions reuse retired attrs");
        assert_eq!(vm.charsets.ids.len(), 6, "redefinition preserves ID");
    }
    #[test]
    fn offset_publication_ids_aliases_and_final_iso_mule_state() {
        let mut vm = Vm::new();
        define(&mut vm, "offset-native", 5000, "74", "129", "nil");
        vm.eval_source("(define-charset-alias 'offset-old 'offset-native)").unwrap();
        let old = vm.intern("offset-old");
        let original = vm.charsets.lookup(old).unwrap();
        define(&mut vm, "offset-native", 70000, "66", "160", "t");
        assert_eq!(vm.charsets.lookup(old), Some(original), "alias retains old attrs");
        let r = vm.charsets.native_record(old).unwrap();
        assert_eq!((r.attrs[0].integer_value(), r.offset, r.min_char, r.max_char), (Some(5), Some(70000), 70000, 70100), "alias ID resolves new native record");
        assert_eq!(vm.charsets.ids.len(), 6);
        assert_eq!(vm.charsets.iso_list, [0, 5]);
        assert_eq!(vm.charsets.mule_list, [0, 5]);
        assert_eq!(vm.charsets.jis_roman, Some(5));
        assert!(vm.charsets.iso.contains(&((1, false, 66), 5)));
        assert!(vm.charsets.mule.contains(&(129, 5, 2)));
        assert!(vm.charsets.mule.contains(&(160, 5, 3)));
        assert_eq!(vm.charsets.ordered_tick, 6);
        let before = (vm.charsets.records.len(), vm.charsets.iso.clone(), vm.charsets.mule.clone(), vm.charsets.ordered.clone());
        assert!(vm.eval_source("(define-charset-internal 'offset-native nil [0 100 0 0 0 0 0 0] nil nil 75 nil 161 nil nil nil 9000 nil nil nil 7 nil)").is_err());
        assert_eq!(before, (vm.charsets.records.len(), vm.charsets.iso.clone(), vm.charsets.mule.clone(), vm.charsets.ordered.clone()), "late failure must not publish");
        assert_ne!(vm.charsets.native_record(old).unwrap().fast_map[(70000 >> 15) + 62], 0);
    }
}

/// map_char_table_for_charset over a mapping TABLE's characters: maximal
/// runs of succeeding characters, keeping only those whose CODE lies in
/// FROM..TO when the walk is PARTIAL.
pub(crate) fn char_runs(chars: impl Iterator<Item = u32>, partial: bool, from: u32, to: u32, code: impl Fn(u32) -> Option<u32>) -> Vec<(u32, u32)> {
    let mut runs = Vec::new();
    let mut open: Option<(u32, u32)> = None;
    for c in chars {
        if partial && !code(c).is_some_and(|code| from <= code && code <= to) {
            runs.extend(open.take());
            continue;
        }
        open = match open {
            Some((start, end)) if end + 1 == c => Some((start, c)),
            other => { runs.extend(other); Some((c, c)) }
        };
    }
    runs.extend(open);
    runs
}
