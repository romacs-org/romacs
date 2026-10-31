//! Collector proofs for forwarded (built-in) variable storage at the level the
//! public API cannot see: the raw moving-heap index held in `Forwarded`.
use super::*;

#[test]
fn clear_one_preserves_visit_metadata_and_traces_remaining_slots() {
    let mut vm = Vm::new();
    let setup = "(setq buffer-file-name (copy-sequence \"visit\") buffer-file-format (list (copy-sequence \"format\")) fill-column 42) (kill-local-variable 'fill-column) nil";
    vm.eval_source_raw(setup).unwrap();
    let index = vm.buffer_index().unwrap();
    let stamp = VisitedModtime::Known { seconds: 17, nanos: 23, size: Some(71) };
    let locals = vm.per_buffer_mut(index).unwrap();
    locals.1 = stamp;
    locals.2 = Some((29, 31));
    locals.3 = Some("/visited/truename".into());
    vm.collect(&mut []);
    let baseline = vm.heap_stats();
    for _ in 0..32 {
        vm.eval_source_raw(setup).unwrap();
        vm.collect(&mut []);
        let index = vm.buffer_index().unwrap();
        let locals = vm.per_buffer(index).unwrap();
        assert_eq!((locals.1, locals.2, locals.3.as_deref()), (stamp, Some((29, 31)), Some(&b"/visited/truename"[..])), "clear-one must preserve every visit field");
        let read = vm.eval_source_raw("(and (equal buffer-file-name \"visit\") (equal buffer-file-format '(\"format\")) (= fill-column 70) (not (local-variable-p 'fill-column)))").unwrap();
        assert_eq!(read, Value::TRUE, "surviving per-buffer values must be rewritten by moving GC");
        vm.collect(&mut []);
    }
    let now = vm.heap_stats();
    eprintln!("surviving per-buffer retention: {} objects / {} bytes (baseline {} / {})", now.live_objects, now.live_bytes, baseline.live_objects, baseline.live_bytes);
    assert_eq!((now.live_objects, now.live_bytes), (baseline.live_objects, baseline.live_bytes));
}


fn garbage(vm: &mut Vm) {
    vm.eval_source_raw("(let ((i 0) (g nil)) (while (< i 2000) (setq g (cons i g)) (setq i (1+ i))) nil)").unwrap();
}

fn fwd(vm: &mut Vm, name: &str) -> Fwd {
    let slot = vm.slot(name);
    vm.forwarding(slot).expect("a built-in variable")
}

/// The Value stored for minibuffer-local-map is an index into the cons
/// arena. Garbage allocated before it is compacted away by a forced
/// collection, so the index MUST change, and the moved value must read back
/// identically through Lisp and through the raw storage.
#[test]
fn forwarded_object_index_moves_and_reads_back() {
    let mut vm = Vm::new();
    garbage(&mut vm);
    vm.eval_source_raw("(setq minibuffer-local-map (list 'moved (copy-sequence \"qqq\") 42))").unwrap();
    let f = fwd(&mut vm, "minibuffer-local-map");
    let before = vm.forwarded.object(f);
    let shown_before = vm.display(before).unwrap();
    let collections = vm.heap_stats().collections;
    vm.collect(&mut []);
    assert!(vm.heap_stats().collections > collections);
    let after = vm.forwarded.object(f);
    eprintln!("minibuffer-local-map: {before:?} -> {after:?} ({shown_before})");
    assert_ne!(before, after, "the fixture must move the forwarded value");
    assert_eq!(vm.display(after).unwrap(), shown_before);
    let read = vm.eval_source_raw("minibuffer-local-map").unwrap();
    assert_eq!(read, after, "Lisp reads the forwarded storage itself");
    assert_eq!(shown_before, "(moved \"qqq\" 42)");
}

/// The per-buffer default (GNU `buffer_defaults`) and a per-buffer value
/// held by a live buffer object are traced with the default storage and the
/// buffer respectively, and both move.
#[test]
fn per_buffer_default_and_buffer_value_indices_move() {
    let mut vm = Vm::new();
    garbage(&mut vm);
    vm.eval_source_raw("(set-default 'fill-column (list 'dflt (copy-sequence \"dd\")))").unwrap();
    let f = fwd(&mut vm, "fill-column");
    let Fwd::Buffer(i) = f else { panic!("fill-column is per-buffer") };
    // A per-buffer value can only be a non-integer through the raw store
    // (set applies the integerp predicate); install one directly in the
    // current buffer to prove the buffer traces it.
    let buffer = vm.buffer_index().unwrap();
    let local = vm.eval_source_raw("(list 'local (copy-sequence \"ll\"))").unwrap();
    vm.per_buffer_mut(buffer).unwrap().set(i, local);
    let default_before = vm.forwarded.object(f);
    vm.collect(&mut []);
    let buffer = vm.buffer_index().unwrap();
    let default_after = vm.forwarded.object(f);
    let local_after = vm.per_buffer(buffer).unwrap().get(i).unwrap();
    eprintln!("default {default_before:?} -> {default_after:?}; local {local:?} -> {local_after:?}");
    assert_ne!(default_before, default_after);
    assert_ne!(local, local_after);
    assert_eq!(vm.display(default_after).unwrap(), "(dflt \"dd\")");
    assert_eq!(vm.display(local_after).unwrap(), "(local \"ll\")");
    let seen = vm.eval_source_raw("(list fill-column (default-value 'fill-column))").unwrap();
    assert_eq!(vm.display(seen).unwrap(), "((local \"ll\") (dflt \"dd\"))");
}

