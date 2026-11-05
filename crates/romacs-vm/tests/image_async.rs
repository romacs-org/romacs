//! Images decoded off the VM (images.md section 7): display takes the
//! header's size at once and never waits; a Lisp query waits for the final
//! facts, and a C-g quits that wait; a decode that fails after its header
//! lays the windows out again at GNU's placeholder and logs the loader's
//! message to *Messages*.
use romacs_vm::Vm;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// What the fake worker does with a decode a query waits for.
#[derive(Clone, Copy, PartialEq)]
enum Wait { Finish, Fail, Hang(u32) }

#[derive(Default)]
struct Log { begun: u32, waited: u32, decoded: u32, finished: Vec<(u64, Result<romacs_vm::ImageFacts, String>)> }

/// A window system whose every image is 40x24 by its header and decodes
/// as WAIT says.
struct AsyncDisplay { log: Rc<RefCell<Log>>, wait: Rc<RefCell<Wait>>, quit: Arc<romacs_vm::QuitSignal> }

fn facts(id: u64, pending: bool) -> romacs_vm::ImageFacts {
    romacs_vm::ImageFacts { id, width: 40, height: 24, mask: false,
                            metadata: romacs_display_protocol::ImageEmbeddedMetadata::EMPTY, background: None, pending }
}

impl romacs_vm::DisplayHost for AsyncDisplay {
    fn window_system(&self) -> &str { "pgtk" }
    fn display_name(&self) -> String { String::new() }
    fn list_fonts(&mut self, _: &romacs_vm::FontQuery) -> Vec<romacs_vm::FontEntity> { vec![] }
    fn font_families(&mut self) -> Vec<String> { vec![] }
    fn open_font(&mut self, _: &romacs_vm::FontEntity, _: i64) -> Option<romacs_vm::OpenedFont> { None }
    fn char_font(&mut self, _: &romacs_vm::FaceFont, _: char) -> Option<(romacs_vm::OpenedFont, u32)> { None }
    fn decode_image(&mut self, _: romacs_vm::ImageQuery) -> Result<romacs_vm::ImageFacts, String> {
        self.log.borrow_mut().decoded += 1;
        Err("still corrupt".into())
    }
    fn begin_decode(&mut self, _: romacs_vm::ImageQuery) -> Result<romacs_vm::ImageFacts, String> {
        let mut log = self.log.borrow_mut();
        log.begun += 1;
        Ok(facts(u64::from(log.begun), true))
    }
    fn finish_decode(&mut self, id: u64, quit: &dyn Fn() -> bool) -> Result<romacs_vm::ImageFacts, String> {
        self.log.borrow_mut().waited += 1;
        let wait = *self.wait.borrow();
        match wait {
            Wait::Finish => Ok(facts(id, false)),
            Wait::Fail => Err("corrupt after its header".into()),
            Wait::Hang(presses) => {
                // The user's C-g, typed PRESSES times while the query waits,
                // once each time it looks; once none is pending the decode
                // finishes.
                if presses > 0 {
                    assert!(self.quit.interrupt(), "the evaluator is busy");
                    *self.wait.borrow_mut() = Wait::Hang(presses - 1);
                }
                if !quit() { return Ok(facts(id, false)) }
                Err(romacs_vm::DECODE_QUIT.into())
            }
        }
    }
    fn finished_decodes(&mut self) -> Vec<(u64, Result<romacs_vm::ImageFacts, String>)> {
        std::mem::take(&mut self.log.borrow_mut().finished)
    }
}

