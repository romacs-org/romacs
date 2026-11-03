#[path = "../src/command.rs"]
mod command;
#[path = "../src/echo.rs"]
mod echo;
#[path = "../src/eval_last_sexp.rs"]
mod eval_last_sexp;
#[path = "../src/loadup.rs"]
mod loadup;
#[path = "../src/native.rs"]
mod native;
#[path = "../src/presentation.rs"]
mod presentation;
#[path = "../src/startup.rs"]
mod startup;

use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, Vm};
use std::collections::VecDeque;

fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> {
    vm.frame_snapshot(width, height)
}
fn printed(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("VM-ERROR {error}"),
    }
}
fn setup() -> Vm {
    let mut vm = Vm::new();
    vm.set_frame_geometry(1000, 700, 20, 10).unwrap();
    startup::initialize_files(&mut vm, &[]).unwrap();
    vm.eval_source("(switch-to-buffer (get-buffer-create \"editing-contract\")) (insert \"first λ\nsecond\n\") (goto-char 5)").unwrap();
    vm
}
struct Keys(VecDeque<HostEvent>);
impl HostInput for Keys {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> {
        Ok(vec![])
    }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.0.pop_front().ok_or_else(|| "editing input exhausted".into())
    }
}
fn input(vm: &mut Vm, text: &str) {
    vm.set_host_input(Some(Box::new(Keys(text.chars().map(|c| HostEvent::Key(CommandEvent::Character(c as i64))).collect()))));
}
#[test]
fn active_selection_is_deleted_or_replaced_by_ordinary_keys() {
    for reverse in [false, true] {
        for (event, expected, point) in [
            (CommandEvent::Character(127), "ac", 2),
            (CommandEvent::Character(8), "ac", 2),
            (CommandEvent::Character(4), "ac", 2),
            (CommandEvent::Named("backspace".into()), "ac", 2),
            (CommandEvent::Named("delete".into()), "ac", 2),
            (CommandEvent::Character(88), "aXc", 3),
        ] {
            let mut vm = setup();
            vm.eval_source("(erase-buffer) (insert \"aλb界c\") (setq kill-ring '(\"sentinel\"))").unwrap();
            vm.eval_source(if reverse { "(goto-char 2) (set-mark 5)" } else { "(goto-char 5) (set-mark 2)" }).unwrap();
            let mut commands = command::Commands::default();
            let outcome = commands.dispatch(&mut vm, event.clone()).unwrap().unwrap();
            assert!(outcome.signal.is_none(), "SELECTION_EDIT_EXECUTES: {event:?} {outcome:?}");
            assert_eq!(printed(&mut vm, "(buffer-string)"), serde_json::to_string(expected).unwrap(), "ACTIVE_SELECTION_REPLACED: {event:?}, reverse={reverse}");
            assert_eq!(printed(&mut vm, "(point)"), point.to_string());
            assert_eq!(printed(&mut vm, "(list mark-active kill-ring)"), "(nil (\"sentinel\"))", "DELETE_SELECTION_DOES_NOT_KILL");
        }
    }
}

#[test]
fn selection_replacement_respects_readonly_and_inactive_marks() {
    let mut vm = setup();
    vm.eval_source("(erase-buffer) (insert \"abcde\") (goto-char 4) (set-mark 2) (setq buffer-read-only t saved-tick (buffer-chars-modified-tick))").unwrap();
    let mut commands = command::Commands::default();
    for event in [127,4,88] {
        let out = commands.dispatch(&mut vm, CommandEvent::Character(event)).unwrap().unwrap();
        assert_eq!(out.signal.unwrap().condition,"buffer-read-only");
        assert_eq!(printed(&mut vm,"(list (buffer-string) (point) mark-active (= saved-tick (buffer-chars-modified-tick)))"),"(\"abcde\" 4 t t)","READONLY_SELECTION_MUST_REMAIN_INTACT");
    }
    vm.eval_source("(setq buffer-read-only nil) (deactivate-mark t)").unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(4)).unwrap();
    assert_eq!(printed(&mut vm,"(buffer-string)"),"\"abce\"","INACTIVE_MARK_DOES_NOT_SELECT_TEXT");
}

