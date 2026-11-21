//! romacs-term over GNU's Lisp (`--loadup'): the keys a terminal routes
//! through the command loop, with GNU's minibuffer, keymaps and commands.
use romacs_vm_gui::{command, startup};
use romacs_vm::{CommandEvent, TerminalHost, TerminalSnapshot, Vm};
use std::{cell::RefCell, rc::Rc};
#[path = "support/home.rs"]
mod home;

#[derive(Default)]
struct State { sent: Vec<Vec<u8>>, alternate: bool, bracketed: bool }
#[derive(Clone)]
struct Host(Rc<RefCell<State>>);
impl TerminalHost for Host {
    fn create(&mut self, _: &str, _: &str, _: u16, _: u16) -> Result<u64, String> { Ok(1) }
    fn send(&mut self, _: u64, bytes: &[u8]) -> Result<(), String> { self.0.borrow_mut().sent.push(bytes.to_vec()); Ok(()) }
    fn resize(&mut self, _: u64, _: u16, _: u16) -> Result<(), String> { Ok(()) }
    fn close(&mut self, _: u64) -> Result<(), String> { Ok(()) }
    fn input_state(&mut self, _: u64) -> Result<romacs_vm::TerminalInputState, String> {
        let s = self.0.borrow();
        Ok(romacs_vm::TerminalInputState { running: true, application_cursor: false, bracketed_paste: s.bracketed, alternate_screen: s.alternate, shell_foreground: true })
    }
    fn changed_sessions(&mut self) -> Vec<u64> { vec![] }
    fn snapshot(&mut self, _: u64) -> Result<TerminalSnapshot, String> {
        let s = self.0.borrow();
        Ok(TerminalSnapshot { revision: 1, text: "$ ".into(), styles: std::sync::Arc::from([]), default_colors: [None; 2], cursor_byte: 2,
            screen_start_byte: 0, columns: 80, rows: 24, alternate_screen: s.alternate, running: true, application_cursor: false, bracketed_paste: s.bracketed })
    }
}
struct Keyboard(std::collections::VecDeque<CommandEvent>);
impl romacs_vm::HostInput for Keyboard {
    fn input_pending(&mut self) -> bool { !self.0.is_empty() }
    fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<romacs_vm::HostEvent, String> {
        self.0.pop_front().map(romacs_vm::HostEvent::Key).ok_or_else(|| "keys exhausted".into())
    }
}
fn printed(vm: &mut Vm, s: &str) -> String { vm.eval_source(s).unwrap().display().unwrap() }
fn chars(text: &str) -> Vec<CommandEvent> { text.chars().map(|c| CommandEvent::Character(c as i64)).collect() }
/// Type KEYS through the command loop, COMMANDS commands of them.
fn type_keys(vm: &mut Vm, keys: Vec<CommandEvent>, commands: usize) -> Vec<Option<String>> {
    vm.set_host_input(Some(Box::new(Keyboard(keys.into()))));
    let out = (0..commands).map(|_| command::run(vm).unwrap().signal.map(|s| format!("{} {}", s.condition, s.data))).collect();
    vm.set_host_input(None);
    out
}

