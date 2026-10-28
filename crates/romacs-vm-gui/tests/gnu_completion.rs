//! GNU's minibuffer completion in the `--loadup` GUI, through the GUI's
//! keyboard (`romacs_vm::HostInput`) on the real chain: Romacs's completion
//! defaults are the options' standard values; M-x, C-x C-f and C-x b read
//! with GNU's `execute-extended-command`, `read-file-name` and `read-buffer`;
//! icomplete shows the candidates in the mini-window as an overlay string,
//! the mini-window grows to hold them and shrinks back when the minibuffer
//! exits. The GUI delivers Return as C-j (DIVERGENCES VM2-HOSTIN1).
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
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> {
    vm.frame_snapshot(width, height)
}

const LINE: u32 = 20;

/// Each prompt redisplay's mini-window: its text and height.
type Frames = Rc<RefCell<Vec<(String, u32)>>>;

struct Keys {
    keys: VecDeque<HostEvent>,
    frames: Frames,
}
impl HostInput for Keys {
    fn input_pending(&mut self) -> bool {
        false
    }
    fn redisplay(&mut self, frame: HostFrame) -> Result<Vec<(u64, usize)>, String> {
        let mini = &frame.frame.windows.last().unwrap().snapshot;
        self.frames.borrow_mut().push((mini.text.utf8.clone(), mini.height));
        Ok(vec![])
    }
    fn next_event(&mut self) -> Result<HostEvent, String> {
        self.keys.pop_front().ok_or_else(|| "completion host script exhausted".into())
    }
}

/// Attach a keyboard that types TEXT, Return as the GUI sends it (C-j).
fn attach(vm: &mut Vm, text: &str) -> Frames {
    let frames = Frames::default();
    let keys = text.chars().map(|c| HostEvent::Key(CommandEvent::Character(c as i64))).collect();
    vm.set_host_input(Some(Box::new(Keys { keys, frames: frames.clone() })));
    frames
}

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

/// Run the command of KEYS as the GUI's outer loop does.
fn keys(vm: &mut Vm, codes: &[i64]) -> Option<romacs_vm::CommandOutcome> {
    let (mut commands, mut presentation) = (command::Commands::default(), presentation::Presentation::new());
    let mut outcome = None;
    for &code in codes {
        outcome = presentation.command(vm, &mut commands, CommandEvent::Character(code)).unwrap();
    }
    outcome
}

/// The tallest mini-window a prompt showed, and the texts it showed.
fn shown(frames: &Frames) -> (u32, Vec<String>) {
    let frames = frames.borrow();
    (frames.iter().map(|f| f.1).max().unwrap_or(0), frames.iter().map(|f| f.0.clone()).collect())
}

fn mini_height(vm: &mut Vm) -> u32 {
    vm.frame_snapshot(1280, 800).unwrap().windows.last().unwrap().snapshot.height
}

#[test]
fn gnu_completion_reads_commands_files_and_buffers_through_icomplete() {
    let root = std::env::temp_dir().join(format!("romacs-gnu-completion-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("work/sub")).unwrap();
    std::fs::create_dir_all(root.join("home/config")).unwrap();
    std::fs::write(root.join("work/sub/notes.txt"), "notes\n").unwrap();
    let work = format!("{}/", root.join("work").display());
    let home = root.join("home");
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, LINE, 10).unwrap();
        // The session's user directory is a private one, never the owner's.
        vm.eval_source(&format!("(setq process-environment (append (list {} {}) process-environment))",
            quoted(&format!("HOME={}", home.display())),
            quoted(&format!("XDG_CONFIG_HOME={}", home.join("config").display())))).unwrap();
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        let mut report = vec![];
        // Romacs's defaults are standard values, set through their :set.
        report.push(printed(&mut vm, "(list completion-styles completion-ignore-case completions-format completions-sort
            completion-category-overrides icomplete-compute-delay
            (equal (eval (car (get 'completions-sort 'standard-value)) t) completions-sort)
            fido-vertical-mode fido-mode icomplete-mode (eval (car (get 'fido-vertical-mode 'standard-value)) t)
            (lookup-key icomplete-fido-mode-map (kbd \"TAB\")) resize-mini-windows)"));

        // M-x: the vertical list, TAB takes the selected command, Return runs it.
        vm.eval_source("(defun gc-probe-command () (interactive) (setq gc-probe-ran 'ran))").unwrap();
        let frames = attach(&mut vm, "gc-probe\t\n");
        let outcome = keys(&mut vm, &[(1 << 27) | 'x' as i64]);
        let (tallest, texts) = shown(&frames);
        report.push(format!("{:?}", outcome.and_then(|o| o.signal).map(|s| s.condition)));
        report.push(printed(&mut vm, "(list (bound-and-true-p gc-probe-ran) (car extended-command-history))"));
        report.push(format!("{} {}", tallest > LINE, mini_height(&mut vm) == LINE));
        report.push(texts.iter().any(|t| t.contains("M-x gc-probe \ngc-probe-command")).to_string());
        report.push(texts.iter().any(|t| t.contains("M-x gc-probe-command \n")).to_string());

        // C-x C-f: partial completion of s/n to sub/notes.txt, then visit.
        vm.eval_source(&format!("(setq default-directory {})", quoted(&work))).unwrap();
        let frames = attach(&mut vm, "s/n\t\n");
        let outcome = keys(&mut vm, &[24, 6]);
        let (tallest, texts) = shown(&frames);
        report.push(format!("{:?}", outcome.and_then(|o| o.signal).map(|s| s.condition)));
        report.push(printed(&mut vm, &format!("(list (file-relative-name buffer-file-name {}) (buffer-string))", quoted(&work))));
        report.push(format!("{} {}", tallest > LINE, mini_height(&mut vm) == LINE));
        report.push(texts.iter().any(|t| t.contains(&format!("Find file: {work}s/n \nsub/notes.txt"))).to_string());

        // C-x b: the default in the prompt, completion over the buffers.
        vm.eval_source("(get-buffer-create \"alpha-kt-buffer\")").unwrap();
        let frames = attach(&mut vm, "alpha-kt\t\n");
        let outcome = keys(&mut vm, &[24, 98]);
        let (_, texts) = shown(&frames);
        report.push(format!("{:?}", outcome.and_then(|o| o.signal).map(|s| s.condition)));
        report.push(printed(&mut vm, "(list (buffer-name (window-buffer (selected-window))) (car buffer-name-history))"));
        report.push(texts.iter().any(|t| t.contains("Switch to buffer (default ") && t.contains("alpha-kt-buffer")).to_string());
        report
    }).unwrap().join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(report, [
        "((basic flex) t one-column historical ((file (styles partial-completion))) 0.15 t t t t t icomplete-force-complete grow-only)",
        "None", "(ran \"gc-probe-command\")", "true true", "true", "true",
        "None", r#"("sub/notes.txt" "notes\n")"#, "true true", "true",
        "None", "(\"alpha-kt-buffer\" \"alpha-kt-buffer\")", "true",
    ]);
}