#[test]
fn selection_replacement_preserves_prefix_narrowing_and_empty_region_semantics() {
    let mut vm = setup();
    let mut commands = command::Commands::default();
    vm.eval_source("(erase-buffer) (insert \"abcdef\") (set-mark 2) (goto-char 5) (narrow-to-region 3 6)").unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(4)).unwrap();
    assert_eq!(printed(&mut vm,"(progn (widen) (buffer-string))"),"\"abef\"","SELECTION_DELETE_CLIPS_TO_ACCESSIBLE_REGION");
    vm.eval_source("(set-mark 2) (goto-char 3)").unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(21)).unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(88)).unwrap();
    assert_eq!(printed(&mut vm,"(buffer-string)"),"\"aXXXXef\"","PREFIX_REPLACES_SELECTION_ONCE_THEN_REPEATS_INSERTION");
    vm.eval_source("(goto-char 2) (set-mark 2) (setq use-empty-active-region t)").unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(4)).unwrap();
    assert_eq!(printed(&mut vm,"(buffer-string)"),"\"aXXXef\"","EMPTY_REGION_DOES_NOT_SWALLOW_DELETE_COMMAND");
    vm.eval_source("(delete-selection-mode -1) (goto-char 2) (set-mark 4)").unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(90)).unwrap();
    assert_eq!(printed(&mut vm,"(buffer-string)"),"\"aZXXXef\"","DISABLED_DELETE_SELECTION_DOES_NOT_REPLACE_TEXT");
    vm.eval_source("(delete-selection-mode 1) (goto-char 2) (set-mark 6) (delete-char 1)").unwrap();
    assert_eq!(printed(&mut vm,"(buffer-string)"),"\"aXXXef\"","PROGRAMMATIC_DELETION_DOES_NOT_REPLACE_SELECTION");
}

#[test]
fn selection_replacement_also_runs_inside_recursive_prompts() {
    let mut vm = setup();
    input(&mut vm,"abcd\u{2}\u{2}\u{2}\u{2}\0\u{6}\u{6}Z\r");
    assert_eq!(printed(&mut vm,"(read-string \"Choose: \")"),"\"Zcd\"","PROMPT_TYPING_REPLACES_ONLY_SELECTED_INPUT");
    input(&mut vm,"abcd\u{2}\u{2}\u{2}\u{2}\0\u{6}\u{6}\u{7f}\r");
    assert_eq!(printed(&mut vm,"(read-string \"Choose: \")"),"\"cd\"","PROMPT_BACKSPACE_DELETES_ONLY_SELECTED_INPUT");
    // C-a stops at the input's start, after the read-only prompt, so the
    // selection holds only input (pinned GNU's answers).
    for (code, expected) in [('Z', "\"Zcd\""), ('\u{7f}', "\"cd\""), ('\u{4}', "\"cd\"")] {
        input(&mut vm, &format!("abcd\u{1}\0\u{6}\u{6}{code}\r"));
        assert_eq!(printed(&mut vm,"(read-string \"Choose: \")"),expected,"PROMPT_BEGINNING_SELECTS_ONLY_INPUT");
    }
    assert_eq!(printed(&mut vm,"(list (buffer-string) (point) (minibuffer-depth))"),"(\"first λ\\nsecond\\n\" 5 0)","PROMPT_SELECTION_RESTORES_CALLER_STATE");
}

