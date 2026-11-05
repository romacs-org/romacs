use super::*;

#[test]
fn make_keymap_contains_a_real_character_table() {
    let mut vm = Vm::new();
    let got = vm.eval_source_raw("(let ((m (make-keymap))) (list (car m) (char-table-p (car (cdr m))) (keymapp m) (length m)))").and_then(|v| vm.display(v));
    assert_eq!(got, Ok("(keymap t t 2)".to_owned()), "make-keymap must contain a char-table, not a sparse list");
}


#[test]
fn dense_keymap_prompt_and_character_slot_roundtrip() {
    let mut vm = Vm::new();
    check(&mut vm, "(setq dense (make-keymap \"Menu\")) (list (car (cdr (cdr dense))) (char-table-subtype (car (cdr dense))) (keymapp (make-sparse-keymap)) (keymapp (car (cdr dense))) (car (cdr (cdr (make-keymap 42)))))", "(\"Menu\" keymap t nil 42)");
    check(&mut vm, "(define-key dense [97] 'bound) (define-key dense [945] 'alpha) (list (lookup-key dense [97]) (aref (car (cdr dense)) 97) (lookup-key dense [945]) (aref (car (cdr dense)) 945) (length dense))", "(bound bound alpha alpha 3)");
    check(&mut vm, "(aset (car (cdr dense)) 98 'direct) (define-key dense [f1] 'function) (define-key dense [24 6] 'prefix) (list (lookup-key dense [98]) (lookup-key dense [f1]) (lookup-key dense [24 6]) (char-table-p (car (cdr dense))))", "(direct function prefix t)");
    check(&mut vm, "(define-key dense [97] nil) (list (lookup-key dense [97]) (aref (car (cdr dense)) 97))", "(nil t)");
    check(&mut vm, "(define-key dense [97] nil t) (list (lookup-key dense [97]) (aref (car (cdr dense)) 97))", "(nil nil)");
    check(&mut vm, "(aset (car (cdr dense)) 4194303 'last) (list (aref (car (cdr dense)) 4194303) (length (car (cdr dense))))", "(last 4194304)");
}

#[test]
fn dense_keymap_slots_survive_moving_gc_without_retention() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    // Fixed source, fixed symbols, copied payload: neither fresh interned symbols
    // nor a compiler-retained literal can masquerade as retained table storage.
    const SOURCE: &str = "(setq dense-root (make-keymap)) (define-key dense-root [97] (vector (copy-sequence \"slot-payload\"))) (define-key dense-root [945] (vector (copy-sequence \"unicode-payload\"))) nil";
    let mut retained = vec![];
    for round in 0..4 {
        for _ in 0..200 { vm.allocate_object(data::Object::Vector(vec![])); }
        check(&mut vm, SOURCE, "nil");
        let map = vm.slots[vm.names["dense-root"]].value;
        let old_table = vm.cons_pair(vm.cons_pair(map).unwrap().1).unwrap().0;
        let before = vm.heap_stats().collections;
        vm.collect(&mut []);
        assert_eq!(vm.heap_stats().collections, before + 1, "retention probe must force collection");
        let map = vm.slots[vm.names["dense-root"]].value;
        let table = vm.cons_pair(vm.cons_pair(map).unwrap().1).unwrap().0;
        assert_ne!(old_table, table, "fixture must relocate the character table");
        let stats = vm.heap_stats();
        eprintln!("dense retention {round}: objects={} bytes={}", stats.live_objects, stats.live_bytes);
        retained.push((stats.live_objects, stats.live_bytes));
        // Check after collecting, before any new evaluation can reuse stale indices.
        let data::Object::CharTable(t) = vm.object(table).unwrap() else { panic!("character table"); };
        let payload = t.get(97).unwrap();
        assert_eq!(vm.display(payload), Ok("[\"slot-payload\"]".into()), "character-table slot must trace its Value");
        check(&mut vm, "(list (lookup-key dense-root [97]) (lookup-key dense-root [945]))", "([\"slot-payload\"] [\"unicode-payload\"])");
    }
    assert!(retained.windows(2).all(|pair| pair[0] == pair[1]), "dense keymaps must plateau in both objects and bytes: {retained:?}");
}