/// An initial value needing the heap is materialised on first access, so a
/// bare VM allocates nothing (several collector tests depend on that) and the
/// first read always returns the same fresh object afterwards.
#[test]
fn heap_initial_values_are_materialised_lazily_once() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_objects(), 0);
    let a = vm.eval_source_raw("minibuffer-local-map").unwrap();
    assert!(vm.heap_objects() > 0);
    let b = vm.eval_source_raw("minibuffer-local-map").unwrap();
    assert_eq!(a, b);
    assert_eq!(vm.display(a).unwrap(), "(keymap)");
}

#[test]
fn system_name_is_materialised_lazily_once() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_objects(), 0, "system-name must not allocate at startup");
    let f = fwd(&mut vm, "system-name");
    assert_eq!(vm.forwarded.object(f), Value::UNBOUND);
    let first = vm.eval_source_raw("system-name").unwrap();
    assert_eq!(vm.heap_objects(), 1);
    assert_eq!(vm.forwarded.object(f), first);
    let second = vm.eval_source_raw("system-name").unwrap();
    assert_eq!(first, second);
    assert_eq!(vm.heap_objects(), 1, "repeated access must reuse the string");
    vm.collect(&mut []);
    let retained = vm.eval_source_raw("system-name").unwrap();
    assert_eq!(vm.forwarded.object(f), retained);
    assert_eq!(vm.heap_objects(), 1);
}

/// An uninterned symbol named like a built-in variable is an ordinary
/// variable, as in GNU (forwarding belongs to the interned symbol).
#[test]
fn uninterned_namesake_is_not_forwarded() {
    let mut vm = Vm::new();
    let v = vm.eval_source_raw("(let ((s (make-symbol \"undo-limit\"))) (set s 'x) (list (symbol-value s) undo-limit))").unwrap();
    assert_eq!(vm.display(v).unwrap(), "(x 160000)");
}

/// Every entry cites its GNU DEFVAR as FILE.c:LINE and names a distinct
/// variable; the table is the single source of what VM2 defines.
#[test]
fn table_entries_are_unique_and_cite_gnu() {
    let mut seen = std::collections::HashSet::new();
    for v in C_VARIABLES {
        assert!(seen.insert(v.name), "duplicate {}", v.name);
        let (file, line) = v.gnu.split_once(':').expect("FILE:LINE");
        assert!(file.ends_with(".c") && line.parse::<u32>().is_ok(), "{}: {}", v.name, v.gnu);
    }
    let store = Forwarded::default();
    for v in C_VARIABLES {
        assert!(store.lookup(v.name).is_some());
    }
}

/// Plain C specials cite their DEFVAR line exactly, never overlap the
/// forwarded table, and stay localizable like ordinary variables.
#[test]
fn c_specials_cite_their_defvar_and_are_localizable() {
    let source = std::path::PathBuf::from(std::env::var("ROMACS_GNU_SOURCE").expect("pinned GNU source")).join("src");
    let forwarded: std::collections::HashSet<_> = C_VARIABLES.iter().map(|v| v.name).collect();
    let mut seen = std::collections::HashSet::new();
    for v in C_SPECIALS {
        assert!(seen.insert(v.name) && !forwarded.contains(v.name), "duplicate {}", v.name);
        let (file, line) = v.gnu.split_once(':').expect("FILE:LINE");
        let text = std::fs::read_to_string(source.join(file)).unwrap();
        let at = text.lines().nth(line.parse::<usize>().unwrap() - 1).unwrap_or("");
        assert!(at.contains("DEFVAR_") && at.contains(&format!("\"{}\"", v.name)), "{}: {} is not its DEFVAR", v.name, v.gnu);
    }
    let mut vm = Vm::new();
    let source = "(list (special-variable-p 'show-trailing-whitespace) (with-temp-buffer (set (make-local-variable 'show-trailing-whitespace) t) (local-variable-p 'show-trailing-whitespace)) show-trailing-whitespace)";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(t t nil)");
}

/// These loader DEFVAR_LISPs begin at Qnil in syms_of_lread. The dumped GNU
/// image has run Lisp that filled its history (and `load' rebinds its list), so
/// the transcripts cannot show the C-time value; this pins it, and that both
/// are special and forwarded to object storage.
#[test]
fn load_variables_start_as_gnus_c_nil() {
    let mut vm = Vm::new();
    let v = vm
        .eval_source_raw("(prin1-to-string (condition-case e (list after-load-alist current-load-list load-history eval-buffer-list (special-variable-p 'after-load-alist) (special-variable-p 'current-load-list) (special-variable-p 'load-history) (special-variable-p 'eval-buffer-list)) (error e)))")
        .unwrap();
    assert_eq!(vm.display(v).unwrap(), "\"(nil nil nil nil t t t t)\"");
    for name in ["after-load-alist", "current-load-list", "load-history", "eval-buffer-list"] {
        assert!(matches!(fwd(&mut vm, name), Fwd::Obj(_)), "{name} is a DEFVAR_LISP");
    }
}

/// window-persistent-parameters starts as GNU's list1 (Fcons (Qclone_of, Qt)),
/// with the immediate t, not a symbol named "t"; strings are build_string's:
/// unibyte when ASCII (GNU: (multibyte-string-p emacs-version) => nil).
#[test]
fn list_initial_values_use_gnus_immediates() {
    let mut vm = Vm::new();
    let v = vm.eval_source_raw("(prin1-to-string (list (eq (cdr (car window-persistent-parameters)) t) window-persistent-parameters emacs-version (multibyte-string-p emacs-version) help-char standard-display-table load-suffixes load-file-rep-suffixes (multibyte-string-p (car load-file-rep-suffixes))))").unwrap();
    assert_eq!(vm.display(v).unwrap(), "\"(t ((clone-of . t)) \\\"32.0.50\\\" nil 8 nil (\\\".so\\\" \\\".elc\\\" \\\".el\\\") (\\\"\\\") nil)\"");
}