#[test]
fn ordinary_region_copy_kill_yank_and_registers_share_state() {
    let mut vm = setup();
    assert_eq!(printed(&mut vm, "(list (fboundp 'copy-region-as-kill) (not (null (interactive-form 'kill-region))) (fboundp 'copy-to-register))"),
               "(t t t)", "SHARED_EDITING_COMMANDS_AVAILABLE");
    assert_eq!(printed(&mut vm, "(progn (goto-char 1) (call-interactively 'set-mark-command) (goto-char 8) (call-interactively 'copy-region-as-kill) (list (car kill-ring) mark-active (buffer-string)))"),
               "(\"first λ\" nil \"first λ\\nsecond\\n\")", "COPY_REGION_USES_ORDINARY_MARK");
    assert_eq!(printed(&mut vm, "(progn (copy-to-register ?a 1 8) (point-to-register ?p) (goto-char (point-max)) (insert-register ?a) (jump-to-register ?p) (point))"), "8", "TEXT_AND_POINT_REGISTERS_SHARE_BUFFER_MARKERS");
    assert_eq!(printed(&mut vm, "(progn (setq last-command nil) (kill-region 1 3) (setq last-command 'kill-region) (kill-region 1 3) (equal (car kill-ring) \"firs\"))"), "t", "CONSECUTIVE_KILLS_APPEND");
    assert_eq!(printed(&mut vm, "(progn (kill-new \"replacement\") (goto-char 1) (yank) (setq last-command 'yank) (yank-pop 1) (equal (buffer-substring-no-properties 1 5) \"firs\"))"), "t", "YANK_POP_REPLACES_ONLY_ITS_INSERTION");
    vm.eval_source("(setq buffer-read-only t last-command nil)").unwrap();
    assert_eq!(printed(&mut vm, "(condition-case e (kill-region 1 5) (buffer-read-only (list (car e) (equal (car kill-ring) \"firs\"))))"), "(buffer-read-only t)", "READONLY_KILL_COPIES_BUT_NEVER_DELETES");
}

