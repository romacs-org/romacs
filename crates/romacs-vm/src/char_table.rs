//! Character arrays retaining GNU's 64/16/32/128 subdivision history.
//! Native nodes have stable identities, independent of the moving Lisp heap.
use super::*;
use std::sync::{Arc, Mutex};
mod literal;
#[cfg(test)]
mod literal_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn run(vm: &mut Vm, source: &str) -> Value { vm.eval_source_raw(source).unwrap() }

    fn child(node: &CharTableNodeHandle, slot: usize) -> CharTableNodeHandle {
        let node = node.lock().unwrap();
        let CharTableSlot::Node(child) = &node.slots[slot] else { panic!("expected retained subdivision") };
        Arc::clone(child)
    }

    #[test]
    fn equal_contents_retain_distinct_subdivision_history() {
        fn assert_send<T: Send>() {}
        assert_send::<CharTable>();
        assert_send::<CharTableNodeHandle>();
        let mut plain = CharTable::new(Value::NIL);
        plain.initialize(0, Value::TRUE).unwrap();
        let mut historical = plain.clone();
        let bytes = historical.storage_bytes(true);
        // Even an equal single-character assignment must allocate every level.
        historical.set(65537, Value::TRUE).unwrap();
        let one = child(&historical.root, 1);
        let two = child(&one, 0);
        let three = child(&two, 0);
        for (handle, depth, min_char, count) in [
            (&historical.root, 0, 0, 64), (&one, 1, 65536, 16),
            (&two, 2, 65536, 32), (&three, 3, 65536, 128),
        ] {
            let node = handle.lock().unwrap();
            assert_eq!((node.depth, node.min_char, node.slots.len()), (depth, min_char, count));
        }
        historical.set(65537, Value::NIL).unwrap();
        historical.set(65537, Value::TRUE).unwrap();
        assert!(Arc::ptr_eq(&three, &child(&child(&child(&historical.root, 1), 0), 0)));
        for index in [0, 65535, 65536, 65537, 65663, 131071, CharTable::LEN - 1] {
            assert_eq!(plain.get(index).unwrap(), historical.get(index).unwrap());
        }
        assert!(plain.uniform_print_fields().is_some());
        assert!(historical.uniform_print_fields().is_none());
        assert!(historical.storage_bytes(true) > bytes);
    }

    #[test]
    fn whole_slot_overwrites_detach_but_do_not_mutate_retained_nodes() {
        let mut table = CharTable::new(Value::NIL);
        table.set(65537, Value::TRUE).unwrap();
        let retained = child(&table.root, 1);
        let weak = Arc::downgrade(&retained);
        table.set_range(65536, 131071, Value::NIL).unwrap();
        assert!(matches!(table.root.lock().unwrap().slots[1], CharTableSlot::Value(Value::NIL)));
        assert_eq!(table.get(65537).unwrap(), Value::NIL);
        assert_eq!(CharTableNode::get(&retained, 65537), Value::TRUE);
        assert_eq!(CharTableNode::get(&retained, 65538), Value::NIL);
        // A later write must build a new subtree, never resurrect the detached one.
        table.set(65538, Value::TRUE).unwrap();
        assert!(!Arc::ptr_eq(&retained, &child(&table.root, 1)));
        assert_eq!(CharTableNode::get(&retained, 65538), Value::NIL);
        drop(retained);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn whole_slot_overwrite_destroys_only_covered_history() {
        let mut table = CharTable::new(Value::NIL);
        table.set(1, Value::NIL).unwrap();
        let one = child(&table.root, 0);
        let two = child(&one, 0);
        table.set_range(0, 127, Value::NIL).unwrap();
        assert!(matches!(two.lock().unwrap().slots[0], CharTableSlot::Value(Value::NIL)));
        assert!(Arc::ptr_eq(&two, &child(&one, 0)));
        table.set_range(0, 4095, Value::NIL).unwrap();
        assert!(matches!(one.lock().unwrap().slots[0], CharTableSlot::Value(Value::NIL)));
        assert!(Arc::ptr_eq(&one, &child(&table.root, 0)));
        table.set_range(0, 65535, Value::NIL).unwrap();
        assert!(table.uniform_print_fields().is_some());
        table.set(CharTable::LEN - 1, Value::TRUE).unwrap();
        let root = Arc::clone(&table.root);
        table.set_range(0, CharTable::LEN - 1, Value::NIL).unwrap();
        assert!(Arc::ptr_eq(&root, &table.root));
        assert!(table.uniform_print_fields().is_some());
        assert_eq!(table.storage_bytes(true), CharTable::new(Value::NIL).storage_bytes(true));
    }

    #[test]
    fn copies_preserve_history_but_share_no_native_nodes() {
        let mut original = CharTable::new(Value::NIL);
        original.set(65537, Value::TRUE).unwrap();
        let mut copy = original.clone();
        let mut left = Arc::clone(&original.root);
        let mut right = Arc::clone(&copy.root);
        for slot in [1, 0, 0] {
            assert!(!Arc::ptr_eq(&left, &right));
            left = child(&left, slot);
            right = child(&right, slot);
        }
        assert!(!Arc::ptr_eq(&left, &right));
        copy.set(65537, Value::NIL).unwrap();
        original.set(65538, Value::TRUE).unwrap();
        assert_eq!(original.get(65537).unwrap(), Value::TRUE);
        assert_eq!(copy.get(65537).unwrap(), Value::NIL);
        assert_eq!(original.get(65538).unwrap(), Value::TRUE);
        assert_eq!(copy.get(65538).unwrap(), Value::NIL);
        assert!(copy.uniform_print_fields().is_none());
    }

    #[test]
    fn node_visits_deduplicate_shared_identity_per_walk() {
        let mut table = CharTable::new(Value::NIL);
        let shared = CharTableNode::new(1, 0, Value::TRUE);
        table.root.lock().unwrap().slots[0] = CharTableSlot::Node(Arc::clone(&shared));
        table.root.lock().unwrap().slots[1] = CharTableSlot::Node(Arc::clone(&shared));
        let mut count = 0;
        table.visit_values(&mut |value| {
            if *value == Value::TRUE { count += 1; }
        });
        assert_eq!(count, 16, "each physical slot is visited once, not each incoming edge");
        let mut seen = HashSet::default();
        visit_char_table_node_values(&table.root, &mut seen, &mut |_| {});
        let mut revisits = 0;
        visit_char_table_node_values(&shared, &mut seen, &mut |_| { revisits += 1; });
        assert_eq!(revisits, 0, "shared collector state also covers a retained handle");
    }

    #[test]
    fn retained_leaf_payload_is_forwarded_by_existing_collector() {
        let mut vm = Vm::new();
        run(&mut vm, "(make-vector 100 nil)");
        run(&mut vm, "(setq ct-leaf (make-char-table nil)) (aset ct-leaf 65537 (list (copy-sequence \"leaf-payload\")))");
        let table_value = vm.slots[vm.names["ct-leaf"]].value;
        let data::Object::CharTable(table) = vm.object(table_value).unwrap() else { panic!() };
        let leaf = child(&child(&child(&table.root, 1), 0), 0);
        let before = CharTableNode::get(&leaf, 65537);
        vm.collect(&mut []);
        let table_value = vm.slots[vm.names["ct-leaf"]].value;
        let data::Object::CharTable(table) = vm.object(table_value).unwrap() else { panic!() };
        let after_leaf = child(&child(&child(&table.root, 1), 0), 0);
        assert!(Arc::ptr_eq(&leaf, &after_leaf), "native identity survives moving GC");
        let after = CharTableNode::get(&leaf, 65537);
        assert_ne!(before, after, "fixture must move the payload");
        assert_eq!(vm.display(after).unwrap(), "(\"leaf-payload\")");
        assert_eq!(table.get(65537).unwrap(), after);
    }

    #[test]
    fn initial_contents_and_default_are_independent_raw_values() {
        let mut table = CharTable::new(Value::NIL);
        table.initialize(0, Value::TRUE).unwrap();
        table.default = Value::NIL;
        assert_eq!(table.get(3).unwrap(), Value::TRUE);
        table.set(3, Value::NIL).unwrap();
        table.default = Value::integer(42).unwrap();
        assert_eq!(CharTableNode::get(&table.root, 3), Value::NIL);
        assert_eq!(table.get(3).unwrap(), table.default);
        assert_eq!(table.get(4).unwrap(), Value::TRUE);
    }

    #[test]
    fn standard_case_table_is_lazy_identity_bearing_ascii_graph() {
        let mut vm = Vm::new();
        assert_eq!(vm.heap_stats().live_objects, 0, "case table must be lazy");
        let metadata = run(&mut vm, "(get 'case-table 'char-table-extra-slots)");
        assert_eq!(vm.display(metadata).unwrap(), "3", "case-table metadata exists before first standard table access");
        let extra = run(&mut vm, "(char-table-extra-slot (make-char-table 'case-table) 2)");
        assert_eq!(extra, Value::NIL);
        let got = vm.eval_source_raw("(let ((d (standard-case-table))) (list (eq d (standard-case-table)) (char-table-subtype d) (get 'case-table 'char-table-extra-slots) (aref d 65) (aref d 127) (aref d 128) (aref (char-table-extra-slot d 0) 97) (aref (char-table-extra-slot d 1) 65) (aref (char-table-extra-slot d 2) 65) (aref (char-table-extra-slot d 2) 97) (eq (char-table-extra-slot (char-table-extra-slot d 1) 2) (char-table-extra-slot d 2))))");
        assert!(got.is_ok(), "standard-case-table must be callable: {got:?}");
        assert_eq!(vm.display(got.unwrap()).unwrap(), "(t case-table 3 97 127 nil 65 97 97 65 t)", "ASCII graph and stable identity");
        // No Lisp binding retains the table: only the native standard root does.
        run(&mut vm, "(aset (standard-case-table) 65 999)");
        vm.collect(&mut []);
        let got = run(&mut vm, "(aref (standard-case-table) 65)");
        assert_eq!(vm.display(got).unwrap(), "999", "native root survives moving GC");
    }

    #[test]
    fn keymap_character_table_inherits_parent() {
        let mut vm = Vm::new();
        let value = run(&mut vm, "(let* ((m (make-keymap)) (p (make-char-table nil))) (set-char-table-range p 65 'foo) (set-char-table-parent (nth 1 m) p) (list (aref (nth 1 m) 65) (lookup-key m [65])))");
        assert_eq!(vm.display(value).unwrap(), "(foo foo)");
    }

    #[test]
    fn parent_and_range_referents_survive_moving_collection() {
        let mut vm = Vm::new();
        assert_eq!(vm.heap_stats().live_objects, 0, "character tables must not allocate at startup");
        run(&mut vm, "(make-vector 100 nil)");
        run(&mut vm, "(setq ct-chain (make-char-table nil)) (set-char-table-parent ct-chain (make-char-table nil)) (set-char-table-range (char-table-parent ct-chain) '(128 . 4194303) (list (copy-sequence \"range-payload\")))");
        let before = vm.slots[vm.names["ct-chain"]].value;
        vm.collect(&mut []);
        let after = vm.slots[vm.names["ct-chain"]].value;
        assert_ne!(before, after, "fixture must move the rooted child");
        let value = run(&mut vm, "(aref ct-chain 128512)");
        assert_eq!(vm.display(value).unwrap(), "(\"range-payload\")");
    }

    #[test]
    fn interval_overwrites_match_dense_reference() {
        let mut table = CharTable::new(Value::NIL);
        let mut expected = [Value::NIL; 256];
        for step in 0..256usize {
            let start = (step * 97) % 256;
            let end = (start + step % 31).min(255);
            let value = Value::integer((step % 5) as i64).unwrap();
            table.set_range(start, end, value).unwrap();
            expected[start..=end].fill(value);
            for (i, &want) in expected.iter().enumerate() { assert_eq!(table.get(i).unwrap(), want); }
        }
    }

    #[test]
    fn broad_ranges_remain_compressed() {
        let mut vm = Vm::new();
        let result = vm.eval_source_raw("(setq ct-wide (make-char-table nil 'init)) (set-char-table-range ct-wide '(128 . 4194303) 'wide)");
        assert!(result.is_ok(), "set-char-table-range must be callable: {result:?}");
        let data::Object::CharTable(table) = vm.object(vm.slots[vm.names["ct-wide"]].value).unwrap() else { panic!() };
        assert!(table.storage_bytes(true) < 4096, "a broad range must remain compressed");
        eprintln!("compressed 128..4194303: {} bytes", table.storage_bytes(true));
        let value = run(&mut vm, "(list (aref ct-wide 127) (aref ct-wide 128) (aref ct-wide 128512) (aref ct-wide 4194303))");
        assert_eq!(vm.display(value).unwrap(), "(init wide wide wide)");
    }

    #[test]
    fn ranges_and_parent_fallback() {
        let mut vm = Vm::new();
        let result = vm.eval_source_raw("(let ((p (make-char-table nil 'parent)) (c (make-char-table nil 'init))) (set-char-table-parent c p) (set-char-table-range c '(65 . 90) 'range) (set-char-table-range c nil 'fallback) (aset c 66 nil) (list (aref c 65) (aref c 66) (aref c 91) (progn (set-char-table-range c t nil) (set-char-table-range c nil nil) (aref c 128512))))");
        assert!(result.is_ok(), "range and parent primitives must be callable: {result:?}");
        assert_eq!(vm.display(result.unwrap()).unwrap(), "(range fallback init parent)");
    }

    #[test]
    fn extra_slots_move_and_reclaim_without_retention() {
        let mut vm = Vm::new();
        // Startup allocation counts are guarded by the existing GC suites.
        run(&mut vm, "(put 'ct-purpose 'char-table-extra-slots 3)");
        // Dead prefix ensures both the table and its sole-owned referent move.
        run(&mut vm, "(make-vector 100 nil)");
        run(&mut vm, "(setq ct-root (make-char-table 'ct-purpose)) (set-char-table-extra-slot ct-root 1 (list (copy-sequence \"payload\") 42))");
        let table_before = vm.slots[vm.names["ct-root"]].value;
        let data::Object::CharTable(table) = vm.object(table_before).unwrap() else { panic!() };
        let payload_before = table.extras[1];
        let collections = vm.heap_stats().collections;
        vm.collect(&mut []);
        assert!(vm.heap_stats().collections > collections);
        let table_after = vm.slots[vm.names["ct-root"]].value;
        let data::Object::CharTable(table) = vm.object(table_after).unwrap() else { panic!() };
        let payload_after = table.extras[1];
        assert_ne!(table_before, table_after, "fixture must move table");
        assert_ne!(payload_before, payload_after, "fixture must move slot referent");
        assert_eq!(vm.display(payload_after).unwrap(), "(\"payload\" 42)");
        let source = "(setq ct-root (make-char-table 'ct-purpose)) (set-char-table-extra-slot ct-root 1 (list (make-vector 17 nil) (copy-sequence \"payload\"))) (set-char-table-extra-slot ct-root 2 ct-root) (setq ct-root nil)";
        // Warm precisely the same source/symbols before taking a baseline.
        run(&mut vm, source);
        vm.collect(&mut []);
        let baseline = vm.heap_stats();
        for _ in 0..32 {
            run(&mut vm, source);
            vm.collect(&mut []);
            let now = vm.heap_stats();
            assert_eq!((now.live_objects, now.live_bytes), (baseline.live_objects, baseline.live_bytes));
        }
        eprintln!("extra-slot retention: objects={} bytes={} across 32 cycles", baseline.live_objects, baseline.live_bytes);
    }

    #[test]
    fn extra_slot_storage_bytes_include_payload() {
        let mut table = CharTable::new(Value::NIL);
        let empty = table.storage_bytes(false);
        table.initialize(10, Value::TRUE).unwrap();
        assert_eq!(table.storage_bytes(false) - empty, 10 * size_of::<Value>());
        assert!(table.storage_bytes(true) >= table.storage_bytes(false));
    }

    #[test]
    fn constructor_nil_entries_fall_back_to_init() {
        let mut vm = Vm::new();
        let v = run(&mut vm, "(let ((ct (make-char-table nil 'initial))) (aset ct 65 nil) (list (aref ct 65) (aref ct 66)))");
        assert_eq!(vm.display(v).unwrap(), "(initial initial)");
    }

    #[test]
    fn extra_slots_construct_read_write_and_shallow_copy() {
        let mut vm = Vm::new();
        let value = vm.eval_source_raw("(progn (put 'ct-purpose 'char-table-extra-slots 3) (let* ((ct (make-char-table 'ct-purpose 'init)) (payload (list 42)) (copy nil)) (set-char-table-extra-slot ct 1 payload) (setq copy (copy-sequence ct)) (set-char-table-extra-slot copy 0 'changed) (list (char-table-subtype ct) (aref ct 65) (char-table-extra-slot ct 0) (char-table-extra-slot copy 0) (eq payload (char-table-extra-slot copy 1)) (char-table-extra-slot ct 2))))").unwrap();
        assert_eq!(vm.display(value).unwrap(), "(ct-purpose init init changed t init)");
    }
}


