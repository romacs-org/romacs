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

#[derive(Clone)]
struct Host(Rc<RefCell<Vec<Vec<u8>>>>);
impl TerminalHost for Host {
    fn create(&mut self, _: &str, _: &str, _: u16, _: u16) -> Result<u64,String> { Ok(1) }
    fn send(&mut self, _: u64, bytes: &[u8]) -> Result<(),String> { self.0.borrow_mut().push(bytes.to_vec()); Ok(()) }
    fn resize(&mut self, _: u64, _: u16, _: u16) -> Result<(),String> { Ok(()) }
    fn close(&mut self, _: u64) -> Result<(),String> { Ok(()) }
    fn input_state(&mut self, _: u64) -> Result<romacs_vm::TerminalInputState,String> { Ok(romacs_vm::TerminalInputState { running:true, application_cursor:false, bracketed_paste:false }) }
    fn changed_sessions(&mut self) -> Vec<u64> { vec![] }
    fn snapshot(&mut self, _: u64) -> Result<TerminalSnapshot,String> { Ok(TerminalSnapshot { revision:1, text:"$ ".into(), styles:std::sync::Arc::from([]), default_colors:[None;2], cursor_byte:2, screen_start_byte:0, columns:80, rows:24, alternate_screen:false, running:true, application_cursor:false, bracketed_paste:false }) }
}
fn setup() -> (Vm,Rc<RefCell<Vec<Vec<u8>>>>) {
    let sent=Rc::new(RefCell::new(vec![])); let mut vm=Vm::new();
    vm.set_collection_floor(1); vm.set_frame_geometry(1000,700,20,10).unwrap();
    vm.set_terminal_host(Some(Box::new(Host(sent.clone())))).unwrap();
    startup::initialize_files(&mut vm,&[]).unwrap(); (vm,sent)
}
fn printed(vm:&mut Vm,s:&str)->String {vm.eval_source(s).unwrap().display().unwrap()}
fn open(vm:&mut Vm) {
    assert_eq!(printed(vm,"(fboundp 'romacs-term)"),"t","TERMINAL_COMMAND_MUST_BE_AVAILABLE");
    vm.eval_source("(romacs-term \"/bin/sh\")").unwrap(); vm.poll_terminal_output().unwrap();
}
#[test]
fn terminal_commands_toggle_and_interrupt_through_normal_keymaps() {
    let (mut vm,sent)=setup(); open(&mut vm); let mut commands=command::Commands::default();
    let mut presentation=presentation::Presentation::new();
    assert_eq!(printed(&mut vm,"(list major-mode romacs-term-emacs-input buffer-read-only (buffer-string))"),"(romacs-term-mode nil t \"$ \")");
    for key in [3,3] {presentation.command(&mut vm,&mut commands,CommandEvent::Character(key)).unwrap();}
    assert_eq!(echo::text(&mut vm).unwrap(), "Emacs input: navigate/copy; C-c C-c returns to terminal", "TERMINAL_MODE_FEEDBACK_IS_PAINTED");
    assert_eq!(printed(&mut vm,"romacs-term-emacs-input"),"t","C_C_C_C_ENTERS_EMACS_INPUT");
    commands.dispatch(&mut vm,CommandEvent::Character(2)).unwrap();
    assert_eq!(printed(&mut vm,"(point)"),"2");
    for key in [3,11] {commands.dispatch(&mut vm,CommandEvent::Character(key)).unwrap();}
    assert_eq!(*sent.borrow(),vec![vec![3]],"C_C_C_K_SENDS_EXACTLY_ETX");
    for key in [3,3] {commands.dispatch(&mut vm,CommandEvent::Character(key)).unwrap();}
    assert_eq!(printed(&mut vm,"romacs-term-emacs-input"),"nil");
}
#[test]
fn terminal_c_g_cancels_prefix_and_does_not_send_interrupt() {
    let (mut vm,sent)=setup(); open(&mut vm); let mut commands=command::Commands::default();
    commands.dispatch(&mut vm,CommandEvent::Character(3)).unwrap();
    let quit=commands.dispatch(&mut vm,CommandEvent::Character(7)).unwrap().unwrap();
    assert_eq!(quit.signal.unwrap().condition,"quit","C_G_IS_EDITOR_QUIT_EVEN_DURING_PREFIX");
    assert!(!commands.in_prefix()); assert!(sent.borrow().is_empty());
    assert_eq!(printed(&mut vm,"(buffer-string)"),"\"$ \"");
}
#[test]
fn terminal_close_protects_running_sessions_and_confirmed_kill_closes() {
    let (mut vm,_)=setup(); open(&mut vm);
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
fn bundled_feature_and_navigation_copy_search_share_the_terminal_buffer() {
    let (mut vm,sent)=setup(); open(&mut vm);
    assert_eq!(printed(&mut vm,"(require 'romacs-term)"),"romacs-term","BUNDLED_TERMINAL_IS_THE_LOADED_AUTHORITY");
    vm.eval_source("(romacs-term-emacs-mode) (goto-char 1) (romacs-term-set-mark) (goto-char 3) (romacs-term-copy-region)").unwrap();
    assert_eq!(printed(&mut vm,"(list (car kill-ring) (buffer-string) (point) buffer-read-only (buffer-modified-p))"),"(\"$ \" \"$ \" 3 t nil)","NAVIGATION_COPY_NEVER_EDITS_TERMINAL");
    vm.eval_source("(goto-char 1) (romacs-term-search \"$\")").unwrap();
    assert_eq!(printed(&mut vm,"(point)"),"2","NAVIGATION_SEARCH_MOVES_ONLY_EDITOR_POINT");
    assert!(sent.borrow().is_empty(),"NAVIGATION_AND_COPY_SEND_NO_CHILD_INPUT");
}
