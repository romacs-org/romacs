//! The GUI's commands run through the Lisp command loop
//! (lisp/romacs/vm/keyboard.el `romacs--host-command'): its reader takes the
//! unread queues, a keyboard macro, then the host keyboard, and applies
//! the translation maps, as GNU's read_key_sequence does.
use romacs_vm::{CommandEvent, CommandOutcome, CommandSignal, Vm};
#[cfg(test)]
use romacs_vm::Fixed;
use romacs_display_runtime::thread_comm::InputEvent;

pub fn unwrap_event(event: InputEvent) -> InputEvent {
    match event {
        InputEvent::Tracked { event, .. } => unwrap_event(*event),
        event => event,
    }
}

/// Whether read_char has an event queued in Lisp: in
/// `unread-post-input-method-events', `unread-command-events' or
/// `unread-input-method-events'.
pub fn unread_pending(vm: &mut Vm) -> Result<bool, String> {
    Ok(!vm.eval_source("(or unread-post-input-method-events unread-command-events unread-input-method-events)")?.is_nil())
}

/// Run one command through the Lisp command loop, which reads its whole
/// key sequence: what ran and what it signalled.
pub fn run(vm: &mut Vm) -> Result<CommandOutcome, String> {
    let (command, error) = vm.eval_source("(romacs--run-host-command)")?.cons_pair()?;
    let command = if command.is_nil() { None } else { Some(command.display()?) };
    let signal = if error.is_nil() { None } else {
        let (condition, data) = error.cons_pair()?;
        Some(CommandSignal { condition: condition.display()?, data: data.display()? })
    };
    let message = signal.as_ref().map(|s| format!("{}: {}", s.condition, s.data));
    Ok(CommandOutcome { command, bell: signal.is_some(), signal, message, redisplay: true })
}

