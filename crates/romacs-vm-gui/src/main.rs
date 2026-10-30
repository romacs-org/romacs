//! Launchable VM2 editor; --wayland retains the JSON-line oracle transport.
use romacs_display_runtime::paint_host::{PaintHost, PaintSender};
use romacs_display_runtime::thread_comm::InputEvent;
use romacs_vm::Vm;
use serde_json::{Value as Json, json};
use std::io::{BufRead, Write};
use std::time::Instant;
static TRACE_LATENCY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SCENE_DUMPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
fn tracing() -> bool { TRACE_LATENCY.load(std::sync::atomic::Ordering::Relaxed) }

mod native;
mod loadup;
mod startup;
mod command;
mod eval_last_sexp;
mod presentation;
mod latency;
mod echo;
mod prompt;
mod terminal_input;
mod terminal_service;
mod terminal_wait;
mod batch;
mod user_init;
mod measure;
mod fonts;
mod timers;
mod world;

/// Every allocation in the world arena, so a world image can be this
/// process's heap (`romacs_vm::world::arena`).
#[global_allocator]
static ALLOCATOR: romacs_vm::world::arena::Arena = romacs_vm::world::arena::Arena;

const SCENE: &str = include_str!("../scene.el");

// GNU's widest real forms (cus-start.el:86, 208 operands at 45b0f7699df;
// comp-common.el:71, 270) overflow a default 2 MiB thread in unoptimized
// builds; optimized builds overflow 2 MiB near 1750 operands. Keep the
// evaluator off the default stack. Guarded by evaluator_stack_tests (R209).
const EVALUATOR_STACK_SIZE: usize = 64 * 1024 * 1024;

/// `kill-emacs''s exit status, unwinding the live evaluator (`evaluate').
struct SessionEnd(i32);

fn spawn_evaluator<F, T>(evaluate: F) -> Result<std::thread::JoinHandle<T>, String>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .name("romacs-evaluator".into())
        .stack_size(EVALUATOR_STACK_SIZE)
        .spawn(evaluate)
        .map_err(|error| format!("spawn evaluator: {error}"))
}

#[cfg(test)]
mod native_wake_tests {
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    /// A terminal host whose drain lasts until a background command completes:
    /// the completion lands between the drain and the readiness check of an
    /// order that checks readiness first.
    struct SlowDrain(Receiver<()>);

    impl romacs_vm::TerminalHost for SlowDrain {
        fn create(&mut self, _: &str, _: &str, _: u16, _: u16) -> Result<u64, String> { Err("no terminals".into()) }
        fn send(&mut self, _: u64, _: &[u8]) -> Result<(), String> { Err("no terminals".into()) }
        fn resize(&mut self, _: u64, _: u16, _: u16) -> Result<(), String> { Err("no terminals".into()) }
        fn input_state(&mut self, _: u64) -> Result<romacs_vm::TerminalInputState, String> { Err("no terminals".into()) }
        fn changed_sessions(&mut self) -> Vec<u64> {
            self.0.recv_timeout(Duration::from_secs(10)).expect("background command completed");
            vec![]
        }
        fn snapshot(&mut self, _: u64) -> Result<romacs_vm::TerminalSnapshot, String> { Err("no terminals".into()) }
        fn close(&mut self, _: u64) -> Result<(), String> { Err("no terminals".into()) }
    }

    /// A completion whose wake the terminal drain consumes is still dispatched.
    #[test]
    fn a_completion_during_the_terminal_drain_is_dispatched() {
        let mut vm = romacs_vm::Vm::new();
        let (completed, drain) = channel();
        let completed = std::sync::Mutex::new(completed);
        vm.set_background_wake(std::sync::Arc::new(move || { let _ = completed.lock().unwrap().send(()); }));
        vm.set_terminal_host(Some(Box::new(SlowDrain(drain)))).unwrap();
        vm.eval_source("(defun romacs-process-dispatch-ready () (setq dispatched (romacs--process-ready)))
                        (romacs--process-start '(\"sleep\" \"0.2\") \"\" \"/\" 5)").unwrap();
        assert!(super::service_native(&mut vm).unwrap());
        assert_eq!(vm.eval_source("dispatched").unwrap().display().unwrap(), "(1)");
        assert!(!vm.background_processes_ready());
    }
}

#[cfg(test)]
mod lent_tests {
    use super::*;

    /// Lisp the outer loop runs (a command, a timer) reads with the loop's
    /// presentation state, and what its reads did comes back.
    #[test]
    fn a_read_inside_lisp_sees_and_returns_the_loops_presentation_state() {
        let host = std::rc::Rc::new(std::cell::RefCell::new(prompt::Shared::default()));
        host.borrow_mut().presentation = 7;
        let mut live = presentation::Presentation::new();
        live.submitted(7, Instant::now());
        let (mut queue, mut width, mut height) = (std::collections::VecDeque::new(), 10, 20);
        queue.push_back(InputEvent::WindowClose { emacs_frame_id: 0 });
        let lent = Lent { queue: &mut queue, live: &mut live, width: &mut width, height: &mut height };
        let (seen, closed) = lent.run(&host, |_| {
            let mut shared = host.borrow_mut();
            let seen = (shared.presentation, shared.in_flight, shared.pending.len());
            // What a prompt's redisplay does: settle the loop's frame, submit its own.
            shared.settled = shared.in_flight.take();
            shared.presentation += 1;
            shared.size = Some((30, 40));
            shared.closed = true;
            seen
        });
        assert_eq!(seen, (7, Some(7), 1), "the read starts from the loop's state");
        assert!(closed);
        assert_eq!((queue.len(), host.borrow().presentation, width, height), (1, 8, 30, 40));
        assert!(live.in_flight().is_none(), "the settled presentation is no longer in flight");
        assert_eq!(host.borrow().in_flight, None);
    }

    /// A prompt read inside any Lisp the loop runs (a timer, a background
    /// callback) and dismissed without changing anything still covered the
    /// last paint: the loop must repaint, and compare against the prompt's
    /// presentation, not a copy taken before it.
    #[test]
    fn a_prompt_inside_any_entry_leaves_the_frame_to_repaint() {
        let mut vm = romacs_vm::Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        let host = std::rc::Rc::new(std::cell::RefCell::new(prompt::Shared::default()));
        host.borrow_mut().presentation = 3;
        let shown = Shown { frame: frame(&mut vm, false, 1200, 800).unwrap(), ends: vec![], presentation: 3 };
        let mut live = presentation::Presentation::new();
        let (mut queue, mut width, mut height) = (std::collections::VecDeque::new(), 10, 20);
        let lent = Lent { queue: &mut queue, live: &mut live, width: &mut width, height: &mut height };
        // The callback's prompt: one presentation, then a dismissal that changes nothing.
        lent.run(&host, |_| host.borrow_mut().presentation += 1);
        assert!(live.ready(), "the prompt's presentation left no repaint scheduled");
        let next = frame(&mut vm, false, 1200, 800).unwrap();
        assert!(changed_frame(&mut vm, &shown, next, host.borrow().presentation).unwrap().is_some(),
            "the prompt-covered paint was kept");
    }
}

#[cfg(test)]
mod evaluator_stack_tests {
    use std::os::unix::process::ExitStatusExt;

    const CHILD: &str = "ROMACS_GUI_EVALUATOR_STACK_CHILD";
    const COMPLETE: &str = "romacs-evaluator-stack-workload-complete";
    /// GNU's widest real forms, byte-exact from the pinned oracle tree
    /// 45b0f7699df (our lisp/cus-start.el is edited in place and is one
    /// operand narrower, so it is not the fixture). Operand counts were
    /// measured by GNU 32.0.50 `read`.
    const GNU_WIDEST: [(&str, &str, usize); 2] = [
        ("lisp/cus-start.el:86", include_str!("../tests/fixtures/gnu-45b0f7699df-cus-start-86.el"), 208),
        ("lisp/emacs-lisp/comp-common.el:71", include_str!("../tests/fixtures/gnu-45b0f7699df-comp-common-71.el"), 270),
    ];
    /// Optimized code spends roughly an order of magnitude less native stack
    /// per operand than debug code, so GNU's real forms alone fit in 2 MiB
    /// there (measured: 2 MiB overflows at ~1750 operands). Ten times GNU's
    /// widest form keeps the same guard meaningful in optimized builds.
    const OPTIMIZED_WIDTH: usize = 2700;

    fn workload() {
        let mut vm = romacs_vm::Vm::new();
        for (origin, source, operands) in GNU_WIDEST {
            vm.compile_source(source)
                .unwrap_or_else(|error| panic!("compile GNU {origin} on the evaluator: {error}"));
            // Compiling is the stack-hungry step. Executing would need the rest
            // of cus-start.el's environment, so the form is compiled, not run.
            let counted = vm.eval_source(&format!(
                "(let ((form (quote {source}))) (length (car (cdr (if (eq (car form) 'defconst) (car (cdr (cdr form))) form)))))"))
                .unwrap_or_else(|error| panic!("count GNU {origin}: {error}"));
            assert_eq!(counted.display().unwrap(), operands.to_string(), "fixture {origin} is not GNU's form");
        }
        if !cfg!(debug_assertions) {
            let source = format!("(and {}42)", "t ".repeat(OPTIMIZED_WIDTH));
            let value = vm.eval_source(&source).expect("compile 10x GNU's widest form on the evaluator");
            assert_eq!(value.display().unwrap(), "42");
        }
        println!("{COMPLETE}");
    }

    /// R209: the GUI evaluator's 64 MiB reservation is load-bearing. Remove
    /// it (or shrink it to the 2 MiB default) and this child dies of stack
    /// overflow, so the parent assertion below fails.
    #[test]
    fn evaluator_compiles_wider_than_gnu_startup_forms() {
        if std::env::var_os(CHILD).is_some() {
            super::spawn_evaluator(workload)
                .expect("spawn evaluator")
                .join()
                .expect("evaluator completed");
            return;
        }
        // Stack overflow aborts the process, so run the production helper in a child.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "evaluator_stack_tests::evaluator_compiles_wider_than_gnu_startup_forms", "--nocapture"])
            .env(CHILD, "1")
            .env_remove("RUST_MIN_STACK")
            .output()
            .expect("run evaluator stack regression child");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && output.status.signal().is_none() && stdout.contains(COMPLETE),
            "evaluator could not compile GNU's widest forms on its reserved stack \
             (status {}, signal {:?}, overflow {})\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status, output.status.signal(), stderr.contains("has overflowed its stack"),
        );
    }
}

#[cfg(test)]
mod frame_tests {
    /// The live loop skips a repaint whose frame equals the last one painted,
    /// so equality must change exactly when the paint would.
    #[test]
    fn frames_differ_exactly_when_the_paint_would() {
        let mut vm = romacs_vm::Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        vm.eval_source("(insert \"(a b)\") (goto-char 1)").unwrap();
        let frame = |vm: &mut romacs_vm::Vm| super::frame(vm, false, 1200, 800).unwrap();
        let first = frame(&mut vm);
        assert!(first.selected.is_some());
        assert!(frame(&mut vm) == first, "an unchanged frame repaints");
        // A timer that changes nothing on screen, though restoring point
        // moves the VM's conservative display revision.
        let version = vm.display_version();
        vm.eval_source("(save-excursion (forward-char 1)) (setq romacs-test-probe (buffer-substring 1 3))").unwrap();
        assert!(vm.display_version() > version);
        assert!(frame(&mut vm) == first, "a timer that changes nothing repaints");
        vm.eval_source("(forward-char 1)").unwrap();
        let moved = frame(&mut vm);
        assert!(moved != first, "point motion must repaint");
        vm.eval_source("(select-window (next-window))").unwrap();
        let selected = frame(&mut vm);
        assert!(selected.selected != moved.selected && selected != moved, "selecting another window must repaint");
    }

