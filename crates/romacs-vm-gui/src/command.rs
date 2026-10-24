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
        if prefix { return Ok(None); }
        let multi = self.pending.len() > 1;
        self.pending.clear();
        if !multi { return vm.command_event(event).map(Some); }
        // command_event currently consumes complete single events. A private
        // named transport event projects the already resolved full key sequence
        // into that existing command path, without retaining a heap index.
        vm.eval_source(&format!("(define-key (current-global-map) [romacs-gui-sequence] (key-binding {sequence}))"))?;
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
}