impl Vm {
    pub(super) fn initialize_case_table_slot(&mut self, name: &str, slot: usize) {
        // The syms_of_* Fput (PURPOSE, Qchar_table_extra_slots, N) calls
        // (casetab.c, coding.c, character.c, xdisp.c).
        let Some(count) = (match name {
            "case-table" => Some(3),
            "translation-table" => Some(2),
            "char-script-table" | "glyphless-char-display" => Some(1),
            _ => None,
        }) else { return; };
        // Realize GNU's predefined plist on first symbol access, not Vm::new.
        let property = self.intern("char-table-extra-slots");
        let tail = Value::reference(self.conses.len(), 3);
        self.conses.push((Value::integer(count).unwrap(), Value::NIL));
        let plist = Value::reference(self.conses.len(), 3);
        self.conses.push((property, tail));
        self.slots[slot].plist = plist;
    }

    /// GNU UNIPROP_TABLE_P: purpose `char-code-property-table', 5 extras.
    pub(crate) fn is_uniprop_table(&self, t: &CharTable) -> bool {
        self.names.get("char-code-property-table")
            .is_some_and(|&slot| t.purpose == Value::reference(slot + 1, 7) && t.extras.len() == 5)
    }

    /// The codes of VALUE when it is a compressed uniprop block
    /// (GNU UNIPROP_COMPRESSED_FORM_P: a string starting with 1 or 2).
    pub(crate) fn uniprop_compressed(&self, value: Value) -> Option<Vec<u32>> {
        let codes: Vec<u32> = self.str_view(value)?.chars().collect();
        matches!(codes.first(), Some(1 | 2)).then_some(codes)
    }

