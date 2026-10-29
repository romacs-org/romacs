#[path = "../src/command.rs"] mod command;
#[path = "../src/echo.rs"] mod echo;
#[path = "../src/eval_last_sexp.rs"] mod eval_last_sexp;
#[path = "../src/loadup.rs"] mod loadup;
#[path = "../src/native.rs"] mod native;
#[path = "../src/presentation.rs"] mod presentation;
#[path = "../src/startup.rs"] mod startup;
use romacs_vm::Vm;
fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> { vm.frame_snapshot(width, height) }

fn setup() -> Vm {
    let mut vm = Vm::new();
    vm.set_frame_geometry(1000, 700, 20, 10).unwrap();
    startup::initialize_files(&mut vm, &[]).unwrap();
    vm
}
fn printed(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) { Ok(v) => v.display().unwrap(), Err(e) => format!("ERROR {e}") }
}
#[test]
fn mail_entrypoints_are_real_interactive_commands() {
    let mut vm = setup();
    assert_eq!(printed(&mut vm,"(list (not (null (interactive-form 'notmuch))) (not (null (interactive-form 'notmuch-search))) (not (null (interactive-form 'notmuch-mua-new-mail))))"),"(t t t)","NOTMUCH_COMMANDS_INSTALLED");
}
#[test]
fn search_rows_preserve_exact_thread_identity_and_sanitize_display() {
    let mut vm = setup();
    // Return arrives as `return', which `local-function-key-map' makes RET;
    // C-j is C-j, which the search map leaves to the global map.
    assert_eq!(printed(&mut vm, "(list (lookup-key romacs-mail-search-map [13]) (lookup-key romacs-mail-search-map [10]) (lookup-key local-function-key-map [return]))"), "(romacs-mail-open-tree nil [13])", "NATIVE_RETURN_OPENS_MAIL");
    assert_eq!(printed(&mut vm,r#"(equal (romacs-mail-clean "a\tb\nc\177\205" t) "a b c  ")"#),"t","MAIL_CONTROLS_CANNOT_REACH_LAYOUT");
    assert_eq!(printed(&mut vm,r#"(substring-no-properties (romacs-mail-search-line (romacs-notmuch--summary 'fixture (romacs-json-read "{\"thread\":\"abc\",\"date_relative\":\"Today\",\"timestamp\":1,\"matched\":1,\"total\":2,\"authors\":\"Alice\",\"subject\":\"Hello λ\",\"tags\":[\"inbox\"]}"))))"#),"\"Today         [1/2] Alice                    Hello λ (inbox)\\n\"");
}
#[test]
fn compose_is_editable_but_sending_requires_a_transport_and_preserves_draft() {
    let mut vm=setup();
    assert_eq!(printed(&mut vm,r#"(progn (notmuch-mua-new-mail "hermes@example.invalid" "Draft λ") (insert "Body") (list major-mode buffer-read-only (buffer-modified-p)))"#),"(romacs-mail-compose-mode nil t)");
    assert_eq!(printed(&mut vm,"(condition-case e (romacs-mail-send) (user-error 'no-transport))"),"no-transport");
    assert_eq!(printed(&mut vm,"(list (buffer-modified-p) romacs-mail-send-state)"),"(t nil)");
    assert_eq!(printed(&mut vm,"(not (null (string-match \"Subject: Draft λ\" (buffer-string))))"),"t");
}

fn settle(vm: &mut Vm) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while printed(vm, "(null romacs-process-handlers)") != "t" {
        vm.eval_source("(romacs-process-dispatch-ready)").unwrap();
        assert!(std::time::Instant::now() < deadline, "MAIL_CALLBACK_SETTLES");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
fn quote(text: &str) -> String { serde_json::to_string(text).unwrap() }

#[test]
fn actual_notmuch_search_show_and_hidden_callback_preserve_the_editor() {
    let root = std::env::temp_dir().join(format!("romacs-mail-{}", std::process::id()));
    std::fs::create_dir_all(root.join("mail/new")).unwrap();
    std::fs::create_dir_all(root.join("mail/cur")).unwrap();
    std::fs::create_dir_all(root.join("mail/tmp")).unwrap();
    let config = root.join("config");
    std::fs::write(&config, format!("[database]\npath={}/mail\n[user]\nname=Fixture\nprimary_email=fixture@example.invalid\n[new]\ntags=inbox;\n", root.display())).unwrap();
    std::fs::write(root.join("mail/new/one"), "From: Alice <alice@example.invalid>\nTo: Fixture <fixture@example.invalid>\nSubject: Actual Notmuch λ\nDate: Sat, 24 Oct 2026 10:00:00 +0000\nMessage-ID: <romacs-fixture@example.invalid>\nMIME-Version: 1.0\nContent-Type: text/plain; charset=utf-8\n\nPlain body λ\tcolumn\n").unwrap();
    assert!(std::process::Command::new("/usr/bin/notmuch").arg(format!("--config={}",config.display())).arg("new").output().unwrap().status.success());
    let mut vm = setup();
    vm.eval_source(&format!("(setq romacs-notmuch-command (list \"/usr/bin/notmuch\" {}))",quote(&format!("--config={}",config.display())))).unwrap();
    vm.eval_source("(switch-to-buffer (get-buffer-create \"mail-sentinel\")) (insert \"keep\") (goto-char 3) (setq sentinel (current-buffer)) (notmuch-search \"tag:inbox\") (setq search-buffer (current-buffer)) (switch-to-buffer sentinel)").unwrap();
    settle(&mut vm);
    assert_eq!(printed(&mut vm,"(list (buffer-name) (point) (buffer-string))"),"(\"mail-sentinel\" 3 \"keep\")","HIDDEN_MAIL_MUST_NOT_SELECT_OR_EDIT_ANOTHER_BUFFER");
    assert_eq!(printed(&mut vm,"(with-current-buffer search-buffer (list (length romacs-mail-rows) buffer-read-only (not (null (string-match \"Actual Notmuch λ\" (buffer-string))))))"),"(1 t t)");
    vm.eval_source("(switch-to-buffer search-buffer) (goto-char (point-min)) (romacs-mail-open-tree)").unwrap();
    settle(&mut vm);
    assert_eq!(printed(&mut vm,"major-mode"),"romacs-mail-tree-mode","SEARCH_RETURN_MUST_OPEN_THREAD_TREE_BEFORE_MESSAGE");
    assert_eq!(printed(&mut vm,"(list (length romacs-mail-rows) buffer-read-only romacs-mail-pair-window)"),"(1 t nil)");
    vm.eval_source("(romacs-mail-show-message)").unwrap();
    settle(&mut vm);
    assert_eq!(printed(&mut vm, r#"(list (cdr (plist-get romacs-mail-message :key)) buffer-read-only (not (null (string-match "Plain body λ column" (buffer-string)))))"#), r#"("romacs-fixture@example.invalid" t t)"#, "REAL_NOTMUCH_JSON_REACHES_MESSAGE_BUFFER");
    vm.eval_source("(notmuch-show-reply-sender)").unwrap();
    settle(&mut vm);
    assert_eq!(printed(&mut vm, r#"(list major-mode (cdr (plist-get romacs-mail-envelope :source)) (not (null (string-match "> Plain body λ column" (buffer-string)))))"#), r#"(romacs-mail-compose-mode "romacs-fixture@example.invalid" t)"#);
    vm.eval_source("(notmuch-search \"tag:inbox\") (setq buffer-file-name \"/owned-elsewhere\") (let ((inhibit-read-only t)) (erase-buffer) (insert \"new owner\"))").unwrap();
    settle(&mut vm);
    assert_eq!(printed(&mut vm,"(buffer-string)"),"\"new owner\"","RETIRED_MAIL_VIEW_MUST_NOT_OVERWRITE_A_VISITING_BUFFER");
    std::fs::remove_dir_all(root).unwrap();
}

struct Confirm(std::collections::VecDeque<romacs_vm::HostEvent>);
impl romacs_vm::HostInput for Confirm {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<(u64,usize)>,String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<romacs_vm::HostEvent,String> { self.0.pop_front().ok_or_else(|| "confirmation exhausted".into()) }
}
#[test]
fn compose_submits_exact_bytes_once_and_saves_failed_drafts() {
    let path = std::env::temp_dir().join(format!("romacs-submission-{}", std::process::id()));
    let mut vm = setup();
    let drafts = path.with_extension("drafts");
    vm.eval_source(&format!("(setq romacs-mail-draft-directory {})", quote(&format!("{}/", drafts.display())))).unwrap();
    vm.eval_source(&format!(r#"(setq romacs-notmuch-from "Fixture <fixture@example.invalid>"
        romacs-notmuch-send-command (list "/bin/sh" "-c" "cat > \"$1\"" "fixture" {}))
        (notmuch-mua-new-mail "hermes@example.invalid" "Test λ") (insert "Body λ\n")"#,quote(&path.to_string_lossy()))).unwrap();
    assert_eq!(printed(&mut vm, "(string-match \"User-Agent:\" (buffer-string))"), "nil", "CLIENT_HEADER_DOES_NOT_CLUTTER_COMPOSE");
    vm.set_host_input(Some(Box::new(Confirm("yes\r".chars().map(|c|romacs_vm::HostEvent::Key(romacs_vm::CommandEvent::Character(c as i64))).collect()))));
    vm.eval_source("(setq submitted-buffer (current-buffer)) (romacs-mail-send)").unwrap();
    assert_eq!(printed(&mut vm,"(with-current-buffer submitted-buffer (list romacs-mail-send-state buffer-read-only))"),"(sending t)","PENDING_SUBMISSION_FREEZES_ITS_WIRE_SNAPSHOT");
    settle(&mut vm);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("Subject: Test λ\n") && text.ends_with("\n\nBody λ\n"));
    assert!(!text.contains("--text follows this line--"));
    assert_eq!(text.matches("User-Agent: Romacs notmuch 0.1\n").count(), 1, "MAIL_IDENTIFIES_ROMACS_CLIENT");
    assert_eq!(printed(&mut vm,"(buffer-live-p submitted-buffer)"),"nil","VERIFIED_SEND_REMOVES_ITS_COMPOSE_BUFFER");
    assert_eq!(printed(&mut vm,"(condition-case e (romacs-mail-send) (user-error 'refused))"),"refused","SUBMISSION_CANNOT_REPEAT");
    vm.eval_source("(setq romacs-notmuch-send-command '(\"/bin/sh\" \"-c\" \"cat >/dev/null; exit 7\")) (notmuch-mua-new-mail \"hermes@example.invalid\" \"Failure\") (insert \"retain me\")").unwrap();
    vm.set_host_input(Some(Box::new(Confirm("yes\r".chars().map(|c|romacs_vm::HostEvent::Key(romacs_vm::CommandEvent::Character(c as i64))).collect()))));
    vm.eval_source("(setq failed-buffer (current-buffer)) (romacs-mail-send)").unwrap();
    settle(&mut vm);
    assert_eq!(printed(&mut vm,"(with-current-buffer failed-buffer (list romacs-mail-send-state buffer-read-only (buffer-modified-p) (not (null (file-attributes romacs-mail-draft-file)))))"),"(nil nil t t)");
    assert_eq!(printed(&mut vm,"(condition-case e (romacs-mail-send) (user-error 'refused))"),"refused");
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(drafts).unwrap();
}


#[test]
fn retired_compose_settles_without_touching_the_new_document() {
    let mut vm = setup();
    vm.eval_source(r#"(setq romacs-notmuch-from "Fixture <fixture@example.invalid>"
      romacs-notmuch-send-command '("/bin/sh" "-c" "cat >/dev/null"))
      (notmuch-mua-new-mail "hermes@example.invalid" "Retire") (insert "Draft")"#).unwrap();
    vm.set_host_input(Some(Box::new(Confirm("yes\r".chars().map(|c|romacs_vm::HostEvent::Key(romacs_vm::CommandEvent::Character(c as i64))).collect()))));
    vm.eval_source("(setq retired-buffer (current-buffer)) (romacs-mail-send) (set-buffer retired-buffer) (setq buffer-file-name \"/new-owner\") (let ((inhibit-read-only t)) (erase-buffer) (insert \"new document\")) (goto-char 3) (setq tick (buffer-chars-modified-tick))").unwrap();
    settle(&mut vm);
    assert_eq!(printed(&mut vm,"(list (buffer-string) (point) (= tick (buffer-chars-modified-tick)) (eq romacs-mail-send-state 'sending))"),"(\"new document\" 3 t nil)","RETIRED_SEND_SETTLES_WITHOUT_REWRITING_NEW_OWNER");
}