fn check(vm: &mut Vm, source: &str, want: &str) {
    let got = vm.eval_source_raw(source).and_then(|v| vm.display(v));
    assert_eq!(got, Ok(want.to_owned()), "keymap behavior: {source}");
}

#[test]
fn keymap_lists_are_mutable_and_preserve_binding_identity() {
    let mut vm = Vm::new();
    check(&mut vm, "(setq m (make-sparse-keymap \"Menu\")) (list (car m) (car (cdr m)) (keymapp m) (keymapp 1))", "(keymap \"Menu\" t nil)");
    check(&mut vm, "(define-key m [97] 'first) (setq cell (car (cdr m))) (define-key m \"a\" 'second) (list (eq cell (car (cdr m))) (lookup-key m [97]))", "(t second)");
    check(&mut vm, "(setcdr cell 'external) (lookup-key m \"a\")", "external");
}

#[test]
fn keymap_prefixes_symbols_meta_and_too_long() {
    let mut vm = Vm::new();
    check(&mut vm, "(setq m (make-sparse-keymap)) (define-key m \"ab\" 'nested) (list (lookup-key m \"ab\") (keymapp (lookup-key m \"a\")) (lookup-key m \"abc\") (lookup-key m \"xy\") (eq m (lookup-key m [])))", "(nested t 2 1 t)");
    check(&mut vm, "(fset 'prefix (make-sparse-keymap)) (define-key m [f1] 'prefix) (define-key m [f1 f2] 'symbol-prefix) (define-key m [134217848] 'meta-x) (list (keymapp 'prefix) (lookup-key m [f1 f2]) (lookup-key m \"\\ex\"))", "(t symbol-prefix meta-x)");
    check(&mut vm, "(define-key m \"z\" 'command) (condition-case nil (define-key m \"zz\" 'bad) (error 'blocked))", "blocked");
    check(&mut vm, "(list (lookup-key m [134217848 97]) (lookup-key m [134217849 97]))", "(1 1)");
}

#[test]
fn keymap_parent_tail_defaults_nil_and_removal() {
    let mut vm = Vm::new();
    check(&mut vm, "(setq p (make-sparse-keymap)) (define-key p [97] 'parent) (setq m (make-sparse-keymap)) (setcdr m p) (list (lookup-key m \"a\") (define-key m \"a\" nil) (lookup-key m \"a\"))", "(parent nil nil)");
    check(&mut vm, "(define-key m \"a\" nil t) (define-key m [t] 'default) (list (lookup-key m \"a\") (lookup-key m \"z\") (lookup-key m \"z\" t))", "(parent nil default)");
    check(&mut vm, "(lookup-key m \"a\" t)", "parent");
}

#[test]
fn keymap_inherited_prefixes_and_static_menu_bindings() {
    let mut vm = Vm::new();
    check(&mut vm, "(setq p (make-sparse-keymap)) (setq m (make-sparse-keymap)) (define-key p \"ab\" 'inherited) (define-key m \"ac\" 'own) (setcdr (cdr m) p) (list (lookup-key m \"ab\") (lookup-key m \"ac\"))", "(inherited own)");
    check(&mut vm, "(define-key m [menu] '(menu-item \"Name\" action)) (define-key m [old] '(\"Old\" . old-action)) (list (lookup-key m [menu]) (lookup-key m [old]))", "(action old-action)");
    check(&mut vm, "(define-key (lookup-key m \"a\") \"d\" 'stored) (lookup-key m \"ad\")", "stored");
}

/// Reading a form interns `global-map' before any keymap primitive runs;
/// that must not count as installed keymaps (it panicked before).
#[test]
fn naming_global_map_before_first_keymap_use_still_installs_keymaps() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    check(&mut vm, "(progn (current-global-map) (mapc #'makunbound '(global-map esc-map ctl-x-map)) (list (keymapp (current-global-map)) (boundp 'global-map)))", "(t nil)");
    let mut vm = Vm::new();
    check(&mut vm, "(progn 'global-map (global-set-key [24 6] 'find) (key-binding [24 6]))", "find");
}

