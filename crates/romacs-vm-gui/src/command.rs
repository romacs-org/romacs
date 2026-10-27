//! Native key-sequence accumulation. VM2 still resolves every binding and runs
//! every command through command_event; only owned event spellings live here.
use romacs_vm::{CommandEvent, CommandOutcome, Fixed, Vm};
use romacs_display_runtime::thread_comm::InputEvent;

pub fn unwrap_event(event: InputEvent) -> InputEvent {
    match event {
        InputEvent::Tracked { event, .. } | InputEvent::Observed { event, .. } => unwrap_event(*event),
        event => event,
    }
}

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
        let prefix = vm.eval_source(&format!("(keymapp (key-binding {sequence}))"))? != Fixed::NIL;
        if prefix {
            vm.record_recent_event(&event);
            return Ok(None);
        }
        let multi = self.pending.len() > 1;
        self.pending.clear();
        if !multi { return vm.command_event(event).map(Some); }
        vm.record_recent_event(&event);
        // command_event currently consumes complete single events. A private
        // named transport event projects the already resolved full key sequence
        // into that existing command path, without retaining a heap index.
        vm.eval_source(&format!("(define-key (current-global-map) [romacs-gui-sequence] (key-binding {sequence} nil t))"))?;
        vm.command_event(CommandEvent::Named("romacs-gui-sequence".into())).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let start_line = |s: &romacs_layout_input::FrameSnapshot| {
            let w = &s.windows[0].snapshot;
            w.text.utf8[..w.start_byte].bytes().filter(|b| *b == b'\n').count()
        };
        assert_eq!(start_line(&after), start_line(&before) + 1, "AUTO_SCROLL_MINIMAL_REVEAL");
        for _ in 0..60 {
            keys.dispatch(&mut vm, CommandEvent::Character(14)).unwrap();
            vm.frame_snapshot(800, 280).unwrap();
        }
        let stable = vm.frame_snapshot(800, 280).unwrap();
        let w = &stable.windows[0].snapshot;
        let expected = (w.start_byte, w.point_byte);
        for _ in 0..5 {
            let outcome = keys.dispatch(&mut vm, CommandEvent::Character(14)).unwrap().unwrap();
            assert_eq!(outcome.signal.unwrap().condition, "end-of-buffer");
            let snapshot = vm.frame_snapshot(800, 280).unwrap();
            let w = &snapshot.windows[0].snapshot;
            assert_eq!((w.start_byte, w.point_byte), expected, "EOF_POINT_AND_VIEWPORT_STABLE");
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
    /// The owner's journey through the real key path: visit a file, type into
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
