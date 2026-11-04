//! A unit's encoding round trip: each form compiled, encoded, decoded
//! into the same VM and run gives what the form gives.
use super::*;

/// FORM's value through a unit, displayed.
fn through_unit(vm: &mut Vm, form: &str) -> String {
    let datum = vm.eval_source_raw(form).unwrap();
    let form = vm.expansion_form(datum, 0).unwrap();
    let mut names = unit::Names::default();
    let bytes = vm.compile_unit(&form, NameSet::default(), &mut names).unwrap();
    let slots = Arc::new(names.names.iter().map(|n| vm.slot(n)).collect::<Vec<usize>>());
    let bytes = Arc::new(bytes);
    let mut reader = codec::Reader::new(&bytes);
    let entry = unit::Install { bytes: &bytes, slots: &slots }.unit(vm, &mut reader).unwrap();
    assert!(reader.done());
    let value = vm.execute_raw(entry).unwrap();
    vm.release_request_entry(entry);
    vm.display(value).unwrap()
}

/// The bytes FORM's unit charges as it installs (materializes its
/// constants), its reading and compiling excluded.
fn install_charge(vm: &mut Vm, form: &str) -> u64 {
    let datum = vm.eval_source_raw(form).unwrap();
    let form = vm.expansion_form(datum, 0).unwrap();
    let mut names = unit::Names::default();
    let bytes = vm.compile_unit(&form, NameSet::default(), &mut names).unwrap();
    let slots = Arc::new(names.names.iter().map(|n| vm.slot(n)).collect::<Vec<usize>>());
    let bytes = Arc::new(bytes);
    let mut reader = codec::Reader::new(&bytes);
    let before = vm.free.allocated;
    let entry = unit::Install { bytes: &bytes, slots: &slots }.unit(vm, &mut reader).unwrap();
    let charged = vm.free.allocated - before;
    vm.release_request_entry(entry);
    charged
}

#[test]
fn units_round_trip() {
    let mut vm = Vm::new();
    vm.eval_source("(setq unit-test-fn (let ((k 1)) (lambda (x) (+ x k))))").unwrap();
    for (form, expected) in [
        // A closure over a mutable variable: a captured cell.
        ("'(let ((c (let ((n 0)) (lambda () (setq n (1+ n)))))) (funcall c) (funcall c))", "2"),
        // Equal constants of one function are coalesced, as bytecomp does.
        ("'(let ((a '(x)) (b '(x))) (eq a b))", "t"),
        // Shared and circular structure survives.
        ("'(let ((l '#1=(a . #1#))) (eq l (cdr l)))", "t"),
        ("'(let ((l '(#1=#:g #1#))) (and (eq (car l) (car (cdr l))) (not (eq (car l) (intern \"g\")))))", "t"),
        ("'(get-text-property 1 'face #(\"abc\" 0 3 (face bold)))", "bold"),
        ("'(let ((v [1 2.5 \"s\"])) (and (= (aref v 1) 2.5) (equal (aref v 2) \"s\")))", "t"),
        // A function constant, as an inlined defsubst's frozen copy is.
        ("(list 'funcall (list 'quote unit-test-fn) 41)", "42"),
    ] {
        assert_eq!(through_unit(&mut vm, form), expected, "{form}");
    }
}

/// A closure constant materialized from a unit is charged its captures,
/// which are filled in after the funnel made it empty: a constant closing
/// over nine variables costs eight Values more than one closing over one.
#[test]
fn materialized_closures_charge_their_captures() {
    let mut vm = Vm::new();
    vm.eval_source("(setq unit-one (let ((a 1)) (lambda () a)) unit-nine (let ((a 1) (b 2) (c 3) (d 4) (e 5) (f 6) (g 7) (h 8) (i 9)) (lambda () (list a b c d e f g h i))))").unwrap();
    let one = install_charge(&mut vm, "(list 'functionp (list 'quote unit-one))");
    let nine = install_charge(&mut vm, "(list 'functionp (list 'quote unit-nine))");
    assert_eq!(nine, one + 8 * 8, "nine captures charged {nine}, one {one}");
}

/// A propertized string constant materialized from a unit is charged its
/// property runs, which are put after the funnel made it plain.
#[test]
fn materialized_strings_charge_their_properties() {
    let mut vm = Vm::new();
    let plain = install_charge(&mut vm, "'(length \"abcdef\")");
    let runs = install_charge(&mut vm, "'(length #(\"abcdef\" 0 2 (face bold) 2 4 (face italic) 4 6 (face bold)))");
    assert!(runs > plain + 3 * 8, "runs charged {runs}, plain {plain}");
}

