//! The GUI's own keys over GNU's Lisp (`--loadup'): Ctrl+m, Ctrl+i and
//! Ctrl+[ are keys of their own, RET, TAB and ESC where nothing binds them,
//! and the Tab key is the function key `tab' (ledger/divergences/VM2-GUI-KEYS).
//! Every key here is made by the host's own translation (`native.rs').
use romacs_display_runtime::{backend::wgpu::*, host_types::FrontendKey, thread_comm::InputEvent};
use romacs_vm::{CommandEvent, Vm};
use romacs_vm_gui::{command, native, startup};
#[path = "support/home.rs"]
mod home;

struct Keyboard(std::collections::VecDeque<CommandEvent>);
impl romacs_vm::HostInput for Keyboard {
    fn input_pending(&mut self) -> bool { !self.0.is_empty() }
    fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<romacs_vm::HostEvent, String> {
        self.0.pop_front().map(romacs_vm::HostEvent::Key).ok_or_else(|| "keys exhausted".into())
    }
}

fn printed(vm: &mut Vm, s: &str) -> String { vm.eval_source(s).unwrap().display().unwrap() }

/// The event the host makes of KEY pressed with MODIFIERS.
fn press(key: FrontendKey, modifiers: u32) -> CommandEvent {
    native::translate(InputEvent::Key { key, modifiers, pressed: true, emacs_frame_id: 0 }).unwrap()
}
fn ctrl(c: char) -> CommandEvent { press(FrontendKey::Character(c), ROMACS_CTRL_MASK) }
fn plain(c: char) -> CommandEvent { press(FrontendKey::Character(c), 0) }
const RETURN: FrontendKey = FrontendKey::Keysym(0xff0d);
const TAB: FrontendKey = FrontendKey::Keysym(0xff09);

/// Run KEYS through the command loop, one command each of COMMANDS; the
/// commands run and their signals.
fn type_keys(vm: &mut Vm, keys: Vec<CommandEvent>, commands: usize) -> Vec<String> {
    vm.set_host_input(Some(Box::new(Keyboard(keys.into()))));
    let out = (0..commands).map(|_| {
        let outcome = command::run(vm).unwrap();
        match outcome.signal {
            Some(s) => format!("{:?} signalled {} {}", outcome.command, s.condition, s.data),
            None => outcome.command.unwrap_or_default(),
        }
    }).collect();
    vm.set_host_input(None);
    out
}

