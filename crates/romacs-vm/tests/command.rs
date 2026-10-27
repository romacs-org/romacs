use romacs_vm::{CommandEvent, Vm};
fn text(vm: &mut Vm, source: &str) -> String {
    let v = vm.eval_source(source).unwrap();
    v.display().unwrap()
}
#[test]
fn command_primitives_unicode_counts_bounds_and_lines() {
    let mut vm = Vm::new();
    for (cmd, want, buffer) in [
        ("(forward-char 2)", "3", "αβ\\nγδ"),
        ("(backward-char -2)", "3", "αβ\\nγδ"),
        ("(end-of-line)", "3", "αβ\\nγδ"),
        ("(beginning-of-line 2)", "4", "αβ\\nγδ"),
        ("(delete-char 1)", "1", "β\\nγδ"),
        (
            "(progn (goto-char 2) (delete-backward-char 1))",
            "1",
            "β\\nγδ",
        ),
        ("(self-insert-command 2 955)", "3", "λλαβ\\nγδ"),
    ] {
        vm.eval_source("(erase-buffer) (insert \"αβ\\nγδ\") (goto-char 1)")
            .unwrap();
        assert_eq!(
            text(
                &mut vm,
                &format!("(condition-case e (progn {cmd} (point)) (error 'missing))")
            ),
            want,
            "{cmd}"
        );
        assert_eq!(
            text(&mut vm, "(buffer-string)"),
            format!("\"{buffer}\""),
            "{cmd} changes text"
        );
    }
    assert_eq!(
        text(
            &mut vm,
            "(goto-char (point-max)) (condition-case e (forward-char) (end-of-buffer (car e)))"
        ),
        "end-of-buffer"
    );
    assert_eq!(
        text(
            &mut vm,
            "(goto-char 1) (condition-case e (delete-char 100) (end-of-buffer (list (point) (buffer-string))))"
        ),
        "(1 \"λλαβ\\nγδ\")"
    );
}
#[test]
fn interactive_declarations_supply_arguments() {
    let mut vm = Vm::new();
    assert_eq!(
        text(
            &mut vm,
            "(condition-case e (progn (defun sample (n raw ignored) (interactive \"p\\nP\\ni\") (list n raw ignored)) (setq current-prefix-arg '(3)) (call-interactively 'sample)) (error 'missing))"
        ),
        "(3 (3) nil)"
    );
    assert_eq!(
        text(
            &mut vm,
            "(defun empty () (interactive) 71) (call-interactively 'empty)"
        ),
        "71"
    );
}
#[test]
fn owned_events_keymaps_unbound_signal_and_recovery() {
    let mut vm = Vm::new();
    let out = vm
        .command_event(CommandEvent::Character('λ' as i64))
        .unwrap();
    assert_eq!(out.command.as_deref(), Some("self-insert-command"));
    assert!(out.redisplay);
    assert_eq!(vm.display_snapshot(640, 480).unwrap().text.utf8, "λ");
    let out = vm
        .command_event(CommandEvent::Named("right".into()))
        .unwrap();
    assert_eq!(out.signal.unwrap().condition, "end-of-buffer");
    assert!(out.redisplay);
    let out = vm
        .command_event(CommandEvent::Named("unbound-test".into()))
        .unwrap();
    assert!(out.signal.is_none());
    assert!(out.bell);
    assert!(out.message.unwrap().contains("undefined"));
    assert!(out.redisplay);
    vm.eval_source("(global-set-key [left] 'beginning-of-line)")
        .unwrap();
    vm.command_event(CommandEvent::Named("left".into()))
        .unwrap();
    vm.command_event(CommandEvent::Character('Ω' as i64))
        .unwrap();
    assert_eq!(vm.display_snapshot(640, 480).unwrap().text.utf8, "Ωλ");
    assert_eq!(
        text(
            &mut vm,
            "(list last-command-event this-command last-command (this-command-keys))"
        ),
        "(937 self-insert-command self-insert-command [937])"
    );
}
#[test]
fn explicit_nil_unbinds_default_character_range() {
    let mut vm = Vm::new();
    vm.eval_source("(global-set-key [955] nil)").unwrap();
    let out = vm.command_event(CommandEvent::Character(955)).unwrap();
    assert!(
        out.command.is_none(),
        "explicit nil must beat default range"
    );
    assert!(out.bell);
    assert_eq!(vm.display_snapshot(640, 480).unwrap().text.utf8, "");
}
#[test]
fn unicode_default_lookup_and_event_diversity_do_not_retain_a_binding_per_event() {
    let mut stats = Vec::new();
    for n in [100, 200, 400] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.eval_source("(make-sparse-keymap)").unwrap();
        assert_eq!(text(&mut vm, "(key-binding [955])"), "self-insert-command");
        for c in 1000..1000 + n {
            vm.command_event(CommandEvent::Character(c)).unwrap();
            vm.eval_source("(erase-buffer)").unwrap();
            vm.collect_garbage();
        }
        vm.collect_garbage();
        let s = vm.heap_stats();
        println!(
            "event-diversity n={n} collections={} objects={} bytes={}",
            s.collections, s.live_objects, s.live_bytes
        );
        assert!(s.collections >= n as u64);
        stats.push((s.live_objects, s.live_bytes));
    }
    assert_eq!(stats[0], stats[1]);
    assert_eq!(stats[1], stats[2]);
}
#[test]
fn command_gc_reclaims_objects_and_bytes_at_three_sizes() {
    let mut stats = Vec::new();
    for n in [100, 200, 400] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.eval_source("(let ((payload (copy-sequence \"captured payload\"))) (defun allocating () (interactive) (setq seen (aref (vector payload) 0)))) (global-set-key [allocating] 'allocating)").unwrap();
        for _ in 0..n {
            let out = vm
                .command_event(CommandEvent::Named("allocating".into()))
                .unwrap();
            assert!(out.redisplay);
            assert!(out.signal.is_none(), "{out:?}");
            vm.collect_garbage();
        }
        assert_eq!(text(&mut vm, "seen"), "\"captured payload\"");
        vm.collect_garbage();
        let s = vm.heap_stats();
        println!(
            "captured-command n={n} collections={} objects={} bytes={}",
            s.collections, s.live_objects, s.live_bytes
        );
        assert!(s.collections >= n, "counted collections");
        stats.push((s.live_objects, s.live_bytes));
    }
    assert_eq!(
        stats[0], stats[1],
        "no proportional retained objects or container bytes"
    );
    assert_eq!(stats[1], stats[2]);
}