    /// TABLE's own value for INDEX, its default for nil, never its parent's
    /// (char_table_ref_simple).
    pub(crate) fn char_table_own(&self, table: Value, index: usize) -> Result<Value> {
        let data::Object::CharTable(t) = self.object(table)? else { return Err("char-table required".into()); };
        let mut value = t.get(index)?;
        // GNU sub_char_table_ref uncompresses the block in place; reading
        // it without storing keeps this lookup read-only.
        if self.is_uniprop_table(t) {
            if let Some(codes) = self.uniprop_compressed(value) {
                value = uniprop_block(&codes)?[index % 128];
                if value == Value::NIL { value = t.default; }
            }
        }
        Ok(value)
    }

    pub(crate) fn char_table_get(&self, mut table: Value, index: usize) -> Result<Value> {
        loop {
            let value = self.char_table_own(table, index)?;
            let data::Object::CharTable(t) = self.object(table)? else { unreachable!("checked char-table") };
            if value != Value::NIL || !matches!(self.object_if(t.parent), Some(data::Object::CharTable(_))) { return Ok(value); }
            table = t.parent;
        }
    }
}

/// GNU uniprop_table_uncompress: the 128 values of a compressed block, whose
/// CODES start with 1 (simple: start index, then values, 0 meaning nil) or
/// 2 (run-length: each value optionally followed by 128 + count).
pub(crate) fn uniprop_block(codes: &[u32]) -> Result<Vec<Value>> {
    let mut values = vec![Value::NIL; 128];
    let mut chars = codes[1..].iter().copied().peekable();
    if codes[0] == 1 {
        let start = chars.next().ok_or("incomplete Unicode property block")? as usize;
        for (index, code) in (start..128).zip(chars) {
            values[index] = if code == 0 { Value::NIL } else { Value::integer(i64::from(code)).unwrap() };
        }
    } else {
        let mut index = 0;
        while let Some(code) = chars.next() {
            let count = if chars.peek().is_some_and(|&next| next >= 128) { chars.next().unwrap() as usize - 128 } else { 1 };
            if index + count > 128 { return Err("Unicode property block exceeds 128 characters".into()); }
            values[index..index + count].fill(Value::integer(i64::from(code)).unwrap());
            index += count;
        }
    }
    Ok(values)
}