#[test]
fn gui_control_keys_are_their_own_keys_and_fall_back_to_ret_tab_and_esc() {
    std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        home::private_home(&mut vm, "gui-keys");
        let started = startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        vm.eval_source("(setq inhibit-startup-screen t)").unwrap();
        startup::finish_loadup(&mut vm, &started).unwrap();
        vm.eval_source("(switch-to-buffer (get-buffer-create \"gui-keys\")) (emacs-lisp-mode)
                        (defun gk-own () (interactive) (setq gk-ran (cons last-command-event gk-ran)))
                        (setq gk-ran nil)").unwrap();
        assert_eq!(press(TAB, 0), CommandEvent::Named("tab".into()));
        assert_eq!(ctrl('m'), CommandEvent::Named("C-m".into()));

        // Unbound, Ctrl+m is RET as Return is: `newline', its event 13.
        assert_eq!(type_keys(&mut vm, vec![press(RETURN, 0), ctrl('m')], 2), ["newline", "newline"]);
        assert_eq!(printed(&mut vm, "(list (buffer-string) last-command-event (this-command-keys-vector))"), "(\"\\n\\n\" 13 [13])");

        // Bound with the syntax `describe-key' shows, Ctrl+m is its own key;
        // Return stays RET.
        vm.eval_source("(keymap-set global-map \"C-<m>\" #'gk-own)").unwrap();
        assert_eq!(type_keys(&mut vm, vec![ctrl('m'), press(RETURN, 0)], 2), ["gk-own", "newline"]);
        assert_eq!(printed(&mut vm, "(list gk-ran (key-description (where-is-internal 'gk-own nil t)))"), "((C-m) \"C-<m>\")");

        // Ctrl+i and the Tab key unbound indent, as TAB; bound, Ctrl+i is
        // its own key and Tab still indents.
        vm.eval_source("(erase-buffer) (insert \"(progn\\nx)\") (setq gk-ran nil)").unwrap();
        assert_eq!(type_keys(&mut vm, vec![ctrl('i'), press(TAB, 0)], 2), ["indent-for-tab-command", "indent-for-tab-command"]);
        assert_eq!(printed(&mut vm, "(list (buffer-string) last-command-event)"), "(\"(progn\\n  x)\" 9)");
        vm.eval_source("(keymap-set emacs-lisp-mode-map \"C-<i>\" #'gk-own)").unwrap();
        assert_eq!(type_keys(&mut vm, vec![ctrl('i'), press(TAB, 0)], 2), ["gk-own", "indent-for-tab-command"]);

        // Ctrl+[ unbound is ESC, the Meta prefix: C-[ f is M-f.
        vm.eval_source("(goto-char (point-min))").unwrap();
        assert_eq!(type_keys(&mut vm, vec![ctrl('['), plain('f')], 1), ["forward-word"]);
        // Meta+Tab and Ctrl+Meta+i are M-TAB, completion in Lisp: M-tab
        // through the alternatives a window-system frame installs
        // (romacs-gui-loadup.el), which this process, with no display, does
        // as that startup does.
        vm.eval_source("(x-setup-function-keys (selected-frame))").unwrap();
        let meta_i = press(FrontendKey::Character('i'), ROMACS_CTRL_MASK | ROMACS_META_MASK);
        assert_eq!(meta_i, CommandEvent::Named("C-M-i".into()));
        assert_eq!(type_keys(&mut vm, vec![meta_i, press(TAB, ROMACS_META_MASK)], 2), ["completion-at-point", "completion-at-point"]);
        // Ctrl+Space is C-SPC, which sets the mark.
        assert_eq!(type_keys(&mut vm, vec![ctrl(' ')], 1), ["set-mark-command"]);

        // A prompt reading a character takes Ctrl+m as RET, as it takes Return.
        assert_eq!(printed(&mut vm, "(list (progn (setq unread-command-events (list 'C-m)) (read-char))
                                          (progn (setq unread-command-events (list 'C-M-m)) (read-char))
                                          (progn (setq unread-command-events (list 'C-m)) (read-key))
                                          (progn (setq unread-command-events (list 'C-m)) (read-event)))"),
                   "(13 134217741 13 C-m)");

        // In the minibuffer, Ctrl+m ends the read as RET does.
        vm.eval_source("(setq gk-read nil)").unwrap();
        let mut keys = vec![press(FrontendKey::Character(':'), ROMACS_META_MASK)];
        keys.extend("(setq gk-read 1)".chars().map(plain));
        keys.push(ctrl('m'));
        assert_eq!(type_keys(&mut vm, keys, 1), ["eval-expression"]);
        assert_eq!(printed(&mut vm, "gk-read"), "1");

        // A keyboard macro keeps the key and replays it.
        vm.eval_source("(keymap-unset global-map \"C-<m>\") (erase-buffer)").unwrap();
        let keys = vec![ctrl('x'), plain('('), plain('a'), ctrl('m'), ctrl('x'), plain(')'), ctrl('x'), plain('e')];
        assert_eq!(type_keys(&mut vm, keys, 5), ["kmacro-start-macro", "self-insert-command", "newline", "kmacro-end-macro", "kmacro-end-and-call-macro"]);
        assert_eq!(printed(&mut vm, "(list (buffer-string) last-kbd-macro)"), "(\"a\\na\\n\" [97 C-m])");

        // Every GUI key, with every modifier, reads back from its description
        // in the syntax `keymap-set' takes, alone and after a prefix.
        assert_eq!(printed(&mut vm, "(let (bad)
            (dolist (pair (romacs-gui-control-key-events) bad)
              (let* ((key (vector (car pair))) (text (key-description key)) (prefixed (key-description key [24])))
                (unless (and (key-valid-p text) (equal (key-parse text) key)
                             (key-valid-p prefixed) (equal (key-parse prefixed) (vconcat [24] key)))
                  (push text bad)))))"), "nil");
    }).unwrap().join().unwrap();
}