    /// A prompt read mid-command presents over the last paint; finishing it
    /// without changing the editor (switching to the shown buffer) must still
    /// repaint, or the prompt stays on screen.
    #[test]
    fn a_prompt_presented_since_the_last_paint_forces_the_next_one() {
        let mut vm = romacs_vm::Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        let shown = super::Shown { frame: super::frame(&mut vm, false, 1200, 800).unwrap(), ends: vec![], presentation: 4 };
        let same = super::frame(&mut vm, false, 1200, 800).unwrap();
        assert!(super::changed_frame(&mut vm, &shown, same, 4).unwrap().is_none(), "an unchanged frame still on screen repaints");
        let same = super::frame(&mut vm, false, 1200, 800).unwrap();
        assert!(super::changed_frame(&mut vm, &shown, same, 5).unwrap().is_some(), "a prompt's presentation left the frame unrepainted");
    }

    /// Selecting the buffer a window already shows resets its end, though its
    /// frame stays equal; keeping the paint must keep `window-end' too.
    #[test]
    fn a_kept_paint_keeps_its_window_ends() {
        let mut vm = romacs_vm::Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        vm.eval_source("(erase-buffer) (let ((i 0)) (while (< i 300) (insert \"a long line of text\\n\") (setq i (1+ i)))) (goto-char 1)").unwrap();
        let painted = super::frame(&mut vm, false, 1200, 800).unwrap();
        let cursor = painted.selected.map(|id| (id, romacs_display_protocol::Color::from_pixel(0x9099d9)));
        let (_, ends) = romacs_layout2::LayoutEngine::new().unwrap().layout_frame(&painted.snapshot, 1, cursor).unwrap();
        vm.display_apply_layout(&ends).unwrap();
        let end = vm.eval_source("(window-end)").unwrap().display().unwrap();
        assert_ne!(end, vm.eval_source("(point-max)").unwrap().display().unwrap(), "the window shows part of the buffer");
        let shown = super::Shown { frame: painted, ends, presentation: 1 };
        vm.eval_source("(romacs-select-buffer (current-buffer))").unwrap();
        let next = super::frame(&mut vm, false, 1200, 800).unwrap();
        assert!(super::changed_frame(&mut vm, &shown, next, 1).unwrap().is_none(), "the frame did not change");
        assert_eq!(vm.eval_source("(window-end)").unwrap().display().unwrap(), end);
    }
}

/// What one paint shows: the windows and which of them has the cursor.
struct Frame {
    snapshot: romacs_layout_input::FrameSnapshot,
    selected: Option<u64>,
}

/// Equal frames paint identically, so an equal one needs no repaint.
/// `buffer_version` is left out: it is the VM's conservative redraw signal,
/// which also moves when nothing shown changed (a `save-excursion` restoring
/// point), and layout never reads it.
impl PartialEq for Frame {
    fn eq(&self, other: &Self) -> bool {
        self.selected == other.selected && frames_alike(&self.snapshot, &other.snapshot)
    }
}

/// Whether frames A and B paint identically; the prompt host (prompt.rs)
/// asks it too.
pub(crate) fn frames_alike(a: &romacs_layout_input::FrameSnapshot, b: &romacs_layout_input::FrameSnapshot) -> bool {
    a.font_selection == b.font_selection
        && a.font_alternatives == b.font_alternatives && (a.width, a.height) == (b.width, b.height)
        && a.windows.len() == b.windows.len()
        && a.windows.iter().zip(&b.windows).all(|(a, b)| (a.x, a.y) == (b.x, b.y) && paints_alike(&a.snapshot, &b.snapshot))
}

fn paints_alike(a: &romacs_layout_input::WindowSnapshot, b: &romacs_layout_input::WindowSnapshot) -> bool {
    a.lays_out_like(b) && a.cursor_off == b.cursor_off
}

/// What the live loop last painted: the frame, the window ends its layout
/// reported, and the presentation that showed it.
struct Shown {
    frame: Frame,
    ends: Vec<romacs_vm::WindowEnd>,
    presentation: u64,
}

/// The frame to paint after a batch, or None when the paint on screen still
/// shows it: equal, and still the latest presentation. A prompt read in the
/// middle of a command presents over that paint and advances PRESENTATION,
/// so the frame after the prompt is painted even when nothing else changed.
/// A kept paint's window ends are applied again: a command can drop them
/// without changing the frame (`set-window-buffer' to the buffer shown), and
/// the same frame lays out to the same ends.
fn changed_frame(vm: &mut Vm, shown: &Shown, next: Frame, presentation: u64) -> Result<Option<Frame>, String> {
    if shown.presentation != presentation || shown.frame != next { return Ok(Some(next)); }
    vm.display_apply_layout(&shown.ends)?;
    Ok(None)
}

fn frame(vm: &mut Vm, g1: bool, width: u32, height: u32) -> Result<Frame, String> {
    let snapshot = snapshot(vm, g1, width, height)?;
    let selected = selected_window(vm, &snapshot)?;
    Ok(Frame { snapshot, selected })
}

/// The selected window's id, once the GUI's own Lisp lays out windows.
fn selected_window(vm: &mut Vm, snapshot: &romacs_layout_input::FrameSnapshot) -> Result<Option<u64>, String> {
    if vm.eval_source("(boundp 'romacs-gui-quit-requested)")? == romacs_vm::Fixed::NIL { return Ok(None); }
    let remaining = vm.eval_source(startup::SELECTED_TAIL)?;
    let remaining = remaining.display()?.parse::<usize>().map_err(|e| e.to_string())?;
    let index = snapshot.windows.len().checked_sub(1 + remaining).ok_or("selected window not in frame")?;
    Ok(Some(snapshot.windows[index].snapshot.id))
}

fn snapshot(vm: &mut Vm, g1: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> {
    if g1 {
        // Preserve G1's explicitly full-height legacy selected-window fixture.
        Ok(romacs_layout_input::FrameSnapshot { font_selection: vm.font_selection_state(),
            font_alternatives: vm.font_alternative_state()?,
            width: 400, height: 300,
            windows: vec![romacs_layout_input::PlacedWindowSnapshot {
                x: 0, y: 0, snapshot: vm.display_snapshot(400, 300)?,
            }],
        })
    } else { vm.frame_snapshot(width, height) }
}

/// What a first launch shows while it builds its world.
const PREPARING: &str = "Preparing Romacs (first start)…";

/// Show PREPARING in the window as presentation 1, before any VM can
/// paint: one window of that text in a default face, laid out by an engine
/// of its own. The launch's own frames then start at 2.
fn preparing_frame(sender: &PaintSender, width: u32, height: u32) -> Result<(), String> {
    use romacs_layout_input::{FaceHandle, FrameSnapshot, PlacedWindowSnapshot, TextSlice, WindowSnapshot};
    let face = Vm::new().display_default_face()?;
    let snapshot = FrameSnapshot {
        font_selection: Default::default(), font_alternatives: Default::default(), width, height,
        windows: vec![PlacedWindowSnapshot { x: 0, y: 0, snapshot: WindowSnapshot {
            id: 1, width, height, start_byte: 0, point_byte: 0, buffer_version: 0,
            text: TextSlice { utf8: PREPARING.into(), face_runs: vec![] }, mode_line: None,
            mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0), vscroll_px: 0, tab_width: 8, wrap: Default::default(), cursor_off: false, stretches: vec![], line_numbers: None, faces: vec![face],
        } }],
    };
    let (frame, _) = romacs_layout2::LayoutEngine::new()?.layout_frame(&snapshot, 1, None)?;
    sender.submit(frame)
}

/// `--help`.
const USAGE: &str = "\
usage: romacs-vm-gui [--loadup | --gnu-lisp | --no-gnu-lisp] [OPTION...] [FILE...]
  --loadup              run GNU's loadup chain; a live launch boots from its world
  --world-store DIR     keep worlds in DIR (default $XDG_CACHE_HOME/romacs/worlds)
  --world-reset         forget the store's failed and refused builds, so the next
                        launch builds again
  --world-reset=all     remove the store's worlds and files altogether
  --geometry WxH        the window's size
  -Q, --no-init         do not load the user's init file
  ROMACS_WORLD=off      boot from source, without a world
";

fn main() {
    if let Err(error) = run() {
        eprintln!("romacs-vm-gui: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut socket = None;
    let mut floor = None;
    let mut force_gc = false;
    let mut g1 = false;
    let mut geometry = None;
    let mut live = true;
    let mut trace_latency = false;
    let mut trace_scene = false;
    let mut load_user_init = true;
    let mut quick = false;
    // Default: VM2's workspace. Running loadup.el by default was measured and
    // reverted: launch-to-ready 0.13s -> 0.95s (median of 5) with zero journey steps gained
    // and zero journey keys taken over by GNU (evidence/loadup/LOADUP.md).
    let mut launch = startup::Launch::Workspace;
    let mut world_store = None;
    let mut world_reset = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--world-store" => world_store = Some(std::path::absolute(args.next().ok_or("missing world store")?).map_err(|e| e.to_string())?),
            // Waiting for a missing world to build is what a launch does
            // now; the flag stays accepted for the scripts that passed it.
            "--world-wait" => {}
            "--world-reset" => world_reset = Some(false),
            "--world-reset=all" => world_reset = Some(true),
            "--help" => { print!("{USAGE}"); return Ok(()); }
            "--wayland" => { socket = Some(args.next().ok_or("missing socket")?); live = false; },
            "--geometry" => geometry = Some(parse_geometry(&args.next().ok_or("missing geometry WIDTHxHEIGHT")?)?),
            "--live" => live = true,
            "-Q" => (quick, load_user_init) = (true, false),
            "--no-init" => load_user_init = false,
            "--trace-latency" => trace_latency = true,
            "--trace-scene" => trace_scene = true,
            "--gc-floor" => {
                floor = Some(
                    args.next()
                        .ok_or("missing GC floor")?
                        .parse::<usize>()
                        .map_err(|e| e.to_string())?,
                )
            }
            "--gc-between-snapshot-and-paint" => force_gc = true,
            "--g1" => g1 = true,
            "--gnu-lisp" => launch = startup::Launch::GnuOnly,
            "--loadup" => launch = startup::Launch::Loadup,
            "--no-gnu-lisp" => launch = startup::Launch::Workspace,
            "--loadup-report" => return loadup_report(args.collect()),
            "--batch" => return batch::run(args.collect(), quick),
            "--" => {
                startup::visit_at_startup("--");
                for file in args.by_ref() { startup::visit_at_startup(&file); }
            },
            file if !file.starts_with('-') => startup::visit_at_startup(file),
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    if let Some(everything) = world_reset {
        let store = world_store.or_else(|| world::default_store(|name| std::env::var_os(name))).ok_or("no world store to reset")?;
        let removed = romacs_vm::world::store::reset(&store, everything).map_err(|e| e.to_string())?;
        eprintln!("romacs-world: {}", serde_json::json!({"event": "reset", "store": store, "everything": everything, "removed": removed}));
        return Ok(());
    }
    TRACE_LATENCY.store(trace_latency, std::sync::atomic::Ordering::Relaxed);
    SCENE_DUMPS.store(!live || trace_scene, std::sync::atomic::Ordering::Relaxed);
    if trace_latency { romacs_display_runtime::paint_host::enable_key_latency_trace(); }
    let (width, height) = geometry.unwrap_or(if live { (1600, 1000) } else { (400, 300) });
    // A live --loadup launch boots from its world, in --world-store or by
    // default in the user's cache (world::default_store), unless
    // ROMACS_WORLD=off. Its builder builds here, before any window; a launch
    // opens and maps the world on the evaluator's thread while this one
    // brings up the window and the GPU.
    let world_store = match (live && launch == startup::Launch::Loadup, romacs_vm::world::store::disabled()) {
        (true, false) => world_store.or_else(|| world::default_store(|name| std::env::var_os(name))),
        (true, true) => {
            eprintln!("romacs-world: {}", serde_json::json!({"mode": "source", "reason": "ROMACS_WORLD=off"}));
            None
        }
        (false, _) => None,
    };
    if let Some(store) = world_store.as_ref().filter(|_| romacs_vm::world::store::building()) {
        world::launch(&world::open(store.clone(), width, height, floor)?, width, height, floor, || {});
    }
    let socket = socket.or_else(|| std::env::var("WAYLAND_DISPLAY").ok()).unwrap_or_else(|| "wayland-0".into());
    let (host, sender) =
        PaintHost::wayland(std::path::Path::new(&socket), width, height, if live { "romacs — VM" } else { "VM G1a" }.into())?;
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let worker = spawn_evaluator(move || {
        let sender = std::sync::Arc::new(sender);
        let mut first_presentation = 1;
        let world = world_store.and_then(|store| match world::open(store, width, height, floor) {
            Ok(world) => Some(world),
            Err(reason) => {
                eprintln!("romacs-world: {}", serde_json::json!({"mode": "source", "reason": reason}));
                None
            }
        });
        let restored = world.as_ref().and_then(|world| world::launch(world, width, height, floor, || {
            // Said first, so a launcher sees it whether or not a window can.
            eprintln!("romacs-world: {}", serde_json::json!({"event": "preparing", "message": PREPARING}));
            let painted = preparing_frame(&sender, width, height);
            if painted.is_ok() { first_presentation = 2; }
            eprintln!("romacs-world: {}", serde_json::json!({"event": "preparing-frame", "painted": painted.is_ok(),
                                                               "error": painted.err()}));
        }));
        // Bounding the store waits for the first frame, on a thread of its own.
        let prune = world.map(|world| move || { std::thread::spawn(move || {
            if let Ok(deleted) = world.prune() && !deleted.is_empty() {
                eprintln!("romacs-world: {}", serde_json::json!({"event": "pruned", "worlds": deleted}));
            }
        }); });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            evaluate(&sender, floor, force_gc, g1, live, launch, width, height, load_user_init, quick, restored,
                     first_presentation, prune)
        }));
        let result = match result {
            Ok(result) => result.map(|()| 0),
            Err(payload) => match payload.downcast::<SessionEnd>() {
                Ok(end) => Ok(end.0),
                Err(payload) => std::panic::resume_unwind(payload),
            },
        };
        if let Err(error) = &result { eprintln!("romacs: evaluator stopped: {error}"); }
        // The status goes first: dropping the sender closes the window, and
        // the window's end lets the main thread look for it.
        let _ = result_tx.send(result);
        drop(sender);
    })?;
    host.run().map_err(|e| e.to_string())?;
    // Native startup failure must not join an evaluator blocked in stdin.
    match result_rx.try_recv() {
        Ok(result) => {
            worker.join().map_err(|_| "evaluator panicked")?;
            match result? {
                0 => Ok(()),
                code => std::process::exit(code),
            }
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("evaluator panicked".into()),
        Err(std::sync::mpsc::TryRecvError::Empty) => Ok(()),
    }
}
/// `--loadup-report [EXPR...]`: run `--loadup`'s GNU Lisp chain exactly as
/// the GUI's startup does, on the same 64 MiB evaluator, with no window; print
/// the per-site table, then each EXPR evaluated in that VM and its printed value.
fn loadup_report(exprs: Vec<String>) -> Result<(), String> {
    spawn_evaluator(move || -> Result<(), String> {
        let mut vm = Vm::new();
        let report = startup::drive_loadup(&mut vm, &startup::lisp_dir())?;
        print!("{}", report.table());
        eprintln!("romacs: {} ({:.1}s); GNU's dump would stop at {}", report.status(), report.seconds,
                  report.gnu_would_stop().unwrap_or_else(|| "nothing".into()));
        for expr in exprs {
            let printed = vm.eval_source(&expr).and_then(|v| v.display());
            println!("eval\t{expr}\t{}", match printed { Ok(p) => p, Err(e) => format!("ERROR {e}") });
        }
        Ok(())
    })?.join().map_err(|_| "evaluator panicked".to_owned())?
}
fn reply(value: Json) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{value}").map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}
fn activate(sender: &PaintSender, presentation: u64, pending: &mut std::collections::VecDeque<InputEvent>) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        match sender
            .input()
            .recv_timeout(wait)
            .map_err(|e| e.to_string())?
        {
            InputEvent::PresentationActivated {
                presentation: actual,
                ..
            } if actual == presentation => return Ok(()),
            InputEvent::WindowClose { .. } => return Err("window closed".into()),
            event => pending.push_back(event),
        }
    }
}
fn paint(
    engine: &mut romacs_layout2::LayoutEngine,
    vm: &mut Vm,
    sender: &PaintSender,
    presentation: u64,
    force_gc: bool,
    floor: Option<usize>,
    shown: Frame,
    pending: &mut std::collections::VecDeque<InputEvent>,
    synchronous: bool,
    timings: &mut Option<latency::Paint>,
) -> Result<Shown, String> {
    if force_gc {
        // Only this explicit root is retained. The snapshot has no heap handles.
        vm.set_collection_floor(1_000_000);
        let witness = vm.eval_source("(progn (vector 9) (copy-sequence \"g1a-gc-witness\"))")?;
        // Debug prints the tagged bits: observable, never usable as a handle.
        let bits = format!("{witness:?}");
        let witness = witness.root();
        let before = vm.heap_stats().collections;
        vm.collect_garbage();
        let after = vm.heap_stats().collections;
        vm.set_collection_floor(floor.unwrap_or(1_000_000));
        let current = witness.get(vm)?;
        let moved = format!("{current:?}") != bits;
        if after <= before || !moved || current.display()? != "\"g1a-gc-witness\"" {
            return Err("forced collector did not preserve the rooted witness".into());
        }
        eprintln!(
            "{}",
            json!({"diagnostic":"gc-between-snapshot-and-paint","presentation":presentation,
            "collections_before":before,"collections_after":after,"witness_moved":moved})
        );
    }
    let layout_start = Instant::now();
    let (snapshot, selected) = (&shown.snapshot, shown.selected);
    let (mut frame, ends) = if let Some(selected) = selected {
        engine.layout_frame(snapshot, presentation, Some((selected, romacs_display_protocol::Color::from_pixel(0x9099d9))))?
    } else { engine.layout_frame(snapshot, presentation, None)? };
    if selected.is_some() {
        let mut effects = romacs_display_protocol::EffectsConfig::default();
        effects.cursor_color_cycle.enabled = false;
        for window in &snapshot.windows {
            frame.set_window_cursor_effects(romacs_display_protocol::DisplayWindowId::new(window.snapshot.id as i64), effects.clone());
        }
    }
    let layout_ns = layout_start.elapsed().as_nanos();
    let diagnostic_start = Instant::now();
    let resolved_fonts = frame.fonts.len();
    let hex = |color: romacs_display_protocol::Color| {
        let channel = romacs_display_protocol::Color::linear_component_to_srgb_u8;
        format!(
            "#{:02x}{:02x}{:02x}",
            channel(color.r),
            channel(color.g),
            channel(color.b)
        )
    };
    eprintln!("{}", json!({"diagnostic":"cursor-publication", "presentation":presentation,
        "cursors":frame.window_cursors.iter().map(|c| json!({"window":c.window_id.get(),"x":c.x,"y":c.y,"active":c.active,"color":hex(c.color)})).collect::<Vec<_>>(),
        // Point as a buffer byte: a window publishes its text from a base byte.
        "points":snapshot.windows.iter().map(|w| json!({"id":w.snapshot.id,"point_byte":vm.published_buffer_byte(w.snapshot.id, w.snapshot.point_byte)})).collect::<Vec<_>>() }));
    let glyphs: Vec<_> = frame.glyphs.iter().filter(|_| SCENE_DUMPS.load(std::sync::atomic::Ordering::Relaxed)).filter_map(|glyph| {
        if let romacs_display_protocol::FrameGlyph::Char { char, row_role, face_id, x, y, clip_rect, .. } = glyph {
            let face = &frame.faces[face_id];
            Some(json!({"char":char,"role":format!("{row_role:?}"),"foreground":hex(face.foreground),
                "background":hex(face.background),"x":x,"y":y,"clip":clip_rect}))
        } else { None }
    }).collect();
    let diagnostics_prepare_ns = diagnostic_start.elapsed().as_nanos();
    let submit_start = Instant::now();
    sender.submit(frame)?;
    let submit_ns = submit_start.elapsed().as_nanos();
    let activation_start = Instant::now();
    if synchronous { activate(sender, presentation, pending)?; }
    let activation_ns = activation_start.elapsed().as_nanos();
    let wait_start = Instant::now();
    let diagnostic = if synchronous {
        Some(sender.wait_for_render(presentation, std::time::Duration::from_secs(30))?)
    } else { None };
    let wait_render_ns = wait_start.elapsed().as_nanos();
    let diagnostic_write_start = Instant::now();
    eprintln!(
        "{}",
        json!({"diagnostic":if synchronous { "paint" } else { "paint-submitted" },"presentation":presentation,
        "buffer_versions":snapshot.windows.iter().map(|w|w.snapshot.buffer_version).collect::<Vec<_>>(),"resolved_fonts":resolved_fonts,
        "unresolved_face_text_total":diagnostic.map(|sample| sample.unresolved_face_text_total)})
    );
    if SCENE_DUMPS.load(std::sync::atomic::Ordering::Relaxed) { eprintln!(
        "{}",
        json!({"diagnostic":"submitted-scene", "presentation":presentation,
        "vscroll_px":snapshot.windows.first().map(|w|w.snapshot.vscroll_px),"glyphs":glyphs})
    ); }
    if tracing() {
        eprintln!("{}", json!({"diagnostic":if synchronous { "latency-paint" } else { "latency-submit" }, "presentation":presentation, "units":"ns",
            "layout_ns":layout_ns,"diagnostics_prepare_ns":diagnostics_prepare_ns,"submit_ns":submit_ns,
            "activation_ns":activation_ns,"wait_render_ns":wait_render_ns,
            "activation_and_render_ns":activation_ns + wait_render_ns,
            "diagnostics_write_ns":diagnostic_write_start.elapsed().as_nanos(),
            "render_semantics":"renderer-successful-submit-not-compositor-present",
            "timing_scope":"activation/wait split is scheduler-dependent; diagnostics_prepare includes cursor log; key total includes diagnostic writes and layout apply"}));
    }
    let diagnostics_write_ns = diagnostic_write_start.elapsed().as_nanos();
    let apply_start = tracing().then(Instant::now);
    vm.display_apply_layout(&ends)?;
    let layout_apply_ns = apply_start.map(|start| start.elapsed().as_nanos()).unwrap_or(0);
    let mode_line_start = tracing().then(Instant::now);
    for message in vm.take_display_diagnostics() {
        eprintln!(
            "{}",
            json!({"diagnostic":"mode-line","presentation":presentation,"message":message})
        );
    }
    if tracing() {
        *timings = Some(latency::Paint { submitted: submit_start, layout_ns,
            diagnostic_ns: diagnostics_prepare_ns + diagnostics_write_ns + mode_line_start.unwrap().elapsed().as_nanos(),
            submit_ns, layout_apply_ns });
    }
    Ok(Shown { frame: shown, ends, presentation })
}
/// Lay out and submit a frame for a prompt waiting inside a command
/// (prompt.rs). The same cursor policy as `paint`; no VM access, because the
/// evaluator is suspended in Lisp and redisplay already ran in its own engine.
fn paint_prompt(
    engine: &mut romacs_layout2::LayoutEngine,
    sender: &PaintSender,
    presentation: u64,
    frame: &romacs_vm::HostFrame,
) -> Result<(usize, Vec<romacs_vm::WindowEnd>), String> {
    let cursor = Some((frame.selected, romacs_display_protocol::Color::from_pixel(0x9099d9)));
    let (mut glyphs, ends) = engine.layout_frame(&frame.frame, presentation, cursor)?;
    let mut effects = romacs_display_protocol::EffectsConfig::default();
    effects.cursor_color_cycle.enabled = false;
    for window in &frame.frame.windows {
        glyphs.set_window_cursor_effects(romacs_display_protocol::DisplayWindowId::new(window.snapshot.id as i64), effects.clone());
    }
    let count = glyphs.glyphs.len();
    let observed = SCENE_DUMPS.load(std::sync::atomic::Ordering::Relaxed).then(|| {
        glyphs.glyphs.iter().filter_map(|glyph| {
            if let romacs_display_protocol::FrameGlyph::Char { char, row_role, x, y, clip_rect, .. } = glyph {
                Some(json!({"char":char,"role":format!("{row_role:?}"),"x":x,"y":y,"clip":clip_rect}))
            } else { None }
        }).collect::<Vec<_>>()
    });
    sender.submit(glyphs)?;
    if let Some(observed) = observed {
        eprintln!("{}", json!({"diagnostic":"submitted-scene","presentation":presentation,"glyphs":observed}));
    }
    Ok((count, ends))
}
#[allow(clippy::too_many_arguments)]
fn evaluate(
    sender: &std::sync::Arc<PaintSender>,
    floor: Option<usize>,
    force_gc: bool,
    g1: bool,
    live: bool,
    launch: startup::Launch,
    mut width: u32,
    mut height: u32,
    load_user_init: bool,
    quick: bool,
    restored: Option<(Box<Vm>, world::Carry)>,
    first_presentation: u64,
    after_first_frame: Option<impl FnOnce()>,
) -> Result<(), String> {
    let engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout2::LayoutEngine::new()?));
    let (mut vm, carry) = match restored {
        Some((vm, carry)) => {
            let mut vm = *vm;
            vm.world_reinit();
            (vm, Some(carry))
        }
        None => (Vm::new(), None),
    };
    // Measuring primitives ask the engine that paints (VM2-WTPS1).
    vm.set_layout_host(Some(Box::new(measure::Host { engine: engine.clone() })));
    // The window's session ends with `kill-emacs': its hook has run, so the
    // host unwinds the evaluator to the worker, which drops the VM (killing
    // its children, as shut_down_emacs does) and exits with the status.
    if live { vm.set_session_host(Some(Box::new(|code| std::panic::resume_unwind(Box::new(SessionEnd(code)))))); }
    // Its window system, which the session opens (fonts.rs).
    vm.set_display_host(Some(Box::new(fonts::Display::new()?)));
    let terminal_service = terminal_service::Service::new();
    let terminal_ready = terminal_service.ready();
    // A completed background command wakes the same wait as a terminal.
    if live { vm.set_background_wake(terminal_service.waker()); }
    // SIGTERM and SIGHUP end the session through `kill-emacs' (terminate):
    // the signal wakes the same wait.
    if live { romacs_vm::host::catch_termination_signals(terminal_service.waker()).map_err(|e| e.to_string())?; }
    if live { vm.set_terminal_host(Some(Box::new(terminal_service)))?; }
    // The window shows an echo area: printing to `t' goes there, as on GNU's terminal.
    if live { vm.set_echo_area_host(true); }
    if let Some(floor) = floor {
        vm.set_collection_floor(floor);
    }
    // A restored VM holds the geometry and what follows, up to the chain's
    // end: its world's recipe fixed them.
    if carry.is_none() {
        let (line_height, column_width) = romacs_layout2::frame_metrics(&vm.display_default_face()?)?;
        vm.set_frame_geometry(width, height, line_height, column_width)?;
    }
    if live {
        if launch != startup::Launch::GnuOnly && carry.is_none() {
            vm.eval_source(world::INTERPROGRAM)?;
        }
        // The launch font's face, as source: declared BEFORE GNU's Lisp, so
        // GNU's own `defface' (custom.el) leaves the declared face alone, and
        // after the workspace's own faces otherwise, exactly as before.
        let font = world::launch_font()?;
        let started = if launch == startup::Launch::Workspace {
            let started = startup::initialize(&mut vm)?;
            if let Some((source, _)) = &font { vm.eval_source(source)?; }
            started
        } else if let Some(carry) = carry {
            // As a fresh process takes the per-process startup steps.
            vm.eval_source("(romacs-world-restart)")?;
            startup::resume_loadup(&mut vm, launch, carry.loadup)?
        } else {
            startup::initialize_loadup(&mut vm, launch, font.as_ref().map(|(source, _)| source.as_str()))?
        };
        if quick && launch == startup::Launch::Loadup {
            vm.eval_source(startup::QUICK_START)?;
        }
        if load_user_init && launch != startup::Launch::GnuOnly {
            if let Some(path) = user_init::path() {
                if let Err(error) = user_init::load(&mut vm, &path) {
                    eprintln!("romacs: init failed: {error}");
                    let _ = echo::show(&mut vm, "Error loading init.el; see stderr for details");
                }
            }
        }
        if launch == startup::Launch::Loadup {
            if let Err(error) = startup::finish_loadup(&mut vm, &started) {
                eprintln!("romacs: startup after init failed: {error}");
            }
        }
        eprintln!("romacs: {}", started.status());
        if let Some(loadup) = &started.loadup {
            for line in loadup.table().lines() { eprintln!("romacs-loadup: {line}"); }
        }
        // Per journey key: whose command it runs. The journey judge reads these
        // so a reply can never pass a stand-in off as GNU's command.
        for (key, gnu, romacs) in startup::STAND_INS {
            if let Some(b) = started.bindings.iter().find(|b| b.key == key) {
                eprintln!("romacs-binding: {key}\t{gnu}\t{}", if b.gnu_defined { gnu } else { romacs });
            }
        }
        let face = vm.display_default_face()?;
        if let Some((_, points)) = font {
            eprintln!("romacs: font family={} points={} pixels={}", face.family, points, face.pixel_size);
        } else {
            eprintln!("romacs: font family={} pixels={}", face.family, face.pixel_size);
        }
        let (lh, cw) = romacs_layout2::frame_metrics(&face)?;
        vm.set_frame_geometry(width, height, lh, cw)?;
    } else {
        vm.eval_source(if g1 { include_str!("../g1.el") } else { SCENE })?;
    }
    // A live GUI has a keyboard: prompts read it mid-command (prompt.rs).
    let host = std::rc::Rc::new(std::cell::RefCell::new(prompt::Shared::default()));
    host.borrow_mut().presentation = first_presentation;
    let last_presentation = || host.borrow().presentation;
    if live {
        vm.set_host_input(Some(Box::new(prompt::GuiHost { shared: host.clone(), sender: sender.clone(), engine: engine.clone(), terminal_ready: terminal_ready.clone(), processes: vec![] })));
    }
    let initial = frame(&mut vm, g1, width, height)?;
    let mut pending = std::collections::VecDeque::new();
    let mut paint_timings = None;
    let mut key_trace = tracing().then(latency::Trace::default);
    let mut previous = paint(&mut engine.borrow_mut(), &mut vm, sender, last_presentation(), force_gc, floor, initial, &mut pending, true, &mut paint_timings)?;
    let mut live_presentation = presentation::Presentation::new();
    let mut terminal_modal = false;
    reply(json!({"ready":true,"presentation":last_presentation()}))?;
    if let Some(after) = after_first_frame { after(); }
    let (input_tx, input_rx) = std::sync::mpsc::sync_channel(1);
    if !live { std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if input_tx.send(line).is_err() {
                break;
            }
        }
    }); }
    loop {
        if live {
            terminate(&mut vm, &host, Lent { queue: &mut pending, live: &mut live_presentation, width: &mut width, height: &mut height })?;
            // Background callbacks are Lisp the loop runs, and may prompt.
            let lent = Lent { queue: &mut pending, live: &mut live_presentation, width: &mut width, height: &mut height };
            let (changed, closed) = lent.run(&host, |_| service_native(&mut vm));
            if changed? { live_presentation.changed(); }
            if closed && close_window(&mut vm, &host, Lent { queue: &mut pending, live: &mut live_presentation, width: &mut width, height: &mut height })? {
                return shutdown_editor(&mut vm, sender);
            }
        }
        let available = sender.input().len();
        let mut batch: std::collections::VecDeque<_> = presentation::take_batch(&mut pending, sender.input().try_iter(), available).collect();
        loop {
            // read_char takes Lisp's unread queues before host input: a key
            // Lisp read and pushed back (sit-for in a timer) starts the next
            // command, which the Lisp command loop reads.
            let unread = live && command::unread_pending(&mut vm)?;
            let event = if unread { None } else {
                match batch.pop_front() { Some(event) => Some(event), None => break }
            };
            let (received, command_start);
            // A command to run, and the host key starting it if one does.
            let translated: Option<Option<romacs_vm::CommandEvent>> = if event.is_none() {
                vm.timer_stop_idle();
                (received, command_start) = (None, Instant::now());
                Some(None)
            } else {
                let event = event.expect("host input");
                if matches!(event, InputEvent::WindowClose { .. }) {
                    if !live || close_window(&mut vm, &host, Lent { queue: &mut batch, live: &mut live_presentation, width: &mut width, height: &mut height })? {
                        return shutdown_editor(&mut vm, sender);
                    }
                    terminal_modal = true;
                    continue;
                }
                received = if tracing() {
                    romacs_display_runtime::paint_host::take_key_enqueue_time(&event)
                } else { None };
                // read_char's timer_stop_idle: Emacs is idle until a key arrives.
                if live && timers::ends_idle(&event) { vm.timer_stop_idle(); }
                let event = command::unwrap_event(event);
                command_start = Instant::now();
                if live && let InputEvent::WindowResize { width: w, height: h, .. } = event {
                    if w > 0 && h > 0 && (width, height) != (w, h) {
                        width = w; height = h;
                        let (lh, cw) = romacs_layout2::frame_metrics(&vm.display_default_face()?)?;
                        vm.set_frame_geometry(width, height, lh, cw)?;
                        if let Err(error) = vm.resize_selected_terminal() { echo::show(&mut vm, &error)?; }
                        live_presentation.changed();
                    }
                    continue;
                }
                if live && let InputEvent::WindowFocus { focused, .. } = event {
                    // keyboard.c's FOCUS_IN_EVENT and FOCUS_OUT_EVENT: special
                    // events (frame.el handle-focus-in), which blink-cursor-mode
                    // and other focus state follow.
                    let event = if focused { "(list 'focus-in (selected-frame))" } else { "(list 'focus-out (selected-frame))" };
                    if let Err(error) = vm.eval_source(&format!("(romacs--run-special-event {event})")) { echo::show(&mut vm, &error)?; }
                    live_presentation.changed();
                    continue;
                }
                if live && let Some((session, input)) = vm.selected_terminal()? {
                    let mode = if input { terminal_input::Mode::Terminal } else { terminal_input::Mode::Emacs };
                    match terminal_input::route(&event, mode, terminal_modal) {
                        terminal_input::Target::Terminal => {
                            let sent = vm.terminal_input_state(session).and_then(|state| {
                                let bytes = terminal_input::encode(&event, state.application_cursor)?;
                                if bytes.is_empty() { Ok(()) }
                                else if !state.running { Err("Terminal is not running".into()) }
                                else { vm.terminal_send(session, &bytes) }
                            });
                            if let Err(error) = sent { echo::show(&mut vm, &error)?; }
                            live_presentation.changed();
                            continue;
                        }
                        terminal_input::Target::Ignore => continue,
                        terminal_input::Target::Editor => {}
                    }
                }
                // The Lisp command loop reads the key itself, from the queue
                // lent to the host keyboard.
                let raw = match &event {
                    &InputEvent::Key { ref key, modifiers, pressed, emacs_frame_id } if live =>
                        Some(InputEvent::Key { key: key.clone(), modifiers, pressed, emacs_frame_id }),
                    _ => None,
                };
                let key = native::translate(event);
                if let (Some(_), Some(raw)) = (&key, raw) { batch.push_front(raw); }
                key.map(Some)
            };
            if let Some(first) = translated {
                let outcome = if live {
                    let lent = Lent { queue: &mut batch, live: &mut live_presentation, width: &mut width, height: &mut height };
                    let (outcome, closed) = lent.run(&host, |live| live.host_command(&mut vm, first.as_ref()));
                    if closed && close_window(&mut vm, &host, Lent { queue: &mut batch, live: &mut live_presentation, width: &mut width, height: &mut height })? {
                        return shutdown_editor(&mut vm, sender);
                    }
                    outcome
                } else {
                    vm.eval_source("(set-buffer (window-buffer (selected-window)))")?;
                    vm.command_event(first.expect("a host key")).map(Some)
                };
                if live && vm.eval_source("romacs-gui-quit-requested")? != romacs_vm::Fixed::NIL {
                    return shutdown_editor(&mut vm, sender);
                }
                if live {
                    terminal_modal = vm.eval_source("(and (boundp 'romacs-gui-pending) romacs-gui-pending)")? != romacs_vm::Fixed::NIL;
                    if let Err(error) = vm.resize_selected_terminal() { echo::show(&mut vm, &error)?; }
                }
                let command_ns = command_start.elapsed().as_nanos();
                let snapshot_ns = if !live {
                    let snapshot_start = Instant::now();
                    let next = frame(&mut vm, g1, width, height)?;
                    let snapshot_ns = snapshot_start.elapsed().as_nanos();
                    let presentation = next_presentation(&host)?;
                    previous = paint(&mut engine.borrow_mut(), &mut vm, sender, presentation, force_gc, floor, next, &mut pending, true, &mut paint_timings)?;
                    Some(snapshot_ns)
                } else { None };
                if tracing() && !live {
                    eprintln!("{}",json!({"diagnostic":"latency-key","presentation":last_presentation(),"units":"ns",
                        "queue_ns":received.map(|t| command_start.duration_since(t).as_nanos()),
                        "command_ns":command_ns,"snapshot_ns":snapshot_ns,
                        "key_to_paint_complete_ns":if live { None } else { received.map(|t| t.elapsed().as_nanos()) },
                        "key_to_command_complete_ns":received.map(|t| t.elapsed().as_nanos()),
                        "presentation_pending":live,
                        "evaluator_total_ns":command_start.elapsed().as_nanos(),
                        "force_gc":force_gc,"gc_floor":floor,"scene_dumps":SCENE_DUMPS.load(std::sync::atomic::Ordering::Relaxed)}));
                }
                let diagnostic_start = key_trace.as_ref().map(|_| Instant::now());
                let mut reply_ns = 0;
                match outcome {
                    Ok(Some(outcome)) => {
                        if outcome.bell { eprint!("\x07"); }
                        if let Some(message) = &outcome.message { eprintln!("{message}"); }
                        let reply_start = diagnostic_start.map(|_| Instant::now());
                        reply(json!({"native_command":outcome.command,"signal":outcome.signal.map(|s|json!({"condition":s.condition,"data":s.data})),
                            "bell":outcome.bell,"message":outcome.message,"presentation":last_presentation()}))?;
                        reply_ns = reply_start.map(|start| start.elapsed().as_nanos()).unwrap_or(0);
                    }
                    Ok(None) => {},
                    Err(error) => {
                        let reply_start = diagnostic_start.map(|_| Instant::now());
                        reply(json!({"native_error":error,"presentation":last_presentation()}))?;
                        reply_ns = reply_start.map(|start| start.elapsed().as_nanos()).unwrap_or(0);
                    },
                }
                if live && let Some(trace) = &mut key_trace {
                    let ready = Instant::now();
                    let sequence = trace.sequence();
                    trace.push(latency::Key { sequence, received, start: command_start, ready,
                        command_ns, diagnostic_ns: ready.duration_since(diagnostic_start.unwrap()).as_nanos() - reply_ns, reply_ns });
                }
            }
        }
        if live {
            // A paint retired this pass lets the idle auto-save in, though a
            // repainting timer has already submitted the next.
            let mut retired = false;
            if let Some((in_flight, submitted)) = live_presentation.in_flight() {
                if let Ok(diagnostic) = sender.wait_for_render(in_flight, std::time::Duration::ZERO) {
                    let observed = Instant::now();
                    if let Some(trace) = &mut key_trace {
                        for sample in trace.completed(in_flight, observed) { eprintln!("{sample}"); }
                    }
                    eprintln!("{}", json!({"diagnostic":"paint","presentation":diagnostic.presentation,
                        "buffer_versions":previous.frame.snapshot.windows.iter().map(|w|w.snapshot.buffer_version).collect::<Vec<_>>(),
                        "unresolved_face_text_total":diagnostic.unresolved_face_text_total}));
                    if tracing() {
                        eprintln!("{}", json!({"diagnostic":"latency-render","presentation":in_flight,
                            "units":"ns","submit_to_render_observed_ns":submitted.elapsed().as_nanos(),
                            "render_semantics":"renderer-successful-submit-not-compositor-present"}));
                    }
                    live_presentation.completed(in_flight);
                    retired = true;
                } else if submitted.elapsed() >= std::time::Duration::from_secs(30) {
                    return Err(format!("no rendered atlas diagnostic for presentation {in_flight}"));
                }
            }
            if let Some(result) = live_presentation.after_batch(|| -> Result<_, String> {
                let snapshot_start = Instant::now();
                let next = frame(&mut vm, g1, width, height)?;
                let snapshot_ns = snapshot_start.elapsed().as_nanos();
                // GNU's redisplay draws nothing when nothing changed: a timer
                // such as show-paren's runs after every key.
                let Some(next) = changed_frame(&mut vm, &previous, next, last_presentation())? else { return Ok(None) };
                let presentation = next_presentation(&host)?;
                previous = paint(&mut engine.borrow_mut(), &mut vm, sender, presentation, force_gc, floor, next, &mut pending, false, &mut paint_timings)?;
                Ok(Some((snapshot_start, snapshot_ns, presentation)))
            }) {
                if let Some((snapshot_start, snapshot_ns, presentation)) = result? {
                    live_presentation.submitted(presentation, Instant::now());
                    if let Some(trace) = &mut key_trace {
                        trace.submitted(presentation, paint_timings.take().expect("traced paint"), snapshot_start, snapshot_ns);
                    }
                    if tracing() {
                        eprintln!("{}", json!({"diagnostic":"latency-snapshot","presentation":presentation,
                            "units":"ns","snapshot_ns":snapshot_ns}));
                    }
                } else {
                    live_presentation.unchanged();
                    if let Some(trace) = &mut key_trace {
                        for sample in trace.unchanged() { eprintln!("{sample}"); }
                    }
                }
            }
            if pending.is_empty() && sender.input().is_empty() {
                // read_char: with no key to read, idle and run ripe timers;
                // after any ran, redisplay and check again before blocking.
                // A timer may read input (`read-event' with a timeout, a
                // prompt) as a command may, so it gets the same handoff.
                let auto_save = retired || live_presentation.in_flight().is_none();
                let lent = Lent { queue: &mut pending, live: &mut live_presentation, width: &mut width, height: &mut height };
                let (idle, closed) = lent.run(&host, |_| timers::idle(&mut vm, auto_save));
                if closed && close_window(&mut vm, &host, Lent { queue: &mut pending, live: &mut live_presentation, width: &mut width, height: &mut height })? {
                    return shutdown_editor(&mut vm, sender);
                }
                let idle = idle.unwrap_or_else(|error| {
                    let _ = echo::show(&mut vm, &format!("Timer: {error}"));
                    timers::Idle { ran: true, message: false, delay: None }
                });
                if idle.ran {
                    if idle.message { echo::sync_current_message(&mut vm)?; }
                    live_presentation.changed();
                    continue;
                }
                // read_char's wait_reading_process_output also selects on
                // the processes: one ready wakes the loop to read it.
                let processes = vm.process_wait_fds();
                // A termination signal whose wake other work took: back to the top.
                if romacs_vm::host::termination_signal(true).is_some() { continue; }
                match terminal_wait::wait(sender.input(), &terminal_ready, timers::sooner(live_presentation.wait(), idle.delay), &processes) {
                    terminal_wait::Wake::Input(event) => pending.push_back(event),
                    terminal_wait::Wake::Closed => return Ok(()),
                    terminal_wait::Wake::Terminal | terminal_wait::Wake::Process | terminal_wait::Wake::Timeout => {},
                }
            }
            continue;
        }
        let line = match input_rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(line) => line.map_err(|e| e.to_string())?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let result = serde_json::from_str::<Json>(&line)
            .map_err(|e| e.to_string())
            .and_then(|request| {
                if request.get("apply_journal").and_then(Json::as_bool) == Some(true) {
                    Ok(json!({"journal_applied":vm.display_apply_journal()}))
                } else {
                    vm.submit(&request)
                }
            });
        // An evaluation may mutate before signalling; its error must not hide that paint.
        let next = frame(&mut vm, g1, width, height)?;
        if let Some(next) = changed_frame(&mut vm, &previous, next, last_presentation())? {
            let presentation = next_presentation(&host)?;
            previous = paint(&mut engine.borrow_mut(), &mut vm, sender, presentation, force_gc, floor, next, &mut pending, true, &mut paint_timings)?;
        }
        match result {
            Err(error) => reply(json!({"error":error,"presentation":last_presentation()}))?,
            Ok(result) => reply(json!({"result":result,"presentation":last_presentation()}))?,
        }
    }
    shutdown_editor(&mut vm, sender)
}

