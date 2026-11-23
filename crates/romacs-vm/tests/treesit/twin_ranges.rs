use super::{SYSTEM_SETUP, Vm, deferred_frame, first_parse_vm, gnu_lisp, wait_landed};

/// Parsers of one buffer that see the same text with the same grammar parse
/// an edit once: one due a reparse takes the tree of another that has parsed
/// the text since (VM2-TREESIT-TWIN), in whichever order they are asked,
/// edits between them included. Each still runs its own notifiers, with what
/// changed since its own last tree, and a parser of other ranges parses for
/// itself.
#[test]
fn twin_parsers_parse_an_edit_once() {
    let mut vm = gnu_lisp(&[]);
    vm.eval_source("(require 'treesit)").unwrap();
    let setup = format!("(progn {SYSTEM_SETUP} \
        (set-buffer (get-buffer-create \"twin\")) (insert \"int a;\\nint b = ;\\nint c;\\n\") \
        (setq twin-a (treesit-parser-create 'c) twin-b (treesit-parser-create 'c nil nil 'twin) seen-a nil seen-b nil) \
        (defalias 'twin-seen-a (lambda (ranges _) (push ranges seen-a))) \
        (defalias 'twin-seen-b (lambda (ranges _) (push ranges seen-b))) \
        (treesit-parser-add-notifier twin-a 'twin-seen-a) (treesit-parser-add-notifier twin-b 'twin-seen-b) \
        (defalias 'twin-string (lambda (p) (treesit-node-string (treesit-parser-root-node p)))) \
        (defalias 'twin-fresh (lambda (&optional ranges) \
          (let ((p (treesit-parser-create 'c nil t 'fresh))) \
            (when ranges (treesit-parser-set-included-ranges p ranges)) \
            (prog1 (twin-string p) (treesit-parser-delete p))))) \
        (twin-string twin-a) (twin-string twin-b) (setq seen-a nil seen-b nil))");
    vm.eval_source(&setup).unwrap();
    let check = |vm: &mut Vm, form: &str| vm.eval_source(form).and_then(|v| v.display()).unwrap();
    let compared = romacs_vm::treesit_comparisons_on_this_thread;
    // One edit, the parsers asked in turn: both trees are the text's parse,
    // and each parser is told the same change once.
    let before = compared();
    assert_eq!(check(&mut vm, "(progn (goto-char 1) (insert \"long \") (twin-string twin-a) \
        (list (equal (twin-string twin-b) (twin-string twin-a)) (equal (twin-string twin-b) (twin-fresh)) \
              (length seen-a) (length seen-b) (equal seen-a seen-b)))"), "(t t 1 1 t)");
    assert_eq!(compared() - before, 1, "the two notifiers use one comparison");
    // An edit between them: the first asked parses for itself, the other then
    // takes its tree, and is told what changed since its own last one.
    let before = compared();
    assert_eq!(check(&mut vm, "(progn (setq seen-a nil seen-b nil) (goto-char 14) (insert \"1\") (twin-string twin-a) \
        (goto-char (point-max)) (insert \"int d;\\n\") (twin-string twin-b) \
        (list (equal (twin-string twin-a) (twin-string twin-b)) (equal (twin-string twin-a) (twin-fresh)) \
              (length seen-a) (length seen-b) \
              (and (<= (caar (car seen-b)) 14) (>= (cdar (last (car seen-b))) (- (point-max) 7)) t)))"), "(t t 2 1 t)");
    assert_eq!(compared() - before, 3, "the skipped generation needs its own comparison");
    // A parser of other ranges is no twin.
    let before = compared();
    assert_eq!(check(&mut vm, "(progn (treesit-parser-set-included-ranges twin-b '((1 . 12))) (twin-string twin-b) \
        (goto-char (point-max)) (insert \"int e;\") (twin-string twin-a) \
        (list (equal (twin-string twin-b) (twin-string twin-a)) (equal (twin-string twin-b) (twin-fresh '((1 . 12))))))"), "(nil t)");
    assert_eq!(compared() - before, 3, "the restricted parser compares on both requests");
}