#[test]
fn keymap_active_maps_are_buffer_local_and_kbd_parses() {
    let mut vm = Vm::new();
    check(&mut vm, "(global-set-key (kbd \"C-x C-f\") 'find) (list (key-binding [24 6]) (lookup-key global-map [24 6]) (length (kbd \"M-x <f1> RET\")))", "(find find 3)");
    check(&mut vm, "(setq local (make-sparse-keymap)) (define-key local [24 6] 'local-find) (use-local-map local) (key-binding [24 6])", "local-find");
    check(&mut vm, "(set-buffer (get-buffer-create (generate-new-buffer-name \"other\"))) (key-binding [24 6])", "find");
    check(&mut vm, "(use-local-map nil) (key-binding [24 6])", "find");
    check(&mut vm, "(setq global-map (make-sparse-keymap)) (key-binding [24 6])", "find");
}

// The bound payloads are VECTORS, not lists, and must stay vectors. GNU's
// get_keyelt treats a binding of the form (STRING . DEFN) as an old-style menu
// item and returns DEFN (src/keymap.c:718), so `(list "payload")` would be read
// as ("payload" . nil) and lookup would yield nil -- the fixture would assert
// nothing about GC. A vector is returned verbatim and carries the same property:
// a heap Value held only by a keymap, whose payload must survive relocation.
#[test]
fn keymap_roots_survive_counted_compaction() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    for _ in 0..200 { vm.conses.push((Value::NIL, Value::NIL)); }
    check(&mut vm, "(setq m (make-sparse-keymap)) (define-key m [97 98] (vector \"payload\")) (global-set-key [f1] m) (use-local-map m)", "nil");
    check(&mut vm, "(use-local-map (make-sparse-keymap)) (define-key (progn (setq private (make-sparse-keymap)) private) [97 98] (vector \"local-only\")) (use-local-map private) (setq private nil)", "nil");
    let buffer = vm.buffer_index().unwrap();
    let data::Object::Buffer(b) = &vm.objects[buffer] else { panic!("buffer"); };
    let old = b.local_keymap;
    let before = vm.heap_stats().collections;
    vm.collect(&mut []);
    assert!(vm.heap_stats().collections > before, "forced collection must count");
    let buffer = vm.buffer_index().unwrap();
    let data::Object::Buffer(b) = &vm.objects[buffer] else { panic!("buffer"); };
    assert_ne!(b.local_keymap, old, "fixture must move the local map");
    check(&mut vm, "(key-binding [97 98])", "[\"local-only\"]");
    check(&mut vm, "(use-local-map m)", "nil");
    vm.set_collection_floor(1);
    for _ in 0..20 {
        check(&mut vm, "(list 1 2 3 4 5 6 7 8)", "(1 2 3 4 5 6 7 8)");
    }
    assert!(vm.heap_stats().collections > before, "fixture must actually collect");
    check(&mut vm, "(list (lookup-key m [97 98]) (key-binding [97 98]) (lookup-key (key-binding [f1]) [97 98]))", "([\"payload\"] [\"payload\"] [\"payload\"])");
}

/// keyboard.c read_char runs a special event's `special-event-map'
/// binding at once with the event as its keys; focus-in and focus-out are
/// bound as keys_of_keyboard binds them, and `internal-handle-focus-in'
/// accepts only a focus-in event naming a frame.
#[test]
fn focus_events_run_their_special_event_bindings() {
    let mut vm = Vm::new();
    // A bare VM installs its keymaps on first keymap use (`ensure_keymaps').
    check(&mut vm, "(progn (current-global-map) (list (lookup-key special-event-map [focus-in]) (lookup-key special-event-map [focus-out])))", "(handle-focus-in handle-focus-out)");
    check(&mut vm, "(romacs--run-special-event (list 'focus-in (selected-frame)))", "nil");
    check(&mut vm, "(defun handle-focus-in (event) (interactive \"e\") (internal-handle-focus-in event) (setq focused (list (car event) (framep (car (cdr event))) (eq last-input-event event))))
                    (list (romacs--run-special-event (list 'focus-in (selected-frame))) focused)", "(t (focus-in t t))");
    check(&mut vm, "(list (romacs--run-special-event '(no-such-event)) (condition-case e (internal-handle-focus-in '(focus-out nil)) (error e)))", "(nil (error \"Invalid focus-in event\"))");
}
