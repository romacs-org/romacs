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
mod timers;

const SCENE: &str = include_str!("../scene.el");

// GNU's widest real forms (cus-start.el:86, 208 operands at 45b0f7699df;
// comp-common.el:71, 270) overflow a default 2 MiB thread in unoptimized
// builds; optimized builds overflow 2 MiB near 1750 operands. Keep the
// evaluator off the default stack. Guarded by evaluator_stack_tests (R209).
const EVALUATOR_STACK_SIZE: usize = 64 * 1024 * 1024;

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
        let mut live = presentation::Presentation::new();
        live.submitted(7, Instant::now());
        let (mut queue, mut presentation, mut width, mut height) = (std::collections::VecDeque::new(), 7, 10, 20);
        queue.push_back(InputEvent::WindowClose { emacs_frame_id: 0 });
        let mut lent = Lent { queue: &mut queue, presentation: &mut presentation, live: &mut live, width: &mut width, height: &mut height };
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
        assert_eq!((queue.len(), presentation, width, height), (1, 8, 30, 40));
        assert!(live.in_flight().is_none(), "the settled presentation is no longer in flight");
        assert_eq!(host.borrow().in_flight, None);
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
    // Default: VM2's workspace. Running loadup.el by default was measured and
    // reverted: launch-to-ready 0.13s -> 0.95s (median of 5) with zero journey steps gained
    // and zero journey keys taken over by GNU (evidence/loadup/LOADUP.md).
    let mut launch = startup::Launch::Workspace;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--wayland" => { socket = Some(args.next().ok_or("missing socket")?); live = false; },
            "--geometry" => geometry = Some(parse_geometry(&args.next().ok_or("missing geometry WIDTHxHEIGHT")?)?),
            "--live" => live = true,
            "-Q" | "--no-init" => load_user_init = false,
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
            "--batch" => return batch::run(args.collect()),
            "--" => {
                startup::visit_at_startup("--");
                for file in args.by_ref() { startup::visit_at_startup(&file); }
            },
            file if !file.starts_with('-') => startup::visit_at_startup(file),
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    TRACE_LATENCY.store(trace_latency, std::sync::atomic::Ordering::Relaxed);
    SCENE_DUMPS.store(!live || trace_scene, std::sync::atomic::Ordering::Relaxed);
    if trace_latency { romacs_display_runtime::paint_host::enable_key_latency_trace(); }
    let socket = socket.or_else(|| std::env::var("WAYLAND_DISPLAY").ok()).unwrap_or_else(|| "wayland-0".into());
    let (width, height) = geometry.unwrap_or(if live { (1600, 1000) } else { (400, 300) });
    let (host, sender) =
        PaintHost::wayland(std::path::Path::new(&socket), width, height, if live { "romacs — VM" } else { "VM G1a" }.into())?;
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let worker = spawn_evaluator(move || {
        let sender = std::sync::Arc::new(sender);
        let result = evaluate(&sender, floor, force_gc, g1, live, launch, width, height, load_user_init);
        if let Err(error) = &result { eprintln!("romacs: evaluator stopped: {error}"); }
        let _ = result_tx.send(result);
        drop(sender);
    })?;
    host.run().map_err(|e| e.to_string())?;
    // Native startup failure must not join an evaluator blocked in stdin.
    match result_rx.try_recv() {
        Ok(result) => {
            worker.join().map_err(|_| "evaluator panicked")?;
            result
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
    snapshot: romacs_layout_input::FrameSnapshot,
    pending: &mut std::collections::VecDeque<InputEvent>,
    synchronous: bool,
    timings: &mut Option<latency::Paint>,
) -> Result<romacs_layout_input::FrameSnapshot, String> {
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
    let selected = if vm.eval_source("(boundp 'romacs-gui-quit-requested)")? != romacs_vm::Fixed::NIL {
        let remaining = vm.eval_source(startup::SELECTED_TAIL)?;
        let remaining = remaining.display()?.parse::<usize>().map_err(|e| e.to_string())?;
        let index = snapshot.windows.len().checked_sub(1 + remaining).ok_or("selected window not in frame")?;
        Some(snapshot.windows[index].snapshot.id)
    } else { None };
    let (mut frame, ends) = if let Some(selected) = selected {
        engine.layout_frame(&snapshot, presentation, Some((selected, romacs_display_protocol::Color::from_pixel(0x9099d9))))?
    } else { engine.layout_frame(&snapshot, presentation, None)? };
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
        "points":snapshot.windows.iter().map(|w| json!({"id":w.snapshot.id,"point_byte":w.snapshot.point_byte})).collect::<Vec<_>>() }));
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
        "unresolved_face_text_total":diagnostic.map(|sample| sample.unresolved_face_text_total),
        "collections":vm.heap_stats().collections})
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
    Ok(snapshot)
}
/// Lay out and submit a frame for a prompt waiting inside a command
/// (prompt.rs). The same cursor policy as `paint`; no VM access, because the
/// evaluator is suspended in Lisp and redisplay already ran in its own engine.
fn paint_prompt(
    engine: &mut romacs_layout2::LayoutEngine,
    sender: &PaintSender,
    presentation: u64,
    frame: &romacs_vm::HostFrame,
) -> Result<(usize, Vec<(u64, usize)>), String> {
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
#[allow(clippy::too_many_arguments)] // host options, as for paint()
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
) -> Result<(), String> {
    let engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout2::LayoutEngine::new()?));
    let mut vm = Vm::new();
    // Measuring primitives ask the engine that paints (VM2-WTPS1).
    vm.set_layout_host(Some(Box::new(measure::Host { engine: engine.clone() })));
    let terminal_service = terminal_service::Service::new();
    let terminal_ready = terminal_service.ready();
    // A completed background command wakes the same wait as a terminal.
    if live { vm.set_background_wake(terminal_service.waker()); }
    if live { vm.set_terminal_host(Some(Box::new(terminal_service)))?; }
    // The window shows an echo area: printing to `t' goes there, as on GNU's terminal.
    if live { vm.set_echo_area_host(true); }
    if let Some(floor) = floor {
        vm.set_collection_floor(floor);
    }
    let (line_height, column_width) = romacs_layout2::frame_metrics(&vm.display_default_face()?)?;
    vm.set_frame_geometry(width, height, line_height, column_width)?;
    if live {
        if launch != startup::Launch::GnuOnly {
            vm.eval_source("(defvar interprogram-cut-function 'romacs-clipboard-select-text) (defvar interprogram-paste-function 'romacs-clipboard-selection-value)")?;
        }
        // The launch font's face, as source: declared BEFORE GNU's Lisp, so
        // GNU's own `defface' (custom.el) leaves the declared face alone, and
        // after the workspace's own faces otherwise, exactly as before.
        let font = match std::env::var("ROMACS_GUI_FONT") {
            Ok(family) => {
                if family.chars().any(char::is_control) { return Err("font family contains a control character".into()); }
                let points = std::env::var("ROMACS_GUI_FONT_SIZE").unwrap_or_else(|_| "14".into())
                    .parse::<u32>().map_err(|e| e.to_string())?;
                if !(6..=72).contains(&points) { return Err("font size must be 6..72 points".into()); }
                Some((format!("(defface default '((t (:foreground \"#d4c5a0\" :background \"#000000\" :family {} :height {}))) \"Launch font\")", serde_json::to_string(&family).map_err(|e|e.to_string())?, points * 10), points))
            }
            Err(_) => None,
        };
        let started = if launch == startup::Launch::Workspace {
            let started = startup::initialize(&mut vm)?;
            if let Some((source, _)) = &font { vm.eval_source(source)?; }
            started
        } else {
            startup::initialize_loadup(&mut vm, launch, font.as_ref().map(|(source, _)| source.as_str()))?
        };
        if load_user_init && launch != startup::Launch::GnuOnly {
            if let Some(path) = user_init::path() {
                if let Err(error) = user_init::load(&mut vm, &path) {
                    eprintln!("romacs: init failed: {error}");
                    let _ = echo::show(&mut vm, "Error loading init.el; see stderr for details");
                }
            }
        }
        if launch == startup::Launch::Loadup {
            if let Err(error) = vm.eval_source("(romacs-gui-loadup-after-init)") {
                eprintln!("romacs: after-init failed: {error}");
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
    let mut commands = command::Commands::default();
    let mut presentation = 1;
    // A live GUI has a keyboard: prompts read it mid-command (prompt.rs).
    let host = std::rc::Rc::new(std::cell::RefCell::new(prompt::Shared::default()));
    if live {
        vm.set_host_input(Some(Box::new(prompt::GuiHost { shared: host.clone(), sender: sender.clone(), engine: engine.clone(), terminal_ready: terminal_ready.clone() })));
    }
    let initial = snapshot(&mut vm, g1, width, height)?;
    let mut pending = std::collections::VecDeque::new();
    let mut paint_timings = None;
    let mut key_trace = tracing().then(latency::Trace::default);
    let mut previous = paint(&mut engine.borrow_mut(), &mut vm, sender, presentation, force_gc, floor, initial, &mut pending, true, &mut paint_timings)?;
    // The presentation `previous` shows; a prompt's paint advances past it.
    let mut painted = presentation;
    let mut live_presentation = presentation::Presentation::new();
    let mut terminal_modal = false;
    reply(json!({"ready":true,"presentation":presentation}))?;
    let (input_tx, input_rx) = std::sync::mpsc::sync_channel(1);
    if !live { std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if input_tx.send(line).is_err() {
                break;
            }
        }
    }); }
    loop {
        if live && service_native(&mut vm)? { live_presentation.changed(); }
        let available = sender.input().len();
        let mut batch: std::collections::VecDeque<_> = presentation::take_batch(&mut pending, sender.input().try_iter(), available).collect();
        while let Some(event) = batch.pop_front() {
            if matches!(event, InputEvent::WindowClose { .. }) {
                if !live || live_presentation.window_close(&mut vm)? {
                    return shutdown_editor(&mut vm, sender);
                }
                terminal_modal = true;
                continue;
            }
            let received = if tracing() {
                romacs_display_runtime::paint_host::take_key_enqueue_time(&event)
            } else { None };
            // read_char's timer_stop_idle: Emacs is idle until a key arrives.
            if live && timers::ends_idle(&event) { vm.timer_stop_idle(); }
            let event = command::unwrap_event(event);
            let command_start = Instant::now();
            let gc_before = vm.heap_stats().collections;
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
            if live && let Some((session, input)) = vm.selected_terminal()? {
                let mode = if input { terminal_input::Mode::Terminal } else { terminal_input::Mode::Emacs };
                match terminal_input::route(&event, mode, commands.in_prefix() || terminal_modal) {
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
            if let Some(event) = native::translate(event) {
                let outcome = if live {
                    let mut lent = Lent { queue: &mut batch, presentation: &mut presentation, live: &mut live_presentation, width: &mut width, height: &mut height };
                    let (outcome, closed) = lent.run(&host, |live| live.command(&mut vm, &mut commands, event));
                    if closed && live_presentation.window_close(&mut vm)? {
                        return shutdown_editor(&mut vm, sender);
                    }
                    outcome
                } else {
                    vm.eval_source("(set-buffer (window-buffer (selected-window)))")?;
                    vm.command_event(event).map(Some)
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
                    let next = snapshot(&mut vm, g1, width, height)?;
                    let snapshot_ns = snapshot_start.elapsed().as_nanos();
                    presentation = presentation.checked_add(1).ok_or("presentation exhausted")?;
                    previous = paint(&mut engine.borrow_mut(), &mut vm, sender, presentation, force_gc, floor, next, &mut pending, true, &mut paint_timings)?;
                    painted = presentation;
                    Some(snapshot_ns)
                } else { None };
                if tracing() && !live {
                    eprintln!("{}",json!({"diagnostic":"latency-key","presentation":presentation,"units":"ns",
                        "queue_ns":received.map(|t| command_start.duration_since(t).as_nanos()),
                        "command_ns":command_ns,"snapshot_ns":snapshot_ns,
                        "key_to_paint_complete_ns":if live { None } else { received.map(|t| t.elapsed().as_nanos()) },
                        "key_to_command_complete_ns":received.map(|t| t.elapsed().as_nanos()),
                        "presentation_pending":live,
                        "evaluator_total_ns":command_start.elapsed().as_nanos(),
                        "collections_before":gc_before,"collections_after":vm.heap_stats().collections,
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
                            "bell":outcome.bell,"message":outcome.message,"presentation":presentation}))?;
                        reply_ns = reply_start.map(|start| start.elapsed().as_nanos()).unwrap_or(0);
                    }
                    Ok(None) => {},
                    Err(error) => {
                        let reply_start = diagnostic_start.map(|_| Instant::now());
                        reply(json!({"native_error":error,"presentation":presentation}))?;
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
            if let Some((in_flight, submitted)) = live_presentation.in_flight() {
                if let Ok(diagnostic) = sender.wait_for_render(in_flight, std::time::Duration::ZERO) {
                    let observed = Instant::now();
                    if let Some(trace) = &mut key_trace {
                        for sample in trace.completed(in_flight, observed) { eprintln!("{sample}"); }
                    }
                    eprintln!("{}", json!({"diagnostic":"paint","presentation":diagnostic.presentation,
                        "buffer_versions":previous.windows.iter().map(|w|w.snapshot.buffer_version).collect::<Vec<_>>(),
                        "unresolved_face_text_total":diagnostic.unresolved_face_text_total,
                        "collections":vm.heap_stats().collections}));
                    if tracing() {
                        eprintln!("{}", json!({"diagnostic":"latency-render","presentation":in_flight,
                            "units":"ns","submit_to_render_observed_ns":submitted.elapsed().as_nanos(),
                            "render_semantics":"renderer-successful-submit-not-compositor-present"}));
                    }
                    live_presentation.completed(in_flight);
                } else if submitted.elapsed() >= std::time::Duration::from_secs(30) {
                    return Err(format!("no rendered atlas diagnostic for presentation {in_flight}"));
                }
            }
            if let Some(result) = live_presentation.after_batch(|| -> Result<_, String> {
                let snapshot_start = Instant::now();
                let next = snapshot(&mut vm, g1, width, height)?;
                let snapshot_ns = snapshot_start.elapsed().as_nanos();
                // GNU's redisplay draws nothing when nothing changed: a timer
                // that left the frame as painted submits no frame.
                if next == previous && painted == presentation { return Ok(None); }
                presentation = presentation.checked_add(1).ok_or("presentation exhausted")?;
                previous = paint(&mut engine.borrow_mut(), &mut vm, sender, presentation, force_gc, floor, next, &mut pending, false, &mut paint_timings)?;
                painted = presentation;
                Ok(Some((snapshot_start, snapshot_ns)))
            }) {
                let Some((snapshot_start, snapshot_ns)) = result? else {
                    live_presentation.unchanged();
                    continue;
                };
                live_presentation.submitted(presentation, Instant::now());
                if let Some(trace) = &mut key_trace {
                    trace.submitted(presentation, paint_timings.take().expect("traced paint"), snapshot_start, snapshot_ns);
                }
                if tracing() {
                    eprintln!("{}", json!({"diagnostic":"latency-snapshot","presentation":presentation,
                        "units":"ns","snapshot_ns":snapshot_ns}));
                }
            }
            if pending.is_empty() && sender.input().is_empty() {
                // read_char: with no key to read, idle and run ripe timers;
                // after any ran, redisplay and check again before blocking.
                // A timer may read input (`read-event' with a timeout, a
                // prompt) as a command may, so it gets the same handoff.
                let mut lent = Lent { queue: &mut pending, presentation: &mut presentation, live: &mut live_presentation, width: &mut width, height: &mut height };
                let (idle, closed) = lent.run(&host, |_| timers::idle(&mut vm));
                if closed && live_presentation.window_close(&mut vm)? {
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
                match terminal_wait::wait(sender.input(), &terminal_ready, timers::sooner(live_presentation.wait(), idle.delay)) {
                    terminal_wait::Wake::Input(event) => pending.push_back(event),
                    terminal_wait::Wake::Closed => return Ok(()),
                    terminal_wait::Wake::Terminal | terminal_wait::Wake::Timeout => {},
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
        let next = snapshot(&mut vm, g1, width, height)?;
        if next != previous {
            presentation = presentation
                .checked_add(1)
                .ok_or("presentation exhausted")?;
            previous = paint(&mut engine.borrow_mut(), &mut vm, sender, presentation, force_gc, floor, next, &mut pending, true, &mut paint_timings)?;
            painted = presentation;
        }
        match result {
            Err(error) => reply(json!({"error":error,"presentation":presentation}))?,
            Ok(result) => reply(json!({"result":result,"presentation":presentation}))?,
        }
    }
    shutdown_editor(&mut vm, sender)
}

/// The outer loop's input and presentation state, lent to the prompt host
/// (prompt.rs) while the loop runs Lisp that may read input mid-way: the
/// keys queued ahead, the last presentation submitted and the one in flight.
/// What the Lisp did not consume, and what its reads presented, settled or
/// saw (a resize, a close), come back. A read that redisplays from stale
/// state submits a presentation the renderer has already passed and waits
/// for it in vain.
struct Lent<'a> {
    queue: &'a mut std::collections::VecDeque<InputEvent>,
    presentation: &'a mut u64,
    live: &'a mut presentation::Presentation,
    width: &'a mut u32,
    height: &'a mut u32,
}

impl Lent<'_> {
    /// RUN's value, and whether a read inside it saw the frame close.
    fn run<T>(&mut self, host: &std::rc::Rc<std::cell::RefCell<prompt::Shared>>, run: impl FnOnce(&mut presentation::Presentation) -> T) -> (T, bool) {
        {
            let mut shared = host.borrow_mut();
            shared.pending = std::mem::take(self.queue);
            shared.presentation = *self.presentation;
            shared.in_flight = self.live.in_flight().map(|(p, _)| p);
        }
        let value = run(self.live);
        let mut shared = host.borrow_mut();
        *self.queue = std::mem::take(&mut shared.pending);
        *self.presentation = shared.presentation;
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