/// Number the next presentation. The loop and every prompt read inside the
/// Lisp it runs advance this one counter (prompt::Shared), so neither numbers
/// or compares a paint from a stale copy.
fn next_presentation(host: &std::rc::Rc<std::cell::RefCell<prompt::Shared>>) -> Result<u64, String> {
    let mut shared = host.borrow_mut();
    shared.presentation = shared.presentation.checked_add(1).ok_or("presentation exhausted")?;
    Ok(shared.presentation)
}

/// A native close: GUI quit, lent like any Lisp the loop runs, whose
/// confirmation may read input. Whether to shut down.
fn close_window(vm: &mut Vm, host: &std::rc::Rc<std::cell::RefCell<prompt::Shared>>, mut lent: Lent<'_>) -> Result<bool, String> {
    terminate(vm, host, Lent { queue: &mut *lent.queue, live: &mut *lent.live, width: &mut *lent.width, height: &mut *lent.height })?;
    lent.run(host, |live| live.window_close(vm)).0
}

/// emacs.c terminate_due_to_signal: a SIGTERM or SIGHUP the host noted runs
/// `(kill-emacs SIGNAL)', whose hook and auto-save run before the session
/// host exits with that status; a prompt it interrupted has unwound first.
fn terminate(vm: &mut Vm, host: &std::rc::Rc<std::cell::RefCell<prompt::Shared>>, lent: Lent<'_>) -> Result<(), String> {
    let Some(signal) = romacs_vm::host::termination_signal(false) else { return Ok(()) };
    lent.run(host, |_| vm.eval_source(&format!("(kill-emacs {signal})"))).0.map(|_| ())
}