/// Cloning a handle retains native storage after its parent slot is replaced.
/// A detached handle's Lisp payloads must also be rooted and forwarded through
/// `visit_char_table_node_values`; an Arc alone is not a Lisp GC root.
pub(super) type CharTableNodeHandle = Arc<Mutex<CharTableNode>>;

#[derive(Clone)]
pub(super) enum CharTableSlot {
    Value(Value),
    Node(CharTableNodeHandle),
}
romacs_snapshot::snapshot_enum!(CharTableSlot {
    CharTableSlot::Value(f0) => (f0),
    CharTableSlot::Node(f0) => (f0),
});

pub(super) struct CharTableNode {
    pub(super) depth: usize,
    pub(super) min_char: usize,
    pub(super) slots: Vec<CharTableSlot>,
}
romacs_snapshot::snapshot_struct!(CharTableNode { depth, min_char, slots });

impl CharTableNode {
    pub(super) const WIDTHS: [usize; 4] = [65536, 4096, 128, 1];
    pub(super) const SIZES: [usize; 4] = [64, 16, 32, 128];

    fn new(depth: usize, min_char: usize, value: Value) -> CharTableNodeHandle {
        Arc::new(Mutex::new(Self {
            depth,
            min_char,
            slots: vec![CharTableSlot::Value(value); Self::SIZES[depth]],
        }))
    }

