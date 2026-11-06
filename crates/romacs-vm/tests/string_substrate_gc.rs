use romacs_vm::{Vm, Fixed};
fn printed(vm: &mut Vm, source: &str) -> String {
    let result = vm.eval_source(&format!("(prin1-to-string (condition-case e {source} (error e)))")).unwrap();
    serde_json::from_str(&result.display().unwrap()).unwrap()
}
// GNU: (nil t nil 3 3 0 0 0), from the live oracle in p1a-extra-gnu.json.
#[test]
fn p1a_clear_identity() {
    assert_eq!(printed(&mut Vm::new(), "(let* ((s (copy-sequence \"aλ\")) (alias s) (r (clear-string s))) (list r (eq s alias) (multibyte-string-p s) (length s) (string-bytes s) (aref s 0) (aref s 1) (aref s 2)))"), "(nil t nil 3 3 0 0 0)");
}
// Live GNU: (t nil nil): raw character strings are multibyte; ASCII constructors are unibyte.
#[test]
fn p1a_string_constructor_kind() {
    assert_eq!(printed(&mut Vm::new(), "(list (multibyte-string-p (string 4194303)) (multibyte-string-p (string 97)) (multibyte-string-p (char-to-string 97)))"), "(t nil nil)");
}
// GNU: t. P1a must not reinterpret or widen existing ASCII literals.
#[test]
fn p1a_raw_string_equal() {
    assert_eq!(printed(&mut Vm::new(), "(let ((s (char-to-string 4194303))) (and (equal s (char-to-string 4194303)) (string-equal s (char-to-string 4194303)) (= (sxhash-equal s) (sxhash-equal (char-to-string 4194303)))))"), "t");
}
// GNU: (t p1a-gc-only 2 6 1114112 4194303). D001 independently permits the aset width change.
const GC_SOURCE: &str = "(let ((p (cons 'p1a-gc-car 'p1a-gc-cdr)) (s (string 1114112 4194303)) (i 0)) (put-text-property 0 2 'p1a-gc-key p s) (put-text-property 0 2 'p1a-gc-only-key (cons 'p1a-gc-only nil) s) (while (< i 10000) (cons i i) (setq i (1+ i))) (list s p))";
#[test]
fn p1a_gc_raw_properties_and_retention() {
    let mut vm = Vm::new(); vm.set_collection_floor(1);
    // Warm compilation and interning on the SAME fixed source before measuring.
    let root = vm.eval_source(GC_SOURCE).ok().map(|v| v.root());
    vm.collect_garbage();
    let check = "(list (eq (get-text-property 0 'p1a-gc-key (car p1a-gc-result)) (car (cdr p1a-gc-result))) (car (get-text-property 0 'p1a-gc-only-key (car p1a-gc-result))) (length (car p1a-gc-result)) (string-bytes (car p1a-gc-result)) (aref (car p1a-gc-result) 0) (aref (car p1a-gc-result) 1))";
    match &root { Some(r) => vm.declare_special_rooted("p1a-gc-result",r).unwrap(), None => vm.declare_special("p1a-gc-result",Fixed::NIL) }
    drop(root);
    assert_eq!(printed(&mut vm,check), "(t p1a-gc-only 2 6 1114112 4194303)");
    vm.declare_special("p1a-gc-result",Fixed::NIL); vm.collect_garbage();
    // Warm the fixed check's own interned constants too.
    let before=vm.heap_stats();
    for _ in 0..3 {
        let result=vm.eval_source(GC_SOURCE).unwrap().root(); vm.collect_garbage();
        vm.declare_special_rooted("p1a-gc-result",&result).unwrap(); drop(result);
        assert_eq!(printed(&mut vm,check), "(t p1a-gc-only 2 6 1114112 4194303)");
        vm.declare_special("p1a-gc-result",Fixed::NIL); vm.collect_garbage();
    }
    let after=vm.heap_stats();
    assert!(after.collections>before.collections,"retention cycles must actually collect");
    eprintln!("raw GC retention control: objects {} -> {}, bytes {} -> {}, collections {} -> {}, allocations {} -> {}",before.live_objects,after.live_objects,before.live_bytes,after.live_bytes,before.collections,after.collections,before.allocations,after.allocations);
    assert_eq!(before.live_objects,after.live_objects,"raw string retention objects");
    assert_eq!(before.live_bytes,after.live_bytes,"raw string retention bytes");
}
#[test]
fn p1a_aset_reclassifies_slot_between_allocations() {
    let mut vm=Vm::new();vm.set_collection_floor(1);
    // GNU refuses width changes (D001); VM2 permits all GNU character codes.
    assert_eq!(printed(&mut vm,"(let* ((s (string-to-multibyte \"ab\")) (alias s) (p (cons 'p1a-aset-left 'p1a-aset-right)) (i 0)) (put-text-property 0 1 'p1a-aset-prop p s) (aset s 0 4194303) (while (< i 10000) (cons i i) (setq i (1+ i))) (list (eq s alias) (eq p (get-text-property 0 'p1a-aset-prop s)) (aref s 0) (string-bytes s)))"), "(t t 4194303 3)");
}