#[test]
fn twin_comparisons_require_the_same_view() {
    let mut vm = gnu_lisp(&[]);
    vm.eval_source("(require 'treesit)").unwrap();
    vm.eval_source(SYSTEM_SETUP).unwrap();
    let compared = romacs_vm::treesit_comparisons_on_this_thread;
    for (configure, query, reason) in [
        ("", "(goto-char 1) (delete-char 1) (insert \"x\") \
              (treesit-parser-set-included-ranges a (list (cons (point-min) (point-max)))) \
              (treesit-parser-root-node a) (treesit-parser-root-node b)", "different whole-view ranges"),
        ("", "(save-restriction (narrow-to-region 8 (point-max)) \
                  (treesit-parser-set-included-ranges a (list (cons (point-min) (point-max))))) \
              (save-restriction (narrow-to-region 9 (point-max)) \
                  (treesit-parser-set-included-ranges b (list (cons (point-min) (point-max))))) \
              (treesit-parser-set-included-ranges a nil) (treesit-parser-set-included-ranges b nil) \
              (treesit-parser-root-node a) (treesit-parser-root-node b)",
              "equal numbers of different narrowing edits to the same generation"),
        ("(setq-local treesit-languages-require-line-column-tracking '(c))",
         "(goto-char 1) (insert \"static \") (treesit-parser-root-node a) (treesit-parser-root-node b)",
         "different line and column tracking"),
    ] {
        let before = compared();
        let source = format!("(with-temp-buffer (insert \"int a; int b;\") \
            (let* ((a (treesit-parser-create 'c)) \
                   (b (progn {configure} (treesit-parser-create 'c nil t 'twin)))) \
              (treesit-parser-root-node a) (treesit-parser-root-node b) {query}))");
        vm.eval_source(&source).unwrap();
        assert_eq!(compared() - before, 2, "{reason}");
    }
}

#[test]
fn twin_ranges_follow_the_registry_that_edited_each_tree() {
    let mut vm = gnu_lisp(&[]);
    vm.eval_source(&format!("(progn {SYSTEM_SETUP} (require 'treesit) \
        (setq treesit-languages-require-line-column-tracking nil) \
        (defalias 'twin-refresh (lambda (p) \
          (treesit-parser-set-included-ranges p (list (cons (point-min) (point-max)))) \
          (treesit-parser-set-included-ranges p nil))))")).unwrap();
    for (edit_x, edit_y, refresh, expected) in [
        ("(goto-char 8) (insert \"static \")", "(goto-char 1) (insert \"static \")",
         "", "(((1 . 8)) ((1 . 14)))"),
        ("", "(subst-char-in-region 1 2 ?i ?*)", "(twin-refresh a)", "(((1 . 7)) ((1 . 7)))"),
    ] {
        let parses = romacs_vm::treesit_parses_on_this_thread();
        vm.eval_source("(progn (setq x (generate-new-buffer \" *twin-x*\")) \
          (setq y (generate-new-buffer \" *twin-y*\")) \
          (with-current-buffer x (insert \"int a; int b;\n\")) \
          (with-current-buffer y (insert \"int a; int b;\n\")) \
          (with-current-buffer x \
            (setq a (treesit-parser-create 'c nil t 'a)) (treesit-parser-root-node a) \
            (buffer-swap-text y) \
            (setq b (treesit-parser-create 'c nil t 'b)) (treesit-parser-root-node b) \
            (twin-refresh a) (treesit-parser-root-node a) (buffer-swap-text y)))").unwrap();
        assert_eq!(romacs_vm::treesit_parses_on_this_thread() - parses, 2,
                   "the refresh of a copied b's tree");
        let before = romacs_vm::treesit_comparisons_on_this_thread();
        let ranges = vm.eval_source(&format!("(progn (with-current-buffer x {edit_x}) \
            (with-current-buffer y {edit_y}) \
            (with-current-buffer x (buffer-swap-text y) {refresh} \
              (list (treesit-parser-changed-regions b) (treesit-parser-changed-regions a))))"))
            .unwrap().display().unwrap();
        assert_eq!(ranges, expected);
        assert_eq!(romacs_vm::treesit_comparisons_on_this_thread() - before, 2);
        vm.eval_source("(kill-buffer x) (kill-buffer y)").unwrap();
    }
}