    /// Test-only convenience for known-valid native nodes. Runtime callers
    /// propagate checked_span errors instead of indexing imported geometry.
    #[cfg(test)]
    pub(super) fn get(handle: &CharTableNodeHandle, index: usize) -> Value {
        Self::checked_span(handle, index).expect("valid native character table").0
    }

    /// INDEX's raw value under HANDLE and the inclusive character range of
    /// the slot holding it, refusing geometry a lookup cannot trust.
    pub(super) fn checked_span(handle: &CharTableNodeHandle, index: usize) -> Result<(Value, usize, usize)> {
        let mut current = Arc::clone(handle);
        let mut seen = HashSet::default();
        loop {
            if !seen.insert(char_table_node_id(&current)) { return Err("cyclic sub-char-table".into()); }
            let (slot, from, width) = {
                let node = current.lock().unwrap();
                let offset = index.checked_sub(node.min_char).ok_or("invalid sub-char-table geometry")?;
                let width = *Self::WIDTHS.get(node.depth).ok_or("invalid sub-char-table geometry")?;
                let slot_index = offset / width;
                let slot = node.slots.get(slot_index).cloned().ok_or("invalid sub-char-table geometry")?;
                let depth = node.depth;
                let from = node.min_char + slot_index * width;
                drop(node);
                if let CharTableSlot::Node(child) = &slot {
                    if depth == 3 { return Err("invalid sub-char-table geometry".into()); }
                    let child = child.lock().unwrap();
                    if child.depth != depth + 1 || child.min_char != from {
                        return Err("invalid sub-char-table geometry".into());
                    }
                }
                (slot, from, width)
            };
            match slot {
                CharTableSlot::Value(value) => return Ok((value, from, from + width - 1)),
                CharTableSlot::Node(child) => current = child,
            }
        }
    }

