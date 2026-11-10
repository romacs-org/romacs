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

    thread_local! {
        /// Calls of `wrote` on this thread.
        pub(super) static WROTE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// The calls of `wrote` that WRITE makes.
    fn funnelled(write: impl FnOnce()) -> usize {
        let before = WROTE.with(std::cell::Cell::get);
        write();
        WROTE.with(std::cell::Cell::get) - before
    }

    /// A copy whose root lands where a freed table's was is still another
    /// table to what keys on serials, and a new table and a literal read each
    /// take a serial of their own.
    #[test]
    fn a_copy_where_a_freed_table_was_has_its_own_serial() {
        let template = CharTable::new(Value::NIL);
        let mut landed = 0;
        for _ in 0..1000 {
            let freed = CharTable::new(Value::NIL);
            let (address, serial) = (Arc::as_ptr(&freed.root), freed.serial);
            drop(freed);
            let copy = template.clone();
            if Arc::as_ptr(&copy.root) == address {
                landed += 1;
                assert_ne!(copy.serial, serial, "a copy where a freed table was took its serial");
            }
        }
        assert!(landed > 0, "no copy landed where a freed table was: the case is not exercised");
        let mut vm = Vm::new();
        let literal = run(&mut vm, &format!("#^[nil nil test nil{}]", " nil".repeat(64)));
        let made = run(&mut vm, "(make-char-table 'test)");
        let serials = [vm.char_table_serial(literal), vm.char_table_serial(made), template.serial];
        assert!(serials.iter().all(|&serial| serial != 0), "{serials:?}");
        assert!(serials[0] != serials[1] && serials[0] != serials[2] && serials[1] != serials[2], "{serials:?}");
    }

    /// A read of the entries sees what a map would, in its order, once for a
    /// node two slots share, and passes nothing to the funnel nor counts an edit.
    #[test]
    fn a_read_of_the_entries_is_a_maps_walk_and_writes_nothing() {
        let int = |n| Value::integer(n).unwrap();
        let mut table = CharTable::new(Value::NIL);
        table.set(200, int(1)).unwrap();
        table.set(300, int(2)).unwrap();
        table.set(0x10005, int(3)).unwrap();
        table.set_range(0x20000, 0x2ffff, int(4)).unwrap();
        // A node two root slots share, made here by hand (`set-char-table-range`
        // with a sub-char-table shares one too): the walk visits it once.
        let shared = table.root.lock().unwrap().slots[0].clone();
        table.root.lock().unwrap().slots[5] = shared;
        let mut before = vec![];
        table.map_entries(&mut |value| before.push(*value));
        let edits = table.edits;
        let mut read = vec![];
        assert_eq!(funnelled(|| read = table.entry_values()), 0, "a read passes nothing");
        assert_eq!(table.edits, edits, "a read counts no edit");
        let mut after = vec![];
        table.map_entries(&mut |value| after.push(*value));
        assert_eq!(read, before, "a read sees what a map would, in its order");
        assert_eq!(after, before, "a read leaves the entries as they were");
        assert!(read.contains(&int(2)) && read.contains(&int(3)) && read.contains(&int(4)));
        assert_eq!(read.iter().filter(|&&v| v == int(1)).count(), 1, "the shared node once");
    }

    /// Each way to write a table's contents tells the funnel what it stores.
    #[test]
    fn every_content_write_passes_the_funnel() {
        let one = Value::integer(1).unwrap();
        let mut table = CharTable::new(Value::NIL);
        assert_eq!(funnelled(|| table.set_range(0x10000, 0x1ffff, one).unwrap()), 1, "a whole slot");
        assert_eq!(funnelled(|| table.set(200, one).unwrap()), 4, "three splits, then the character");
        assert_eq!(funnelled(|| table.set(5, one).unwrap()), 3, "a split, the character, the ASCII slot");
        assert!(matches!(table.ascii, CharTableSlot::Node(_)));
        assert_eq!(funnelled(|| table.set(6, one).unwrap()), 1, "the ASCII path");
        assert_eq!(funnelled(|| table.fill(&CharTableSlot::Value(Value::NIL))), 65, "every root slot, then the ASCII slot");
        assert_eq!(funnelled(|| table.set_default(one)), 1, "the default");
        assert_eq!(funnelled(|| table.set_parent(Value::NIL)), 1, "the parent");
        table.set(200, one).unwrap();
        let block = child(&child(&table.root, 0), 0);
        assert_eq!(block.lock().unwrap().depth, 2);
        let expanded = funnelled(|| { table.expand(&block, 1, vec![one; 128]); });
        assert_eq!(expanded, 2, "the new block, then the ASCII slot");
        // Uniform but subdivided: the block, its parents and the root collapse.
        table.fill(&CharTableSlot::Value(one));
        table.set(200, one).unwrap();
        let lifted = funnelled(|| optimize(&table.root, &mut |a, b| Ok::<bool, ()>(a == b)).unwrap());
        assert_eq!(lifted, 3, "a Value lifted into each of three parents");
        assert!(matches!(table.root.lock().unwrap().slots[0], CharTableSlot::Value(v) if v == one));
        let mapped = funnelled(|| table.map_entries(&mut |value| *value = Value::NIL));
        assert_eq!(mapped, 65, "each root Value written back, then the ASCII slot");
        let mut extra = CharTable::new(Value::NIL);
        extra.initialize(2, Value::NIL).unwrap();
        assert_eq!(funnelled(|| extra.set_extra(0, one)), 1, "an extra slot");
        assert_eq!(funnelled(|| extra.set_extras(&[one, one])), 2, "every extra slot");
    }

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
        table.collector_visit_values(&mut HashSet::default(), &mut |value| {
            if *value == Value::TRUE { count += 1; }
        });
        assert_eq!(count, 16, "each physical slot is visited once, not each incoming edge");
        let mut seen = HashSet::default();
        collector_visit_node(&table.root, &mut seen, &mut |_| {});
        let mut revisits = 0;
        collector_visit_node(&shared, &mut seen, &mut |_| { revisits += 1; });
        assert_eq!(revisits, 0, "shared collector state also covers a retained handle");
    }

    #[test]
    fn retained_leaf_payload_is_forwarded_by_existing_collector() {
        let mut vm = Vm::new();
        vm.set_gc_moving(true);
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
        vm.set_gc_moving(true);
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
        vm.set_gc_moving(true);
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
        let tail = self.alloc_cons(Value::integer(count).unwrap(), Value::NIL);
        let plist = self.alloc_cons(property, tail);
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

    /// TABLE's serial, which no other table has had (0 for a value that is
    /// not a char table).
    pub(crate) fn char_table_serial(&self, table: Value) -> u64 {
        match self.object_if(table) {
            Some(data::Object::CharTable(t)) => t.serial,
            _ => 0,
        }
    }

    /// TABLE's own runs of one value, in character order, each slot of the
    /// table once: char_table_ref_and_range walked over the characters.
    pub(crate) fn char_table_edits(&self, table: Value) -> Result<u64> {
        match self.object(table)? {
            data::Object::CharTable(t) => Ok(t.edits),
            _ => Err("char-table required".into()),
        }
    }

    pub(crate) fn char_table_runs(&self, table: Value) -> Result<Vec<(u32, u32, Value)>> {
        let data::Object::CharTable(t) = self.object(table)? else { return Err("char-table required".into()) };
        let (mut runs, mut at): (Vec<(u32, u32, Value)>, usize) = (Vec::new(), 0);
        while at <= crate::lstring::MAX_CHAR as usize {
            let (value, _, to) = t.span(at)?;
            match runs.last_mut() {
                Some(run) if run.2 == value => run.1 = to as u32,
                _ => runs.push((at as u32, to as u32, value)),
            }
            at = to + 1;
        }
        Ok(runs)
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
/// `collector_visit_node`; an Arc alone is not a Lisp GC root.
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
    depth: usize,
    min_char: usize,
    slots: Vec<CharTableSlot>,
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

    pub(super) fn depth(&self) -> usize {
        self.depth
    }
    pub(super) fn min_char(&self) -> usize {
        self.min_char
    }
    /// This node's slots, to read.
    pub(super) fn slots(&self) -> &[CharTableSlot] {
        &self.slots
    }

    /// Store SLOT at I, through the funnel.
    fn store(&mut self, i: usize, slot: CharTableSlot) {
        wrote(&slot);
        self.slots[i] = slot;
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
                node.store(i, value.clone());
            } else {
                if let CharTableSlot::Value(old) = node.slots[i] {
                    // The split moves OLD into a node no entry has traced yet.
                    node.store(i, CharTableSlot::Node(Self::new(depth + 1, start, old)));
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

/// chartab.c Foptimize_char_table under ROOT: collapse every sub-table whose
/// slots are all values SAME as the first, children first. The caller resets
/// the table's ASCII slot after.
pub(super) fn optimize<E>(root: &CharTableNodeHandle, same: &mut impl FnMut(Value, Value) -> std::result::Result<bool, E>) -> std::result::Result<(), E> {
    let slots = root.lock().unwrap().slots.clone();
    for (i, slot) in slots.into_iter().enumerate() {
        if let CharTableSlot::Node(child) = slot {
            let optimized = optimize_node(&child, same)?;
            root.lock().unwrap().store(i, optimized);
        }
    }
    Ok(())
}

/// optimize_sub_char_table: NODE's slot in its parent after optimizing.
fn optimize_node<E>(node: &CharTableNodeHandle, same: &mut impl FnMut(Value, Value) -> std::result::Result<bool, E>) -> std::result::Result<CharTableSlot, E> {
    let mut slots = node.lock().unwrap().slots.clone();
    let mut children = Vec::new();
    for (i, slot) in slots.iter_mut().enumerate() {
        if let CharTableSlot::Node(child) = slot {
            *slot = optimize_node(&child.clone(), same)?;
            children.push(i);
        }
    }
    // Each child's slot, a collapsed one's Value moving up into NODE, once
    // every child has optimized.
    let mut parent = node.lock().unwrap();
    for i in children {
        parent.store(i, slots[i].clone());
    }
    drop(parent);
    let CharTableSlot::Value(first) = slots[0] else {
        return Ok(CharTableSlot::Node(node.clone()));
    };
    for slot in &slots[1..] {
        let CharTableSlot::Value(this) = *slot else {
            return Ok(CharTableSlot::Node(node.clone()));
        };
        if !same(this, first)? {
            return Ok(CharTableSlot::Node(node.clone()));
        }
    }
    Ok(CharTableSlot::Value(first))
}

/// Every write to a char table's contents passes here first, with what it
/// stores: a slot a range write, fill or expansion puts in, a node a split
/// makes, a Value an optimize lifts into a parent, a new default or parent,
/// each Value a map writes back. The collector's barrier hooks here, and the
/// write generation is bumped here. A node may be shared by several entries
/// (a `CharTable`, a `SubCharTable`, a keymap cursor), so this is not
/// `Table`'s barrier, which hooks one entry. It runs with the parent node
/// locked, and a node it is handed may be that parent, so a hook must queue
/// a `Node`'s handle, never lock it here.
#[inline]
fn wrote(_stored: &CharTableSlot) {
    WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    tests::WROTE.with(|n| n.set(n.get() + 1));
}

/// The writes to char tables' contents so far in this process, all tables
/// counted together: what is derived from a table's contents (the syntax
/// summaries) keys on it with the table's serial, so a write
/// anywhere, through a parent or a node shared with another table
/// included, renews the key.
static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) fn writes() -> u64 {
    WRITES.load(std::sync::atomic::Ordering::Relaxed)
}

/// The next table serial: one per table made, copied or read, never handed
/// out twice in a process or across the world image it restores.
static SERIALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn fresh_serial() -> u64 {
    SERIALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
/// The counters a world image records: the next serial and the writes.
pub(crate) fn image_counters() -> [u64; 2] {
    [SERIALS.load(std::sync::atomic::Ordering::Relaxed), writes()]
}
/// Continue a restored world's COUNTERS: a table in it keeps its serial,
/// and what was derived under its writes stays valid until the next one.
pub(crate) fn restore_counters([serials, writes]: [u64; 2]) {
    SERIALS.fetch_max(serials, std::sync::atomic::Ordering::Relaxed);
    WRITES.fetch_max(writes, std::sync::atomic::Ordering::Relaxed);
}

/// The collector's own visit of HANDLE's Values, which forwarding rewrites in
/// place past the funnel (`tests/no_remote_references.rs` keeps it there).
/// `map_entries` and `entry_values` walk by it too, so their orders match.
/// Callers may share SEEN across table and cursor roots during a collection.
/// The visitor must not reenter these node locks or run Lisp callbacks.
pub(super) fn collector_visit_node(
    handle: &CharTableNodeHandle,
    seen: &mut HashSet<usize>,
    visit: &mut impl FnMut(&mut Value),
) {
    if !seen.insert(char_table_node_id(handle)) { return; }
    let mut node = handle.lock().unwrap();
    for slot in &mut node.slots {
        match slot {
            CharTableSlot::Value(value) => visit(value),
            CharTableSlot::Node(child) => collector_visit_node(child, seen, visit),
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
    purpose: Value,
    root: CharTableNodeHandle,
    ascii: CharTableSlot,
    default: Value,
    parent: Value,
    extras: Vec<Value>,
    /// Counts the writes to its characters, for what is derived from them.
    edits: u64,
    /// This table among all others, made, copied or read (`fresh_serial`):
    /// what derived data keys on, since an address may be reused or moved.
    serial: u64,
}
romacs_snapshot::snapshot_struct!(CharTable { purpose, root, ascii, default, parent, extras, edits, serial });

impl CharTable {
    /// Whether no character has a value, as a new table's: read without
    /// walking any subdivision, which a table with values has.
    pub(crate) fn is_empty(&self) -> bool {
        let nil = |slot: &CharTableSlot| matches!(slot, CharTableSlot::Value(value) if *value == Value::NIL);
        self.default == Value::NIL && self.parent == Value::NIL
            && self.root.lock().unwrap_or_else(|poison| poison.into_inner()).slots.iter().all(nil)
    }
}

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
            edits: 0,
            serial: fresh_serial(),
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
    /// A new table sharing OTHER's nodes, as no Lisp operation makes one.
    #[cfg(test)]
    pub(super) fn sharing_nodes_of(other: &Self) -> Self {
        Self { root: Arc::clone(&other.root), ascii: other.ascii.clone(), ..Self::new(Value::NIL) }
    }
    pub(super) fn root(&self) -> &CharTableNodeHandle {
        &self.root
    }
    pub(super) fn default(&self) -> Value {
        self.default
    }
    pub(super) fn parent(&self) -> Value {
        self.parent
    }
    pub(super) fn purpose(&self) -> Value {
        self.purpose
    }
    pub(super) fn extras(&self) -> &[Value] {
        &self.extras
    }
    /// Extra slot I, which must exist, as VALUE.
    pub(super) fn set_extra(&mut self, i: usize, value: Value) {
        wrote(&CharTableSlot::Value(value));
        self.extras[i] = value;
    }
    /// Every extra slot, as VALUES, one each.
    pub(super) fn set_extras(&mut self, values: &[Value]) {
        for &value in values { wrote(&CharTableSlot::Value(value)); }
        self.extras.copy_from_slice(values);
    }
    pub fn new(purpose: Value) -> Self {
        Self { purpose, root: CharTableNode::new(0, 0, Value::NIL), ascii: CharTableSlot::Value(Value::NIL), default: Value::NIL, parent: Value::NIL, extras: vec![], edits: 0, serial: fresh_serial() }
    }
    // Called before publication, in a leaf allocation window (never collects).
    pub fn initialize(&mut self, count: usize, init: Value) -> Result<()> {
        self.extras.try_reserve_exact(count).map_err(|_| "Memory exhausted".to_owned())?;
        self.extras.resize(count, init);
        self.set_default(init);
        self.fill(&CharTableSlot::Value(init));
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
        self.edits += 1;
        // Preserve this API's historical no-op for inverted ranges. GNU may
        // retain subdivisions for such a write; that structural divergence is intentional.
        if from > to { return Ok(()); }
        if from == to && from < 128 {
            if let CharTableSlot::Node(node) = &self.ascii {
                let mut node = node.lock().unwrap();
                if node.depth != 3 || node.min_char != 0 { return Err("invalid ASCII sub-char-table".into()); }
                if from >= node.slots.len() { return Err("invalid ASCII sub-char-table".into()); }
                node.store(from, value);
                return Ok(());
            }
        }
        CharTableNode::validate_geometry(&self.root, 0, 0, from, to)?;
        CharTableNode::set_range(&self.root, from, to, &value);
        if from < 128 { self.reset_ascii(); }
        Ok(())
    }
    /// Every root slot SLOT, as a new table's are.
    pub(super) fn fill(&mut self, slot: &CharTableSlot) {
        let mut root = self.root.lock().unwrap();
        for i in 0..root.slots.len() {
            root.store(i, slot.clone());
        }
        drop(root);
        self.reset_ascii();
    }
    pub(super) fn set_default(&mut self, value: Value) {
        wrote(&CharTableSlot::Value(value));
        self.default = value;
    }
    pub(super) fn set_parent(&mut self, value: Value) {
        wrote(&CharTableSlot::Value(value));
        self.parent = value;
    }
    /// The ASCII slot read again from the root, after a write below it
    /// (set_char_table_ascii).
    pub(super) fn reset_ascii(&mut self) {
        let ascii = Self::ascii_from_root(&self.root);
        wrote(&ascii);
        self.ascii = ascii;
    }
    /// Slot INDEX of NODE, one of this table's 128-character blocks, as the
    /// depth-three node of VALUES that a compressed block decodes to
    /// (uniprop_table_uncompress). The new slot.
    pub(super) fn expand(&mut self, node: &CharTableNodeHandle, index: usize, values: Vec<Value>) -> CharTableSlot {
        let mut parent = node.lock().unwrap();
        let from = parent.min_char + index * CharTableNode::WIDTHS[parent.depth];
        let slots = values.into_iter().map(CharTableSlot::Value).collect();
        let expanded = CharTableSlot::Node(Arc::new(Mutex::new(CharTableNode { depth: 3, min_char: from, slots })));
        parent.store(index, expanded.clone());
        drop(parent);
        self.reset_ascii();
        expanded
    }
    /// Every character entry's Value, by `map_entries`' own walk: a read,
    /// whose visitor writes nothing, so nothing passes the funnel.
    pub(crate) fn entry_values(&self) -> Vec<Value> {
        let mut out = Vec::new();
        collector_visit_node(&self.root, &mut HashSet::default(), &mut |value| out.push(*value));
        out
    }
    /// Rewrite every character entry (not purpose, default, parent or
    /// extras), as map_char_table + set-char-table-range would.
    pub(crate) fn map_entries(&mut self, map: &mut impl FnMut(&mut Value)) {
        self.edits += 1;
        collector_visit_node(&self.root, &mut HashSet::default(), &mut |value| {
            let mut new = *value;
            map(&mut new);
            wrote(&CharTableSlot::Value(new));
            *value = new;
        });
        self.reset_ascii();
    }
    /// The collector's own visit of every Value, past the funnel, as
    /// `collector_visit_node`'s.
    pub(super) fn collector_visit_values(&mut self, seen: &mut HashSet<usize>, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.purpose);
        visit(&mut self.default);
        visit(&mut self.parent);
        for value in &mut self.extras { visit(value); }
        match &mut self.ascii {
            CharTableSlot::Value(value) => visit(value),
            CharTableSlot::Node(node) => collector_visit_node(node, seen, visit),
        }
        collector_visit_node(&self.root, seen, visit);
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