/// The outer loop's input state, lent to the prompt host (prompt.rs) for
/// every piece of Lisp the loop runs (a command, a timer, a background
/// callback, a close), since any of them may read input mid-way: the keys
/// queued ahead and the presentation in flight. What the Lisp did not
/// consume, and what its reads settled or saw (a resize, a close), come
/// back; a read that presented a prompt leaves the frame to repaint, even
/// when the Lisp changed nothing else. A read that redisplays without the
/// in-flight presentation waits for one the renderer has already passed.
struct Lent<'a> {
    queue: &'a mut std::collections::VecDeque<InputEvent>,
    live: &'a mut presentation::Presentation,
    width: &'a mut u32,
    height: &'a mut u32,
}

impl Lent<'_> {
    /// RUN's value, and whether a read inside it saw the frame close.
    fn run<T>(self, host: &std::rc::Rc<std::cell::RefCell<prompt::Shared>>, run: impl FnOnce(&mut presentation::Presentation) -> T) -> (T, bool) {
        let before = {
            let mut shared = host.borrow_mut();
            shared.pending = std::mem::take(self.queue);
            shared.in_flight = self.live.in_flight().map(|(p, _)| p);
            shared.presentation
        };
        let value = run(self.live);
        let mut shared = host.borrow_mut();
        *self.queue = std::mem::take(&mut shared.pending);
        if shared.presentation != before { self.live.changed(); }
        if let Some(settled) = shared.settled.take() { self.live.completed(settled); }
        shared.in_flight = None;
        if let Some((w, h)) = shared.size.take() { (*self.width, *self.height) = (w, h); }
        (value, std::mem::take(&mut shared.closed))
    }
}