    /// HANDLE's own geometry, without its children: a node keeps its depth
    /// and first character for life.
    pub(super) fn validate_node(handle: &CharTableNodeHandle, depth: usize, min_char: usize) -> Result<()> {
        let node = handle.lock().unwrap();
        if depth > 3 || node.depth != depth || node.min_char != min_char || node.slots.len() != Self::SIZES[depth] {
            return Err("invalid sub-char-table geometry".into());
        }
        Ok(())
    }
    pub(super) fn validate_traversal(handle: &CharTableNodeHandle, depth: usize, min_char: usize) -> Result<()> {
        let slots = {
            let node = handle.lock().unwrap();
            if depth > 3 || node.depth != depth || node.min_char != min_char || node.slots.len() != Self::SIZES[depth] {
                return Err("invalid sub-char-table geometry".into());
            }
            node.slots.clone()
        };
        for (i, slot) in slots.iter().enumerate() {
            if let CharTableSlot::Node(child) = slot {
                if depth == 3 { return Err("invalid sub-char-table geometry".into()); }
                Self::validate_traversal(child, depth + 1, min_char + i * Self::WIDTHS[depth])?;
            }
        }
        Ok(())
    }

    fn validate_geometry(handle: &CharTableNodeHandle, depth: usize, min_char: usize, from: usize, to: usize) -> Result<()> {
        let slots = {
            let node = handle.lock().unwrap();
            if node.depth != depth || node.min_char != min_char { return Err("invalid sub-char-table geometry".into()); }
            node.slots.clone()
        };
        for (i, slot) in slots.iter().enumerate() {
            if let CharTableSlot::Node(child) = slot {
                let start = min_char + i * Self::WIDTHS[depth];
                let end = start + Self::WIDTHS[depth] - 1;
                if start > to || end < from || (from <= start && end <= to) { continue; }
                if depth == 3 { return Err("invalid sub-char-table geometry".into()); }
                Self::validate_geometry(child, depth + 1, start, from, to)?;
            }
        }
        Ok(())
    }

    fn deep_copy(handle: &CharTableNodeHandle, copies: &mut HashMap<usize, CharTableNodeHandle>) -> CharTableNodeHandle {
        let id = char_table_node_id(handle);
        if let Some(copy) = copies.get(&id) { return Arc::clone(copy); }
        let node = handle.lock().unwrap();
        let copy = Arc::new(Mutex::new(Self { depth: node.depth, min_char: node.min_char, slots: vec![] }));
        copies.insert(id, Arc::clone(&copy));
        let slots = node.slots.clone();
        drop(node);
        copy.lock().unwrap().slots = slots.iter().map(|slot| match slot {
            CharTableSlot::Value(value) => CharTableSlot::Value(*value),
            CharTableSlot::Node(child) => CharTableSlot::Node(Self::deep_copy(child, copies)),
        }).collect();
        copy
    }

    fn set_range(handle: &CharTableNodeHandle, from: usize, to: usize, value: &CharTableSlot) {
        let mut node = handle.lock().unwrap();
        let depth = node.depth;
        let width = Self::WIDTHS[depth];
        let first = (from.max(node.min_char) - node.min_char) / width;
        for i in first..node.slots.len() {
            let start = node.min_char + i * width;
            if start > to { break; }
            if from <= start && start + width - 1 <= to {
                // Whole-slot writes discard subdivision even when values compare equal.
                node.slots[i] = value.clone();
            } else {
                if let CharTableSlot::Value(old) = node.slots[i] {
                    node.slots[i] = CharTableSlot::Node(Self::new(depth + 1, start, old));
                }
                let CharTableSlot::Node(child) = &node.slots[i] else { unreachable!() };
                Self::set_range(child, from, to, value);
            }
        }
    }

    pub(super) fn storage_bytes(handle: &CharTableNodeHandle, capacity: bool, seen: &mut HashSet<usize>) -> usize {
        if !seen.insert(char_table_node_id(handle)) { return 0; }
        let node = handle.lock().unwrap();
        // Arc's strong/weak counters, the mutex/node, and the separately allocated slots.
        2 * size_of::<usize>() + size_of::<Mutex<Self>>()
            + if capacity { node.slots.capacity() } else { node.slots.len() } * size_of::<CharTableSlot>()
            + node.slots.iter().map(|slot| match slot {
                CharTableSlot::Value(_) => 0,
                CharTableSlot::Node(child) => Self::storage_bytes(child, capacity, seen),
            }).sum::<usize>()
    }
}

/// Identity stays valid while a handle is retained, including across Lisp GC.
pub(super) fn char_table_node_id(node: &CharTableNodeHandle) -> usize {
    Arc::as_ptr(node) as usize
}

/// Callers may share SEEN across table and cursor roots during a collection.
/// The visitor must not reenter these node locks or run Lisp callbacks.
pub(super) fn visit_char_table_node_values(
    handle: &CharTableNodeHandle,
    seen: &mut HashSet<usize>,
    visit: &mut impl FnMut(&mut Value),
) {
    if !seen.insert(char_table_node_id(handle)) { return; }
    let mut node = handle.lock().unwrap();
    for slot in &mut node.slots {
        match slot {
            CharTableSlot::Value(value) => visit(value),
            CharTableSlot::Node(child) => visit_char_table_node_values(child, seen, visit),
        }
    }
}

