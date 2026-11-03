#[path = "../src/command.rs"] mod command;
#[path = "../src/echo.rs"] mod echo;
#[path = "../src/eval_last_sexp.rs"] mod eval_last_sexp;
#[path = "../src/loadup.rs"] mod loadup;
#[path = "../src/native.rs"] mod native;
#[path = "../src/presentation.rs"] mod presentation;
#[path = "../src/startup.rs"] mod startup;
use romacs_vm::{CommandEvent, TerminalHost, TerminalSnapshot, Vm};
use std::{cell::RefCell, rc::Rc};
fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> { vm.frame_snapshot(width,height) }

thread_local! {
    /// The foreground of the first cell this thread's terminal shows.
    static FOREGROUND: std::cell::Cell<romacs_vm::TerminalColor> = const { std::cell::Cell::new(romacs_vm::TerminalColor::Rgb([200,30,20])) };
}
/// The fake terminal: what it shows and the bytes it was sent.
struct State { text: String, cursor: usize, screen: usize, revision: u64, alternate: bool, bracketed: bool, pending: bool, sent: Vec<Vec<u8>> }
type Shared = Rc<RefCell<State>>;
impl State {
    /// Show TEXT with the cursor at its end, from SCREEN, as a publication.
    fn show(&mut self, text: &str, screen: usize) { self.text = text.into(); self.cursor = text.len(); self.screen = screen; self.revision += 1; self.pending = true; }
}
#[derive(Clone)]
struct Host(Shared);
impl TerminalHost for Host {
    fn create(&mut self, _: &str, _: &str, _: u16, _: u16) -> Result<u64,String> { Ok(1) }
    fn send(&mut self, _: u64, bytes: &[u8]) -> Result<(),String> { self.0.borrow_mut().sent.push(bytes.to_vec()); Ok(()) }
    fn resize(&mut self, _: u64, _: u16, _: u16) -> Result<(),String> { Ok(()) }
    fn close(&mut self, _: u64) -> Result<(),String> { Ok(()) }
    fn input_state(&mut self, _: u64) -> Result<romacs_vm::TerminalInputState,String> {
        let s = self.0.borrow();
        Ok(romacs_vm::TerminalInputState { running:true, application_cursor:false, bracketed_paste:s.bracketed, alternate_screen:s.alternate })
    }
    fn changed_sessions(&mut self) -> Vec<u64> { if std::mem::take(&mut self.0.borrow_mut().pending) { vec![1] } else { vec![] } }
    fn snapshot(&mut self, _: u64) -> Result<TerminalSnapshot,String> {
        let s = self.0.borrow();
        Ok(TerminalSnapshot { revision:s.revision, text:s.text.as_str().into(), styles:std::sync::Arc::from([romacs_vm::TerminalStyleRun { bytes:0..1, style:romacs_vm::TerminalStyle { foreground:FOREGROUND.get(), ..Default::default() } }]),
            default_colors:[None;2], cursor_byte:s.cursor, screen_start_byte:s.screen, columns:80, rows:24, alternate_screen:s.alternate, running:true, application_cursor:false, bracketed_paste:s.bracketed })
    }
}
/// A host keyboard: each key, and what the terminal does while the key is
/// awaited, so it is live when the key arrives.
type Before = Option<Box<dyn Fn(&mut State)>>;
struct Keyboard(std::collections::VecDeque<(CommandEvent, Before)>, Shared);
impl romacs_vm::HostInput for Keyboard {
    fn input_pending(&mut self) -> bool { !self.0.is_empty() }
    fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<romacs_vm::HostEvent, String> {
        let (key, before) = self.0.pop_front().ok_or("keys exhausted")?;
        if let Some(before) = before { before(&mut self.1.borrow_mut()); }
        Ok(romacs_vm::HostEvent::Key(key))
    }
}
fn setup() -> (Vm,Shared) {
    let state=Rc::new(RefCell::new(State { text:"$ ".into(), cursor:2, screen:0, revision:1, alternate:false, bracketed:false, pending:false, sent:vec![] }));
    let mut vm=Vm::new();
    vm.set_collection_floor(1); vm.set_frame_geometry(1000,700,20,10).unwrap();
    vm.set_terminal_host(Some(Box::new(Host(state.clone())))).unwrap();
    startup::initialize_files(&mut vm,&[]).unwrap(); (vm,state)
}
fn printed(vm:&mut Vm,s:&str)->String {vm.eval_source(s).unwrap().display().unwrap()}
fn open(vm:&mut Vm) {
    assert_eq!(printed(vm,"(fboundp 'romacs-term)"),"t","TERMINAL_COMMAND_MUST_BE_AVAILABLE");
    vm.eval_source("(romacs-term \"/bin/sh\")").unwrap(); vm.poll_terminal_output().unwrap();
}
const META: i64 = 0x8000000;
fn ch(c: char) -> CommandEvent { CommandEvent::Character(c as i64) }
fn named(name: &str) -> CommandEvent { CommandEvent::Named(name.into()) }
/// Type KEYS through the Lisp command loop, COMMANDS commands of them;
/// return what each command signalled.
fn type_keys(vm: &mut Vm, state: &Shared, keys: Vec<(CommandEvent, Before)>, commands: usize) -> Vec<Option<String>> {
    vm.set_host_input(Some(Box::new(Keyboard(keys.into(), state.clone()))));
    let out = (0..commands).map(|_| command::run(vm).unwrap().signal.map(|s| format!("{} {}", s.condition, s.data))).collect();
    vm.set_host_input(None);
    out
}
fn keys(events: &[CommandEvent]) -> Vec<(CommandEvent, Before)> { events.iter().map(|e| (e.clone(), None)).collect() }
fn sent(state: &Shared) -> Vec<Vec<u8>> { std::mem::take(&mut state.borrow_mut().sent) }