#[test]
fn fresh_interactive_prefix_is_nil_and_dynamically_special_before_keymaps() {
    let mut dynamic = Vm::new();
    assert_eq!(text(&mut dynamic, "(defun first-args (n raw ignored) (interactive \"p\\nP\\ni\") (list n raw ignored)) (let ((current-prefix-arg '(3))) (equal (call-interactively 'first-args) '(3 (3) nil)))"), "t");
    for earlier in ["", "(ignore current-prefix-arg)", "(setq current-prefix-arg '(8))"] {
        let mut vm = Vm::new();
        // Referencing an unbound variable is deliberate: it interns the slot before
        // the compiler reaches a command caller, without bootstrapping a keymap.
        if !earlier.is_empty() { let _ = vm.eval_source(earlier); }
        let expected = if earlier.contains("setq") { "(8 (8) nil)" } else { "(1 nil nil)" };
        assert_eq!(text(&mut vm, &format!(
            "(defun fresh-args (n raw ignored) (interactive \"p\\nP\\ni\") (list n raw ignored)) (equal (call-interactively 'fresh-args) '{expected})")),
            "t", "earlier={earlier}");
        assert_eq!(text(&mut vm,
            "(let ((current-prefix-arg '(3))) (equal (call-interactively 'fresh-args) '(3 (3) nil)))"), "t", "dynamic earlier={earlier}");
    }
}
#[test]
fn recent_keys_record_events_and_their_commands() {
    let mut vm = Vm::new();
    assert_eq!(text(&mut vm, "(list (recent-keys) (recent-keys t))"), "([] [])");
    vm.command_event(CommandEvent::Character('a' as i64)).unwrap();
    vm.command_event(CommandEvent::Named("unbound-test".into())).unwrap();
    vm.command_event(CommandEvent::Named("right".into())).unwrap();
    assert_eq!(
        text(&mut vm, "(list (recent-keys) (recent-keys t))"),
        "([97 unbound-test right] [97 (nil . self-insert-command) unbound-test (nil) right (nil . forward-char)])"
    );
    // keyboard.c keeps lossage_limit (300) entries, oldest dropped first.
    for _ in 0..200 {
        vm.command_event(CommandEvent::Character('b' as i64)).unwrap();
    }
    assert_eq!(text(&mut vm, "(list (length (recent-keys)) (length (recent-keys t)) (aref (recent-keys t) 0))"), "(150 300 98)");
    // A host that resolves a prefix sequence records its keys; the transport
    // event that runs the sequence's command is not an input key.
    vm.record_recent_event(&CommandEvent::Character(24));
    vm.record_recent_event(&CommandEvent::Character(6));
    vm.eval_source("(define-key (current-global-map) [romacs-gui-sequence] 'ignore)").unwrap();
    vm.command_event(CommandEvent::Named("romacs-gui-sequence".into())).unwrap();
    assert_eq!(text(&mut vm, "(let ((k (recent-keys t))) (list (aref k (- (length k) 3)) (aref k (- (length k) 2)) (aref k (- (length k) 1))))"), "(24 6 (nil . ignore))");
}