/// INDEX's value in the table of ROOT, ASCII and DEFAULT (the default for
/// nil) and the inclusive character range of its slot.
fn span(root: &CharTableNodeHandle, ascii: &CharTableSlot, default: Value, index: usize) -> Result<(Value, usize, usize)> {
    if index >= CharTable::LEN { return Err("index out of range".into()); }
    let (value, from, to) = match ascii {
        _ if index >= 128 => {
            let (value, from, to) = CharTableNode::checked_span(root, index)?;
            (value, from.max(128), to)
        }
        CharTableSlot::Value(value) => (*value, 0, 127),
        CharTableSlot::Node(node) => {
            let node = node.lock().unwrap();
            if node.depth != 3 || node.min_char != 0 { return Err("invalid ASCII sub-char-table".into()); }
            match node.slots.get(index) {
                Some(CharTableSlot::Value(value)) => (*value, index, index),
                _ => return Err("invalid ASCII sub-char-table".into()),
            }
        }
    };
    Ok((if value == Value::NIL { default } else { value }, from, to))
}

/// A char-table's own values, read without the VM, for the length of one
/// scan: its nodes are shared, not copied.  `WidthRule` reads
/// `char-width-table` through these, its parents' after it.
#[derive(Clone)]
pub(crate) struct CharTableView {
    root: CharTableNodeHandle,
    ascii: CharTableSlot,
    default: Value,
}

impl CharTableView {
    /// INDEX's value, the default for nil (nil out of range).
    pub(crate) fn get(&self, index: usize) -> Value {
        span(&self.root, &self.ascii, self.default, index).map_or(Value::NIL, |(value, ..)| value)
    }
}

pub(super) struct CharTable {
    pub purpose: Value,
    pub(super) root: CharTableNodeHandle,
    pub(super) ascii: CharTableSlot,
    pub default: Value,
    pub parent: Value,
    pub extras: Vec<Value>,
}
romacs_snapshot::snapshot_struct!(CharTable { purpose, root, ascii, default, parent, extras });

// Lisp copy-sequence shares payload values, never native subdivision nodes.
impl Clone for CharTable {
    fn clone(&self) -> Self {
        let mut copies = HashMap::default();
        let root = CharTableNode::deep_copy(&self.root, &mut copies);
        let ascii = Self::ascii_from_root(&root);
        Self {
            purpose: self.purpose,
            root,
            ascii,
            default: self.default,
            parent: self.parent,
            extras: self.extras.clone(),
        }
    }
}

