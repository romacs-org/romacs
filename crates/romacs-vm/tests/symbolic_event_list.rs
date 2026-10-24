use romacs_vm::Vm;

fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    let result = vm.eval_source(source).and_then(|value| value.display());
    assert_eq!(result.as_deref(), Ok(expected), "{source}");
}

// READER-ORACLES/bindings.symbolic-event-list: (:ok (delete-char C-kp-delete)).
// Identity assertions avoid a dependency on key-description / boolean vectors.
#[test]
fn symbolic_event_list_banked_keypad_binding() {
    check(
        "(let ((m (make-sparse-keymap)))
             (define-key m [(kp-delete)] 'delete-char)
             (list (lookup-key m [kp-delete])
                   (eq (car (car (cdr m))) 'kp-delete)))",
        "(delete-char t)",
    );
    check(
        "(let ((m (make-sparse-keymap)))
             (define-key m [(control kp-delete)] 'delete-char)
             (list (lookup-key m [C-kp-delete])
                   (eq (car (car (cdr m))) 'C-kp-delete)))",
        "(delete-char t)",
    );
}

// GNU 45b0f7699df36b3849edc3bdef37d4cb9e75b9ea, keyboard.c:
// event-convert-list, parse_solitary_modifier, apply_modifiers_uncached;
// keymap.c: store_in_keymap/access_keymap reorder symbolic modifiers.
#[test]
fn symbolic_event_list_all_bindings_keypad_modifiers() {
    for (description, canonical) in [
        ("kp-delete", "kp-delete"),
        ("control kp-delete", "C-kp-delete"),
        ("meta kp-delete", "M-kp-delete"),
        ("control meta kp-delete", "C-M-kp-delete"),
        ("shift kp-delete", "S-kp-delete"),
        ("control shift kp-delete", "C-S-kp-delete"),
        ("meta shift kp-delete", "M-S-kp-delete"),
        ("control meta shift kp-delete", "C-M-S-kp-delete"),
        ("M C S kp-delete", "C-M-S-kp-delete"),
        ("ctrl control C kp-delete", "C-kp-delete"),
        ("s S M H C A kp-delete", "A-C-H-M-S-s-kp-delete"),
        (
            "super shift meta hyper control alt kp-delete",
            "A-C-H-M-S-s-kp-delete",
        ),
        ("control M-kp-delete", "C-M-kp-delete"),
        ("click double down mouse-1", "double-down-mouse-1"),
        ("up triple drag mouse-1", "triple-up-drag-mouse-1"),
    ] {
        check(
            &format!(
                "(let ((m (make-sparse-keymap)))
                 (define-key m [({description})] 'bound)
                 (list (lookup-key m [{canonical}])
                       (lookup-key m [({description})])
                       (eq (car (car (cdr m))) '{canonical})))"
            ),
            "(bound bound t)",
        );
        check(
            &format!(
                "(let ((m (make-sparse-keymap)))
                 (define-key m [{canonical}] 'bound)
                 (lookup-key m [({description})]))"
            ),
            "bound",
        );
    }
}

#[test]
fn symbolic_event_list_character_bases_and_modifier_order() {
    for (description, canonical) in [
        ("a", 97),
        ("control a", 1),
        ("shift a", 65),
        ("shift control a", 33554433),
        ("control shift a", 33554433),
        ("control A", 33554433),
        ("control 63", 67108927),
        ("control 32", 67108896),
        ("control 1", 1),
        ("control 945", 67109809),
    ] {
        check(
            &format!(
                "(let ((m (make-sparse-keymap)))
                 (define-key m [({description})] 'bound)
                 (list (lookup-key m [{canonical}])
                       (eq (car (car (cdr m))) {canonical})))"
            ),
            "(bound t)",
        );
    }
}

#[test]
fn symbolic_event_list_canonicalizes_prefixed_bases_and_lookup() {
    check(
        "(let ((m (make-sparse-keymap)))
             (define-key m [(S C M-kp-delete)] 'bound)
             (list (eq (car (car (cdr m))) 'C-M-S-kp-delete)
                   (lookup-key m [S-M-C-kp-delete])
                   (lookup-key m [(meta shift C-kp-delete)])))",
        "(t bound bound)",
    );
}