/// Native work that woke the wait: terminal output, then completed background
/// commands. Draining the shared wake first means a completion that lands
/// after the drain signals the next wait, and one before it is dispatched
/// here. Whether a displayed buffer changed.
fn service_native(vm: &mut Vm) -> Result<bool, String> {
    let mut changed = vm.poll_terminal_output()?;
    if vm.background_processes_ready() {
        match vm.eval_source("(if (fboundp 'romacs-process-dispatch-ready) (romacs-process-dispatch-ready) (progn (romacs--process-ready) nil))") {
            Ok(value) => changed |= !value.is_nil(),
            Err(error) => { echo::show(vm, &format!("Background command: {error}"))?; changed = true; },
        }
    }
    Ok(changed)
}

fn shutdown_editor(vm: &mut Vm, sender: &PaintSender) -> Result<(), String> {
    vm.shutdown_background_processes()?;
    sender.shutdown()
}

fn parse_geometry(value: &str) -> Result<(u32, u32), String> {
    let (w, h) = value.split_once('x').ok_or("geometry must be WIDTHxHEIGHT")?;
    let w = w.parse::<u32>().map_err(|e| e.to_string())?;
    let h = h.parse::<u32>().map_err(|e| e.to_string())?;
    if !(320..=16384).contains(&w) || !(200..=16384).contains(&h) { return Err("geometry out of range".into()); }
    Ok((w, h))
}
