//! Collector proofs for symbol property lists (GNU `mark_object` marks
//! `ptr->u.s.plist`, src/alloc.c:6673). A plist is a moving heap Value held by
//! the symbol, outside every Lisp-visible root, so it must be marked AND
//! forwarded by the compaction. These tests measure live objects and bytes
//! across forced collections and read the payload back after it has moved.
use super::*;

fn load(vm: &mut Vm, name: &str, text: &str) -> Result<Value> {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/plist-unit");
    let dir = std::env::var_os("CARGO_TARGET_DIR").map_or(dir, |t| std::path::PathBuf::from(t).join("plist-unit"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}-{name}", std::process::id()));
    std::fs::write(&path, format!(";;; -*- lexical-binding: t -*-\n{text}")).unwrap();
    vm.load_file_raw(&path)
}

/// The one generated (uninterned) slot named NAME. Found by scanning the
/// symbol table, so the lookup itself creates no Lisp reference.
fn uninterned(vm: &Vm, name: &str) -> Value {
    let found: Vec<_> = vm
        .generated_slots
        .keys()
        .copied()
        .filter(|&slot| vm.symbol_names[slot] == name)
        .collect();
    assert_eq!(found.len(), 1, "exactly one uninterned {name}: {found:?}");
    Value::reference(found[0] + 1, 7)
}

/// The printed value, or the failure as text, so a missing capability fails
/// at the assertion that names it rather than at an unwrap.
fn show(vm: &mut Vm, form: &str) -> String {
    match vm.eval_source_raw(form) {
        Ok(v) => vm.display(v).unwrap(),
        Err(e) => format!("ERR {e}"),
    }
}

// Garbage first, so compacting it away must MOVE everything allocated later.
const JUNK: &str = "(setq plist-gc-junk (let ((i 0) (g nil)) (while (< i 3000) (setq g (cons (make-string 8 ?j) g)) (setq i (1+ i))) g))\n";

#[test]
fn an_uninterned_symbol_referenced_only_by_its_own_plist_keeps_it_across_compaction() {
    let mut vm = Vm::new();
    let defined = load(
        &mut vm,
        "orphan.el",
        &format!(
            "{JUNK}(let ((s (make-symbol \"plist-gc-orphan\")))\n\
               (put s 'self s)\n\
               (put s 'payload (list (make-string 4000 ?z) (vector 'plist-gc-a 7 \"vv\"))))\n\
             (setq plist-gc-junk nil)\n"
        ),
    );
    assert_eq!(defined.map(|_| ()), Ok(()), "put on an uninterned symbol must succeed");
    vm.collect(&mut []);
    let dropped = vm.heap_stats();
    // Nothing in Lisp names the symbol; only its own plist does (self).
    let s = uninterned(&vm, "plist-gc-orphan");
    vm.declare_special_raw("plist-gc-probe", s);
    assert_eq!(
        show(&mut vm, "(let ((p (get plist-gc-probe 'payload))) (list (length (car p)) (aref (car p) 3999) (car (cdr p)) (eq (get plist-gc-probe 'self) plist-gc-probe)))"),
        "(4000 122 [plist-gc-a 7 \"vv\"] t)",
        "payload intact after the junk before it was compacted away"
    );
    // Live objects AND bytes: the plist (2 pairs = 4 conses), the payload
    // list (2 conses), its string and vector are exactly what dropping the
    // plist releases. A plist the collector failed to mark would already be
    // gone (and the reads above would be garbage); one it failed to forward
    // would read the junk's old slots.
    assert_eq!(show(&mut vm, "(progn (setplist plist-gc-probe nil) nil)"), "nil");
    vm.collect(&mut []);
    vm.collect(&mut []);
    let released = vm.heap_stats();
    let objects = dropped.live_objects - released.live_objects;
    let bytes = dropped.live_bytes - released.live_bytes;
    eprintln!("orphan plist: retained {} objects / {} bytes; released {objects} objects / {bytes} bytes", dropped.live_objects, dropped.live_bytes);
    assert_eq!(objects, 9, "plist (4 conses) + payload list (2) + its 2 strings + vector were live until setplist");
    assert!(bytes >= 4000, "the 4000-byte payload string was live until setplist: {bytes} bytes released");
    // And nothing else: the heap is back at a fresh VM's collected baseline.
    let mut fresh = Vm::new();
    fresh.collect(&mut []);
    let base = fresh.heap_stats();
    assert_eq!((released.live_objects, released.live_bytes), (base.live_objects, base.live_bytes), "dead == baseline");
}

#[test]
fn a_plist_value_reachable_only_through_a_lazily_held_body_survives_compaction() {
    let mut vm = Vm::new();
    let defined = load(
        &mut vm,
        "lazy.el",
        &format!(
            "{JUNK}(let ((s (make-symbol \"plist-gc-lazy-owner\")))\n\
               (put s 'payload (list (make-string 3000 ?q) (vector 7 8 9)))\n\
               (eval (list 'defalias ''plist-gc-reader (list 'function (list 'lambda nil (list 'get (list 'quote s) ''payload))))))\n\
             (setq plist-gc-junk nil)\n"
        ),
    );
    assert_eq!(defined.map(|_| ()), Ok(()), "put on an uninterned symbol must succeed");
    let slot = vm.names["plist-gc-reader"];
    let closure = vm.slots[slot].function.index_if(4).expect("a closure");
    let function = vm.closures[closure].function;
    assert!(vm.functions[function].lazy.is_some(), "defined in the non-eager regime: body held lazily");
    let before = vm.heap_stats();
    vm.collect(&mut []);
    let after = vm.heap_stats();
    eprintln!("lazy: {} -> {} objects, {} -> {} bytes", before.live_objects, after.live_objects, before.live_bytes, after.live_bytes);
    assert!(after.live_objects < before.live_objects, "the junk was reclaimed, so later objects moved");
    assert!(vm.functions[function].lazy.is_some(), "still lazy: nothing has called it");
    assert_eq!(
        show(&mut vm, "(let ((p (plist-gc-reader))) (list (length (car p)) (aref (car p) 0) (car (cdr p))))"),
        "(3000 113 [7 8 9])",
        "the only path to the payload is reader -> lazy body -> uninterned symbol -> plist"
    );
}

#[test]
fn distinct_same_name_symbols_keep_distinct_plists_across_compaction() {
    let mut vm = Vm::new();
    let defined = load(
        &mut vm,
        "twins.el",
        &format!(
            "{JUNK}(setq plist-gc-twins (list (make-symbol \"twin\") (make-symbol \"twin\") 'twin))\n\
             (put (nth 0 plist-gc-twins) 'v (make-string 100 ?a))\n\
             (put (nth 1 plist-gc-twins) 'v (make-string 200 ?b))\n\
             (put (nth 2 plist-gc-twins) 'v (make-string 300 ?c))\n\
             (setq plist-gc-junk nil)\n"
        ),
    );
    assert_eq!(defined.map(|_| ()), Ok(()), "put on an uninterned symbol must succeed");
    vm.collect(&mut []);
    assert_eq!(
        show(&mut vm, "(let ((out nil) (l plist-gc-twins)) (while l (let ((v (get (car l) 'v))) (setq out (cons (list (length v) (aref v 0)) out))) (setq l (cdr l))) (nreverse out))"),
        "((100 97) (200 98) (300 99))"
    );
}

#[test]
fn code_images_publish_each_symbol_objects_plist() {
    let store = std::env::temp_dir().join(format!("vm2-plist-image-{}", std::process::id()));
    let mut source = Vm::new();
    assert_eq!(
        show(&mut source, "(progn (put 'plist-img 'a (list 1 \"one\")) (put 'plist-img 'b 2) (setq plist-img-u (make-symbol \"plist-img\")) (put plist-img-u 'a 'uninterned) nil)"),
        "nil",
        "put on interned and uninterned symbols"
    );
    let entry = source.compile(&Form::Int(0)).unwrap();
    let key = source.publish_code_image(&store, entry, [0; 32]).unwrap();
    let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
    let (mut vm, _) = Vm::from_code_image(image, [0; 32]).unwrap();
    assert_eq!(show(&mut vm, "(symbol-plist 'plist-img)"), "(a (1 \"one\") b 2)", "interned plist survives the image");
    assert_eq!(show(&mut vm, "(list (get plist-img-u 'a) (eq plist-img-u 'plist-img))"), "(uninterned nil)", "uninterned plist survives the image");
    // The mapped plist is immutable; put gives the symbol a private copy,
    // in GNU's order (replace in place, append at the end).
    assert_eq!(show(&mut vm, "(progn (put 'plist-img 'b 3) (put 'plist-img 'c 4) (symbol-plist 'plist-img))"), "(a (1 \"one\") b 3 c 4)");
    vm.collect(&mut []);
    assert_eq!(show(&mut vm, "(symbol-plist 'plist-img)"), "(a (1 \"one\") b 3 c 4)", "the private copy is traced");
    // A fresh world from the same image whose FIRST mutation is an append:
    // the mapped last cell cannot take a new tail, so it is copied too.
    let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
    let (mut appended, _) = Vm::from_code_image(image, [0; 32]).unwrap();
    assert_eq!(show(&mut appended, "(progn (put 'plist-img 'z 9) (symbol-plist 'plist-img))"), "(a (1 \"one\") b 2 z 9)");
    appended.collect(&mut []);
    assert_eq!(show(&mut appended, "(get 'plist-img 'z)"), "9", "the appended private copy is traced");
    std::fs::remove_dir_all(store).unwrap();
}