#[test]
fn a_terminal_over_gnu_lisp_routes_keys_and_keeps_the_minibuffer_s() {
    std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
        let state = Rc::new(RefCell::new(State::default()));
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        vm.set_terminal_host(Some(Box::new(Host(state.clone())))).unwrap();
        home::private_home(&mut vm, "terminal-loadup");
        let started = startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        vm.eval_source("(setq inhibit-startup-screen t)").unwrap();
        startup::finish_loadup(&mut vm, &started).unwrap();
        // The terminal host here is a stand-in in this process: no keeper.
        vm.eval_source("(setq romacs-keeper-enabled nil)").unwrap();
        vm.eval_source("(romacs-term \"/bin/sh\")").unwrap();
        vm.poll_terminal_output().unwrap();
        let signals = type_keys(&mut vm, chars("ab"), 2);
        assert!(signals.iter().all(Option::is_none), "{signals:?}");
        assert_eq!(std::mem::take(&mut state.borrow_mut().sent), vec![b"a".to_vec(), b"b".to_vec()], "KEYS_REACH_THE_PROGRAM");
        vm.eval_source("(romacs-complete-mode -1) (fido-vertical-mode 1)").unwrap();
        let mut keys = vec![CommandEvent::Character(24), CommandEvent::Character(6)];
        keys.extend(chars("ls"));
        keys.push(CommandEvent::Character(7));
        // C-x C-f in the terminal, then C-g in GNU's minibuffer.
        let signals = type_keys(&mut vm, keys, 1);
        assert_eq!(signals, vec![Some("minibuffer-quit nil".into())]);
        assert_eq!(printed(&mut vm, "(list (buffer-name) (minibuffer-depth) (eq (current-local-map) romacs-term-mode-map) romacs-term--latches)"),
                   "(\"*romacs-term*\" 0 t nil)", "THE_MINIBUFFER_S_KEYS_ARE_EMACS_S");
        // Plain passthrough keys skip the command loop under GNU's Lisp too.
        assert!(vm.terminal_passthrough_key(&CommandEvent::Character('c' as i64)).unwrap(), "GNU_LISP_S_DEFAULTS_KEEP_THE_FAST_PATH");
        assert!(!vm.terminal_passthrough_key(&CommandEvent::Character(25)).unwrap());
        // An init's global command hooks are skipped; a buffer-local one is a
        // package watching this buffer, so its keys go through the loop.
        vm.eval_source("(add-hook 'pre-command-hook #'ignore) (add-hook 'post-command-hook #'ignore)").unwrap();
        assert!(vm.terminal_passthrough_key(&CommandEvent::Character('c' as i64)).unwrap(), "GLOBAL_HOOKS_KEEP_THE_FAST_PATH");
        vm.eval_source("(add-hook 'post-command-hook #'identity nil t)").unwrap();
        assert!(!vm.terminal_passthrough_key(&CommandEvent::Character('c' as i64)).unwrap(), "A_BUFFER_LOCAL_HOOK_DECLINES_IT");
        vm.eval_source("(remove-hook 'post-command-hook #'identity t)").unwrap();
        assert!(vm.terminal_passthrough_key(&CommandEvent::Character('c' as i64)).unwrap(), "ONLY_OUR_LOCAL_HOOK_LEFT");
        state.borrow_mut().sent.clear();
        // The GUI's own control keys (VM2-GUI-KEYS) take the fast path and
        // reach the program as the control characters a terminal has for
        // them, as the Tab key does; through the command loop too.
        let key = |c: char, modifiers: u32| romacs_vm_gui::native::translate(romacs_display_runtime::thread_comm::InputEvent::Key {
            key: romacs_display_runtime::host_types::FrontendKey::Character(c), modifiers, pressed: true, emacs_frame_id: 0 }).unwrap();
        use romacs_display_runtime::backend::wgpu::{ROMACS_CTRL_MASK as C, ROMACS_META_MASK as M};
        let keys = vec![key('m', C), key('i', C), key('[', C), key('m', C | M), key(' ', C), key('?', C),
                        CommandEvent::Named("tab".into())];
        let bytes = [&b"\r"[..], b"\t", b"\x1b", b"\x1b\r", b"\0", b"\x7f", b"\t"].map(<[u8]>::to_vec);
        for event in &keys {
            assert!(vm.terminal_passthrough_key(event).unwrap(), "GUI_KEY_ON_THE_FAST_PATH {event:?}");
        }
        assert_eq!(std::mem::take(&mut state.borrow_mut().sent), bytes, "GUI_KEYS_SEND_THEIR_CONTROL_CHARACTERS");
        let signals = type_keys(&mut vm, keys, 7);
        assert!(signals.iter().all(Option::is_none), "{signals:?}");
        assert_eq!(std::mem::take(&mut state.borrow_mut().sent), bytes, "AND_THROUGH_THE_COMMAND_LOOP");
        // An exception keeps a key with Emacs by either spelling: "TAB" the
        // Tab key and Ctrl+i, "C-<m>" Ctrl+m alone.
        vm.eval_source("(let ((romacs-term-keymap-exceptions (append '(\"TAB\" \"C-<m>\") romacs-term-keymap-exceptions))
                          (romacs-term-shell-exceptions nil))
                      (romacs-term-update-keymaps))").unwrap();
        for (event, sent) in [("tab", false), ("C-i", false), ("C-m", false), ("return", true)] {
            assert_eq!(vm.terminal_passthrough_key(&CommandEvent::Named(event.into())).unwrap(), sent, "EXCEPTION_BY_EITHER_SPELLING {event}");
        }
        vm.eval_source("(romacs-term-update-keymaps)").unwrap();
        state.borrow_mut().sent.clear();
        // While a minibuffer reads, even with the terminal's window selected,
        // the keys are the minibuffer's.
        vm.eval_source("(setq romacs--minibuf-level 1)").unwrap();
        assert!(!vm.terminal_passthrough_key(&CommandEvent::Character('c' as i64)).unwrap(), "A_MINIBUFFER_READ_KEEPS_ITS_KEYS");
        vm.eval_source("(setq romacs--minibuf-level 0)").unwrap();
        // M-x reads its command in the minibuffer, whose keys are Emacs's.
        vm.eval_source("(setq ran nil) (defun tl-run () (interactive) (setq ran (list (buffer-name) romacs-term--latches)))").unwrap();
        let mut keys = vec![CommandEvent::Character(0x8000000 | 'x' as i64)];
        keys.extend(chars("tl-run"));
        keys.push(CommandEvent::Named("return".into()));
        let signals = type_keys(&mut vm, keys, 1);
        assert!(signals[0].is_none(), "{signals:?}");
        assert_eq!(printed(&mut vm, "ran"), "(\"*romacs-term*\" nil)", "M_X_RUNS_ITS_COMMAND");
        assert!(state.borrow().sent.is_empty(), "THE_MINIBUFFER_S_KEYS_ARE_NOT_SENT");
    }).unwrap().join().unwrap();
}


