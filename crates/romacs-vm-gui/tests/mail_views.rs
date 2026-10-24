#[path = "../src/command.rs"] mod command;
#[path = "../src/echo.rs"] mod echo;
#[path = "../src/eval_last_sexp.rs"] mod eval_last_sexp;
#[path = "../src/loadup.rs"] mod loadup;
#[path = "../src/native.rs"] mod native;
#[path = "../src/presentation.rs"] mod presentation;
#[path = "../src/startup.rs"] mod startup;
use romacs_vm::Vm;
fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> { vm.frame_snapshot(width, height) }
fn printed(vm: &mut Vm, source: &str) -> String { vm.eval_source(source).unwrap().display().unwrap() }
fn setup() -> Vm {
    let mut vm = Vm::new();
    vm.set_frame_geometry(1000, 700, 20, 10).unwrap();
    startup::initialize_files(&mut vm, &[]).unwrap();
    vm.eval_source(r#"
      (defvar test-mail-requests nil)
      (defvar test-mail-key '(fixture "account-one"))
      (defvar test-mail-search-rows
        (list (list :thread (cons test-mail-key "thread-one") :date-label "Today" :matched 1 :total 3
                    :authors-matched "Child" :subject "Thread subject" :tags '("inbox"))))
      (defvar test-mail-tree-rows
        (list (list :key (cons test-mail-key "root@example.invalid") :depth 0 :lasts '(t) :match nil :from "Root" :subject "Context root" :tags '("inbox"))
              (list :key (cons test-mail-key "child@example.invalid") :parent-id "root@example.invalid" :depth 1 :lasts '(t nil) :match t :from "Child" :subject "Matching child" :tags '("inbox"))
              (list :key (cons test-mail-key "excluded@example.invalid") :parent-id "root@example.invalid" :depth 1 :lasts '(t t) :match nil :excluded t :from "Excluded" :subject "Excluded child" :tags '("deleted"))))
      (defvar test-mail-reply
        (list :source (cons test-mail-key "child@example.invalid")
              :headers '(("From" . "Alias <alias@example.invalid>") ("To" . "List <list@example.invalid>")
                         ("Cc" . "Peer <peer@example.invalid>") ("Subject" . "Re: Matching child"))
              :in-reply-to "<child@example.invalid>" :references "<root@example.invalid> <child@example.invalid>"
              :quote-attribution "On Saturday, Child wrote:" :quote-text "Selected child body"))
      (defvar test-mail-account
        (list :name "Fixture" :key test-mail-key :capabilities '(:search :tree :message :reply)
              :search (lambda (&rest args) (cons '("fixture-search") (lambda (json) test-mail-search-rows)))
              :tree (lambda (&rest args) (cons '("fixture-tree") (lambda (json) test-mail-tree-rows)))
              :message (lambda (id)
                         (cons (list "fixture-message" id)
                               (lambda (json)
                                 (list :key (cons test-mail-key id)
                                       :headers (list (cons "From" id) (cons "To" "Fixture")
                                                      (cons "Subject" id) (cons "Date" "Saturday"))
                                       :tags '("inbox")
                                       :parts (list (list :type "text/plain" :content (concat "Body for " id)))))))
              :reply (lambda (&rest args) (cons '("fixture-reply") (lambda (json) test-mail-reply)))))
      (setq romacs-mail-runner
        (lambda (argv input directory timeout callback)
          (setq test-mail-requests (append test-mail-requests (list (cons argv callback))))
          (length test-mail-requests)))
      (defun test-mail-deliver ()
        (let ((request (car test-mail-requests)))
          (setq test-mail-requests (cdr test-mail-requests))
          (funcall (cdr request) '(0 "[]" "" nil))))
    "#).unwrap();
    vm
}
#[test]
fn generic_search_tree_preserves_context_identity_and_requires_second_return() {
    let mut vm = setup();
    vm.eval_source("(romacs-mail-search test-mail-account \"opaque backend query\") (setq test-search (current-buffer)) (test-mail-deliver)").unwrap();
    assert_eq!(printed(&mut vm, "(length romacs-mail-rows)"), "1", "{}", printed(&mut vm,"(buffer-string)"));
    vm.eval_source("(romacs-mail-open-tree) (setq test-tree (current-buffer)) (test-mail-deliver)").unwrap();
    assert_eq!(printed(&mut vm,"(list major-mode (plist-get (romacs-mail-row) :match) (cdr (plist-get (romacs-mail-row) :key)) (length romacs-mail-rows) romacs-mail-pair-window romacs-mail-query)"),"(romacs-mail-tree-mode t \"child@example.invalid\" 3 nil \"opaque backend query\")");
    assert_eq!(printed(&mut vm, r#"(list (not (null (string-match (regexp-quote "├─Matching child") (buffer-string)))) (not (null (string-match (regexp-quote "└─Excluded child [excluded]") (buffer-string)))) buffer-read-only)"#), "(t t t)");
    vm.eval_source("(goto-char (point-max))").unwrap();
    assert_eq!(printed(&mut vm,"(condition-case e (romacs-mail-open-tree) (user-error 'footer-refused))"),"footer-refused");
    vm.eval_source("(romacs-mail-quit)").unwrap();
    assert_eq!(printed(&mut vm,"(list (eq (current-buffer) test-search) (cdr (plist-get (romacs-mail-row) :thread)))"),"(t \"thread-one\")");
}
#[test]
fn generic_reply_uses_selected_message_and_keeps_threading_out_of_draft() {
    let mut vm = setup();
    vm.eval_source("(romacs-mail-search test-mail-account \"query\") (test-mail-deliver) (romacs-mail-open-tree) (test-mail-deliver) (romacs-mail-reply t) (test-mail-deliver)").unwrap();
    assert_eq!(printed(&mut vm,"(list major-mode (cdr (plist-get romacs-mail-envelope :source)) (not (null (string-match \"Cc: Peer\" (buffer-string)))) (not (null (string-match \"Selected child body\" (buffer-string)))) (string-match \"References:\" (buffer-string)))"),"(romacs-mail-compose-mode \"child@example.invalid\" t t nil)");
    assert_eq!(printed(&mut vm,"(let ((wire (romacs-mail-freeze (buffer-string) romacs-mail-envelope \"Sat, 24 Oct 2026 10:00:00 +0000\" \"fixture@example.invalid\"))) (list (not (null (string-match \"In-Reply-To: <child@example.invalid>\" wire))) (not (null (string-match \"References: <root@example.invalid> <child@example.invalid>\" wire))) (not (null (string-match \"User-Agent: Romacs notmuch 0.1\" wire)))))"),"(t t t)");
}
#[test]
fn generic_refresh_and_retired_results_do_not_touch_other_documents() {
    let mut vm = setup();
    vm.eval_source("(romacs-mail-search test-mail-account \"query\") (test-mail-deliver) (setq test-search (current-buffer)) (romacs-mail-refresh) (setq test-old-request (car test-mail-requests)) (setq test-mail-requests nil) (romacs-mail-refresh) (test-mail-deliver) (setq buffer-file-name \"/new-owner\") (let ((inhibit-read-only t)) (erase-buffer) (insert \"new document\")) (goto-char 3) (setq test-tick (buffer-chars-modified-tick)) (funcall (cdr test-old-request) '(0 \"[]\" \"\" nil))").unwrap();
    assert_eq!(printed(&mut vm,"(list (buffer-string) (point) (= test-tick (buffer-chars-modified-tick)))"),"(\"new document\" 3 t)");
}
#[test]
fn generic_message_pair_reuses_exact_pane_rejects_stale_result_and_returns_to_rows() {
    let mut vm = setup();
    vm.eval_source("(romacs-mail-search test-mail-account \"query\") (setq test-search (current-buffer)) (test-mail-deliver) (romacs-mail-open-tree) (setq test-tree (current-buffer)) (test-mail-deliver) (setq original-windows (length (romacs-gui-leaves (romacs-gui-root)))) (romacs-mail-show-message) (setq test-message-buffer (current-buffer) test-message-window (selected-window))").unwrap();
    assert_eq!(printed(&mut vm, "(list (eq (window-buffer (get-buffer-window test-tree)) test-tree) (eq (selected-window) test-message-window) (= (length (romacs-gui-leaves (romacs-gui-root))) (1+ original-windows)))"), "(t t t)");
    vm.eval_source("(select-window (get-buffer-window test-tree)) (set-buffer test-tree) (romacs-mail-next-all) (romacs-mail-show-message) (test-mail-deliver) (test-mail-deliver)").unwrap();
    assert_eq!(printed(&mut vm, r#"(list (eq (current-buffer) test-message-buffer) (eq (selected-window) test-message-window) (cdr (plist-get romacs-mail-message :key)) (= (length (romacs-gui-leaves (romacs-gui-root))) (1+ original-windows)))"#), r#"(t t "excluded@example.invalid" t)"#);
    vm.eval_source("(romacs-mail-quit)").unwrap();
    assert_eq!(printed(&mut vm, r#"(list (eq (current-buffer) test-tree) (cdr (plist-get (romacs-mail-row) :key)) (= original-windows (length (romacs-gui-leaves (romacs-gui-root)))))"#), r#"(t "excluded@example.invalid" t)"#);
    vm.eval_source("(romacs-mail-quit)").unwrap();
    assert_eq!(printed(&mut vm, "(eq (current-buffer) test-search)"), "t");
}
#[test]
fn generic_tree_footer_refuses_without_allocating_a_message_buffer() {
    let mut vm = setup();
    vm.eval_source("(romacs-mail-search test-mail-account \"query\") (test-mail-deliver) (romacs-mail-open-tree) (test-mail-deliver) (goto-char (point-max)) (setq before (length (buffer-list)))").unwrap();
    assert_eq!(printed(&mut vm, "(list (condition-case nil (romacs-mail-show-message) (user-error 'refused)) (= before (length (buffer-list))))"), "(refused t)", "FOOTER_CANNOT_ALLOCATE_AN_ORPHAN_MESSAGE_BUFFER");
}
#[test]
fn refresh_loading_and_failure_retire_the_previous_row_identities() {
    for tree in [false, true] {
        let mut vm = setup();
        vm.eval_source("(romacs-mail-search test-mail-account \"query\") (test-mail-deliver)").unwrap();
        if tree {
            vm.eval_source("(romacs-mail-open-tree) (test-mail-deliver)").unwrap();
        }
        vm.eval_source("(romacs-mail-refresh) (setq test-buffer-count (length (buffer-list)) test-request-count (length test-mail-requests))").unwrap();
        let refusal = if tree { "(romacs-mail-show-message)" } else { "(romacs-mail-open-tree)" };
        let check = format!("(list romacs-mail-rows (condition-case nil {} (user-error 'refused)) (condition-case nil (romacs-mail-reply nil) (user-error 'refused)) (= test-buffer-count (length (buffer-list))) (= test-request-count (length test-mail-requests)))", refusal);
        assert_eq!(printed(&mut vm, &check), "(nil refused refused t t)", "LOADING_TEXT_CANNOT_RETAIN_OLD_THREAD_IDENTITIES");
        vm.eval_source("(funcall (cdr (car test-mail-requests)) '(23 \"\" \"backend failed\" nil))").unwrap();
        assert_eq!(printed(&mut vm, &check), "(nil refused refused t t)", "FAILED_REFRESH_CANNOT_OPEN_OR_REPLY_TO_AN_OLD_MESSAGE");
    }
}
#[test]
fn generic_missing_capability_is_an_explicit_error() {
    let mut vm = setup();
    assert_eq!(printed(&mut vm,"(condition-case e (romacs-mail-request '(:capabilities nil) :tree nil (current-buffer) nil major-mode 'ignore) (user-error 'unsupported))"),"unsupported");
}