#[test]
fn an_empty_clipboard_is_no_new_kill() {
    let mut vm = setup();
    vm.eval_source("(setq kill-ring nil)").unwrap();
    // An empty CLIPBOARD (another program, or a read racing a set) is no
    // new text: `current-kill' keeps the kill, as GNU's select.el does.
    assert_eq!(printed(&mut vm, "(let ((set (symbol-function 'romacs--clipboard-set)) (get (symbol-function 'romacs--clipboard-text)) (interprogram-cut-function 'romacs-clipboard-select-text) (interprogram-paste-function 'romacs-clipboard-selection-value))
        (defvar romacs-test-desktop nil)
        (unwind-protect
            (progn (fset 'romacs--clipboard-set (lambda (text) (setq romacs-test-desktop text)))
                   (fset 'romacs--clipboard-text (lambda () romacs-test-desktop))
                   (kill-new \"killed\") (setq romacs-test-desktop \"\")
                   (list (current-kill 0) (length kill-ring) (progn (setq romacs-test-desktop \"other\") (current-kill 0))))
          (fset 'romacs--clipboard-set set) (fset 'romacs--clipboard-text get)))"),
               "(\"killed\" 1 \"other\")", "EMPTY_CLIPBOARD_IS_NO_NEW_TEXT");
}

#[test]
fn active_regions_are_rendered_without_modifying_text() {
    let mut vm = setup();
    vm.eval_source("(goto-char 2) (set-mark 6) (setq saved-tick (buffer-chars-modified-tick))").unwrap();
    let frame = vm.frame_snapshot(1000, 700).unwrap();
    let selected = frame.windows.iter().find(|w| w.snapshot.text.utf8 == "first λ\nsecond\n").unwrap();
    let snap = &selected.snapshot;
    let has_region = snap.text.face_runs.iter().any(|run| {
        run.bytes.contains(&2) && snap.faces[run.face.0 as usize].background == [62, 68, 81]
    });
    assert!(has_region, "ACTIVE_REGION_MUST_REACH_NATIVE_FACE_RUNS");
    assert_eq!(printed(&mut vm, "(list (point) (mark t) (= saved-tick (buffer-chars-modified-tick)) (get-text-property 3 'face))"), "(2 6 t nil)", "REGION_PAINT_IS_A_STATE_PRESERVING_QUERY");
    vm.eval_source("(deactivate-mark t)").unwrap();
    let frame = vm.frame_snapshot(1000, 700).unwrap();
    assert!(!frame.windows.iter().any(|w| w.snapshot.text.utf8 == "first λ\nsecond\n" && w.snapshot.text.face_runs.iter().any(|run| w.snapshot.faces[run.face.0 as usize].background == [62, 68, 81])));
}

#[test]
fn selection_errors_and_yank_pop_keep_user_orientation() {
    let mut vm = setup();
    assert_eq!(printed(&mut vm, "(condition-case e (call-interactively 'kill-region) (user-error 'no-region))"), "no-region", "NO_MARK_EXPLAINS_MISSING_REGION");
    vm.eval_source("(erase-buffer) (kill-new \"old\") (kill-new \"new\") (yank '(4)) (setq last-command 'yank) (yank-pop 1)").unwrap();
    assert_eq!(printed(&mut vm, "(list (point) (mark t) (equal (buffer-string) \"old\"))"), "(1 4 t)", "YANK_POP_PRESERVES_PREFIXED_YANK_ORIENTATION");
    vm.eval_source("(let ((eval-expression-print-length 2)) (eval-expression '(list 1 2 3)))").unwrap();
    assert_eq!(echo::text(&mut vm).unwrap(), "(1 2 ...)", "EVAL_PRINT_CONTROLS_ARE_DYNAMIC");
}

#[test]
fn interactive_backward_kills_prepend_to_the_same_entry() {
    let mut vm = setup();
    vm.eval_source("(erase-buffer) (insert \"abcdef\") (set-mark 5) (goto-char 7)").unwrap();
    let mut commands = command::Commands::default();
    commands.dispatch(&mut vm, CommandEvent::Character(23)).unwrap();
    vm.eval_source("(set-mark 5) (goto-char 3)").unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(23)).unwrap();
    assert_eq!(printed(&mut vm, "(list (equal (car kill-ring) \"cdef\") (buffer-string))"), "(t \"ab\")", "BACKWARD_INTERACTIVE_KILLS_PREPEND");
}

#[test]
fn mark_history_and_moving_registers_are_buffer_owned() {
    let mut vm = setup();
    assert_eq!(printed(&mut vm, "(progn (goto-char 2) (push-mark 3 t) (push-mark 7 t) (pop-mark) (list (mark t) mark-active))"), "(3 nil)", "MARK_RING_REUSES_MOVING_MARKERS");
    assert_eq!(printed(&mut vm, "(progn (goto-char 6) (point-to-register ?p) (goto-char 1) (insert \"λ\") (jump-to-register ?p) (point))"), "7", "POINT_REGISTER_FOLLOWS_UNICODE_INSERTION");
    assert_eq!(printed(&mut vm, "(progn (copy-to-register ?t 1 2) (append-to-register ?t 2 3) (prepend-to-register ?t 3 4) (get-register ?t))"), "\"iλf\"", "TEXT_REGISTER_APPEND_PREPEND");
    vm.eval_source("(setq original-editing-buffer (current-buffer)) (set-mark 3) (switch-to-buffer (get-buffer-create \"independent-marks\")) (insert \"other\")").unwrap();
    assert_eq!(printed(&mut vm, "(list (mark t) mark-active mark-ring)"), "(nil nil nil)", "OTHER_BUFFER_HAS_NO_SELECTION_OR_RING");
    vm.eval_source("(switch-to-buffer original-editing-buffer) (set-mark 3) (goto-char 8) (narrow-to-region 4 10)").unwrap();
    let mut commands = command::Commands::default();
    commands.dispatch(&mut vm, CommandEvent::Character(134217788)).unwrap();
    assert_eq!(printed(&mut vm, "(point)"), "4", "META_LESS_HONORS_NARROWING");
    vm.eval_source("(widen) (kill-buffer original-editing-buffer)").unwrap();
    assert_eq!(printed(&mut vm, "(condition-case e (jump-to-register ?p) (user-error 'dead-point-register))"), "dead-point-register", "DEAD_POINT_REGISTER_REFUSES_WITHOUT_SWITCHING");
}

#[test]
fn editing_failures_quit_and_one_key_registers_preserve_the_document() {
    let mut vm = setup();
    let mut commands = command::Commands::default();
    input(&mut vm, "(setq must-not-run 1) (\r\u{7}");
    let result = commands.dispatch(&mut vm, CommandEvent::Character(134217786)).unwrap().unwrap();
    assert!(result.signal.is_some(), "MALFORMED_TRAILING_FORM_MUST_REMAIN_AT_PROMPT");
    assert_eq!(printed(&mut vm, "(list (boundp 'must-not-run) (point) read-expression-history (minibuffer-depth))"), "(nil 5 nil 0)");
    input(&mut vm, "(let ((captured 7)) (funcall (lambda () (+ captured 2))))\r");
    commands.dispatch(&mut vm, CommandEvent::Character(134217786)).unwrap();
    assert_eq!(printed(&mut vm, "(car values)"), "9", "EVAL_EXPRESSION_IS_LEXICAL");
    vm.eval_source("(goto-char 1) (set-mark 8)").unwrap();
    input(&mut vm, "z");
    for code in [24, 114, 115] {
        if let Some(outcome) = commands.dispatch(&mut vm, CommandEvent::Character(code)).unwrap() {
            assert!(outcome.signal.is_none(), "REGISTER_COMMAND_EXECUTES: {outcome:?}");
        }
    }
    assert_eq!(printed(&mut vm, "(get-register ?z)"), "\"first λ\"", "REGISTER_READER_ACCEPTS_ONE_KEY_WITHOUT_RETURN");
    vm.eval_source("(goto-char 2) (set-mark 6)").unwrap();
    let mut presentation = presentation::Presentation::new();
    presentation.command(&mut vm, &mut commands, CommandEvent::Character(65)).unwrap();
    assert_eq!(printed(&mut vm, "mark-active"), "nil", "ORDINARY_EDIT_DEACTIVATES_REGION");
}

#[test]
fn universal_prefix_survives_prompts_and_is_consumed_once() {
    let mut vm = setup();
    let mut commands = command::Commands::default();
    commands.dispatch(&mut vm, CommandEvent::Character(21)).unwrap();
    input(&mut vm, "(+ 1 2)\r");
    let result = commands.dispatch(&mut vm, CommandEvent::Character(134217786)).unwrap().unwrap();
    assert!(result.signal.is_none(), "PREFIXED_EXPRESSION_EXECUTES: {result:?}");
    assert_eq!(printed(&mut vm, "(list (buffer-string) (car read-expression-history) prefix-arg)"),
               "(\"firs3t λ\\nsecond\\n\" \"(+ 1 2)\" nil)", "PREFIX_SURVIVES_MINIBUFFER_AND_INSERTS_ONCE");
    commands.dispatch(&mut vm, CommandEvent::Character(21)).unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(7)).unwrap();
    assert_eq!(printed(&mut vm, "prefix-arg"), "nil", "QUIT_CLEARS_PREFIX");
    commands.dispatch(&mut vm, CommandEvent::Character(21)).unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(24)).unwrap();
    commands.dispatch(&mut vm, CommandEvent::Named("f24".into())).unwrap();
    assert_eq!(printed(&mut vm, "prefix-arg"), "nil", "UNDEFINED_SEQUENCE_CONSUMES_PREFIX");
    vm.eval_source("(set-mark 3) (goto-char (point-max))").unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(21)).unwrap();
    commands.dispatch(&mut vm, CommandEvent::Character(134217788)).unwrap();
    assert_eq!(printed(&mut vm, "(list (point) (mark t))"), "(1 3)", "PREFIXED_BUFFER_MOTION_PRESERVES_MARK");
}

#[test]
fn eval_expression_and_buffer_motion_are_real_editor_commands() {
    let mut vm = setup();
    assert_eq!(printed(&mut vm, "(list (fboundp 'eval-expression) (key-binding [134217786]) (fboundp 'beginning-of-buffer) (key-binding [134217788]))"),
               "(t eval-expression t beginning-of-buffer)", "EDITING_EVAL_AND_BUFFER_MOTION_BOUND");
    input(&mut vm, "(+ 20 22)\r");
    let mut commands = command::Commands::default();
    let mut presentation = presentation::Presentation::new();
    let result = presentation.command(&mut vm, &mut commands, CommandEvent::Character(134217786)).unwrap().unwrap();
    assert!(result.signal.is_none(), "EVAL_EXPRESSION_EXECUTES: {result:?}");
    assert_eq!(printed(&mut vm, "(list (car values) (point) (buffer-string))"), "(42 5 \"first λ\\nsecond\\n\")", "EVAL_EXPRESSION_PRESERVES_DOCUMENT");
    let result = presentation.command(&mut vm, &mut commands, CommandEvent::Character(134217788)).unwrap().unwrap();
    assert!(result.signal.is_none(), "BEGINNING_OF_BUFFER_EXECUTES: {result:?}");
    assert_eq!(printed(&mut vm, "(point)"), "1", "META_LESS_MOVES_TO_ACCESSIBLE_START");
}