/// A body left pending at install keeps its constants through a
/// collection that moves them, and decodes against the moved ones.
#[test]
fn pending_bodies_survive_a_moving_collection() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    let datum = vm.eval_source_raw("'(lambda () (list '(q \"r\") [x 2.5] (make-symbol \"g\")))").unwrap();
    let form = vm.expansion_form(datum, 0).unwrap();
    let mut names = unit::Names::default();
    let bytes = Arc::new(vm.compile_unit(&form, NameSet::default(), &mut names).unwrap());
    let slots = Arc::new(names.names.iter().map(|n| vm.slot(n)).collect::<Vec<usize>>());
    // Garbage below what the unit allocates, so the collection moves it.
    vm.eval_source("(dotimes (i 2000) (list i (vector i)))").unwrap();
    let entry = unit::Install { bytes: &bytes, slots: &slots }.unit(&mut vm, &mut codec::Reader::new(&bytes)).unwrap();
    let closure = vm.execute_raw(entry).unwrap();
    let child = vm.closures[closure.index(4).unwrap()].function;
    assert!(vm.functions[child].pending.is_some(), "the closure's body is still pending");
    let slot = vm.slot("t-pending-closure");
    vm.slots[slot].value = closure;
    let held = |vm: &Vm| vm.functions[child].pending.as_ref().unwrap().values.clone();
    let before = held(&vm);
    vm.collect(&mut []);
    assert!(vm.functions[child].pending.is_some(), "a collection does not decode it");
    assert_ne!(held(&vm), before, "the collection moved the body's constants");
    assert_eq!(vm.eval_source("(funcall t-pending-closure)").unwrap().display().unwrap(), "((q \"r\") [x 2.5] g)");
    assert!(vm.functions[child].pending.is_none(), "the first call decoded it");
}

/// FORM installed as a unit in VM, its entry not yet run.
fn installed(vm: &mut Vm, form: &str) -> usize {
    let datum = vm.eval_source_raw(form).unwrap();
    let form = vm.expansion_form(datum, 0).unwrap();
    let mut names = unit::Names::default();
    let bytes = Arc::new(vm.compile_unit(&form, NameSet::default(), &mut names).unwrap());
    let slots = Arc::new(names.names.iter().map(|n| vm.slot(n)).collect::<Vec<usize>>());
    unit::Install { bytes: &bytes, slots: &slots }.unit(vm, &mut codec::Reader::new(&bytes)).unwrap()
}

/// An entry released before its first call (its run failed first) drops
/// its pending body: a moving collection then has nothing stale to rewrite.
#[test]
fn an_entry_released_before_its_first_call_drops_its_body() {
    let mut vm = Vm::new();
    vm.eval_source("(dotimes (i 2000) (list i (vector i)))").unwrap();
    let entry = installed(&mut vm, "'(list '(q \"r\") [x 2.5] (car '(1)))");
    assert!(vm.functions[entry].pending.is_some());
    vm.release_request_entry(entry);
    assert!(vm.functions[entry].pending.is_none());
    vm.eval_source("(dotimes (i 2000) (list i (vector i)))").unwrap();
    vm.collect(&mut []);
}

/// A code image published after a compiled load holds the bodies still
/// pending decoded: a restored world can call them.
#[test]
fn code_images_publish_pending_bodies_decoded() {
    let store = std::env::temp_dir().join(format!("vm2-pending-image-{}", std::process::id()));
    let mut source = Vm::new();
    let entry = installed(&mut source, "'(defalias 't-pending-img (lambda (n) (list n \"x\" 'sym)))");
    source.execute_raw(entry).unwrap();
    source.release_request_entry(entry);
    let slot = source.slot("t-pending-img");
    let id = source.closures[source.slots[slot].function.index(4).unwrap()].function;
    assert!(source.functions[id].pending.is_some(), "the defined function was never called");
    let image_entry = source.compile(&Form::Int(0)).unwrap();
    let key = source.publish_code_image(&store, image_entry, [0; 32]).unwrap();
    let image = unsafe { crate::ImageRegion::open(&store, &key) }.unwrap();
    let (mut vm, _) = Vm::from_code_image(image, [0; 32]).unwrap();
    assert_eq!(vm.eval_source("(t-pending-img 7)").unwrap().display().unwrap(), "(7 \"x\" sym)");
    std::fs::remove_dir_all(store).unwrap();
}

#[test]
fn damaged_artifacts_are_deleted_on_lookup() {
    let base = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"), std::path::PathBuf::from);
    let store = base.join(format!("compiled-lisp-unit-{}", std::process::id()));
    std::fs::create_dir_all(&store).unwrap();
    let key = [7; 32];
    let (bytes, inputs) = store::encode(&store::Status::Compiled, Path::new("/x.el"), &[], &[], &[]);
    let path = store::publish(&store, &key, &inputs, &bytes).unwrap();
    assert!(store::lookup(&store, &key, |_| true).is_some_and(|(found, _)| found == path));
    let mut damaged = bytes.clone();
    *damaged.last_mut().unwrap() ^= 1;
    std::fs::write(&path, damaged).unwrap();
    assert!(store::lookup(&store, &key, |_| true).is_none());
    assert!(!path.exists());
    std::fs::remove_dir_all(&store).unwrap();
}