/// Keys one at a time, for tests without a host keyboard: a prefix waits
/// for the rest of its sequence, which then reaches `run' as the unread
/// events (t . EVENT) that host keys would be.
#[derive(Default)]
pub struct Commands { pending: Vec<String> }
impl Commands {
    /// True between the keys of a prefix sequence such as C-x.
    pub fn in_prefix(&self) -> bool { !self.pending.is_empty() }
    pub fn dispatch(&mut self, vm: &mut Vm, event: CommandEvent) -> Result<Option<CommandOutcome>, String> {
        if event == CommandEvent::Character(7) { self.pending.clear(); }
        let form = match &event {
            CommandEvent::Character(code) => code.to_string(),
            CommandEvent::Named(name) => {
                if !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)) { return Err("invalid native event name".into()); }
                name.clone()
            }
        };
        self.pending.push(form);
        let sequence = format!("[{}]", self.pending.join(" "));
        if !vm.eval_source(&format!("(keymapp (key-binding {sequence}))"))?.is_nil() { return Ok(None); }
        let events: String = std::mem::take(&mut self.pending).iter().map(|e| format!(" (t . {e})")).collect();
        vm.eval_source(&format!("(setq unread-command-events (append '({events}) unread-command-events))"))?;
        run(vm).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host keyboard holding KEYS, then nothing.
    struct Keys(std::collections::VecDeque<CommandEvent>);
    impl romacs_vm::HostInput for Keys {
        fn input_pending(&mut self) -> bool { !self.0.is_empty() }
        fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
        fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<romacs_vm::HostEvent, String> {
            self.0.pop_front().map(romacs_vm::HostEvent::Key).ok_or_else(|| "keys exhausted".into())
        }
    }

    /// A host keyboard giving these keys, None standing for a pause in
    /// which a background command can finish, then a wake as a completion
    /// gives.
    struct Scripted(std::collections::VecDeque<Option<CommandEvent>>);
    impl romacs_vm::HostInput for Scripted {
        fn input_pending(&mut self) -> bool { false }
        fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
        fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<romacs_vm::HostEvent, String> {
            match self.0.pop_front().ok_or("keys exhausted")? {
                Some(key) => Ok(romacs_vm::HostEvent::Key(key)),
                None => { std::thread::sleep(std::time::Duration::from_millis(500)); Ok(romacs_vm::HostEvent::Background) }
            }
        }
    }

    /// The workspace in a fundamental-mode buffer that logs, from
    /// `pre-command-hook' and `post-command-hook', each command with its
    /// event, keys and `recent-keys'.
    fn logged() -> Vm {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1600, 1000, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        // The workspace has no subr.el `add-hook'.
        vm.eval_source("(erase-buffer) (setq log nil)
            (setq pre-command-hook (list (lambda () (setq log (cons (list 'pre this-command) log)))))
            (setq post-command-hook (list (lambda () (setq log (cons (list 'post this-command last-command-event (this-command-keys-vector)) log)))))").unwrap();
        vm
    }
    fn printed(vm: &mut Vm, source: &str) -> String { vm.eval_source(source).unwrap().display().unwrap() }

    /// Host keys go through `input-decode-map', `local-function-key-map'
    /// and `key-translation-map' as GNU's read_key_sequence takes them, a
    /// translation's prefix waiting for its next key. Pinned GNU, batch,
    /// fundamental-mode, the same keys unread: ((10 [10]) (98 [98])), "\nb".
    #[test]
    fn host_keys_pass_the_translation_maps_as_gnus_reader_does() {
        let mut vm = logged();
        vm.eval_source("(define-key key-translation-map [13] [10]) (define-key key-translation-map [f9 97] [98])").unwrap();
        vm.set_host_input(Some(Box::new(Keys([CommandEvent::Named("return".into()), CommandEvent::Named("f9".into()), CommandEvent::Character(97)].into()))));
        for _ in 0..2 { assert!(run(&mut vm).unwrap().signal.is_none()); }
        assert_eq!(printed(&mut vm, "(list (delq nil (mapcar (lambda (e) (and (eq (car e) 'post) (nthcdr 2 e))) (reverse log))) (buffer-string))"),
                   "(((10 [10]) (98 [98])) \"\\nb\")");
        // The adapter feeds the same reader: physical Return reads as C-j.
        let mut keys = Commands::default();
        let out = keys.dispatch(&mut vm, CommandEvent::Named("return".into())).unwrap().unwrap();
        assert_eq!((out.command.as_deref(), printed(&mut vm, "last-command-event")), (Some("self-insert-command"), "10".into()));
    }

    /// A command the active selection's deletion supersedes still consumes
    /// the prefix argument (delsel.el makes it `ignore'). Pinned GNU, batch,
    /// delete-selection-mode, "hello" with 2..4 selected, C-u C-d a: "halo".
    #[test]
    fn a_superseded_command_consumes_the_prefix_argument() {
        let mut vm = logged();
        vm.eval_source("(setq transient-mark-mode t delete-selection-mode t) (insert \"hello\") (set-mark 2) (goto-char 4)").unwrap();
        vm.set_host_input(Some(Box::new(Keys([CommandEvent::Character(21), CommandEvent::Character(4), CommandEvent::Character(97)].into()))));
        for _ in 0..3 { assert!(run(&mut vm).unwrap().signal.is_none()); }
        assert_eq!(printed(&mut vm, "(list (buffer-string) prefix-arg mark-active)"), "(\"halo\" nil nil)");
    }

    /// Point before a command reaches the undo list when the command's
    /// first change is elsewhere (record_point). Pinned GNU, batch: after
    /// an insertion at 1 from point 5, (1 . 2) then 5.
    #[test]
    fn undo_records_the_point_the_command_started_from() {
        let mut vm = logged();
        vm.eval_source("(buffer-enable-undo) (insert \"abcdefgh\") (undo-boundary) (goto-char 5)
            (global-set-key [f9] (lambda () (interactive) (save-excursion (goto-char 1) (insert \"X\"))))").unwrap();
        vm.set_host_input(Some(Box::new(Keys([CommandEvent::Named("f9".into())].into()))));
        assert!(run(&mut vm).unwrap().signal.is_none());
        assert_eq!(printed(&mut vm, "(let ((l (delq nil (copy-sequence buffer-undo-list)))) (list (point) (car l) (car (cdr l))))"), "(6 (1 . 2) 5)");
    }

    /// A background command that completes while a prefix key waits for
    /// the next one has its callback run then, before that key; one that
    /// completes during a minibuffer read waits for the commands to end.
    #[test]
    fn a_background_completion_runs_while_a_prefix_waits() {
        let mut vm = logged();
        let start = "(romacs-process-start (list \"true\") \"\" default-directory 10
            (lambda (result) (setq log (cons (list 'callback (car result)) log)) nil))";
        vm.eval_source(start).unwrap();
        vm.set_host_input(Some(Box::new(Scripted([Some(CommandEvent::Character(24)), None, Some(CommandEvent::Character(104))].into()))));
        let out = run(&mut vm).unwrap();
        assert_eq!(out.command.as_deref(), Some("mark-whole-buffer"), "{out:?}");
        assert_eq!(printed(&mut vm, "(mapcar #'car (reverse log))"), "(callback pre post)");
        // Started by the command, the job can finish only during its read.
        vm.eval_source(&format!("(setq log nil) (global-set-key [f9] (lambda () (interactive) {start} (setq got (read-string \"x: \"))))")).unwrap();
        vm.set_host_input(Some(Box::new(Scripted([Some(CommandEvent::Named("f9".into())), None, Some(CommandEvent::Character(97)), Some(CommandEvent::Character(13))].into()))));
        assert!(run(&mut vm).unwrap().signal.is_none());
        assert_eq!(printed(&mut vm, "(list got (mapcar #'car (reverse log)) (romacs--process-pending-p))"), "(\"a\" (pre post pre post pre post) t)", "the minibuffer's own commands run too, no callback");
    }

    /// A completion whose wake came during a minibuffer read runs before
    /// the next command's keys though no further wake comes: RET ends the
    /// read and C-x h follows at once.
    #[test]
    fn a_completion_held_by_a_minibuffer_read_runs_before_the_next_keys() {
        let mut vm = logged();
        // Started by the command, the job can finish only during its read.
        vm.eval_source("(global-set-key [f9] (lambda () (interactive)
            (romacs-process-start (list \"true\") \"\" default-directory 10
              (lambda (result) (setq log (cons (list 'callback (car result)) log)) nil))
            (setq got (read-string \"x: \"))))").unwrap();
        vm.set_host_input(Some(Box::new(Scripted([Some(CommandEvent::Named("f9".into())), None, Some(CommandEvent::Character(97)),
            Some(CommandEvent::Character(13)), Some(CommandEvent::Character(24)), Some(CommandEvent::Character(104))].into()))));
        assert!(run(&mut vm).unwrap().signal.is_none());
        assert_eq!(printed(&mut vm, "(list got (assq 'callback log))"), "(\"a\" nil)", "held during the read");
        vm.eval_source("(setq log nil)").unwrap();
        assert_eq!(run(&mut vm).unwrap().command.as_deref(), Some("mark-whole-buffer"));
        assert_eq!(printed(&mut vm, "(list (mapcar #'car (reverse log)) (romacs--process-pending-p))"), "((callback pre post) nil)");
    }

    /// pre-command-hook runs before each command and post-command-hook
    /// after it, also after a command that signalled (command_loop_1
    /// runs it when it resumes).
    #[test]
    fn command_hooks_run_around_each_command() {
        let mut vm = logged();
        let mut keys = Commands::default();
        keys.dispatch(&mut vm, CommandEvent::Character(97)).unwrap();
        keys.dispatch(&mut vm, CommandEvent::Character(2)).unwrap();
        let failed = keys.dispatch(&mut vm, CommandEvent::Character(2)).unwrap().unwrap();
        assert_eq!(failed.signal.map(|s| s.condition), Some("beginning-of-buffer".into()));
        assert_eq!(printed(&mut vm, "(mapcar (lambda (e) (list (car e) (car (cdr e)))) (reverse log))"),
                   "((pre self-insert-command) (post self-insert-command) (pre backward-char) (post backward-char) (pre backward-char) (post backward-char))");
    }

    /// read_char's unread queues: (no-record . EVENT) runs EVENT and keys it
    /// for the command but leaves it out of `recent-keys' (pinned GNU:
    /// [120] and no 120 in recent-keys; (t . 120) and 120 are recorded), and
    /// the queues are read post-input-method, command, input-method (pinned
    /// GNU: 97 98 99).
    #[test]
    fn unread_queues_run_with_gnus_recording_and_order() {
        let mut vm = logged();
        let recent = |vm: &mut Vm| printed(vm, "(length (recent-keys))");
        let before = recent(&mut vm);
        vm.eval_source("(setq unread-command-events (list '(no-record . 120)))").unwrap();
        assert!(unread_pending(&mut vm).unwrap());
        assert_eq!(run(&mut vm).unwrap().command.as_deref(), Some("self-insert-command"));
        assert_eq!((printed(&mut vm, "(nthcdr 2 (car log))"), recent(&mut vm)), ("(120 [120])".into(), before.clone()));
        vm.eval_source("(setq unread-command-events (list '(t . 120)))").unwrap();
        run(&mut vm).unwrap();
        assert_ne!(recent(&mut vm), before, "(t . EVENT) is recorded");
        vm.eval_source("(erase-buffer) (setq unread-input-method-events (list ?c) unread-command-events (list ?b) unread-post-input-method-events (list ?a))").unwrap();
        while unread_pending(&mut vm).unwrap() { run(&mut vm).unwrap(); }
        assert_eq!(printed(&mut vm, "(buffer-string)"), "\"abc\"");
    }
    /// The host's `return' is RET through `local-function-key-map' unless
    /// bound itself; C-j stays C-j.
    #[test]
    fn return_reads_as_ret_unless_bound_and_c_j_stays() {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1600, 1000, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        vm.eval_source("(erase-buffer)").unwrap();
        let mut keys = Commands::default();
        let mut run = |vm: &mut Vm, event| {
            let out = keys.dispatch(vm, event).unwrap().unwrap();
            assert!(out.signal.is_none(), "{out:?}");
            (out.command.unwrap(), vm.eval_source("(list last-command-event (buffer-string))").unwrap().display().unwrap())
        };
        assert_eq!(run(&mut vm, CommandEvent::Named("return".into())), ("romacs-newline".into(), "(13 \"\\n\")".into()));
        assert_eq!(run(&mut vm, CommandEvent::Character(10)), ("self-insert-command".into(), "(10 \"\\n\\n\")".into()));
        vm.eval_source("(global-set-key [return] 'beginning-of-line)").unwrap();
        assert_eq!(run(&mut vm, CommandEvent::Named("return".into())).0, "beginning-of-line");
    }
    #[test]
    fn vertical_goal_column_and_boundaries() {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1600, 1000, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        vm.eval_source("(erase-buffer) (insert \"0123456789\\nx\\nabcdefghij\") (goto-char 8)").unwrap();
        let mut keys = Commands::default();
        for (event, expected) in [(14, 13), (14, 21), (16, 13), (16, 8)] {
            let out = keys.dispatch(&mut vm, CommandEvent::Character(event)).unwrap().unwrap();
            assert!(!out.bell, "{out:?}");
            let point = vm.eval_source("(point)").unwrap();
            assert_eq!(point.display().unwrap(), expected.to_string());
        }
        vm.eval_source("(goto-char 1)").unwrap();
        let out = keys.dispatch(&mut vm, CommandEvent::Character(16)).unwrap().unwrap();
        assert_eq!(out.signal.unwrap().condition, "beginning-of-buffer");
        let point = vm.eval_source("(point)").unwrap();
        assert_eq!(point.display().unwrap(), "1");
        vm.eval_source("(goto-char (point-max))").unwrap();
        let out = keys.dispatch(&mut vm, CommandEvent::Character(14)).unwrap().unwrap();
        assert_eq!(out.signal.unwrap().condition, "end-of-buffer");
    }
    #[test]
    fn automatic_scroll_reveals_only_the_next_row_and_eof_is_stable() {
        let mut vm = Vm::new();
        vm.set_frame_geometry(800, 280, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        vm.eval_source(r#"(romacs-gui-delete-other-windows) (erase-buffer)
            (let ((i 1)) (while (<= i 60) (insert (format "line %d λ\n" i)) (setq i (1+ i))))
            (goto-char (point-min))"#).unwrap();
        let mut keys = Commands::default();
        vm.frame_snapshot(800, 280).unwrap();
        for _ in 0..11 {
            keys.dispatch(&mut vm, CommandEvent::Character(14)).unwrap();
            vm.frame_snapshot(800, 280).unwrap();
        }
        let before = vm.frame_snapshot(800, 280).unwrap();
        keys.dispatch(&mut vm, CommandEvent::Character(14)).unwrap();
        let after = vm.frame_snapshot(800, 280).unwrap();
        // A window publishes from its start's line, "line N λ".
        let start_line = |s: &romacs_layout_input::FrameSnapshot| {
            let w = &s.windows[0].snapshot;
            w.text.utf8[w.start_byte..].split(' ').nth(1).unwrap().parse::<usize>().unwrap()
        };
        assert_eq!(start_line(&after), start_line(&before) + 1, "AUTO_SCROLL_MINIMAL_REVEAL");
        for _ in 0..60 {
            keys.dispatch(&mut vm, CommandEvent::Character(14)).unwrap();
            vm.frame_snapshot(800, 280).unwrap();
        }
        let stable = vm.frame_snapshot(800, 280).unwrap();
        let w = &stable.windows[0].snapshot;
        let expected = (w.text.utf8.clone(), w.start_byte, w.point_byte);
        for _ in 0..5 {
            let outcome = keys.dispatch(&mut vm, CommandEvent::Character(14)).unwrap().unwrap();
            assert_eq!(outcome.signal.unwrap().condition, "end-of-buffer");
            let snapshot = vm.frame_snapshot(800, 280).unwrap();
            let w = &snapshot.windows[0].snapshot;
            assert_eq!((w.text.utf8.clone(), w.start_byte, w.point_byte), expected, "EOF_POINT_AND_VIEWPORT_STABLE");
        }
    }

    #[test]
    fn refused_vertical_motion_preserves_point_on_boundary_lines() {
        let mut vm = Vm::new();
        crate::startup::initialize(&mut vm).unwrap();
        vm.eval_source("(erase-buffer) (insert \"first line\\nlast line\")").unwrap();
        let mut keys = Commands::default();
        for (point, key, condition) in [(4, 16, "beginning-of-buffer"), (15, 14, "end-of-buffer")] {
            vm.eval_source(&format!("(goto-char {point})")).unwrap();
            let outcome = keys.dispatch(&mut vm, CommandEvent::Character(key)).unwrap().unwrap();
            assert_eq!(outcome.signal.unwrap().condition, condition);
            assert_eq!(vm.eval_source("(point)").unwrap().display().unwrap(), point.to_string(), "BOUNDARY_MOTION_PRESERVES_POINT");
        }
    }

    #[test]
    fn switching_windows_restores_independent_points_and_buffers() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.set_frame_geometry(1600, 1000, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        vm.eval_source("(setq left (selected-window)) (goto-char 20)").unwrap();
        let mut keys = Commands::default();
        let chord = |keys: &mut Commands, vm: &mut Vm| {
            assert!(keys.dispatch(vm, CommandEvent::Character(24)).unwrap().is_none());
            let out = keys.dispatch(vm, CommandEvent::Character(111)).unwrap().unwrap();
            assert!(!out.bell, "{out:?}");
        };
        chord(&mut keys, &mut vm);
        vm.eval_source("(setq right (selected-window)) (goto-char 5)").unwrap();
        keys.dispatch(&mut vm, CommandEvent::Character(90)).unwrap();
        chord(&mut keys, &mut vm);
        let result = vm.eval_source("(list (eq (selected-window) left) (point) (window-point right) (eq (window-buffer left) (window-buffer right)))").unwrap();
        assert_eq!(result.display().unwrap(), "(t 20 6 nil)");
        chord(&mut keys, &mut vm);
        let point = vm.eval_source("(point)").unwrap();
        assert_eq!(point.display().unwrap(), "6");
    }
    #[test]
    fn native_eval_sequence_and_prefix_reach_the_real_echo_window() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.set_frame_geometry(1600, 1000, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        crate::eval_last_sexp::install(&mut vm).unwrap();
        vm.eval_source("(insert \"(+ 20 22)\")").unwrap();
        let mut keys = Commands::default();
        for key in [24, 5] {
            keys.dispatch(&mut vm, CommandEvent::Character(key)).unwrap();
            crate::eval_last_sexp::after_command(&mut vm).unwrap();
        }
        let echo = vm.eval_source("(with-current-buffer (window-buffer (minibuffer-window)) (buffer-string))").unwrap();
        assert_eq!(echo.display().unwrap(), "\"42\"");
        for key in [21, 24, 5] {
            keys.dispatch(&mut vm, CommandEvent::Character(key)).unwrap();
            crate::eval_last_sexp::after_command(&mut vm).unwrap();
        }
        let tail = vm.eval_source("(buffer-substring-no-properties (- (point) 2) (point))").unwrap();
        assert_eq!(tail.display().unwrap(), "\"42\"");
    }
    #[test]
    fn native_prefix_uses_vm_keymap_and_command_loop() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.set_frame_geometry(1600, 1000, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        let mut keys = Commands::default();
        assert!(keys.dispatch(&mut vm, CommandEvent::Character(24)).unwrap().is_none());
        let out = keys.dispatch(&mut vm, CommandEvent::Character(49)).unwrap().unwrap();
        assert_eq!(out.command.as_deref(), Some("romacs-gui-delete-other-windows"));
        assert!(!out.bell, "{out:?}");
        assert!(keys.dispatch(&mut vm, CommandEvent::Character(24)).unwrap().is_none());
        let out = keys.dispatch(&mut vm, CommandEvent::Character(3)).unwrap().unwrap();
        assert!(!out.bell, "{out:?}");
        assert_ne!(vm.eval_source("romacs-gui-quit-requested").unwrap(), Fixed::NIL);
    }
    /// A user's journey through the real key path: visit a file, type into
    /// it, C-x C-s. The bytes on disk must be exactly the edited text.
    #[test]
    fn visiting_typing_and_c_x_c_s_save_the_file_byte_identically() {
        let dir = std::env::temp_dir().join(format!("romacs-gui-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.org");
        std::fs::write(&file, "* Heading\nλ body\n").unwrap();
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.set_frame_geometry(1600, 1000, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        let path = serde_json::to_string(file.to_str().unwrap()).unwrap();
        vm.eval_source(&format!("(romacs-gui-visit-file {path} \"notes.org\")")).unwrap();
        let shown = vm.eval_source(&format!(
            "(with-current-buffer (window-buffer (selected-window)) (list (equal buffer-file-name {path}) (equal (buffer-string) \"* Heading\\nλ body\\n\") (point)))")).unwrap();
        assert_eq!(shown.display().unwrap(), "(t t 1)");
        let mut keys = Commands::default();
        for key in [5, 32, 0x3bb, 33] {   // C-e, then type " λ!" at the end of the heading
            vm.eval_source("(set-buffer (window-buffer (selected-window)))").unwrap();
            let out = keys.dispatch(&mut vm, CommandEvent::Character(key)).unwrap().unwrap();
            assert!(!out.bell && out.signal.is_none(), "{out:?}");
        }
        vm.eval_source("(set-buffer (window-buffer (selected-window)))").unwrap();
        assert!(keys.dispatch(&mut vm, CommandEvent::Character(24)).unwrap().is_none());
        let out = keys.dispatch(&mut vm, CommandEvent::Character(19)).unwrap().unwrap();
        assert!(out.signal.is_none(), "C-x C-s signalled: {out:?}");
        assert_eq!(out.command.as_deref(), Some("romacs-gui-save-buffer"));
        assert_eq!(std::fs::read(&file).unwrap(), "* Heading λ!\nλ body\n".as_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// Pinned GNU's `prin1' of EXPRESSION's value as a list of characters,
    /// evaluated with `-Q --batch' in DIR under a private HOME and UTF-8 locale.
    fn gnu_value(dir: &std::path::Path, home: &std::path::Path, expression: &str) -> String {
        let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
        let output = std::process::Command::new(&oracle).current_dir(dir)
            .env("HOME", home).env("LANG", "C.UTF-8").env_remove("LC_ALL")
            .args(["-Q", "--batch", "--eval", &format!("(prin1 (append (prin1-to-string {expression}) nil))")])
            .output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    }
    /// Type TEXT through the real key path, at the start of the first line
    /// or at its end, then C-x C-s; each command's outcome in order.
    fn type_and_save(vm: &mut Vm, at_start: bool, text: &str) -> Vec<CommandOutcome> {
        let motion: &[i64] = if at_start { &[1] } else { &[1, 5] };
        let keys = motion.iter().copied().chain(text.chars().map(|c| i64::from(u32::from(c)))).chain([24, 19]);
        let mut commands = Commands::default();
        let mut outcomes = Vec::new();
        for key in keys {
            vm.eval_source("(set-buffer (window-buffer (selected-window)))").unwrap();
            outcomes.extend(commands.dispatch(vm, CommandEvent::Character(key)).unwrap());
        }
        outcomes
    }
    /// The same journey over UTF-8, CRLF, ASCII and Latin-1 files (with and
    /// without a coding cookie, agreeing with the bytes or not) and a UTF-8
    /// file with a BOM: the decoded text, `last-coding-system-used' and
    /// `buffer-file-coding-system' after the visit and after C-x C-s, and the
    /// bytes on disk are what pinned GNU's find-file-noselect, insert and
    /// save-buffer give for a copy of the same file. Typing λ into the
    /// Latin-1 file saves it as UTF-8 (`select-safe-coding-system'), and an X
    /// typed before the BOM's text stays after the signature.
    #[test]
    fn coding_journey_visits_edits_and_saves_as_pinned_gnu() {
        let dir = std::env::temp_dir().join(format!("romacs-gui-coding-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (ours, gnu, home) = (dir.join("romacs"), dir.join("gnu"), dir.join("home"));
        for d in [&ours, &gnu, &home] { std::fs::create_dir_all(d).unwrap(); }
        let files: [(&str, &[u8], bool, &str); 8] = [
            ("utf8.txt", "* Heading\nλ body\n".as_bytes(), false, " é!"),
            ("crlf.txt", "* Heading\r\nλ body\r\n".as_bytes(), false, " é!"),
            ("ascii.txt", b"hi\n", false, " é!"),
            ("cookie.txt", b"-*- coding: latin-1 -*-\ncaf\xe9\n", false, " é!"),
            ("cookie-utf8.txt", b"-*- coding: latin-1 -*-\ncaf\xc3\xa9\n", false, " é!"),
            // 0x93 and 0x94 are C1 bytes mule-conf.el's latin-extra-code-table admits.
            ("latin.txt", b"caf\xe9 \x93q\x94\n", false, " é!"),
            ("lambda.txt", b"caf\xe9\n", false, "λ"),
            ("bom.txt", b"\xef\xbb\xbfcaf\xc3\xa9\n", true, "X"),
        ];
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.set_frame_geometry(1600, 1000, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        let state = "(with-current-buffer (window-buffer (selected-window)) (list (buffer-string) last-coding-system-used buffer-file-coding-system))";
        for (name, bytes, at_start, text) in files {
            std::fs::write(ours.join(name), bytes).unwrap();
            std::fs::write(gnu.join(name), bytes).unwrap();
            let path = |d: &std::path::Path| serde_json::to_string(d.join(name).to_str().unwrap()).unwrap();
            let motion = if at_start { "" } else { "(end-of-line)" };
            let expected = gnu_value(&gnu, &home, &format!(
                "(with-current-buffer (find-file-noselect {}) (let ((visited (list (buffer-string) last-coding-system-used buffer-file-coding-system))) (goto-char (point-min)) {motion} (insert {text:?}) (save-buffer) (list visited (cdr (list (buffer-string) last-coding-system-used buffer-file-coding-system)))))",
                path(&gnu)));
            vm.eval_source(&format!("(romacs-gui-visit-file {} {name:?})", path(&ours))).unwrap();
            let visited = vm.eval_source(&format!("(prin1-to-string {state})")).unwrap().display().unwrap();
            for out in type_and_save(&mut vm, at_start, text) {
                assert!(!out.bell && out.signal.is_none(), "{name}: {out:?}");
            }
            let saved = vm.eval_source(&format!("(prin1-to-string (cdr {state}))")).unwrap().display().unwrap();
            let actual = vm.eval_source(&format!("(append (format \"(%s %s)\" {visited} {saved}) nil)")).unwrap().display().unwrap();
            assert_eq!(actual, expected, "{name}");
            assert_eq!(std::fs::read(ours.join(name)).unwrap(), std::fs::read(gnu.join(name)).unwrap(), "{name} bytes");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// Where no default coding system encodes the text (λ typed under a
    /// latin-1 cookie) pinned GNU asks through
    /// `select-safe-coding-system-interactively'. The workspace refuses
    /// instead: C-x C-s signals `romacs-unsafe-coding' with the unsafe
    /// defaults and safe systems GNU would offer (those the workspace
    /// defines), the file keeps its bytes and the buffer stays modified.
    #[test]
    fn coding_save_refuses_where_pinned_gnu_would_ask() {
        let dir = std::env::temp_dir().join(format!("romacs-gui-unsafe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (ours, gnu, home) = (dir.join("romacs"), dir.join("gnu"), dir.join("home"));
        for d in [&ours, &gnu, &home] { std::fs::create_dir_all(d).unwrap(); }
        let (name, bytes) = ("cookie.txt", b"-*- coding: latin-1 -*-\ncaf\xe9\n");
        std::fs::write(ours.join(name), bytes).unwrap();
        std::fs::write(gnu.join(name), bytes).unwrap();
        let path = |d: &std::path::Path| serde_json::to_string(d.join(name).to_str().unwrap()).unwrap();
        let asked = gnu_value(&gnu, &home, &format!(
            "(with-current-buffer (find-file-noselect {}) (goto-char (point-min)) (end-of-line) (insert \"λ\") (defalias 'select-safe-coding-system-interactively (lambda (_from _to codings unsafe &rest _) (throw 'asked (list unsafe codings)))) (list (catch 'asked (save-buffer) 'saved) (buffer-modified-p)))",
            path(&gnu)));
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.set_frame_geometry(1600, 1000, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        vm.eval_source(&format!("(romacs-gui-visit-file {} {name:?})", path(&ours))).unwrap();
        let outcomes = type_and_save(&mut vm, false, "λ");
        let (save, typed) = outcomes.split_last().unwrap();
        assert!(typed.iter().all(|out| !out.bell && out.signal.is_none()), "{typed:?}");
        let signal = save.signal.as_ref().expect("C-x C-s refuses");
        assert_eq!(save.command.as_deref(), Some("romacs-gui-save-buffer"));
        let expected = vm.eval_source(&format!(
            "(let* ((gnu (car (read-from-string (apply #'string '{asked})))) (asked (car gnu))) (list (list 'romacs-unsafe-coding (car asked) (delq nil (mapcar (lambda (c) (and (coding-system-p c) c)) (car (cdr asked))))) (car (cdr gnu))))")).unwrap().display().unwrap();
        let actual = vm.eval_source(&format!(
            "(list (cons '{} '{}) (with-current-buffer (window-buffer (selected-window)) (buffer-modified-p)))", signal.condition, signal.data)).unwrap().display().unwrap();
        assert_eq!(actual, expected);
        assert_eq!(std::fs::read(ours.join(name)).unwrap(), bytes, "the refused save leaves the file");
        assert_eq!(std::fs::read(gnu.join(name)).unwrap(), bytes, "GNU asked before writing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