#[test]
fn p1a_aset_utf8_slot_invariant_both_directions() {
    let mut vm=Vm::new(); vm.set_collection_floor(1);
    // GNU refuses this write; D001 deliberately accepts it. Unicode-only
    // display must now refuse normally, not panic by treating raw bytes as &str.
    let raw=vm.eval_source("(condition-case nil (let ((s (string-to-multibyte \"a\"))) (aset s 0 4194303) s) (error nil))").unwrap();
    assert!(raw.display().is_err(), "raw mutation must reclassify the String slot");
    let raw=raw.root();
    vm.declare_special_rooted("p1a-slot-invariant",&raw).unwrap();
    let text=vm.eval_source("(progn (aset p1a-slot-invariant 0 97) p1a-slot-invariant)").unwrap().root();
    vm.collect_garbage();
    assert_eq!(text.get(&vm).unwrap().display().unwrap(), "\"a\"", "ASCII mutation must reclassify the RawString slot");
}
#[test]
fn p1a_mapped_fill_refuses_without_panicking() {
    use romacs_vm::{ImageBuilder, ImageRegion};
    let dir=std::env::temp_dir().join(format!("p1a-image-fill-{}",std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut builder=ImageBuilder::default(); let value=builder.string("a");
    let key=builder.publish(&dir,[71;32]).unwrap();
    // SAFETY: this test never overwrites the immutable published inode.
    let image=unsafe { ImageRegion::open(&dir,&key) }.unwrap();
    let mut vm=Vm::new();vm.attach_image(image).unwrap();vm.declare_special("p1a-image-string",value);
    // Image strings refuse writes with GNU's read-only error (DIVERGENCES.md VM2-FROZEN).
    assert_eq!(printed(&mut vm,"(fillarray p1a-image-string 97)"),"(error \"Attempt to modify read-only object\" \"a\")");
    std::fs::remove_dir_all(&dir).unwrap();
}

// GNU: 777 for both owners. A property value reachable only through the string
// must survive a counted collection whether the owner is a Unicode String
// (233) or a RawString (4194303), and live objects/bytes must stay flat.
#[test]
fn p1a_gc_property_only_values_survive_for_both_string_owners() {
    for code in [233, 4194303] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let source = format!("(let ((s (make-string 2 {code}))) (put-text-property 0 2 'p1a-only (cons 777 nil) s) s)");
        let mut baseline = None;
        for _ in 0..4 {
            let collections = vm.heap_stats().collections;
            let root = vm.eval_source(&source).unwrap().root();
            vm.collect_garbage();
            assert!(vm.heap_stats().collections > collections, "collection must run");
            vm.declare_special_rooted("p1a-only-root", &root).unwrap();
            drop(root);
            assert_eq!(printed(&mut vm, "(car (get-text-property 0 'p1a-only p1a-only-root))"), "777", "owner {code}");
            vm.declare_special("p1a-only-root", Fixed::NIL);
            vm.collect_garbage();
            let stats = vm.heap_stats();
            let live = (stats.live_objects, stats.live_bytes);
            match baseline { None => baseline = Some(live), Some(b) => assert_eq!(b, live, "owner {code} retention") }
        }
    }
}