#[test]
fn twin_notifier_edit_requires_another_comparison() {
    let mut vm = gnu_lisp(&[]);
    vm.eval_source(&format!("(progn {SYSTEM_SETUP} (require 'treesit) \
      (set-buffer (get-buffer-create \"twin-notifier\")) (insert \"int a; int b;\") \
      (setq a (treesit-parser-create 'c nil t 'a) b (treesit-parser-create 'c nil t 'b) notices nil) \
      (treesit-parser-root-node a) (treesit-parser-root-node b) \
      (defalias 'twin-record (lambda (ranges parser) \
        (push (cons (treesit-parser-tag parser) ranges) notices) \
        (when (eq parser a) (goto-char (point-max)) (insert \"int c;\")))) \
      (treesit-parser-add-notifier a 'twin-record) (treesit-parser-add-notifier b 'twin-record))")).unwrap();
    let before = romacs_vm::treesit_comparisons_on_this_thread();
    let notices = vm.eval_source("(progn (goto-char (point-min)) (insert \"static \") \
        (treesit-parser-root-node a) (treesit-parser-root-node b) (reverse notices))")
        .unwrap().display().unwrap();
    assert_eq!(notices, "((a (1 . 8)) (b (1 . 8) (21 . 27)))");
    assert_eq!(romacs_vm::treesit_comparisons_on_this_thread() - before, 2);
}

#[test]
fn a_restarted_first_parse_cannot_supply_a_first_twin() {
    let mut vm = first_parse_vm("ts-first-restart", 4000, true);
    let parses = romacs_vm::treesit_parses_on_this_thread();
    deferred_frame(&mut vm);
    wait_landed(&mut vm, "ts-first-restart");
    vm.eval_source("(with-current-buffer \"ts-first-restart\" (goto-char 1) (insert \"static \"))").unwrap();
    deferred_frame(&mut vm);
    wait_landed(&mut vm, "ts-first-restart");
    let before = romacs_vm::treesit_comparisons_on_this_thread();
    let result = vm.eval_source("(with-current-buffer \"ts-first-restart\" \
        (string-prefix-p \"static \" (treesit-node-text (treesit-parser-root-node treesit-test-parser))))")
        .unwrap().display().unwrap();
    assert_eq!(result, "t");
    assert_eq!(romacs_vm::treesit_parses_on_this_thread(), parses, "the worker restarted the first parse");
    vm.eval_source("(with-current-buffer \"ts-first-restart\" \
        (treesit-parser-root-node (treesit-parser-create 'c nil t 'twin)))").unwrap();
    assert_eq!(romacs_vm::treesit_parses_on_this_thread() - parses, 1);
    assert_eq!(romacs_vm::treesit_comparisons_on_this_thread(), before);
}

pub(super) fn check_restarted_background_twins(vm: &mut Vm) {
    let parses = romacs_vm::treesit_parses_in_process();
    let comparisons = romacs_vm::treesit_comparisons_on_this_thread();
    vm.eval_source("(with-current-buffer \"ts-reparse-twins\" (goto-char (point-min)) (delete-char 7))").unwrap();
    deferred_frame(vm);
    wait_landed(vm, "ts-reparse-twins");
    vm.eval_source("(with-current-buffer \"ts-reparse-twins\" (insert \"static \") (setq treesit-test-notifications nil))").unwrap();
    deferred_frame(vm);
    wait_landed(vm, "ts-reparse-twins");
    let notified = vm.eval_source("(with-current-buffer \"ts-reparse-twins\" \
        (treesit-parser-root-node treesit-test-parser) (treesit-parser-root-node treesit-test-twin) \
        (reverse treesit-test-notifications))").unwrap().display().unwrap();
    assert_eq!(notified, "((nil (1 . 8)) (twin (1 . 8)))");
    assert_eq!(romacs_vm::treesit_parses_in_process() - parses, 2, "one worker and its restart");
    assert_eq!(romacs_vm::treesit_comparisons_on_this_thread(), comparisons,
               "both notifiers compare to the old parser trees on the restarted worker");
}