#[test]
fn keys_reach_the_program_through_the_command_loop() {
    let (mut vm,state)=setup(); open(&mut vm);
    vm.eval_source("(setq ran nil) (define-key global-map [24 ?t] (lambda () (interactive) (setq ran 'c-x-t)))").unwrap();
    let events = [ch('a'), CommandEvent::Character(1), CommandEvent::Character(META | 'f' as i64), named("up"), named("return"),
                  ch('λ'), named("escape"), CommandEvent::Character(9), CommandEvent::Character(127), named("C-left"), named("f5")];
    let signals = type_keys(&mut vm, &state, keys(&events), events.len());
    assert!(signals.iter().all(Option::is_none), "{signals:?}");
    assert_eq!(sent(&state), vec![b"a".to_vec(), vec![1], b"\x1bf".to_vec(), b"\x1b[A".to_vec(), b"\r".to_vec(), "λ".as_bytes().to_vec(),
                                  b"\x1b".to_vec(), b"\t".to_vec(), b"\x7f".to_vec(), b"\x1b[1;5D".to_vec(), b"\x1b[15~".to_vec()], "EVERY_KEY_SENDS_THE_TERMINAL_S_BYTES");
    // An exception keeps its Emacs binding; C-c C-c and C-c C-k interrupt.
    let signals = type_keys(&mut vm, &state, keys(&[CommandEvent::Character(24), ch('t'), CommandEvent::Character(3), CommandEvent::Character(3),
                                                    CommandEvent::Character(3), CommandEvent::Character(11)]), 3);
    assert!(signals.iter().all(Option::is_none), "{signals:?}");
    assert_eq!(printed(&mut vm, "ran"), "c-x-t", "C_X_IS_AN_EXCEPTION");
    assert_eq!(sent(&state), vec![vec![3], vec![3]], "C_C_C_C_AND_C_C_C_K_SEND_EXACTLY_ETX");
    // Outside a terminal, Escape is ESC: Escape f is M-f.
    vm.eval_source("(set-window-buffer (selected-window) (get-buffer-create \"plain\")) (set-buffer \"plain\") (insert \"one two\") (goto-char 1)").unwrap();
    type_keys(&mut vm, &state, keys(&[named("escape"), ch('f')]), 1);
    assert_eq!(printed(&mut vm, "(list (point) last-command)"), "(4 forward-word)", "ESCAPE_IS_ESC_OUTSIDE_A_TERMINAL");
    assert!(sent(&state).is_empty());
}