fn session(wait: Wait) -> (Vm, Rc<RefCell<Log>>) {
    session_of(wait, r#"'(image :type png :data "header")"#)
}

fn session_of(wait: Wait, spec: &str) -> (Vm, Rc<RefCell<Log>>) {
    let log = Rc::new(RefCell::new(Log::default()));
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    let quit = vm.quit_signal();
    vm.set_display_host(Some(Box::new(AsyncDisplay { log: log.clone(), wait: Rc::new(RefCell::new(wait)), quit })));
    assert!(vm.open_display());
    vm.eval_source(&format!(r#"(progn (setq im-spec {spec})
                        (with-current-buffer (window-buffer (selected-window))
                          (insert "a" (propertize " " 'display im-spec) "b")))"#)).unwrap();
    (vm, log)
}

fn shown(vm: &mut Vm) -> (i64, i64) {
    let image = vm.frame_snapshot(800, 600).unwrap().windows[0].snapshot.images[0];
    (image.width, image.ascent + image.descent)
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn display_takes_the_header_size_and_a_query_waits_once() {
    let (mut vm, log) = session(Wait::Finish);
    assert_eq!(shown(&mut vm), (40, 24));
    assert_eq!((log.borrow().begun, log.borrow().waited), (1, 0), "display does not wait");
    assert_eq!(eval(&mut vm, "(image-size im-spec t)"), "(40 . 24)");
    assert_eq!(eval(&mut vm, "(image-size im-spec t)"), "(40 . 24)");
    assert_eq!((log.borrow().begun, log.borrow().waited), (1, 1), "the second query has the final facts");
}

#[test]
fn a_failure_after_the_header_reflows_to_the_placeholder() {
    let (mut vm, log) = session(Wait::Finish);
    assert_eq!(shown(&mut vm), (40, 24));
    log.borrow_mut().finished.push((1, Err("corrupt after its header".into())));
    assert!(vm.land_images().unwrap());
    assert!(!vm.land_images().unwrap(), "nothing more finished");
    let placeholder = shown(&mut vm);
    assert_ne!(placeholder, (40, 24));
    assert_eq!(eval(&mut vm, r#"(with-current-buffer "*Messages*" (and (search-backward "corrupt after its header" nil t) t))"#), "t");
    // Shown again, the failed image loads again at once, and stays the
    // placeholder: it is not sized from its header again.
    assert_eq!(shown(&mut vm), placeholder);
    assert_eq!(log.borrow().begun, 1);
    assert!(log.borrow().decoded >= 1);
}

/// GNU's placeholder for an image sized 40x24 is 40x24 too, but without
/// its margins: the windows are laid out again though the box is the same.
#[test]
fn a_failure_as_large_as_its_header_still_reflows() {
    let (mut vm, log) = session_of(Wait::Finish, r#"'(image :type png :data "header" :width 40 :height 24 :margin 5)"#);
    assert_eq!(shown(&mut vm), (50, 34));
    log.borrow_mut().finished.push((1, Err("corrupt after its header".into())));
    assert!(vm.land_images().unwrap());
    assert_eq!(shown(&mut vm), (40, 24));
}

#[test]
fn a_query_waiting_on_a_failed_decode_answers_the_placeholder() {
    let (mut vm, log) = session(Wait::Fail);
    assert_eq!(shown(&mut vm), (40, 24));
    let size = eval(&mut vm, "(image-size im-spec t)");
    assert_ne!(size, "(40 . 24)");
    assert_eq!(log.borrow().waited, 1);
    assert_ne!(shown(&mut vm), (40, 24), "the window is laid out again");
}

#[test]
fn c_g_quits_a_waiting_query() {
    let (mut vm, _) = session(Wait::Hang(1));
    assert_eq!(eval(&mut vm, "(condition-case nil (image-size im-spec t) (quit 'quitted))"), "quitted");
    // Under `inhibit-quit' each press sets `quit-flag' and the wait goes
    // on, still seeing the next press.
    let (mut vm, log) = session(Wait::Hang(2));
    assert_eq!(eval(&mut vm, "(let ((inhibit-quit t)) (list (image-size im-spec t) (prog1 quit-flag (setq quit-flag nil))))"),
               "((40 . 24) t)");
    assert_eq!(log.borrow().waited, 3);
}
