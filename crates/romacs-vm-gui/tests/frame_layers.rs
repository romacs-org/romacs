//! Top-level frames in the `--loadup` GUI's Lisp, over a host that shows
//! more than one: frame.el's `make-frame' makes one, `other-frame' and
//! `delete-frame' move between them, and Romacs's layers (romacs-layer.el,
//! romacs-complete.el) are child frames of the selected frame's top-level
//! frame, never one left over another.
use romacs_vm::{HostEvent, HostFrame, HostInput, Vm};
use romacs_vm_gui::startup;

/// A display of any number of top-level frames, never asked for input.
struct Display;

impl HostInput for Display {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> { Err("no input".into()) }
    fn top_level_frames(&mut self) -> bool { true }
}

/// A window system that answers nothing about fonts.
struct Pgtk;

impl romacs_vm::DisplayHost for Pgtk {
    fn window_system(&self) -> &str { "pgtk" }
    fn display_name(&self) -> String { String::new() }
    fn list_fonts(&mut self, _: &romacs_vm::FontQuery) -> Vec<romacs_vm::FontEntity> { vec![] }
    fn font_families(&mut self) -> Vec<String> { vec![] }
    fn open_font(&mut self, _: &romacs_vm::FontEntity, _: i64) -> Option<romacs_vm::OpenedFont> { None }
    fn char_font(&mut self, _: &romacs_vm::FaceFont, _: char) -> Option<(romacs_vm::OpenedFont, u32)> { None }
}

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

fn on_loadup(test: impl FnOnce(&mut Vm) + Send + 'static) {
    std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let home = std::env::temp_dir().join(format!("romacs-frame-layers-{}", std::process::id()));
        std::fs::create_dir_all(home.join("config")).unwrap();
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, 20, 10).unwrap();
        vm.eval_source(&format!("(setq process-environment (append (list {:?} {:?}) process-environment))",
            format!("HOME={}", home.display()), format!("XDG_CONFIG_HOME={}", home.join("config").display()))).unwrap();
        // The session opens the display it finds attached, as the GUI's does.
        vm.set_display_host(Some(Box::new(Pgtk)));
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        vm.set_host_input(Some(Box::new(Display)));
        test(&mut vm);
        let _ = std::fs::remove_dir_all(&home);
    }).unwrap().join().unwrap();
}

/// frame.el's commands over two top-level frames: `make-frame' makes one
/// without selecting it, `other-frame' selects it, `delete-frame' hands
/// the selection back.
#[test]
fn frame_commands_move_between_top_level_frames() {
    on_loadup(|vm| {
        assert_eq!(printed(vm, "(progn (setq F (selected-frame) G (make-frame '((name . \"two\")))) (list (length (seq-remove #'frame-parent (frame-list))) (eq (selected-frame) F) (frame-parent G)))"),
                   "(2 t nil)");
        assert_eq!(printed(vm, "(progn (other-frame 1) (eq (selected-frame) G))"), "t");
        assert_eq!(printed(vm, "(progn (delete-frame) (list (frame-live-p G) (eq (selected-frame) F) (length (seq-remove #'frame-parent (frame-list)))))"), "(nil t 1)");
    });
}

/// A buffer's layer is a child of the selected frame's top-level frame:
/// asked for over another frame, the one it had goes and a new one is made
/// there; the completion layer moves the same way, giving the first frame
/// its minibuffer window back.
#[test]
fn layers_follow_the_selected_top_level_frame() {
    on_loadup(|vm| {
        vm.eval_source("(progn (require 'romacs-layer) (require 'romacs-complete)
            (setq F (selected-frame) G (make-frame '((name . \"two\")))
                  buf (get-buffer-create \" *layer-probe*\")
                  L1 (romacs-layer--frame buf)))").unwrap();
        assert_eq!(printed(vm, "(list (eq (frame-parent L1) F) (eq (romacs-layer--frame buf) L1))"), "(t t)");
        assert_eq!(printed(vm, "(progn (select-frame G) (setq L2 (romacs-layer--frame buf)) (list (eq (frame-parent L2) G) (frame-live-p L1)))"),
                   "(t nil)");
        assert_eq!(printed(vm, "(progn (select-frame F) (setq C1 (romacs-complete--frame))
            (list (eq (frame-parent C1) F) (eq (romacs--minibuffer-host) C1)))"), "(t t)");
        assert_eq!(printed(vm, "(progn (select-frame G) (setq C2 (romacs-complete--frame))
            (list (eq (frame-parent C2) G) (frame-live-p C1) (eq (romacs--minibuffer-host) C2)
                  (progn (select-frame F) (romacs--minibuffer-host))))"), "(t nil t nil)");
    });
}