impl CharTable {
    pub const LEN: usize = 0x400000;
    pub(super) fn ascii_from_root(root: &CharTableNodeHandle) -> CharTableSlot {
        let mut slot = root.lock().unwrap().slots[0].clone();
        for _ in 0..2 {
            slot = match slot {
                CharTableSlot::Value(_) => return slot,
                CharTableSlot::Node(node) => node.lock().unwrap().slots[0].clone(),
            };
        }
        slot
    }
    pub(super) fn print_slots(&self) -> Vec<CharTableSlot> {
        let mut slots = vec![CharTableSlot::Value(self.default), CharTableSlot::Value(self.parent),
            CharTableSlot::Value(self.purpose), self.ascii.clone()];
        slots.extend(self.root.lock().unwrap().slots.iter().cloned());
        slots.extend(self.extras.iter().copied().map(CharTableSlot::Value));
        slots
    }
    pub fn new(purpose: Value) -> Self {
        Self { purpose, root: CharTableNode::new(0, 0, Value::NIL), ascii: CharTableSlot::Value(Value::NIL), default: Value::NIL, parent: Value::NIL, extras: vec![] }
    }
    // Called before publication, in a leaf allocation window (never collects).
    pub fn initialize(&mut self, count: usize, init: Value) -> Result<()> {
        self.extras.try_reserve_exact(count).map_err(|_| "Memory exhausted".to_owned())?;
        self.extras.resize(count, init);
        self.default = init;
        self.ascii = CharTableSlot::Value(init);
        self.root.lock().unwrap().slots.fill(CharTableSlot::Value(init));
        Ok(())
    }
    /// Raw fields for the uniform-table printer. A retained child is not uniform
    /// structurally, even if all of its leaves contain the same value.
    pub fn uniform_print_fields(&self) -> Option<Vec<Value>> {
        let root = self.root.lock().unwrap();
        let CharTableSlot::Value(content) = root.slots[0] else { return None; };
        if !root.slots.iter().all(|slot| matches!(slot, CharTableSlot::Value(v) if *v == content)) { return None; }
        let mut fields = Vec::with_capacity(68 + self.extras.len());
        let CharTableSlot::Value(ascii) = self.ascii else { return None; };
        fields.extend([self.default, self.parent, self.purpose, ascii]);
        fields.extend(std::iter::repeat_n(content, 64));
        fields.extend(self.extras.iter().copied());
        Some(fields)
    }
    pub(super) fn validate_mapping(&self) -> Result<()> {
        CharTableNode::validate_traversal(&self.root, 0, 0)?;
        let derived = Self::ascii_from_root(&self.root);
        let same = match (&self.ascii, &derived) {
            (CharTableSlot::Value(a), CharTableSlot::Value(b)) => a == b,
            (CharTableSlot::Node(a), CharTableSlot::Node(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        if !same { return Err("mapping independent ASCII slot is not supported".into()); }
        Ok(())
    }
    pub fn get(&self, index: usize) -> Result<Value> {
        self.span(index).map(|(value, ..)| value)
    }
    /// No character has a value of its own, nor a default.
    pub(crate) fn all_nil(&self) -> bool {
        fn nil(slot: &CharTableSlot) -> bool {
            match slot {
                CharTableSlot::Value(value) => *value == Value::NIL,
                CharTableSlot::Node(node) => node.lock().unwrap().slots.iter().all(nil),
            }
        }
        self.default == Value::NIL && nil(&self.ascii) && self.root.lock().unwrap().slots.iter().all(nil)
    }
    /// char_table_ref_and_range at slot granularity: INDEX's value (the
    /// default for nil) and the inclusive character range of its slot. ASCII
    /// reads the table's own ASCII slot, which a literal may detach from the
    /// root; the root's range for a higher character starts above it.
    pub(crate) fn span(&self, index: usize) -> Result<(Value, usize, usize)> {
        span(&self.root, &self.ascii, self.default, index)
    }
    /// A read view of this table's own values that shares its nodes.
    pub(crate) fn view(&self) -> CharTableView {
        CharTableView { root: self.root.clone(), ascii: self.ascii.clone(), default: self.default }
    }
    pub fn set(&mut self, index: usize, value: Value) -> Result<()> {
        self.set_range(index, index, value)
    }
    /// Partial writes retain existing children; whole-slot writes replace them.
    /// A single-character write descends all the way to depth-three leaves,
    /// including writes equal to the old value (GNU char_table_set).
    pub fn set_range(&mut self, from: usize, to: usize, value: Value) -> Result<()> {
        self.set_range_slot(from, to, CharTableSlot::Value(value))
    }
    pub(super) fn set_range_slot(&mut self, from: usize, to: usize, value: CharTableSlot) -> Result<()> {
        if from >= Self::LEN || to >= Self::LEN { return Err("index out of range".into()); }
        // Preserve this API's historical no-op for inverted ranges. GNU may
        // retain subdivisions for such a write; that structural divergence is intentional.
        if from > to { return Ok(()); }
        if from == to && from < 128 {
            if let CharTableSlot::Node(node) = &self.ascii {
                let mut node = node.lock().unwrap();
                if node.depth != 3 || node.min_char != 0 { return Err("invalid ASCII sub-char-table".into()); }
                let slot = node.slots.get_mut(from).ok_or("invalid ASCII sub-char-table")?;
                *slot = value;
                return Ok(());
            }
        }
        CharTableNode::validate_geometry(&self.root, 0, 0, from, to)?;
        CharTableNode::set_range(&self.root, from, to, &value);
        if from < 128 { self.ascii = Self::ascii_from_root(&self.root); }
        Ok(())
    }
    /// Rewrite every character entry (not purpose, default, parent or
    /// extras), as map_char_table + set-char-table-range would.
    pub(crate) fn map_entries(&mut self, map: &mut impl FnMut(&mut Value)) {
        visit_char_table_node_values(&self.root, &mut HashSet::default(), map);
        self.ascii = Self::ascii_from_root(&self.root);
    }
    pub fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        self.visit_values_seen(&mut HashSet::default(), visit);
    }
    pub(super) fn visit_values_seen(&mut self, seen: &mut HashSet<usize>, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.purpose);
        visit(&mut self.default);
        visit(&mut self.parent);
        for value in &mut self.extras { visit(value); }
        match &mut self.ascii {
            CharTableSlot::Value(value) => visit(value),
            CharTableSlot::Node(node) => visit_char_table_node_values(node, seen, visit),
        }
        visit_char_table_node_values(&self.root, seen, visit);
    }
    pub fn storage_bytes(&self, capacity: bool) -> usize {
        self.storage_bytes_seen(capacity, &mut HashSet::default())
    }
    pub(super) fn storage_bytes_seen(&self, capacity: bool, seen: &mut HashSet<usize>) -> usize {
        size_of::<Self>()
            + CharTableNode::storage_bytes(&self.root, capacity, seen)
            + match &self.ascii { CharTableSlot::Value(_) => 0, CharTableSlot::Node(node) => CharTableNode::storage_bytes(node, capacity, seen) }
            + if capacity { self.extras.capacity() } else { self.extras.len() } * size_of::<Value>()
    }
}
