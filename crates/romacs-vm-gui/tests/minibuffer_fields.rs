//! Motion in a minibuffer after its prompt, in the `--loadup` GUI through
//! the GUI's keyboard: C-x C-f, then GNU's C-a, C-e, C-f, M-b and C-b keep
//! to the input field and never enter the prompt (editfns.c fields).
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

struct Keys(VecDeque<HostEvent>);
impl HostInput for Keys {
    fn input_pending(&mut self) -> bool {
        false
    }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> {
        Ok(vec![])
    }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.0.pop_front().ok_or_else(|| "field host script exhausted".into())
    }
}

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

const META: i64 = 1 << 27;

#[test]
fn minibuffer_motion_keeps_to_the_input_field() {
    // The directory's last components are the words M-b crosses.
    let root = std::env::temp_dir().join(format!("romacs-minibuffer-fields-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let work = root.join("field/gnu-journey/work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(root.join("home/config")).unwrap();
    let home = root.join("home");
    let directory = format!("{}/", work.display());
    let probes = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, 20, 10).unwrap();
        // The session's user directory is a private one, never the owner's.
        vm.eval_source(&format!("(setq process-environment (append (list {} {}) process-environment))",
            quoted(&format!("HOME={}", home.display())),
            quoted(&format!("XDG_CONFIG_HOME={}", home.join("config").display())))).unwrap();
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        vm.eval_source(&format!("(setq default-directory {})", quoted(&directory))).unwrap();
        // C-c p records point from the prompt's end and from the input's end.
        vm.eval_source("(progn (defvar field-probes nil)
            (defun field-probe () (interactive)
              (push (list (- (point) (minibuffer-prompt-end)) (- (point-max) (point))) field-probes))
            (global-set-key (kbd \"C-c p\") 'field-probe))").unwrap();
        let probe = [3, 'p' as i64];
        let script: Vec<i64> = [&[5][..], &probe, &[1], &probe, &[6], &probe, &[1, 1], &probe,
            &[5, META | 'b' as i64], &probe, &[META | 'b' as i64, META | 'b' as i64], &probe,
            &[2], &probe, &[1, 5], &probe, &[7]].concat();
        vm.set_host_input(Some(Box::new(Keys(script.into_iter().map(|c| HostEvent::Key(CommandEvent::Character(c))).collect()))));
        let (mut commands, mut presentation) = (command::Commands::default(), presentation::Presentation::new());
        let mut outcome = None;
        for code in [24, 6] {
            outcome = presentation.command(&mut vm, &mut commands, CommandEvent::Character(code)).unwrap();
        }
        let quit = outcome.and_then(|o| o.signal).map(|s| s.condition);
        (quit, printed(&mut vm, "(reverse field-probes)"),
         printed(&mut vm, "(with-temp-buffer (insert (abbreviate-file-name default-directory)) (buffer-size))"))
    }).unwrap().join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let (quit, probes, length) = probes;
    assert_eq!(quit.as_deref(), Some("minibuffer-quit"));
    // GNU: C-e, C-a, C-f, C-a C-a, C-e M-b, M-b M-b, C-b, C-a C-e. The
    // input is the abbreviated directory; offsets past it are GNU's.
    let n: i64 = length.parse().unwrap();
    let expected = [(n, 0), (0, n), (1, n - 1), (0, n), (n - 5, 5), (n - 17, 17), (n - 18, 18), (n, 0)];
    let expected = expected.iter().map(|(a, b)| format!("({a} {b})")).collect::<Vec<_>>().join(" ");
    assert_eq!(probes, format!("({expected})"));
}