#[test]
fn keymaps_regenerated_before_the_first_terminal_still_route() {
    let (mut vm,state)=setup();
    vm.eval_source("(romacs-term-update-keymaps)").unwrap();
    open(&mut vm);
    type_keys(&mut vm, &state, keys(&[ch('a')]), 1);
    assert_eq!(sent(&state), vec![b"a".to_vec()], "THE_HOOKS_ARE_INSTALLED_ANYWAY");
}

#[test]
fn user_bindings_and_exceptions_choose_what_stays_with_emacs() {
    let (mut vm,state)=setup(); open(&mut vm);
    vm.eval_source("(setq ran nil)
      (define-key romacs-term-mode-map [1] (lambda () (interactive) (setq ran 'user-c-a)))
      (define-key global-map [2] (lambda () (interactive) (setq ran 'global-c-b)))
      (define-key global-map [5] (lambda () (interactive) (setq ran 'global-c-e)))
      (define-key global-map [?\\M-x] (lambda () (interactive) (setq ran 'global-m-x)))").unwrap();
    type_keys(&mut vm, &state, keys(&[CommandEvent::Character(1)]), 1);
    assert_eq!(printed(&mut vm, "ran"), "user-c-a", "A_USER_BINDING_ON_THE_MODE_MAP_WINS");
    vm.eval_source("(define-key global-map [?q] (lambda () (interactive) (setq ran 'global-q)))").unwrap();
    type_keys(&mut vm, &state, keys(&[CommandEvent::Character(2), CommandEvent::Character(5), ch('q')]), 3);
    assert_eq!(sent(&state), vec![vec![2], vec![5], b"q".to_vec()], "GLOBAL_BINDINGS_LOSE_TO_THE_PROGRAM");
    type_keys(&mut vm, &state, keys(&[CommandEvent::Character(META | 'x' as i64)]), 1);
    assert_eq!(printed(&mut vm, "ran"), "global-m-x", "M_X_IS_AN_EXCEPTION");
    vm.eval_source("(setq romacs-term-keymap-exceptions (cons \"C-e\" romacs-term-keymap-exceptions)) (romacs-term-update-keymaps)").unwrap();
    type_keys(&mut vm, &state, keys(&[CommandEvent::Character(5)]), 1);
    assert_eq!(printed(&mut vm, "ran"), "global-c-e", "AN_ADDED_EXCEPTION_RUNS_ITS_EMACS_COMMAND");
    type_keys(&mut vm, &state, keys(&[CommandEvent::Character(1)]), 1);
    assert_eq!(printed(&mut vm, "ran"), "user-c-a", "THE_USER_BINDING_SURVIVES_REGENERATION");
    assert!(sent(&state).is_empty(), "AN_ADDED_EXCEPTION_STAYS_WITH_EMACS");
    assert_eq!(printed(&mut vm, "(eq (current-local-map) romacs-term-mode-map)"), "t", "BETWEEN_SEQUENCES_THE_LOCAL_MAP_IS_THE_MODE_S");
}

#[test]
fn yank_sends_the_kill_as_a_paste() {
    let (mut vm,state)=setup(); open(&mut vm);
    vm.eval_source("(setq interprogram-paste-function nil kill-ring (list \"one\\ntwo\" \"a\\e[201~b\\003c\") kill-ring-yank-pointer kill-ring)").unwrap();
    type_keys(&mut vm, &state, keys(&[CommandEvent::Character(25)]), 1);
    assert_eq!(sent(&state), vec![b"one\rtwo".to_vec()], "AN_UNBRACKETED_PASTE_SENDS_LINE_ENDS_AS_RET");
    state.borrow_mut().bracketed = true;
    vm.eval_source("(setq kill-ring-yank-pointer kill-ring)").unwrap();
    // The second kill, bracketed, its ESC and ETX dropped.
    vm.eval_source("(romacs-term-yank 2)").unwrap();
    assert_eq!(printed(&mut vm, "(buffer-string)"), "\"$ \"", "NOTHING_IS_INSERTED_INTO_THE_TERMINAL");
    let got = sent(&state);
    assert_eq!(got.last().unwrap(), &b"\x1b[200~a[201~bc\x1b[201~".to_vec(), "A_BRACKETED_PASTE_CANNOT_END_ITSELF: {got:?}");
}

#[test]
fn a_window_moved_off_the_cursor_is_an_emacs_buffer_until_typing() {
    let (mut vm,state)=setup(); open(&mut vm);
    state.borrow_mut().show("one\ntwo\n$ ", 0); vm.poll_terminal_output().unwrap();
    vm.eval_source("(define-key global-map [24 ?g] (lambda () (interactive) (goto-char 1)))").unwrap();
    type_keys(&mut vm, &state, keys(&[CommandEvent::Character(24), ch('g')]), 1);
    assert_eq!(printed(&mut vm, "(list (point) (plist-get (romacs--term-state romacs-term-session) :following))"), "(1 nil)", "MOVING_POINT_LEAVES_THE_CURSOR");
    state.borrow_mut().show("one\ntwo\n$ ls\nthree\n$ ", 0); vm.poll_terminal_output().unwrap();
    assert_eq!(printed(&mut vm, "(point)"), "1", "OUTPUT_KEEPS_AN_AWAY_WINDOW_WHERE_IT_IS");
    // Emacs keys move and refuse edits there, and send nothing.
    let signals = type_keys(&mut vm, &state, keys(&[CommandEvent::Character(6), CommandEvent::Character(4)]), 2);
    assert_eq!(printed(&mut vm, "(point)"), "2");
    assert!(signals[1].as_deref().is_some_and(|s| s.starts_with("buffer-read-only")), "{signals:?}");
    assert!(sent(&state).is_empty(), "AN_AWAY_WINDOW_SENDS_NOTHING");
    // Typing puts it back on the cursor and sends the key.
    type_keys(&mut vm, &state, keys(&[ch('x')]), 1);
    assert_eq!(sent(&state), vec![b"x".to_vec()], "TYPING_SENDS");
    assert_eq!(printed(&mut vm, "(list (point) (plist-get (romacs--term-state romacs-term-session) :following))"), "(22 t)", "TYPING_SNAPS_BACK");
}

#[test]
fn the_alternate_screen_takes_every_key_but_its_exceptions() {
    let (mut vm,state)=setup(); open(&mut vm);
    vm.eval_source("(romacs--term-follow romacs-term-session nil) (setq ran nil) (define-key global-map [24 ?t] (lambda () (interactive) (setq ran 'c-x-t)))").unwrap();
    { let mut s = state.borrow_mut(); s.alternate = true; s.show("vim", 0); }
    vm.poll_terminal_output().unwrap();
    assert_eq!(printed(&mut vm, "(plist-get (romacs--term-state romacs-term-session) :following)"), "t", "THE_ALTERNATE_SCREEN_HAS_NO_AWAY");
    type_keys(&mut vm, &state, keys(&[CommandEvent::Character(3), CommandEvent::Character(25), CommandEvent::Character(22), named("escape"),
                                      CommandEvent::Character(24), ch('t')]), 5);
    assert_eq!(sent(&state), vec![vec![3], vec![25], vec![22], vec![27]], "C_C_C_Y_C_V_AND_ESCAPE_REACH_THE_PROGRAM_AT_ONCE");
    assert_eq!(printed(&mut vm, "ran"), "c-x-t", "C_X_STAYS_WITH_EMACS");
}

#[test]
fn a_sequence_keeps_the_route_it_started_with() {
    let (mut vm,state)=setup(); open(&mut vm);
    // vim takes the screen between C-c and C-k: still the interrupt.
    let vim: Before = Some(Box::new(|s: &mut State| { s.alternate = true; s.show("vim", 0); }));
    let signals = type_keys(&mut vm, &state, vec![(CommandEvent::Character(3), None), (CommandEvent::Character(11), vim)], 1);
    assert!(signals[0].is_none(), "{signals:?}");
    assert_eq!(printed(&mut vm, "(list last-command (plist-get (romacs--term-state romacs-term-session) :alternate))"), "(romacs-term-interrupt t)");
    assert_eq!(sent(&state), vec![vec![3]], "A_PREFIX_STARTED_ON_THE_NORMAL_SCREEN_KEEPS_ITS_MAPS");
    // A sequence started in another buffer gets Emacs maps in the terminal.
    assert_eq!(printed(&mut vm, "(let ((term (current-buffer)) (other (get-buffer-create \"other\")))
        (set-buffer other) (romacs-term--sequence-start)
        (prog1 (with-current-buffer term (list (eq (current-local-map) romacs-term-mode-map) (key-binding [?a])))
          (romacs-term--sequence-end)))"), "(t self-insert-command)", "ANOTHER_BUFFER_S_SEQUENCE_IS_NOT_THE_TERMINAL_S");
}

#[test]
fn a_window_scrolled_from_another_stops_following() {
    let (mut vm,state)=setup(); open(&mut vm);
    state.borrow_mut().show("one\ntwo\n$ ", 4); vm.poll_terminal_output().unwrap();
    vm.eval_source("(setq term-window (selected-window) term (current-buffer))
      (setq file-window (let* ((w (selected-window)) (h (window-pixel-height w))) (set-window-new-pixel w (- h 300)) (split-window-internal w 300 nil 0.5)))
      (set-window-buffer file-window (get-buffer-create \"file\")) (select-window file-window) (set-buffer \"file\")
      (define-key global-map [24 ?s] (lambda () (interactive) (set-window-start term-window 1)))").unwrap();
    type_keys(&mut vm, &state, keys(&[CommandEvent::Character(24), ch('s')]), 1);
    state.borrow_mut().show("one\ntwo\n$ ls\nthree\n$ ", 4); vm.poll_terminal_output().unwrap();
    vm.frame_snapshot(1000,700).unwrap();
    assert_eq!(printed(&mut vm, "(list (window-start term-window) (window-point term-window))"), "(1 11)", "A_SCROLLED_WINDOW_KEEPS_ITS_PLACE");
    // Back to it, typing snaps back.
    vm.eval_source("(select-window term-window) (set-buffer term)").unwrap();
    type_keys(&mut vm, &state, keys(&[ch('y')]), 1);
    assert_eq!(printed(&mut vm, "(list (window-start term-window) (point))"), "(5 22)");
}

#[test]
fn plain_passthrough_keys_skip_the_command_loop_when_the_keymaps_agree() {
    let (mut vm,state)=setup(); open(&mut vm);
    let fast = |vm: &mut Vm, key: CommandEvent| vm.terminal_passthrough_key(&key).unwrap();
    for key in [ch('a'), ch('λ'), CommandEvent::Character(1), CommandEvent::Character(META | 'f' as i64), named("up"), named("return"), named("escape")] {
        assert!(fast(&mut vm, key.clone()), "{key:?} IS_PLAIN_PASSTHROUGH");
    }
    assert_eq!(sent(&state), vec![b"a".to_vec(), "λ".as_bytes().to_vec(), vec![1], b"\x1bf".to_vec(), b"\x1b[A".to_vec(), b"\r".to_vec(), b"\x1b".to_vec()]);
    // Exceptions, the mode map's own keys and anything a user binds go through the loop.
    for key in [CommandEvent::Character(3), CommandEvent::Character(25), CommandEvent::Character(24), CommandEvent::Character(META | 'x' as i64)] {
        assert!(!fast(&mut vm, key.clone()), "{key:?} IS_NOT_PASSTHROUGH");
    }
    vm.eval_source("(define-key romacs-term-mode-map [2] 'ignore)").unwrap();
    assert!(!fast(&mut vm, CommandEvent::Character(2)), "A_USER_BINDING_WINS");
    vm.eval_source("(define-key romacs-term-mode-map [remap romacs-term-send-key] 'ignore)").unwrap();
    assert!(!fast(&mut vm, ch('a')), "A_REMAPPED_SEND_GOES_THROUGH_THE_LOOP");
    vm.eval_source("(define-key romacs-term-mode-map [remap romacs-term-send-key] nil)").unwrap();
    assert!(fast(&mut vm, ch('a')), "AND_BACK_ONCE_REMOVED");
    vm.eval_source("(setq competing-mode t competing-map (make-sparse-keymap)) (define-key competing-map [?z] 'ignore)
      (setq minor-mode-map-alist (list (cons 'competing-mode competing-map)))").unwrap();
    assert!(!fast(&mut vm, ch('z')) && fast(&mut vm, ch('y')), "A_MINOR_MODE_BINDING_GOES_THROUGH_THE_LOOP");
    for (setup, undo) in [("(setq defining-kbd-macro t)", "(setq defining-kbd-macro nil)"),
                          ("(setq overriding-terminal-local-map (make-sparse-keymap))", "(setq overriding-terminal-local-map nil)"),
                          ("(make-local-variable 'post-command-hook) (setq post-command-hook (cons 'ignore post-command-hook))", "(kill-local-variable 'post-command-hook)"),
                          ("(make-local-variable 'pre-command-hook) (setq pre-command-hook '(ignore t))", "(kill-local-variable 'pre-command-hook)"),
                          ("(setq input-method-function 'ignore)", "(setq input-method-function 'list)"),
                          ("(romacs--term-follow romacs-term-session nil)", "(romacs--term-follow romacs-term-session t)"),
                          ("(romacs-gui-confirm 'quit)", "(romacs-gui-answer-no)"),
                          ("(let ((inhibit-read-only t)) (put-text-property 1 3 'keymap (make-sparse-keymap)))", "(let ((inhibit-read-only t)) (remove-text-properties 1 3 '(keymap nil)))")] {
        vm.eval_source(setup).unwrap();
        assert!(!fast(&mut vm, ch('a')), "{setup} SENDS_THROUGH_THE_LOOP");
        vm.eval_source(undo).unwrap();
        assert!(fast(&mut vm, ch('a')), "{undo} RESTORES_THE_FAST_PATH");
    }
    // A translation the reader would make, a default binding, another
    // global map that remaps the command: the loop decides.
    for (setup, undo) in [("(define-key key-translation-map [t] [?b])", "(define-key key-translation-map [t] nil)"),
                          ("(define-key key-translation-map [?a] [?b])", "(define-key key-translation-map [?a] nil)"),
                          ("(narrow-to-region 1 2)", "(widen)"),
                          ("(define-key input-decode-map [?a] [?b])", "(define-key input-decode-map [?a] nil)"),
                          ("(define-key romacs-term-mode-map [t] 'ignore)", "(define-key romacs-term-mode-map [t] nil)"),
                          ("(setq saved-global (current-global-map) other-global (make-sparse-keymap)) (set-keymap-parent other-global saved-global)
                            (define-key other-global [remap romacs-term-send-key] 'ignore) (use-global-map other-global)", "(use-global-map saved-global)")] {
        assert!(fast(&mut vm, ch('a')), "{setup}: WARM");
        vm.eval_source(setup).unwrap();
        assert!(!fast(&mut vm, ch('a')), "{setup} SENDS_THROUGH_THE_LOOP");
        vm.eval_source(undo).unwrap();
        assert!(fast(&mut vm, ch('a')), "{undo} RESTORES_THE_FAST_PATH");
    }
    // A keymap on the character at point, even one an insertion there
    // would not inherit.
    { let mut st = state.borrow_mut(); st.text = "$ ab".into(); st.cursor = 2; st.revision += 1; st.pending = true; }
    vm.poll_terminal_output().unwrap();
    vm.eval_source("(let ((inhibit-read-only t)) (put-text-property 3 4 'keymap (make-sparse-keymap)))").unwrap();
    assert_eq!(printed(&mut vm, "(list (point) (get-pos-property (point) 'keymap) (keymapp (get-char-property (point) 'keymap)))"), "(3 nil t)");
    assert!(!fast(&mut vm, ch('a')), "A_KEYMAP_ON_THE_CHARACTER_AT_POINT_SENDS_THROUGH_THE_LOOP");
    vm.eval_source("(let ((inhibit-read-only t)) (remove-text-properties 3 4 '(keymap nil)))").unwrap();
    assert!(fast(&mut vm, ch('a')));
    // Global command hooks, as every real init has, keep it.
    vm.eval_source("(set-default 'pre-command-hook '(ignore)) (set-default 'post-command-hook (cons 'ignore (default-value 'post-command-hook)))").unwrap();
    assert!(fast(&mut vm, ch('a')), "GLOBAL_COMMAND_HOOKS_ARE_SKIPPED");
    state.borrow_mut().alternate = true; state.borrow_mut().show("vim", 0); vm.poll_terminal_output().unwrap();
    assert!(fast(&mut vm, CommandEvent::Character(3)) && !fast(&mut vm, CommandEvent::Character(24)), "THE_ALTERNATE_SCREEN_S_MAP_DECIDES");
}

#[test]
fn terminal_c_g_cancels_prefix_and_does_not_send_interrupt() {
    let (mut vm,state)=setup(); open(&mut vm); let mut commands=command::Commands::default();
    commands.dispatch(&mut vm,CommandEvent::Character(3)).unwrap();
    let quit=commands.dispatch(&mut vm,CommandEvent::Character(7)).unwrap().unwrap();
    assert_eq!(quit.signal.unwrap().condition,"quit","C_G_IS_EDITOR_QUIT_EVEN_DURING_PREFIX");
    assert!(!commands.in_prefix()); assert!(sent(&state).is_empty());
    assert_eq!(printed(&mut vm,"(buffer-string)"),"\"$ \"");
}

#[test]
fn a_confirmation_in_the_terminal_takes_its_answer_keys() {
    let (mut vm,state)=setup(); open(&mut vm);
    vm.eval_source("(romacs-gui-confirm 'quit)").unwrap();
    type_keys(&mut vm, &state, keys(&[ch('n')]), 1);
    assert_eq!(printed(&mut vm, "(list romacs-gui-pending (eq (current-local-map) romacs-term-mode-map))"), "(nil t)", "N_ANSWERS_THE_CONFIRMATION");
    assert!(sent(&state).is_empty(), "AN_ANSWER_IS_NOT_SENT");
    type_keys(&mut vm, &state, keys(&[ch('n')]), 1);
    assert_eq!(sent(&state), vec![b"n".to_vec()], "AFTER_THE_ANSWER_KEYS_REACH_THE_PROGRAM_AGAIN");
}

#[test]
fn terminal_close_protects_running_sessions_and_confirmed_kill_closes() {
    let (mut vm,_state)=setup(); open(&mut vm);
    vm.eval_source("(romacs-gui-quit)").unwrap();
    assert_eq!(printed(&mut vm,"(list romacs-gui-quit-requested (car romacs-gui-pending))"),"(nil quit)","RUNNING_TERMINAL_REQUIRES_CLOSE_CONSENT");
    vm.eval_source("(romacs-gui-answer-no)").unwrap();
    assert_eq!(printed(&mut vm,"(romacs--term-running-p romacs-term-session)"),"t");
    vm.eval_source("(romacs-gui-kill-buffer)").unwrap();
    assert_eq!(printed(&mut vm,"(car romacs-gui-pending)"),"kill");
    vm.eval_source("(romacs-gui-answer-yes)").unwrap();
    assert!(!vm.has_terminals(),"CONFIRMED_KILL_RETIRES_SESSION");
}

#[test]
fn an_away_window_copies_searches_and_shows_the_region() {
    let (mut vm,state)=setup(); open(&mut vm);
    assert_eq!(printed(&mut vm,"(require 'romacs-term)"),"romacs-term","BUNDLED_TERMINAL_IS_THE_LOADED_AUTHORITY");
    vm.eval_source("(romacs--term-follow romacs-term-session nil) (goto-char 1) (set-mark 1) (goto-char 3) (copy-region-as-kill (region-beginning) (region-end))").unwrap();
    assert_eq!(printed(&mut vm,"(list (car kill-ring) (buffer-string) (point) buffer-read-only (buffer-modified-p))"),"(\"$ \" \"$ \" 3 t nil)","COPY_NEVER_EDITS_TERMINAL");
    vm.eval_source("(goto-char 1) (search-forward \"$\")").unwrap();
    assert_eq!(printed(&mut vm,"(point)"),"2","SEARCH_MOVES_ONLY_EDITOR_POINT");
    assert!(sent(&state).is_empty(),"COPY_AND_SEARCH_SEND_NO_CHILD_INPUT");
    vm.eval_source("(set-mark 3) (goto-char 1)").unwrap();
    let frame = vm.frame_snapshot(1000,700).unwrap();
    let terminal = frame.windows.iter().find(|window| window.snapshot.text.utf8 == "$ ").unwrap();
    let face = terminal.snapshot.text.face_runs.iter().find(|run| run.bytes.contains(&0)).unwrap().face;
    assert_eq!(terminal.snapshot.faces[face.0 as usize].foreground,[200,30,20],"REGION_PRESERVES_TERMINAL_ANSI_FOREGROUND");
    assert_eq!(terminal.snapshot.faces[face.0 as usize].background,[62,68,81],"SHARED_REGION_OVERRIDES_TERMINAL_BACKGROUND");
}

#[test]
fn terminal_base_colours_follow_the_ansi_color_faces() {
    use romacs_vm::TerminalColor;
    let foreground = |vm: &mut Vm| {
        let frame = vm.frame_snapshot(1000,700).unwrap();
        let terminal = frame.windows.iter().find(|window| window.snapshot.text.utf8 == "$ ").unwrap();
        let face = terminal.snapshot.text.face_runs.iter().find(|run| run.bytes.contains(&0)).unwrap().face;
        terminal.snapshot.faces[face.0 as usize].foreground
    };
    FOREGROUND.set(TerminalColor::Ansi(1));
    let (mut vm,_)=setup(); open(&mut vm);
    assert_eq!(printed(&mut vm,"(internal-get-lisp-face-attribute 'ansi-color-red :foreground nil)"),"\"#ff5f59\"");
    assert_eq!(foreground(&mut vm),[0xff,0x5f,0x59],"BASE_COLOUR_IS_THE_FACE_S");
    vm.eval_source("(internal-set-lisp-face-attribute 'ansi-color-red :foreground \"#604020\" nil)").unwrap();
    assert_eq!(foreground(&mut vm),[0x60,0x40,0x20],"A_FACE_CHANGE_RECOLOURS_THE_TERMINAL");
}
