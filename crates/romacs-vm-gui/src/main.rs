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
static DIAGNOSTICS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// Per-paint lines (`paint', `paint-submitted', `cursor-publication',
/// `prompt-paint'), echo-area messages and the loadup tables on stderr:
/// what harnesses read, never a daily session's log (a message or a prompt
/// can hold what the user typed). On under ROMACS_DIAGNOSTICS=1, the trace
/// flags, and the --wayland oracle transport.
fn diagnostics() -> bool { DIAGNOSTICS.load(std::sync::atomic::Ordering::Relaxed) }
/// Write VALUE to stderr as one line, handed over as one complete buffer
/// (`stderr::line`).
/// `eprintln!("{}", value)` hands serde's pieces to unbuffered stderr a
/// syscall apiece: thousands for a scene dump, which under load held the
/// evaluator for seconds a paint.
fn diagnostic_line(value: &serde_json::Value) {
    romacs_vm_gui::stderr::line(&value.to_string());
}

use romacs_vm_gui::{command, echo, fonts, grammars, loading, loadup, measure, native, presentation, startup, terminal_service, terminal_wait};
mod latency;
mod title;
mod prompt;
mod batch;
mod user_init;
use fonts::residency;
mod defun_census;
mod timers;
mod world;
mod isolate_worker;
mod exit;
mod quit;
mod client;
mod restart;
#[cfg(test)]
#[path = "../tests/support/home.rs"]
mod test_home;
// The unit tests' loadup from a world: the library's module (lib.rs), for
// this executable's tests too.
#[cfg(test)]
mod test_world;

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

/// How the evaluator's RESULT ends the session, and whether it failed (a
/// failed startup's screen then holds, `failed_screen`): an error, or a
/// panic other than `kill-emacs`'s unwinding, which ends it as a panic.
fn evaluator_end(result: std::thread::Result<Result<exit::Exit, String>>) -> (exit::Exit, bool) {
    match result {
        Ok(Ok(end)) => (end, false),
        Ok(Err(error)) => {
            eprintln!("romacs: evaluator stopped: {error}");
            (exit::Exit::new("evaluator-error", error, 1), true)
        }
        Err(payload) => if let Ok(end) = payload.downcast::<SessionEnd>() {
            (exit::Exit::new("kill-emacs", "", end.0), false)
        } else {
            // Reported by the panic hook as it happened (exit.rs); said as
            // an error's end is, which a harness watches for.
            let end = exit::panicked("evaluator");
            eprintln!("romacs: evaluator stopped: {}", end.detail);
            (end, true)
        },
    }
}

#[cfg(test)]
mod evaluator_end_tests {
    use super::*;

    #[test]
    fn an_error_or_a_panic_fails_and_kill_emacs_ends() {
        let ended = |status| exit::Exit::new("kill-emacs", "", status);
        assert_eq!(evaluator_end(Ok(Ok(ended(0)))), (ended(0), false));
        assert_eq!(evaluator_end(Ok(Err("boom".into()))), (exit::Exit::new("evaluator-error", "boom", 1), true));
        assert_eq!(evaluator_end(Err(Box::new(SessionEnd(3)))), (ended(3), false));
        let (end, failed) = evaluator_end(Err(Box::new("a bug")));
        assert_eq!((end.reason, end.status, failed), ("panic", 101, true));
    }
}

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
        let (mut width, mut height) = (10, 20);
        queue(&host).push_back(InputEvent::WindowClose { emacs_frame_id: 0 });
        let lent = Lent { live: &mut live, width: &mut width, height: &mut height };
        let (seen, closed) = lent.run(&host, |_| {
            let mut shared = host.borrow_mut();
            let seen = (shared.presentation, shared.in_flight.map(|(p, _)| p), shared.pending.len());
            // What a prompt's drawn redisplay does: settle the loop's frame, submit its own.
            shared.settled = shared.in_flight.take().map(|(p, _)| (p, Instant::now()));
            shared.presentation += 1;
            shared.size = Some((30, 40));
            shared.closed = true;
            seen
        });
        assert_eq!(seen, (7, Some(7), 1), "the read starts from the loop's state");
        assert!(closed);
        let queued = queue(&host).len();
        assert_eq!((queued, host.borrow().presentation, width, height), (1, 8, 30, 40));
        assert!(live.in_flight().is_none(), "the settled presentation is no longer in flight");
        assert_eq!(host.borrow().in_flight, None);
    }

    /// A key a read took from the channel outside any lend (an
    /// `input-pending-p` in a redisplay, the look at the host a collection
    /// takes when it ends) stays queued: the next Lisp the loop lends to reads
    /// it, and the loop then dispatches it. Each lend used to replace the
    /// host's queue with the loop's own, so keys typed during a collection
    /// in a redisplay never reached the command loop.
    #[test]
    fn a_key_a_read_took_outside_a_lend_stays_queued() {
        use romacs_display_runtime::host_types::FrontendKey;
        let host = std::rc::Rc::new(std::cell::RefCell::new(prompt::Shared::default()));
        let key = |c| InputEvent::Key { key: FrontendKey::Character(c), modifiers: 0, pressed: true, emacs_frame_id: 1 };
        queue(&host).push_back(key('a'));
        let (mut live, mut width, mut height) = (presentation::Presentation::new(), 10, 20);
        let lent = Lent { live: &mut live, width: &mut width, height: &mut height };
        let (seen, _) = lent.run(&host, |_| {
            let mut queue = queue(&host);
            let seen = queue.len();
            queue.push_back(key('b'));
            seen
        });
        assert_eq!(seen, 1, "the lent Lisp reads the key taken before it");
        let remaining: Vec<_> = queue(&host).drain(..).filter_map(crate::native::translate).collect();
        assert_eq!(remaining, [romacs_vm::CommandEvent::Character('a' as i64), romacs_vm::CommandEvent::Character('b' as i64)]);
    }

    /// A presentation a prompt settled inside the loop's Lisp retires as one
    /// the loop saw render, so the latency trace completes its keys and
    /// takes the next paint; it used to stay in flight in the trace, whose
    /// next submission asserted and ended the session.
    #[test]
    fn a_presentation_a_prompt_settled_completes_the_latency_trace() {
        let host = std::rc::Rc::new(std::cell::RefCell::new(prompt::Shared::default()));
        let now = Instant::now();
        let (mut trace, mut live) = (latency::Trace::default(), presentation::Presentation::new());
        let key = |sequence| latency::Key { sequence, received: Some(now), start: now, ready: now,
            command_ns: 0, diagnostic_ns: 0, reply_ns: 0 };
        let paint = || latency::Paint { submitted: now, layout_ns: 0, diagnostic_ns: 0, submit_ns: 0, layout_apply_ns: 0 };
        let first = trace.sequence();
        trace.push(key(first));
        host.borrow_mut().presentation = 7;
        live.submitted(7, now);
        trace.submitted(7, paint(), now, 0);
        let (mut width, mut height) = (10, 20);
        let lent = Lent { live: &mut live, width: &mut width, height: &mut height };
        lent.run(&host, |_| {
            let mut shared = host.borrow_mut();
            shared.settled = shared.in_flight.take().map(|(p, _)| (p, now));
            shared.presentation += 1;
        });
        let [(presentation, observed)] = live.take_retired()[..] else { panic!("the settled presentation retired") };
        assert_eq!((presentation, observed), (7, now));
        assert_eq!(trace.completed(presentation, observed).len(), 1);
        let second = trace.sequence();
        trace.push(key(second));
        live.submitted(9, now);
        trace.submitted(9, paint(), now, 0);
        assert_eq!(trace.completed(9, now).len(), 1);
    }

    /// A prompt settles the loop's traced paint and leaves its own undrawn;
    /// that one renders before the loop takes the first retirement. Both
    /// retire, in order, so the trace completes its keys and takes the next
    /// paint (the second overwrote the first, and the next submission
    /// asserted).
    #[test]
    fn two_retirements_before_the_loop_looks_both_reach_the_trace() {
        let host = std::rc::Rc::new(std::cell::RefCell::new(prompt::Shared::default()));
        let now = Instant::now();
        let (mut trace, mut live) = (latency::Trace::default(), presentation::Presentation::new());
        let paint = || latency::Paint { submitted: now, layout_ns: 0, diagnostic_ns: 0, submit_ns: 0, layout_apply_ns: 0 };
        let first = trace.sequence();
        trace.push(latency::Key { sequence: first, received: Some(now), start: now, ready: now, command_ns: 0, diagnostic_ns: 0, reply_ns: 0 });
        host.borrow_mut().presentation = 7;
        live.submitted(7, now);
        trace.submitted(7, paint(), now, 0);
        let (mut width, mut height) = (10, 20);
        let lent = Lent { live: &mut live, width: &mut width, height: &mut height };
        lent.run(&host, |_| {
            let mut shared = host.borrow_mut();
            shared.settled = shared.in_flight.take().map(|(p, _)| (p, now));
            shared.presentation = 8;
            shared.in_flight = Some((8, now));
        });
        assert_eq!(live.in_flight(), Some((8, now)), "the prompt's undrawn paint is the loop's now");
        live.completed(8, now);
        let retired = live.take_retired();
        assert_eq!(retired.iter().map(|r| r.0).collect::<Vec<_>>(), vec![7, 8]);
        let samples: usize = retired.into_iter().map(|(p, at)| trace.completed(p, at).len()).sum();
        assert_eq!(samples, 1, "the traced key completes with its paint");
        trace.submitted(9, paint(), now, 0);
    }

    /// A prompt that painted into a window the compositor does not draw
    /// leaves the loop's undrawn paint in flight: the stall goes on counting,
    /// and the prompt's render when the window shows settles it, by its own
    /// id, so the latency trace retires it and takes the next paint (it
    /// asserted, ending the session, when the prompt's id replaced it).
    #[test]
    fn an_undrawn_prompt_paint_leaves_the_loops_in_flight() {
        let host = std::rc::Rc::new(std::cell::RefCell::new(prompt::Shared::default()));
        let since = Instant::now();
        let mut live = presentation::Presentation::new();
        let mut trace = latency::Trace::default();
        let paint = || latency::Paint { submitted: since, layout_ns: 0, diagnostic_ns: 0, submit_ns: 0, layout_apply_ns: 0 };
        host.borrow_mut().presentation = 7;
        live.submitted(7, since);
        trace.submitted(7, paint(), since, 0);
        let (mut width, mut height) = (10, 20);
        let lent = Lent { live: &mut live, width: &mut width, height: &mut height };
        lent.run(&host, |_| {
            let mut shared = host.borrow_mut();
            shared.presentation += 1;
            shared.in_flight = Some((shared.presentation, shared.in_flight.unwrap().1));
        });
        assert_eq!(live.in_flight(), Some((7, since)));
        live.completed(8, Instant::now());
        let [(retired, at)] = live.take_retired()[..] else { panic!("the prompt's render retired the loop's paint") };
        assert_eq!(retired, 7);
        trace.completed(retired, at);
        assert!(live.ready(), "the prompt's paint leaves the frame to repaint");
        trace.submitted(9, paint(), since, 0);
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
        let shown = Shown { frame: frame(&mut vm, false, 1200, 800).unwrap(), ends: vec![], presentation: 3, tab_bar: vec![], status_bar: Default::default() };
        let mut live = presentation::Presentation::new();
        let (mut width, mut height) = (10, 20);
        let lent = Lent { live: &mut live, width: &mut width, height: &mut height };
        // The callback's prompt: one presentation, then a dismissal that changes nothing.
        lent.run(&host, |_| host.borrow_mut().presentation += 1);
        assert!(live.ready(), "the prompt's presentation left no repaint scheduled");
        let next = frame(&mut vm, false, 1200, 800).unwrap();
        assert!(changed_frame(&mut vm, &shown, next, host.borrow().presentation, false).unwrap().is_some(),
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
        // Opacity alone, the frame's or a child frame's, repaints.
        let mut translucent = frame(&mut vm);
        translucent.snapshot.background_alpha = 0.8;
        assert!(translucent != selected, "a new `alpha-background' must repaint");
        vm.eval_source("(romacs--create-frame `((parent-frame . ,(selected-frame)) (width . 10) (height . 3)))").unwrap();
        let child = frame(&mut vm);
        let mut translucent = frame(&mut vm);
        translucent.snapshot.children[0].background_alpha = 0.5;
        assert!(translucent != child, "a child's new `alpha-background' must repaint");
    }

    /// A prompt read mid-command presents over the last paint; finishing it
    /// without changing the editor (switching to the shown buffer) must still
    /// repaint, or the prompt stays on screen.
    #[test]
    fn a_prompt_presented_since_the_last_paint_forces_the_next_one() {
        let mut vm = romacs_vm::Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        let shown = super::Shown { frame: super::frame(&mut vm, false, 1200, 800).unwrap(), ends: vec![], presentation: 4, tab_bar: vec![], status_bar: Default::default() };
        let same = super::frame(&mut vm, false, 1200, 800).unwrap();
        assert!(super::changed_frame(&mut vm, &shown, same, 4, false).unwrap().is_none(), "an unchanged frame still on screen repaints");
        let same = super::frame(&mut vm, false, 1200, 800).unwrap();
        assert!(super::changed_frame(&mut vm, &shown, same, 5, false).unwrap().is_some(), "a prompt's presentation left the frame unrepainted");
    }

    /// A button over the painted tab bar runs its binding with the column
    /// of the character under the pointer; anywhere else it is no tab bar
    /// click.
    #[test]
    fn a_click_over_the_tab_bar_names_its_column() {
        use romacs_display_runtime::thread_comm::{InputEvent, PointerAction, PointerPosition, PointerTarget, PositionedPointerInput};
        let mut vm = romacs_vm::Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        let mut painted = super::frame(&mut vm, false, 1200, 800).unwrap();
        painted.snapshot.tab_bar = Some(romacs_layout_input::TabBarSnapshot { height: 20, face: Default::default(), faces: vec![],
            text: romacs_layout_input::TextSlice { extends: vec![], utf8: "ab".into(), face_runs: vec![], raw: vec![] } });
        let shown = super::Shown { frame: painted, ends: vec![], presentation: 1, tab_bar: vec![(0.0, 10.0), (10.0, 12.0)], status_bar: Default::default() };
        let click = |x, y, button, pressed| super::tab_bar_click(&shown, &InputEvent::PositionedPointer(PositionedPointerInput {
            position: PointerPosition { x, y, target_frame_id: 1 }, target: PointerTarget::Unpresented,
            action: PointerAction::Button { button, pressed, modifiers: 0 } }));
        assert_eq!(click(15.0, 5.0, 1, true).as_deref(), Some("(romacs--tab-bar-click 1 'down-mouse-1)"));
        assert_eq!(click(3.0, 5.0, 2, false).as_deref(), Some("(romacs--tab-bar-click 0 'mouse-2)"));
        assert_eq!(click(500.0, 5.0, 1, false).as_deref(), Some("(romacs--tab-bar-click 2 'mouse-1)"));
        assert_eq!(click(15.0, 25.0, 1, true), None, "below the tab bar");
    }

    /// A button over a painted status-bar character runs the shown mode
    /// line's binding with its index; between or past them, and above the
    /// band, it is no click.
    #[test]
    fn a_click_over_the_status_bar_names_its_character() {
        use romacs_display_runtime::thread_comm::{InputEvent, PointerAction, PointerPosition, PointerTarget, PositionedPointerInput};
        let mut vm = romacs_vm::Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        let mut painted = super::frame(&mut vm, false, 1200, 800).unwrap();
        painted.snapshot.status_bar = Some(romacs_layout_input::StatusBarSnapshot { x: 6, y: 760, width: 1188, height: 20,
            text: romacs_layout_input::TextSlice { extends: vec![], utf8: "ab".into(), face_runs: vec![], raw: vec![] },
            face: Default::default(), faces: vec![], window: 1, focused: true, marks: Default::default() });
        let shown = super::Shown { frame: painted, ends: vec![], presentation: 1, tab_bar: vec![],
                                   status_bar: (vec![Some((6.0, 10.0)), Some((16.0, 10.0)), Some((26.0, 100.0)), Some((126.0, 10.0)), None], vec![2]) };
        let click = |x, y, button, pressed| super::status_bar_click(&shown, &InputEvent::PositionedPointer(PositionedPointerInput {
            position: PointerPosition { x, y, target_frame_id: 1 }, target: PointerTarget::Unpresented,
            action: PointerAction::Button { button, pressed, modifiers: 0 } }));
        assert_eq!(click(17.0, 765.0, 1, false).as_deref(), Some("(romacs--status-bar-click 1 'mouse-1)"));
        assert_eq!(click(6.0, 779.0, 3, true).as_deref(), Some("nil"), "a press is consumed and runs nothing");
        assert_eq!(click(500.0, 765.0, 1, false), None, "past the text");
        assert_eq!(click(60.0, 765.0, 1, false), None, "the marker's room");
        assert_eq!(click(130.0, 765.0, 1, false).as_deref(), Some("(romacs--status-bar-click 3 'mouse-1)"));
        assert_eq!(click(17.0, 759.0, 1, false), None, "above the band");
    }

    /// The stock theme's own faces reach a launch without Custom, the bare
    /// workspace, as they reach one with it.
    #[test]
    fn the_workspace_has_the_stock_theme_s_own_faces() {
        let mut vm = romacs_vm::Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        let attribute = |vm: &mut romacs_vm::Vm, face: &str, key: &str| {
            vm.eval_source(&format!("(internal-get-lisp-face-attribute '{face} {key})")).unwrap().display().unwrap()
        };
        assert_eq!(attribute(&mut vm, "mode-line-active", ":overline"), "\"#2e2922\"");
        assert_eq!(attribute(&mut vm, "romacs-mode-line-pill", ":background"), "\"#50269a\"");
        assert_eq!(attribute(&mut vm, "font-lock-variable-use-face", ":foreground"), "\"#d4c5a0\"");
    }

    /// A frame differing only in one chrome value paints differently, for
    /// the paint and the prompt host alike: its cursor's colour, a pane's
    /// corners or dimming, or a token of its style.
    #[test]
    fn a_chrome_value_alone_repaints() {
        let mut vm = romacs_vm::Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        let painted = super::frame(&mut vm, false, 1200, 800).unwrap();
        assert!(super::frames_alike(&painted.snapshot, &painted.snapshot.clone()));
        let changes: [fn(&mut romacs_layout_input::FrameChrome); 5] = [
            |c| c.cursor = Some([1, 2, 3]),
            |c| c.radius += 1,
            |c| c.dim += 0.01,
            |c| c.style.band_radius += 1.0,
            |c| c.style.layer_radius += 1.0,
        ];
        for change in changes {
            let mut other = painted.snapshot.clone();
            change(&mut other.chrome);
            assert!(!super::frames_alike(&painted.snapshot, &other));
        }
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
        let cursor = painted.selected.map(|id| (id, super::cursor_colour(&painted.snapshot)));
        let (_, ends) = romacs_layout::LayoutEngine::new().unwrap().layout_frame(&painted.snapshot, 1, cursor).unwrap();
        vm.display_apply_layout(&ends).unwrap();
        let end = vm.eval_source("(window-end)").unwrap().display().unwrap();
        assert_ne!(end, vm.eval_source("(point-max)").unwrap().display().unwrap(), "the window shows part of the buffer");
        let shown = super::Shown { frame: painted, ends, presentation: 1, tab_bar: vec![], status_bar: Default::default() };
        vm.eval_source("(romacs-select-buffer (current-buffer))").unwrap();
        let next = super::frame(&mut vm, false, 1200, 800).unwrap();
        assert!(super::changed_frame(&mut vm, &shown, next, 1, false).unwrap().is_none(), "the frame did not change");
        assert_eq!(vm.eval_source("(window-end)").unwrap().display().unwrap(), end);
    }
}

/// What one paint shows: the windows and which of them has the cursor.
struct Frame {
    snapshot: romacs_layout_input::FrameSnapshot,
    selected: Option<u64>,
    /// The text animations drawn over it (`Vm::text_animations').
    animations: Vec<(u64, romacs_display_protocol::text_animation::TextAnimation)>,
}

/// Equal frames paint identically, so an equal one needs no repaint.
/// `buffer_version` is left out: it is the VM's conservative redraw signal,
/// which also moves when nothing shown changed (a `save-excursion` restoring
/// point), and layout never reads it.
impl PartialEq for Frame {
    fn eq(&self, other: &Self) -> bool {
        self.selected == other.selected && frames_alike(&self.snapshot, &other.snapshot)
            && self.animations == other.animations
    }
}

/// Whether frames A and B paint identically; the prompt host (prompt.rs)
/// asks it too.
pub(crate) fn frames_alike(a: &romacs_layout_input::FrameSnapshot, b: &romacs_layout_input::FrameSnapshot) -> bool {
    let windows_alike = |a: &[romacs_layout_input::PlacedWindowSnapshot], b: &[romacs_layout_input::PlacedWindowSnapshot]| a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| (a.x, a.y) == (b.x, b.y) && paints_alike(&a.snapshot, &b.snapshot));
    a.font_selection == b.font_selection
        && a.font_alternatives == b.font_alternatives && a.fontsets == b.fontsets && (a.width, a.height, a.background_alpha) == (b.width, b.height, b.background_alpha)
        && a.no_font == b.no_font
        && a.chrome == b.chrome
        && windows_alike(&a.windows, &b.windows)
        && a.children.len() == b.children.len()
        && a.children.iter().zip(&b.children).all(|(a, b)| (a.id, a.x, a.y, a.width, a.height, a.border, a.border_color, a.background, a.background_alpha, a.radius)
            == (b.id, b.x, b.y, b.width, b.height, b.border, b.border_color, b.background, b.background_alpha, b.radius)
            && (a.animation, a.edge, a.selection) == (b.animation, b.edge, b.selection) && windows_alike(&a.windows, &b.windows))
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
    /// The painted tab bar's characters: each one's left edge and width.
    tab_bar: Vec<(f32, f32)>,
    /// The painted status bar's characters, each one's left edge and width
    /// or None where clipped, and its markers' indices.
    status_bar: (Vec<Option<(f32, f32)>>, Vec<usize>),
}

/// The frame to paint after a batch, or None when the paint on screen still
/// shows it: equal, and still the latest presentation. A prompt read in the
/// middle of a command presents over that paint and advances PRESENTATION,
/// so the frame after the prompt is painted even when nothing else changed.
/// A kept paint's window ends are applied again: a command can drop them
/// without changing the frame (`set-window-buffer' to the buffer shown), and
/// the same frame lays out to the same ends.
fn changed_frame(vm: &mut Vm, shown: &Shown, next: Frame, presentation: u64, force: bool) -> Result<Option<Frame>, String> {
    if force || shown.presentation != presentation || shown.frame != next { return Ok(Some(next)); }
    vm.display_apply_layout(&shown.ends)?;
    Ok(None)
}

fn frame(vm: &mut Vm, g1: bool, width: u32, height: u32) -> Result<Frame, String> {
    let snapshot = snapshot(vm, g1, width, height)?;
    let selected = selected_window(vm, &snapshot)?;
    let animations = if g1 { vec![] } else { vm.text_animations()? };
    Ok(Frame { snapshot, selected, animations })
}

/// The selected window's id, once the GUI's own Lisp lays out windows: a
/// window of the root frame or of a child frame shown over it.
fn selected_window(vm: &mut Vm, snapshot: &romacs_layout_input::FrameSnapshot) -> Result<Option<u64>, String> {
    if vm.eval_source("(boundp 'romacs-gui-quit-requested)")? == romacs_vm::Fixed::NIL { return Ok(None); }
    let id = vm.selected_window_id()?;
    let shown = snapshot.windows.iter().chain(snapshot.children.iter().flat_map(|c| &c.windows)).any(|w| w.snapshot.id == id);
    if shown { Ok(Some(id)) } else { Err("selected window not in frame".into()) }
}

fn snapshot(vm: &mut Vm, g1: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> {
    if g1 {
        // Preserve G1's explicitly full-height legacy selected-window fixture.
        Ok(romacs_layout_input::FrameSnapshot { font_selection: vm.font_selection_state(),
            font_alternatives: vm.font_alternative_state()?, fontsets: vm.fontset_state(), no_font: Default::default(),
            width: 400, height: 300,
            background_alpha: 1.0, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None, windows: vec![romacs_layout_input::PlacedWindowSnapshot {
                x: 0, y: 0, snapshot: vm.display_snapshot(400, 300)?,
            }],
        })
    } else { vm.frame_snapshot(width, height) }
}

/// `--help`.
const USAGE: &str = "\
usage: romacs-vm-gui [--loadup | --gnu-lisp | --no-gnu-lisp] [OPTION...] [FILE...]
  --loadup              run GNU's loadup chain; a live launch boots from its world
  --world-store DIR     keep worlds in DIR (default $XDG_CACHE_HOME/romacs/worlds)
  --world-build         make the worlds a live --loadup launch and a --batch
                        session would boot from usable, with no window, and
                        exit (what an installer runs); before --batch, the
                        batch world alone (an installation's, in
                        --world-store EXECUTABLE.worlds)
  --world-census        print what the base world of a live --loadup launch
                        holds, kind by kind (JSON), reading the store only
  --world-reset         forget the store's failed builds and what builds could
                        not capture, so the next launch builds again
  --world-reset=all     remove the store's worlds and files altogether
  --geometry WxH        the window's size
  -Q, -q, --no-init     do not load the user's init file
  -l, --load FILE       load FILE after the init, in order with the files
  -f, --funcall FUNC    call FUNC (as a command when it is one) there
  --eval EXPR           evaluate EXPR there
  --restore IMAGE       restore the session image IMAGE (`romacs-restart') there
  --batch ARG...        GNU's `emacs --batch ARG...', from a batch world of its own
                        once one is prepared in an existing store
  --compile-lisp FILE   compile FILE into the compiled-Lisp store (what a
                        compiled load runs on a miss)
  --compile-lisp-tree [--jobs N]  compile the installation's Lisp into its
                        compiled-Lisp store ahead of use, N at a time (default 4)
  ROMACS_WORLD=off      boot from source, without a world
  ROMACS_COMPILED_LISP=on|off  load GNU-compiled-kind files through compiled
                        artifacts (default: on in a live session, off in --batch)
  ROMACS_DIAGNOSTICS=1  print each paint, message, the render thread's frame counters
                        and the loadup tables on stderr (harnesses)
  ROMACS_RENDER_STALL_MS=N  call a paint the window has not drawn in N ms
                        stalled (default 2000): editing goes on, once told
";

/// A session marks on several threads. Only sessions ask: an isolate worker
/// and the tools may fork, and must hold no thread.
fn use_collector_threads() {
    romacs_vm::set_gc_workers(std::thread::available_parallelism().map_or(1, |n| n.get()).min(8));
}

fn main() {
    // The Vulkan driver reads it when the display makes its instance.
    if let Some(perftest) = romacs_renderer_wgpu::gpu_policy::radv_perftest(std::env::var("RADV_PERFTEST").ok().as_deref()) {
        // SAFETY: main's first statement, so no thread this program
        // starts exists yet. Of what runs before main, only the
        // allocator's constructor can start a thread: it reads its
        // configuration from the environment on this thread and returns,
        // and the purge thread it can be asked to start
        // (`background_thread`) never reads or writes the environment.
        // So nothing touches it concurrently.
        unsafe { std::env::set_var("RADV_PERFTEST", perftest) };
    }
    // A session's every end says why (exit.rs); other modes keep their own.
    let end = match std::panic::catch_unwind(run) {
        Ok(Ok(None)) => return,
        Ok(Ok(Some(end))) => end,
        Ok(Err(error)) if exit::installed() => exit::Exit::new("startup-error", error, 1),
        Ok(Err(error)) => {
            eprintln!("romacs-vm-gui: {error}");
            std::process::exit(1);
        }
        Err(_) if exit::installed() => exit::panicked("main thread"),
        Err(payload) => std::panic::resume_unwind(payload),
    };
    exit::finish(end)
}
/// The session's end, when this process ran one.
fn run() -> Result<Option<exit::Exit>, String> {
    // `--server NAME': a client of that server (client.rs), not a session.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if client::selected(&argv) { std::process::exit(client::main(&argv)); }
    if argv.first().map(String::as_str) == Some("--server-exited") { std::process::exit(client::exited(&argv)); }
    if argv.first().map(String::as_str) == Some("--restart-supervise") { std::process::exit(restart::supervise(&argv)); }
    romacs_vm::host::catch_user_signals().map_err(|e| format!("user signals: {e}"))?;
    grammars::register();
    // Before any font resolver is built (ledger/divergences VM2-FONT-BUNDLED).
    for problem in fonts::register_bundled_fonts() {
        eprintln!("romacs: {problem}");
    }
    let mut args = argv.into_iter();
    let mut serve = None;
    let mut serve_frame = false;
    let mut serve_landing = false;
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
    let mut world_build = false;
    let mut world_census = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--world-build" => world_build = true,
            "--world-census" => world_census = true,
            // The server a client started (client.rs): no window until a client asks.
            // It is GNU's daemon (`--fg-daemon=NAME') from here: `daemonp'.
            "--serve" => {
                let name = client::server_name(&args.next().ok_or("--serve needs a NAME")?)?;
                romacs_vm::host::set_daemon(&name);
                serve = Some(name);
            }
            // The client that starts a server and asks for its window: the
            // window comes up while the world restores, as a session's does.
            "--serve-frame" => serve_frame = true,
            // Its client has nothing to visit: the first paint is the landing.
            "--serve-landing" => serve_landing = true,
            "--world-store" => world_store = Some(std::path::absolute(args.next().ok_or("missing world store")?).map_err(|e| e.to_string())?),
            // Waiting for a missing world to build is what a launch does
            // now; the flag stays accepted for the scripts that passed it.
            "--world-wait" => {}
            "--world-reset" => world_reset = Some(false),
            "--world-reset=all" => world_reset = Some(true),
            "--help" => { print!("{USAGE}"); return Ok(None); }
            "--wayland" => { socket = Some(args.next().ok_or("missing socket")?); live = false; },
            "--geometry" => geometry = Some(parse_geometry(&args.next().ok_or("missing geometry WIDTHxHEIGHT")?)?),
            "--live" => live = true,
            "-Q" => (quick, load_user_init) = (true, false),
            "--no-init" | "-q" | "-no-init-file" | "--no-init-file" => load_user_init = false,
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
            "--loadup-report" => return loadup_report(args.collect()).map(|()| None),
            "--batch" if world_build => {
                if romacs_vm::world::store::disabled() { return Err("--world-build with ROMACS_WORLD=off".into()); }
                let store = world_store.or_else(|| world::default_store(|name| std::env::var_os(name))).ok_or("no world store")?;
                return batch::prepare(store).map(|()| None);
            }
            "--batch" => {
                use_collector_threads();
                return batch::run(args.collect(), quick, world_store).map(|()| None);
            }
            "--compile-lisp-tree" => {
                let jobs = match (args.next().as_deref(), args.next()) {
                    (None, _) => 4,
                    (Some("--jobs"), Some(n)) => n.parse::<usize>().map_err(|e| format!("--jobs: {e}"))?,
                    _ => return Err("usage: --compile-lisp-tree [--jobs N]".into()),
                };
                return batch::compile_lisp_tree(jobs).map(|()| None);
            }
            "--compile-lisp" => {
                let file = args.next().ok_or("--compile-lisp needs a FILE")?;
                return batch::compile_lisp(file.into(), world_store).map(|()| None);
            }
            // A worker's own end is its spawner's to report; its failure says why here.
            // The versions of terminal helper this build's sessions read: what
            // a session about to restart into this build asks first.
            "--terminal-versions" => {
                let versions = romacs_vm_gui::terminal_proto::VERSIONS;
                println!("{} {}", versions.start(), versions.end());
                return Ok(None);
            }
            // A terminal's helper process (terminal_helper.rs): no VM, no display.
            "--terminal-helper" => return Ok(romacs_vm_gui::terminal_helper::run(&args.next().unwrap_or_default()).err()
                                              .map(|error| exit::Exit::new("terminal-helper-error", error, 1))),
            "--isolate" => return Ok(isolate_worker::run(std::iter::once(arg).chain(args).collect()).err()
                                     .map(|error| exit::Exit::new("isolate-error", error, 1))),
            "--" => {
                startup::visit_at_startup("--");
                for file in args.by_ref() { startup::visit_at_startup(&file); }
            },
            option if startup::StartupOption::parse(option).is_some() => {
                startup::visit_at_startup(option);
                startup::visit_at_startup(&args.next().ok_or_else(|| format!("{option} needs an argument"))?);
            }
            // GNU's `--OPTION=VALUE' spelling of the same options.
            packed if packed.split_once('=').is_some_and(|(o, _)| o.starts_with("--") && startup::StartupOption::parse(o).is_some()) => {
                let (option, value) = packed.split_once('=').unwrap();
                startup::visit_at_startup(option);
                startup::visit_at_startup(value);
            }
            file if !file.starts_with('-') => startup::visit_at_startup(file),
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    if let Some(everything) = world_reset {
        let store = world_store.or_else(|| world::default_store(|name| std::env::var_os(name))).ok_or("no world store to reset")?;
        let removed = romacs_vm::world::store::reset(&store, everything).map_err(|e| e.to_string())?;
        romacs_vm::host::event_line("romacs-world", &serde_json::json!({"event": "reset", "store": store, "everything": everything, "removed": removed}));
        return Ok(None);
    }
    use_collector_threads();
    TRACE_LATENCY.store(trace_latency, std::sync::atomic::Ordering::Relaxed);
    SCENE_DUMPS.store(!live || trace_scene, std::sync::atomic::Ordering::Relaxed);
    DIAGNOSTICS.store(!live || trace_scene || trace_latency || std::env::var_os("ROMACS_DIAGNOSTICS").is_some_and(|v| v == "1"),
                      std::sync::atomic::Ordering::Relaxed);
    // Batch and the isolates keep GNU's `noninteractive' messages on stderr.
    romacs_vm::host::set_messages_to_stderr(diagnostics());
    // A frame's cost in instructions, said per frame on the render thread.
    if std::env::var_os("ROMACS_DIAGNOSTICS").is_some_and(|v| v == "1") {
        romacs_display_protocol::meter::install(romacs_vm::host::instruction_count);
    }
    if trace_latency { romacs_display_runtime::paint_host::enable_key_latency_trace(); }
    // Startup phases, on the clock of this process's exec, under the trace.
    let mut phases = trace_latency.then(startup::Phases::from_exec);
    if let Some(phases) = &mut phases { phases.mark("main"); }
    let (width, height) = geometry.unwrap_or(if live { (1600, 1000) } else { (400, 300) });
    // --world-census: what the base world of this launch holds, kind by
    // kind, as JSON on standard output; the store is only read.
    if world_census {
        if !(live && launch == startup::Launch::Loadup) { return Err("--world-census needs a live --loadup launch".into()); }
        // As a launch finds its world: the store named, else the
        // installation's when it holds this launch's, else the user's.
        let user = || world::default_store(|name| std::env::var_os(name)).ok_or_else(|| "no world store".to_owned())
            .and_then(|store| world::census(&store));
        let census = match world_store {
            Some(store) => world::census(&store),
            None => world::installed_store().map(|store| world::census(&store)).filter(Result::is_ok).unwrap_or_else(user),
        };
        println!("{}", census?);
        return Ok(None);
    }
    // --world-build: the world this launch would boot from, made usable
    // (built here, or waited for), with no window, and a batch session's
    // beside it (batch.rs); then exit. What an installer runs so a user's
    // first launch and first batch session restore.
    if world_build {
        if !(live && launch == startup::Launch::Loadup) { return Err("--world-build needs a live --loadup launch".into()); }
        if romacs_vm::world::store::disabled() { return Err("--world-build with ROMACS_WORLD=off".into()); }
        let store = world_store.or_else(|| world::default_store(|name| std::env::var_os(name))).ok_or("no world store")?;
        world::prepare(&world::open(store.clone())?)?;
        batch::prepare_beside(store);
        return Ok(None);
    }
    // A live --loadup launch boots from its world, in --world-store or by
    // default in the user's cache (world::default_store), unless
    // ROMACS_WORLD=off. Its builder builds here, before any window; a launch
    // opens and maps the world on the evaluator's thread while this one
    // brings up the window and the GPU.
    let named_store = world_store.clone();
    let from_installed = named_store.is_none();
    let world_store = match (live && launch == startup::Launch::Loadup, romacs_vm::world::store::disabled()) {
        (true, false) => world_store.or_else(|| world::default_store(|name| std::env::var_os(name))),
        (true, true) => {
            romacs_vm::host::event_line("romacs-world", &serde_json::json!({"mode": "source", "reason": "ROMACS_WORLD=off"}));
            None
        }
        (false, _) => None,
    };
    if let Some(store) = world_store.as_ref().filter(|_| romacs_vm::world::store::building()) {
        world::build_here(&world::open(store.clone())?)?;
    }
    if live && launch == startup::Launch::Loadup && !romacs_vm::world::store::building() {
        batch::configure_compiled_lisp(named_store.as_deref(), true);
    }
    // The session from here: its every end says why.
    exit::install();
    // Fontconfig's configuration loads on a helper while the window and the
    // world open: the evaluator's first font question finds it loaded, or
    // waits for it.
    if live { std::thread::spawn(romacs_font::load_catalog); }
    let socket = socket.or_else(|| std::env::var("WAYLAND_DISPLAY").ok()).unwrap_or_else(|| "wayland-0".into());
    if serve.is_some() && !live { return Err("--serve needs a live session".into()); }
    // A server runs GNU's server.el and its own Lisp on GNU's (romacs-server.el).
    if serve.is_some() && launch == startup::Launch::Workspace {
        return Err("--serve needs GNU's Lisp: --loadup or --gnu-lisp".into());
    }
    let title = if live { "romacs — VM" } else { "VM G1a" };
    let (mut host, sender) = if serve.is_some() {
        PaintHost::wayland_server(std::path::Path::new(&socket), width, height, title.into())?
    } else {
        PaintHost::wayland(std::path::Path::new(&socket), width, height, title.into())?
    };
    // A live session's C-g quits a busy evaluator out of band (quit.rs).
    let quit = live.then(|| std::sync::Arc::new(romacs_vm::QuitSignal::new()));
    if let Some(quit) = &quit { host.set_input_interrupt(std::sync::Arc::new(quit::QuitHook(quit.clone()))); }
    if let Some(phases) = &mut phases { phases.mark("window"); }
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let start = phases.as_ref().map(startup::Phases::start);
    let serving = serve.is_some();
    let mut early_window = match &serve {
        Some(_) if serve_frame => Some(sender.realize(romacs_display_runtime::paint_host::INITIAL_FRAME, width, height, "romacs".into())?),
        _ => None,
    };
    let worker = spawn_evaluator(move || {
        if let Some(start) = start { startup::begin_phases(start); }
        // The main VM's one-second rule, watched from its start (watchdog.rs).
        if live { romacs_vm_gui::watchdog::start(romacs_vm_gui::watchdog::threshold()); }
        let sender = std::sync::Arc::new(sender);
        let mut first_presentation = 1;
        // The launch screen, from the window's first moment until the
        // session's first frame (loading.rs); a server shows it in the
        // window a client asked for at its start.
        let loading = if live && (!serving || serve_frame) {
            let started = loading::start(sender.clone(), width, height, loading::profile_path());
            romacs_vm::host::event_line("romacs-world", &serde_json::json!({"event": "loading-frame", "painted": started.is_ok(),
                                                               "error": started.as_ref().err()}));
            // A first frame that failed may still have been submitted.
            if started.is_err() { first_presentation = 2; }
            started.ok()
        } else { None };
        // The installation's world serves first, unless a store was named:
        // its installer built it, and a launch only reads it (world::installed).
        let (installed, installed_vm) = match world_store.as_ref().filter(|_| from_installed).and_then(|_| world::installed()) {
            Some((world, vm, carry)) => (Some(world), Some((vm, carry))),
            None => (None, None),
        };
        // A store that cannot be used boots from source, and says so once
        // the session is up (`world::store_warning_form`).
        let mut store_problem = None;
        let world = world_store.filter(|_| installed.is_none()).and_then(|store| match world::open(store) {
            Ok(world) => Some(world),
            Err(reason) => {
                romacs_vm::host::event_line("romacs-world", &serde_json::json!({"mode": "source", "reason": reason}));
                store_problem = Some(reason);
                None
            }
        });
        startup::mark_phase("world-open");
        let restored = installed_vm.or_else(|| world.as_ref().and_then(|world| world::launch(world, |reason| {
            // Said on stderr too, so a launcher sees it whether or not a
            // window can, with why the world was not usable.
            romacs_vm::host::event_line("romacs-world", &serde_json::json!({"event": "preparing", "message": loading::FIRST_START, "reason": reason}));
            loading::step("world-build", loading::FIRST_START);
        })));
        // The world this launch booted from, which its worker isolates adopt.
        let isolate_world = installed.as_ref().or(world.as_ref())
            .map(|w| (w.store.clone(), w.key.iter().map(|b| format!("{b:02x}")).collect::<String>()));
        // Bounding the store waits for the first frame, on a thread of its own.
        let prune = world.map(|current| move || { std::thread::spawn(move || {
            if let Ok(deleted) = current.prune() && !deleted.is_empty() {
                romacs_vm::host::event_line("romacs-world", &serde_json::json!({"event": "pruned", "worlds": deleted}));
            }
        }); });
        startup::mark_phase(if restored.is_some() { "world-restored" } else { "world-none" });
        // What both loops share (prompt.rs), the launch screen with it: a
        // startup that fails keeps its screen (`hold_failed_startup`).
        let host = std::rc::Rc::new(std::cell::RefCell::new(prompt::Shared { presentation: first_presentation, loading, ..Default::default() }));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            evaluate(&sender, floor, force_gc, g1, live, launch, width, height, load_user_init, quick, restored,
                     isolate_world, store_problem, &host, quit, prune, serve, serve_landing, &mut early_window)
        }));
        let (end, failed) = evaluator_end(result);
        let error = failed.then(|| end.detail.clone());
        // The end goes first: dropping the sender closes the window, and
        // the window's end lets the main thread look for it.  A failed
        // startup's screen waits for its window and holds after it, so a
        // display that ends meanwhile still ends the session with it.
        let screen = send_end_then(&result_tx, Ended { held: failed, end },
                                   || error.and_then(|error| failed_screen(&host, &mut early_window, &error)));
        if let Some(screen) = screen { hold_failed_startup(screen, &sender, &host); }
        // A screen still up stops before the window goes: its painter holds it.
        drop(host);
        drop(sender);
    })?;
    let host = host.run();
    // An end the evaluator caused was sent before the loop could end; any
    // other leaves it running. Native startup failure must not join an
    // evaluator blocked in stdin; an ended one is joined, as its VM's drop
    // kills its children.
    let wait = if exit::evaluator_ended(&host) { std::time::Duration::from_secs(2) } else { std::time::Duration::ZERO };
    let verdict = match result_rx.recv_timeout(wait) {
        Ok(Ended { end, held: true }) => exit::Verdict::Ended(end),
        Ok(Ended { end, held: false }) => match worker.join() {
            Ok(()) => exit::Verdict::Ended(end),
            Err(_) => exit::Verdict::Panicked,
        },
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => exit::Verdict::Panicked,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => exit::Verdict::Running,
    };
    Ok(Some(exit::session(host, verdict)))
}

/// The evaluator's end, as the main thread gets it.
struct Ended {
    end: exit::Exit,
    /// A failed startup's, sent before its screen waits for its window and
    /// holds (`hold_failed_startup`): its thread, which has no VM left to
    /// drop and may wait on the user, is not joined.
    held: bool,
}

/// Send END to the main thread, then WAIT (a failed startup's screen
/// waiting for its window): the end is there whatever ends the display
/// during the wait.
fn send_end_then<T>(result_tx: &std::sync::mpsc::Sender<Ended>, end: Ended, wait: impl FnOnce() -> T) -> T {
    let _ = result_tx.send(end);
    wait()
}

#[cfg(test)]
mod send_end_tests {
    use super::*;

    #[test]
    fn a_failed_startup_s_end_is_sent_before_its_screen_waits() {
        let (sent, ends) = std::sync::mpsc::channel();
        let end = Ended { held: true, end: exit::Exit::new("evaluator-error", "x", 1) };
        let found = send_end_then(&sent, end, || ends.try_recv().ok());
        let found = found.expect("the end was there when the wait began");
        assert!(found.held);
        assert_eq!(found.end, exit::Exit::new("evaluator-error", "x", 1));
    }
}

/// How often a failed startup's screen looks for a termination signal,
/// whose wake went with the session.
const FAILED_SIGNAL_LOOK: std::time::Duration = std::time::Duration::from_millis(250);

/// Startup failed with ERROR: its launch screen, taken from HOST and saying
/// so.  There is one before the session's first paint takes the window,
/// and only while that window shows: a server's window that Lisp hid
/// shows nothing, and no close would come for it.  A server's window asked
/// for at its start (`early_window`, not yet taken while the screen is
/// up) is waited for as the session waits for it: one never made shows
/// nothing either.
fn failed_screen(host: &std::cell::RefCell<prompt::Shared>, early_window: &mut Option<crossbeam_channel::Receiver<Result<(), String>>>,
                 error: &str) -> Option<loading::Loading> {
    let screen = { let mut shared = host.borrow_mut(); if shared.window_shown() { shared.loading.take() } else { None } }?;
    if let Some(ready) = early_window.take() && !made(&ready) { return None; }
    loading::fail(error, startup::first_noted_lisp_error().as_deref());
    Some(screen)
}

/// Whether the window READY answers for was made within the 30 s the
/// session would wait; a termination signal ends the wait.
fn made(ready: &crossbeam_channel::Receiver<Result<(), String>>) -> bool {
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    while Instant::now() < deadline && romacs_vm::host::termination_signal(true).is_none() {
        match ready.recv_timeout(FAILED_SIGNAL_LOOK) {
            Ok(made) => return made.is_ok(),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return false,
        }
    }
    false
}

/// Keep a failed startup's SCREEN until the user closes the window, a
/// termination signal comes, or the display goes.  Nothing is retried.
fn hold_failed_startup(screen: loading::Loading, sender: &PaintSender, host: &std::cell::RefCell<prompt::Shared>) {
    let mut closed = { let shared = host.borrow(); shared.closed || shared.pending.iter().any(prompt::closes) };
    while !closed && romacs_vm::host::termination_signal(true).is_none() {
        closed = match terminal_wait::wait(sender.input(), &crossbeam_channel::never(), Some(FAILED_SIGNAL_LOOK), &[]) {
            terminal_wait::Wake::Input(event) => prompt::closes(&event),
            terminal_wait::Wake::Closed => true,
            terminal_wait::Wake::Terminal | terminal_wait::Wake::Process | terminal_wait::Wake::Timeout => false,
        };
    }
    drop(screen);
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
/// The loop's one input queue, the prompt host's (`prompt::Shared`): every
/// read takes its keys from here and leaves what it does not use here, so
/// none is lost wherever the Lisp that reads runs. Borrowed for one step,
/// never across Lisp.
fn queue(host: &std::cell::RefCell<prompt::Shared>) -> std::cell::RefMut<'_, std::collections::VecDeque<InputEvent>> {
    std::cell::RefMut::map(host.borrow_mut(), |shared| &mut shared.pending)
}
fn activate(sender: &PaintSender, presentation: u64, host: &std::cell::RefCell<prompt::Shared>) -> Result<(), String> {
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
            event => queue(host).push_back(event),
        }
    }
}
/// How a paint waits for its render before it says so.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
    /// Submit only; the live loop sees the render later. Every live
    /// paint: the evaluator never waits for drawing.
    No,
    /// Rendered within 30 s, or an error: the oracle transport, a test
    /// protocol that never runs live, whose reply to a command promises the
    /// frame drawn.
    Rendered,
    /// The live session's first frame, at startup only: rendered within
    /// 30 s, or left in flight undrawn (its window hidden as it starts). The
    /// `ready' reply follows it, and launchers and harnesses type once ready:
    /// keys sent before the window is up and focused are the compositor's
    /// to drop. No wait after startup is of this kind.
    Bounded,
}

fn paint(
    engine: &mut romacs_layout::LayoutEngine,
    vm: &mut Vm,
    sender: &PaintSender,
    images: &residency::Shared,
    presentation: u64,
    force_gc: bool,
    floor: Option<usize>,
    shown: Frame,
    host: &std::cell::RefCell<prompt::Shared>,
    wait: Wait,
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
        crate::diagnostic_line(&json!({"diagnostic":"gc-between-snapshot-and-paint","presentation":presentation,
            "collections_before":before,"collections_after":after,"witness_moved":moved})
        );
    }
    let layout_start = Instant::now();
    let (snapshot, selected) = (&shown.snapshot, shown.selected);
    let cursor = selected.map(|selected| (selected, cursor_colour(snapshot)));
    let (mut frame, mut ends) = engine.layout_frame(snapshot, presentation, cursor)?;
    let (mut children, child_ends) = engine.layout_children(snapshot, presentation, cursor)?;
    ends.extend(child_ends);
    if selected.is_some() {
        steady_cursors(&mut frame, &mut children, snapshot);
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
    if diagnostics() { crate::diagnostic_line(&json!({"diagnostic":"cursor-publication", "presentation":presentation,
        "cursors":frame.window_cursors.iter().map(|c| json!({"window":c.window_id.get(),"x":c.x,"y":c.y,"active":c.active,"color":hex(c.color)})).collect::<Vec<_>>(),
        // Point as a buffer byte: a window publishes its text from a base byte.
        "points":snapshot.windows.iter().map(|w| json!({"id":w.snapshot.id,"point_byte":vm.published_buffer_byte(w.snapshot.id, w.snapshot.point_byte)})).collect::<Vec<_>>() })); }
    let glyphs: Vec<_> = std::iter::once(&frame).chain(&children).filter(|_| SCENE_DUMPS.load(std::sync::atomic::Ordering::Relaxed)).flat_map(|frame| {
        let (id, origin) = (frame.frame_placement.frame().get(), frame.frame_placement.outer_in_parent());
        frame.glyphs.iter().filter_map(move |glyph| {
            if let romacs_display_protocol::FrameGlyph::Char { char, composed, row_role, face_id, x, y, clip_rect, .. } = glyph {
                let face = &frame.faces[face_id];
                Some(json!({"char":char,"composed":composed,"role":format!("{row_role:?}"),"foreground":hex(face.foreground),
                    "background":hex(face.background),"x":x + origin.x(),"y":y + origin.y(),"clip":clip_rect,"frame":id}))
            } else { None }
        })
    }).collect();
    let diagnostics_prepare_ns = diagnostic_start.elapsed().as_nanos();
    let tab_bar = romacs_layout::tab_bar_extents(&frame);
    let status_bar = shown.snapshot.status_bar.as_ref().map(|bar| romacs_layout::status_bar_extents(&frame, bar)).unwrap_or_default();
    let submit_start = Instant::now();
    images.borrow_mut().publish(sender, std::iter::once(&frame).chain(&children))?;
    sender.submit_scene(frame, children, shown.animations.clone())?;
    images.borrow_mut().retire_surfaces(sender)?;
    let submit_ns = submit_start.elapsed().as_nanos();
    let activation_start = Instant::now();
    if wait != Wait::No { activate(sender, presentation, host)?; }
    let activation_ns = activation_start.elapsed().as_nanos();
    let wait_start = Instant::now();
    let render = || sender.wait_for_render(romacs_display_runtime::paint_host::INITIAL_FRAME, presentation, std::time::Duration::from_secs(30));
    let diagnostic = match wait {
        Wait::No => None,
        Wait::Rendered => Some(render()?),
        Wait::Bounded => render().ok(),
    };
    let wait_render_ns = wait_start.elapsed().as_nanos();
    let diagnostic_write_start = Instant::now();
    if diagnostics() { crate::diagnostic_line(&json!({"diagnostic":if diagnostic.is_some() { "paint" } else { "paint-submitted" },"presentation":presentation,
        "buffer_versions":snapshot.windows.iter().map(|w|w.snapshot.buffer_version).collect::<Vec<_>>(),"resolved_fonts":resolved_fonts,
        "unresolved_face_text_total":diagnostic.map(|sample| sample.unresolved_face_text_total)})
    ); }
    if SCENE_DUMPS.load(std::sync::atomic::Ordering::Relaxed) { crate::diagnostic_line(&json!({"diagnostic":"submitted-scene", "presentation":presentation,
        "vscroll_px":snapshot.windows.first().map(|w|w.snapshot.vscroll_px),"glyphs":glyphs})
    ); }
    if tracing() {
        crate::diagnostic_line(&json!({"diagnostic":if diagnostic.is_some() { "latency-paint" } else { "latency-submit" }, "presentation":presentation, "units":"ns",
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
        crate::diagnostic_line(&json!({"diagnostic":"mode-line","presentation":presentation,"message":message})
        );
    }
    if tracing() {
        *timings = Some(latency::Paint { submitted: submit_start, layout_ns,
            diagnostic_ns: diagnostics_prepare_ns + diagnostics_write_ns + mode_line_start.unwrap().elapsed().as_nanos(),
            submit_ns, layout_apply_ns });
    }
    Ok(Shown { frame: shown, ends, presentation, tab_bar, status_bar })
}

/// A mouse button event over the root frame's tab bar as the Lisp that
/// runs its binding (`romacs--tab-bar-click'): the column of the painted
/// character under the pointer, past the last one when none is.
fn tab_bar_click(shown: &Shown, event: &InputEvent) -> Option<String> {
    let InputEvent::PositionedPointer(romacs_display_runtime::thread_comm::PositionedPointerInput {
        position, action: romacs_display_runtime::thread_comm::PointerAction::Button { button, pressed, .. }, ..
    }) = event else { return None };
    let height = shown.frame.snapshot.tab_bar.as_ref()?.height as f32;
    if position.target_frame_id != 1 || position.y < 0.0 || position.y >= height || !(1..=3).contains(button) { return None; }
    let column = shown.tab_bar.iter().position(|&(x, width)| position.x >= x && position.x < x + width).unwrap_or(shown.tab_bar.len());
    let kind = if *pressed { "down-mouse" } else { "mouse" };
    Some(format!("(romacs--tab-bar-click {column} '{kind}-{button})"))
}

/// A mouse button event over the root frame's status bar as the Lisp that
/// runs the binding of the mode line it shows (`romacs--status-bar-click'):
/// the index of the painted character under the pointer. Between or past
/// the characters, and on a marker's room, it is no click.
fn status_bar_click(shown: &Shown, event: &InputEvent) -> Option<String> {
    let InputEvent::PositionedPointer(romacs_display_runtime::thread_comm::PositionedPointerInput {
        position, action: romacs_display_runtime::thread_comm::PointerAction::Button { button, pressed, .. }, ..
    }) = event else { return None };
    let bar = shown.frame.snapshot.status_bar.as_ref()?;
    let (top, bottom) = (bar.y as f32, (bar.y + bar.height) as f32);
    if position.target_frame_id != 1 || position.y < top || position.y >= bottom || !(1..=3).contains(button) { return None; }
    let (extents, markers) = &shown.status_bar;
    let column = extents.iter().position(|e| e.is_some_and(|(x, width)| position.x >= x && position.x < x + width))?;
    if markers.contains(&column) { return None; }
    // A press is the bar's and runs nothing: its release is the click, so
    // `[mode-line down-mouse-1] mouse-drag-mode-line' never starts a drag of
    // an edge the bar does not have (VM2-STATUS-BAR).
    if *pressed { return Some("nil".into()); }
    Some(format!("(romacs--status-bar-click {column} 'mouse-{button})"))
}

/// No cursor colour cycle in any window of SNAPSHOT, its child frames'
/// too: a minibuffer a layer hosts has its cursor there.
fn steady_cursors(frame: &mut romacs_display_protocol::FrameGlyphBuffer, children: &mut [romacs_display_protocol::FrameGlyphBuffer],
                  snapshot: &romacs_layout_input::FrameSnapshot) {
    let mut effects = romacs_display_protocol::EffectsConfig::default();
    effects.cursor_color_cycle.enabled = false;
    let steady = |buffer: &mut romacs_display_protocol::FrameGlyphBuffer, windows: &[romacs_layout_input::PlacedWindowSnapshot]| {
        for window in windows {
            buffer.set_window_cursor_effects(romacs_display_protocol::DisplayWindowId::new(window.snapshot.id as i64), effects.clone());
        }
    };
    steady(frame, &snapshot.windows);
    for (buffer, child) in children.iter_mut().zip(&snapshot.children) { steady(buffer, &child.windows); }
}

/// Lay out and submit a frame for a prompt waiting inside a command
/// (prompt.rs). The same cursor policy as `paint`; no VM access, because the
/// evaluator is suspended in Lisp and redisplay already ran in its own engine.
/// The cursor's colour: the frame's `cursor-color', which SNAPSHOT carries.
fn cursor_colour(snapshot: &romacs_layout_input::FrameSnapshot) -> romacs_display_protocol::Color {
    let [r, g, b] = snapshot.chrome.cursor.unwrap_or_default();
    romacs_display_protocol::Color::from_pixel(u32::from_be_bytes([0, r, g, b]))
}

fn paint_prompt(
    engine: &mut romacs_layout::LayoutEngine,
    sender: &PaintSender,
    images: &residency::Shared,
    presentation: u64,
    frame: &romacs_vm::HostFrame,
) -> Result<(usize, Vec<romacs_vm::WindowEnd>), String> {
    let cursor = Some((frame.selected, cursor_colour(&frame.frame)));
    let (mut glyphs, mut ends) = engine.layout_frame(&frame.frame, presentation, cursor)?;
    let (mut children, child_ends) = engine.layout_children(&frame.frame, presentation, cursor)?;
    ends.extend(child_ends);
    steady_cursors(&mut glyphs, &mut children, &frame.frame);
    let count = glyphs.glyphs.len();
    let observed = SCENE_DUMPS.load(std::sync::atomic::Ordering::Relaxed).then(|| {
        std::iter::once(&glyphs).chain(&children).flat_map(|frame| {
            let (id, origin) = (frame.frame_placement.frame().get(), frame.frame_placement.outer_in_parent());
            frame.glyphs.iter().filter_map(move |glyph| {
                if let romacs_display_protocol::FrameGlyph::Char { char, row_role, x, y, clip_rect, .. } = glyph {
                    Some(json!({"char":char,"role":format!("{row_role:?}"),"x":x + origin.x(),"y":y + origin.y(),"clip":clip_rect,"frame":id}))
                } else { None }
            })
        }).collect::<Vec<_>>()
    });
    images.borrow_mut().publish(sender, std::iter::once(&glyphs).chain(&children))?;
    sender.submit_scene(glyphs, children, frame.text_animations.clone())?;
    images.borrow_mut().retire_surfaces(sender)?;
    if let Some(observed) = observed {
        crate::diagnostic_line(&json!({"diagnostic":"submitted-scene","presentation":presentation,"glyphs":observed}));
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
    isolate_world: Option<(std::path::PathBuf, String)>,
    // Why the world store could not be used, when it could not: said once
    // the session is up.
    store_problem: Option<String>,
    // What both loops share, the launch screen with it: the caller keeps
    // a failed startup's screen (`hold_failed_startup`).
    shared: &std::rc::Rc<std::cell::RefCell<prompt::Shared>>,
    quit: Option<std::sync::Arc<romacs_vm::QuitSignal>>,
    after_first_frame: Option<impl FnOnce()>,
    serve: Option<String>,
    serve_landing: bool,
    // A server's window asked for at its start, ready when this answers:
    // the caller's, taken by whichever waits for it (a window made after
    // its waiter is dropped is destroyed), so a failed startup's screen
    // can wait for it too.
    early_window: &mut Option<crossbeam_channel::Receiver<Result<(), String>>>,
) -> Result<exit::Exit, String> {
    let engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout::LayoutEngine::new()?));
    startup::mark_phase("layout-engine");
    let (mut vm, carry) = match restored {
        Some((vm, carry)) => {
            let mut vm = *vm;
            vm.world_reinit();
            // Its build's first Lisp error, first, whatever fails after it.
            if let Err(error) = startup::first_noted_from_world(&mut vm) { eprintln!("romacs: world Lisp errors: {error}"); }
            (vm, Some(carry))
        }
        None => (Vm::new(), None),
    };
    if let Some(quit) = quit { vm.share_quit_signal(quit); }
    startup::mark_phase("vm");
    // How this VM booted, which its worker isolates repeat (lisp/vm/isolate.el).
    match (&carry, isolate_world, launch) {
        (Some(_), Some((store, key)), _) => {
            let compiled = romacs_vm::compiled_lisp::store_dir()
                .map_or("nil".to_owned(), |dir| startup::lisp_string(&dir.display().to_string()));
            vm.eval_source(&format!("(setq isolate-boot 'world isolate-world (cons {} {:?}) isolate-compiled-store {compiled})",
                                    startup::lisp_string(&store.display().to_string()), key))?;
        }
        (_, _, startup::Launch::Loadup | startup::Launch::GnuOnly) => { vm.eval_source("(setq isolate-boot 'source)")?; }
        _ => { vm.eval_source("(setq isolate-boot 'bare)")?; }
    }
    // Measuring primitives ask the engine that paints (VM2-WTPS1).
    vm.set_layout_host(Some(Box::new(measure::Host { engine: engine.clone() })));
    // The window's session ends with `kill-emacs': its hook has run, so the
    // host unwinds the evaluator to the worker, which drops the VM (killing
    // its children, as shut_down_emacs does) and exits with the status.
    if live { vm.set_session_host(Some(Box::new(|code| std::panic::resume_unwind(Box::new(SessionEnd(code)))))); }
    // Its window system, which the session opens (fonts.rs).
    let alpha_sender = sender.clone();
    let visual_sender = sender.clone();
    let lose_sender = sender.clone();
    let images = residency::Shared::default();
    let terminal_service = terminal_service::Service::new();
    let terminal_ready = terminal_service.ready();
    // A paint rendered wakes the wait, which then retires it: nothing polls.
    if live { sender.on_render(terminal_service.waker()); }
    // An image decoded off the VM wakes the same wait as a terminal.
    vm.set_display_host(Some(Box::new(fonts::Display::new()?.with_images(images.clone()).with_wake(terminal_service.waker())
        .with_alpha_hook(move |frame, alpha, limit| {
            // Only a closed window refuses, and then nothing shows it.
            let _ = alpha_sender.set_frame_alpha(frame, alpha, limit);
        })
        .with_visual_hook(move |config| {
            // Only a closed render thread refuses, and then nothing paints.
            let _ = visual_sender.set_visual_config(config.clone());
        })
        .with_lose_device_hook(move || lose_sender.debug_lose_device().unwrap_or(false))
        .with_models(sender.clone()))));
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
    // The frame's geometry is this process's: a world holds the chain's on
    // GNU batch's initial frame.
    let (line_height, column_width) = romacs_layout::frame_metrics(&vm.display_default_face()?)?;
    vm.set_frame_geometry(width, height, line_height, column_width)?;
    if live {
        if launch != startup::Launch::GnuOnly && carry.is_none() {
            vm.eval_source(world::INTERPROGRAM)?;
        }
        // The launch font's face, as source: declared BEFORE GNU's Lisp, so
        // GNU's own `defface' (custom.el) leaves the declared face alone, and
        // after the workspace's own faces otherwise, exactly as before. A
        // restored session sets it first, as no world holds it.
        let font = world::LaunchFont::from_env()?;
        let started = if launch == startup::Launch::Workspace {
            let started = startup::initialize(&mut vm)?;
            if let Some(font) = &font { vm.eval_source(&font.declaration())?; }
            started
        } else if let Some(carry) = carry {
            if let Some(font) = &font { vm.eval_source(&font.restored())?; }
            // As a fresh process takes the per-process startup steps.
            vm.eval_source("(romacs-world-restart)")?;
            startup::mark_phase("world-restart");
            startup::resume_loadup(&mut vm, launch, carry.loadup)?
        } else {
            startup::initialize_loadup(&mut vm, launch, font.as_ref().map(world::LaunchFont::declaration).as_deref())?
        };
        startup::mark_phase("session-lisp");
        if let Err(error) = echo::install(&mut vm) { eprintln!("romacs: echo area: {error}"); }
        // This process's command log, from before the init (lisp/session/romacs-usage-log.el).
        if launch == startup::Launch::Loadup && let Err(error) = vm.eval_source("(romacs-usage-log-start)") {
            eprintln!("romacs: usage log: {error}");
        }
        startup::before_init(&mut vm, launch, quick)?;
        // The user's settings, packages, init and what runs after it are
        // Lisp that may not end: C-g quits it, as GNU's startup lets it
        // (the evaluator is busy to the input thread until its first wait).
        // Each step goes on past a quit; Romacs's own defaults before them
        // are not quit (a C-g then waits as a key).
        if live { vm.enter_busy(); }
        if load_user_init && launch != startup::Launch::GnuOnly {
            if let Some(path) = user_init::path() {
                // `load-path' before the packages and the init add to it: a
                // directory they add may shadow a library loaded already
                // (lisp/session/romacs-user-shadows.el).
                if let Err(error) = vm.eval_source("(setq romacs--load-path-at-start load-path)") {
                    eprintln!("romacs: user shadows: {error}");
                }
                if let Err(error) = user_init::start_saved_settings(&mut vm, &path) {
                    eprintln!("romacs: saved settings: {error}");
                }
                // GNU's step before the init (startup.el:1560): installed
                // packages activate.
                if let Err(error) = vm.eval_source("(romacs--activate-packages)") {
                    eprintln!("romacs: packages: {error}");
                }
                if let Err(error) = user_init::load(&mut vm, &path) {
                    eprintln!("romacs: init failed: {error}");
                    // As GNU's startup says it (startup.el `startup--load-user-init-file'):
                    // a *Warnings* buffer the first frame shows, with the
                    // backtrace a button away, and the launch screen's init
                    // stage failed (lisp/session/romacs-init-load.el).
                    let warned = vm.eval_source(&format!("(romacs-init-failed {})", startup::lisp_string(&error)));
                    if warned.is_err() { let _ = echo::show(&mut vm, "Error loading init.el; see stderr for details"); }
                }
                // A user's copy of a library loaded before the init wins, as
                // in a session that first loaded it now.
                if let Err(error) = vm.eval_source("(progn (require 'romacs-user-shadows) (romacs--load-user-shadows romacs--load-path-at-start))") {
                    eprintln!("romacs: user shadows: {error}");
                }
            }
        }
        startup::mark_phase("user-init");
        if let Some(reason) = &store_problem && let Err(error) = vm.eval_source(&world::store_warning_form(reason)) {
            eprintln!("romacs: world store warning: {error}");
        }
        // Romacs's own Lisp that failed to load, which the launch went on without.
        if launch == startup::Launch::Loadup && let Err(error) = startup::report_lisp_errors(&mut vm) {
            eprintln!("romacs: Lisp errors: {error}");
        }
        let finished = match launch {
            startup::Launch::Loadup => startup::finish_loadup(&mut vm, &started),
            startup::Launch::Workspace => startup::finish_workspace(&mut vm, &started),
            startup::Launch::GnuOnly => Ok(()),
        };
        // The startup Lisp a C-g could quit has run: one typed late waits
        // for the first read.
        if live { vm.leave_busy(); }
        // As GNU's command loop reports an error that ends `command-line-1';
        // else startup's own message is shown, as GNU's first redisplay
        // shows it before the command loop's first timer runs.
        match finished {
            Err(error) => {
                eprintln!("romacs: startup after init failed: {error}");
                // Shown with the first Lisp error startup went past, often its cause.
                let shown = startup::first_noted_lisp_error().map_or_else(|| error.clone(),
                    |first| format!("Romacs Lisp failed: {first}; then: {error}"));
                let _ = echo::show(&mut vm, &shown);
            }
            Ok(()) => {
                // A session that came up degraded says why first.
                if let Some(first) = startup::first_noted_lisp_error() {
                    let _ = echo::show(&mut vm, &format!("Romacs Lisp failed: {first}"));
                // Not fatal: a quit typed late (`quit-flag') may end it.
                } else if let Err(error) = echo::show_current_message(&mut vm) {
                    eprintln!("romacs: startup message: {error}");
                }
            }
        }
        startup::mark_phase("finish-loadup");
        eprintln!("romacs: {}", started.status());
        if let Some(loadup) = started.loadup.as_ref().filter(|_| diagnostics()) {
            for line in loadup.table().lines() { eprintln!("romacs-loadup: {line}"); }
        }
        // Per journey key: whose command it runs. The journey judge reads these
        // so a reply can never pass a stand-in off as GNU's command.
        for (key, gnu, romacs) in startup::STAND_INS.into_iter().filter(|_| diagnostics()) {
            if let Some(b) = started.bindings.iter().find(|b| b.key == key) {
                eprintln!("romacs-binding: {key}\t{gnu}\t{}", if b.gnu_defined { gnu } else { romacs });
            }
        }
        let face = vm.display_default_face()?;
        if let Some(font) = font {
            eprintln!("romacs: font family={} points={} pixels={}", face.family, font.points, face.pixel_size);
        } else {
            eprintln!("romacs: font family={} pixels={}", face.family, face.pixel_size);
        }
        let (lh, cw) = romacs_layout::frame_metrics(&face)?;
        vm.set_frame_geometry(width, height, lh, cw)?;
    } else {
        vm.eval_source(if g1 { include_str!("../g1.el") } else { SCENE })?;
    }
    // A live GUI has a keyboard: prompts read it mid-command (prompt.rs).
    let host = shared.clone();
    let last_presentation = || host.borrow().presentation;
    if live {
        vm.set_host_input(Some(Box::new(prompt::GuiHost { shared: host.clone(), sender: sender.clone(), engine: engine.clone(), images: images.clone(), terminal_ready: terminal_ready.clone(), processes: vec![] })));
    }
    // A server listens before its first window, which a client asks for
    // (lisp/session/romacs-server.el); one that cannot listen ends here.
    if let Some(name) = &serve {
        host.borrow_mut().server = Some(prompt::ServerWindow { shown: false, size: (width, height), title: "romacs".into() });
        vm.eval_source(&format!("(progn (require 'romacs-server) (romacs-server-start {}))", startup::lisp_string(name)))
            .map_err(|error| format!("server {name}: {error}"))?;
        // `daemonp' kept startup from showing the screen, and the window
        // paints before the client that asked for it is served.
        if serve_landing && let Err(error) = vm.eval_source("(romacs-server--landing)") {
            eprintln!("romacs: landing: {error}");
        }
        startup::mark_phase("server");
    }
    let initial = frame(&mut vm, g1, width, height)?;
    startup::mark_phase("initial-frame");
    let mut paint_timings = None;
    let mut key_trace = tracing().then(latency::Trace::default);
    let first_submitted = Instant::now();
    // A window asked for at the server's start is the session's first: it
    // gets the first paint, as a session's window does.
    let early = early_window.take().map(|ready| ready.recv_timeout(std::time::Duration::from_secs(30)).map_err(|e| e.to_string()).and_then(|r| r));
    if let Some(Err(error)) = &early { eprintln!("romacs: server window: {error}"); }
    if let (Some(Ok(())), Some(server)) = (&early, host.borrow_mut().server.as_mut()) { server.shown = true; }
    // The session's first paint takes the window from the launch screen.
    host.borrow_mut().end_loading();
    let mut previous = if serve.is_some() && !host.borrow().window_shown() {
        Shown { frame: initial, ends: Vec::new(), presentation: last_presentation(), tab_bar: Vec::new(), status_bar: Default::default() }
    } else {
        paint(&mut engine.borrow_mut(), &mut vm, sender, &images, last_presentation(), force_gc, floor, initial, &host,
              if live { Wait::Bounded } else { Wait::Rendered }, &mut paint_timings)?
    };
    title::consider(&mut vm, sender, &host);
    let mut live_presentation = presentation::Presentation::new();
    if live && host.borrow().window_shown() {
        // In flight, drawn or not: the loop retires it when it sees it drawn
        // (a `paint' line, and the first-paint phase), or tells it stalled
        // when its window is hidden as it starts.
        live_presentation.submitted(previous.presentation, first_submitted);
    }
    startup::mark_phase("first-paint");
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
            terminate(&mut vm, &host, Lent { live: &mut live_presentation, width: &mut width, height: &mut height })?;
            // Background callbacks are Lisp the loop runs, and may prompt.
            let lent = Lent { live: &mut live_presentation, width: &mut width, height: &mut height };
            let (changed, closed) = lent.run(&host, |_| service_native(&mut vm));
            if changed? { live_presentation.changed(); }
            if closed && close_window(&mut vm, &host, Lent { live: &mut live_presentation, width: &mut width, height: &mut height })? {
                return shutdown_editor(&mut vm, sender, exit::Exit::new("window-close", "", 0));
            }
        }
        // This pass handles the input that arrived before it, and any a read
        // inside it took from the channel; later input waits for the next.
        let available = sender.input().len();
        presentation::admit(&mut queue(&host), sender.input().try_iter(), available);
        loop {
            // read_char returns the quit character for a quit still pending
            // (keyboard.c:4102), ahead of the keys typed meanwhile.
            if live && vm.take_quit_key() { queue(&host).push_front(quit::quit_key(romacs_display_runtime::paint_host::INITIAL_FRAME)); }
            // read_char takes Lisp's unread queues before host input: a key
            // Lisp read and pushed back (sit-for in a timer) starts the next
            // command, which the Lisp command loop reads.
            let unread = live && command::unread_pending(&mut vm)?;
            let event = if unread { None } else {
                let next = queue(&host).pop_front();
                match next { Some(event) => Some(event), None => break }
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
                    // A server's closed window may report its end twice.
                    if !host.borrow().window_shown() { continue; }
                    if !live || close_window(&mut vm, &host, Lent { live: &mut live_presentation, width: &mut width, height: &mut height })? {
                        return shutdown_editor(&mut vm, sender, exit::Exit::new("window-close", "", 0));
                    }
                    continue;
                }
                received = if tracing() {
                    romacs_display_runtime::paint_host::take_key_enqueue_time(&event)
                } else { None };
                // Each pressed key's enqueue time on CLOCK_MONOTONIC, for
                // latency to a point outside the editor (terminal_latency_live.py).
                if let Some(at) = received && matches!(command::unwrap_event(event.clone()), InputEvent::Key { pressed: true, .. }) {
                    crate::diagnostic_line(&json!({"diagnostic":"key-ingress","monotonic_ns":latency::monotonic_ns(at)}));
                }
                // read_char's timer_stop_idle: Emacs is idle until a key arrives.
                if live && timers::ends_idle(&event) { vm.timer_stop_idle(); }
                let event = command::unwrap_event(event);
                command_start = Instant::now();
                if live && let InputEvent::WindowResize { width: w, height: h, scale_factor, .. } = event {
                    // A HiDPI output: images decode at its device pixels.
                    if vm.set_device_scale(scale_factor as f32) { live_presentation.changed(); }
                    if w > 0 && h > 0 && (width, height) != (w, h) {
                        width = w; height = h;
                        if let Some(server) = host.borrow_mut().server.as_mut() { server.size = (w, h); }
                        let (lh, cw) = romacs_layout::frame_metrics(&vm.display_default_face()?)?;
                        vm.set_frame_geometry(width, height, lh, cw)?;
                        // The repaint's redisplay runs `window-size-change-functions'
                        // (`frame_snapshot'), as GNU's does.
                        live_presentation.changed();
                    }
                    continue;
                }
                // A click on the tab bar or the status bar runs its binding
                // there and then.
                if live && let Some(click) = tab_bar_click(&previous, &event).or_else(|| status_bar_click(&previous, &event)) {
                    if let Err(error) = vm.eval_source(&click) { echo::show(&mut vm, &error)?; }
                    live_presentation.changed();
                    continue;
                }
                // A native model's event runs its special-event binding
                // (lisp/graphics/romacs-model.el), then the frame repaints.
                // Its callbacks may read input: they get the keys queued
                // ahead, as a command does.
                if live && let InputEvent::Model { id, event } = &event {
                    let lent = Lent { live: &mut live_presentation, width: &mut width, height: &mut height };
                    vm.enter_busy();
                    let (result, closed) = lent.run(&host, |_| vm.run_model_event(*id, event));
                    vm.leave_busy();
                    if let Err(error) = result { echo::show(&mut vm, &error)?; }
                    if closed && close_window(&mut vm, &host, Lent { live: &mut live_presentation, width: &mut width, height: &mut height })? {
                        return shutdown_editor(&mut vm, sender, exit::Exit::new("window-close", "", 0));
                    }
                    live_presentation.changed();
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
                // A key the command loop would only send to the selected
                // terminal goes to its pty now (`Vm::terminal_passthrough_key'):
                // the keymaps decide, without the loop's cost per key.
                if live && matches!(event, InputEvent::Key { pressed: true, .. })
                    && let Some(key) = native::translate(event.clone()) {
                    let fast_start = Instant::now();
                    let fast = vm.terminal_passthrough_key(&key);
                    if tracing() {
                        crate::diagnostic_line(&json!({"diagnostic":"terminal-passthrough","taken":fast == Ok(true),"ns":fast_start.elapsed().as_nanos()}));
                    }
                    match fast {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(error) => echo::show(&mut vm, &error)?,
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
                if let (Some(_), Some(raw)) = (&key, raw) { queue(&host).push_front(raw); }
                key.map(Some)
            };
            if let Some(first) = translated {
                let outcome = if live {
                    let lent = Lent { live: &mut live_presentation, width: &mut width, height: &mut height };
                    vm.enter_busy();
                    let (outcome, closed) = lent.run(&host, |live| live.host_command(&mut vm, first.as_ref()));
                    vm.leave_busy();
                    if closed && close_window(&mut vm, &host, Lent { live: &mut live_presentation, width: &mut width, height: &mut height })? {
                        return shutdown_editor(&mut vm, sender, exit::Exit::new("window-close", "", 0));
                    }
                    outcome
                } else {
                    vm.eval_source("(set-buffer (window-buffer (selected-window)))")?;
                    vm.command_event(first.expect("a host key")).map(Some)
                };
                if live && vm.eval_source("romacs-gui-quit-requested")? != romacs_vm::Fixed::NIL {
                    return shutdown_editor(&mut vm, sender, exit::Exit::new("gui-quit", "", 0));
                }
                let command_ns = command_start.elapsed().as_nanos();
                let snapshot_ns = if !live {
                    let snapshot_start = Instant::now();
                    let next = frame(&mut vm, g1, width, height)?;
                    title::consider(&mut vm, sender, &host);
                    let snapshot_ns = snapshot_start.elapsed().as_nanos();
                    let presentation = next_presentation(&host)?;
                    previous = paint(&mut engine.borrow_mut(), &mut vm, sender, &images, presentation, force_gc, floor, next, &host, Wait::Rendered, &mut paint_timings)?;
                    Some(snapshot_ns)
                } else { None };
                if tracing() && !live {
                    crate::diagnostic_line(&json!({"diagnostic":"latency-key","presentation":last_presentation(),"units":"ns",
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
                        // Each command's reply is a harness protocol: a daily
                        // session's log keeps no record of its commands,
                        // signals or messages.
                        if diagnostics() {
                            if let Some(message) = &outcome.message { eprintln!("{message}"); }
                            let reply_start = diagnostic_start.map(|_| Instant::now());
                            reply(json!({"native_command":outcome.command,"signal":outcome.signal.map(|s|json!({"condition":s.condition,"data":s.data})),
                                "bell":outcome.bell,"message":outcome.message,"presentation":last_presentation()}))?;
                            reply_ns = reply_start.map(|start| start.elapsed().as_nanos()).unwrap_or(0);
                        }
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
                if let Ok(diagnostic) = sender.wait_for_render(romacs_display_runtime::paint_host::INITIAL_FRAME, in_flight, std::time::Duration::ZERO) {
                    live_presentation.completed(in_flight, Instant::now());
                    if diagnostics() { crate::diagnostic_line(&json!({"diagnostic":"paint","presentation":diagnostic.presentation,
                        "buffer_versions":previous.frame.snapshot.windows.iter().map(|w|w.snapshot.buffer_version).collect::<Vec<_>>(),
                        "unresolved_face_text_total":diagnostic.unresolved_face_text_total})); }
                    if tracing() {
                        crate::diagnostic_line(&json!({"diagnostic":"latency-render","presentation":in_flight,
                            "units":"ns","submit_to_render_observed_ns":submitted.elapsed().as_nanos(),
                            "render_semantics":"renderer-successful-submit-not-compositor-present"}));
                    }
                    retired = true;
                } else if let Some((undrawn, waited)) = live_presentation.stall_warning(presentation::render_stall()) {
                    // A window the compositor does not draw (another workspace,
                    // minimized) gets no frame callbacks; its paint renders when
                    // it shows. Editing goes on, its paints coalesced meanwhile.
                    eprintln!("romacs: render stalled: presentation {undrawn} undrawn for {:.1}s \
                               (window hidden?); editing continues and repaints when it shows", waited.as_secs_f64());
                }
            }
            // Here, or in a prompt that settled it first (`Lent').
            for (presentation, observed) in live_presentation.take_retired() {
                if let Some(trace) = &mut key_trace {
                    for sample in trace.completed(presentation, observed) { eprintln!("{sample}"); }
                }
            }
            // Pixels landed, a parse a window waits on did, a publication
            // left images to send, or the renderer lost its images (a
            // device reset woke this wait): paint again.
            if vm.land_images()? || vm.land_treesit() || images.borrow().must_repaint(sender.as_ref()) { live_presentation.changed(); }
            if let Some(result) = live_presentation.after_batch(|| -> Result<_, String> {
                // A server with no window paints nothing until one shows.
                if !host.borrow().window_shown() { return Ok(None); }
                let snapshot_start = Instant::now();
                vm.redisplay_begin();
                vm.defer_parse()?;
                let next = frame(&mut vm, g1, width, height)?;
                title::consider(&mut vm, sender, &host);
                let snapshot_ns = snapshot_start.elapsed().as_nanos();
                // GNU's redisplay draws nothing when nothing changed: a timer
                // such as show-paren's runs after every key.
                let lost = images.borrow().must_repaint(sender.as_ref());
                let Some(next) = changed_frame(&mut vm, &previous, next, last_presentation(), lost)? else {
                    vm.redisplay_done(snapshot_start.elapsed());
                    return Ok(None)
                };
                let presentation = next_presentation(&host)?;
                previous = paint(&mut engine.borrow_mut(), &mut vm, sender, &images, presentation, force_gc, floor, next, &host, Wait::No, &mut paint_timings)?;
                // The evaluator's part: the frame's snapshot, layout and
                // submission; the render thread draws it meanwhile.
                vm.redisplay_done(snapshot_start.elapsed());
                Ok(Some((snapshot_start, snapshot_ns, presentation)))
            }) {
                if let Some((snapshot_start, snapshot_ns, presentation)) = result? {
                    live_presentation.submitted(presentation, Instant::now());
                    if let Some(trace) = &mut key_trace {
                        trace.submitted(presentation, paint_timings.take().expect("traced paint"), snapshot_start, snapshot_ns);
                    }
                    if tracing() {
                        crate::diagnostic_line(&json!({"diagnostic":"latency-snapshot","presentation":presentation,
                            "units":"ns","snapshot_ns":snapshot_ns}));
                    }
                } else {
                    live_presentation.unchanged();
                    if let Some(trace) = &mut key_trace {
                        for sample in trace.unchanged() { eprintln!("{sample}"); }
                    }
                }
            }
            if vm.deferred_parse_done() { live_presentation.changed(); }
            if queue(&host).is_empty() && sender.input().is_empty() {
                // read_char: with no key to read, idle and run ripe timers;
                // after any ran, redisplay and check again before blocking.
                // A timer may read input (`read-event' with a timeout, a
                // prompt) as a command may, so it gets the same handoff.
                let auto_save = retired || live_presentation.in_flight().is_none()
                    || live_presentation.stalled(presentation::render_stall());
                let lent = Lent { live: &mut live_presentation, width: &mut width, height: &mut height };
                // Timers run Lisp as commands do (an agent's tool call is one):
                // busy, so C-g and a call's deadline reach it.
                let (idle, closed) = lent.run(&host, |_| {
                    vm.enter_busy();
                    let idle = timers::idle(&mut vm, auto_save);
                    vm.leave_busy();
                    idle
                });
                if closed && close_window(&mut vm, &host, Lent { live: &mut live_presentation, width: &mut width, height: &mut height })? {
                    return shutdown_editor(&mut vm, sender, exit::Exit::new("window-close", "", 0));
                }
                let idle = idle.unwrap_or_else(|error| {
                    let _ = echo::show(&mut vm, &format!("Timer: {error}"));
                    timers::Idle { ran: true, message: false, delay: None }
                });
                if idle.ran || idle.message {
                    if idle.message { echo::sync_current_message(&mut vm)?; }
                    live_presentation.changed();
                    continue;
                }
                // An event the idle pass took from the channel to see it
                // (`input-pending-p', an idle collection giving way to a
                // key) is queued here now: read it rather than wait.
                if !queue(&host).is_empty() { continue; }
                // read_char's wait_reading_process_output also selects on
                // the processes: one ready wakes the loop to read it.
                let processes = vm.process_wait_fds();
                // A termination signal whose wake other work took: back to the top.
                if romacs_vm::host::termination_signal(true).is_some() { continue; }
                // A user signal `debug-on-event' names sets `quit-flag': its
                // quit key runs into the debugger.
                vm.take_debug_event();
                if vm.take_quit_key() { queue(&host).push_back(quit::quit_key(romacs_display_runtime::paint_host::INITIAL_FRAME)); continue; }
                match terminal_wait::wait(sender.input(), &terminal_ready, timers::sooner(live_presentation.wait(presentation::render_stall()), idle.delay), &processes) {
                    terminal_wait::Wake::Input(event) => queue(&host).push_back(event),
                    terminal_wait::Wake::Closed => return Ok(exit::Exit::new("host-input-closed", "", 1)),
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
        vm.land_images()?;
        let next = frame(&mut vm, g1, width, height)?;
        title::consider(&mut vm, sender, &host);
        // Asked apart: a borrow in the condition would outlive it into paint's publication.
        let lost = images.borrow().must_repaint(sender.as_ref());
        if let Some(next) = changed_frame(&mut vm, &previous, next, last_presentation(), lost)? {
            let presentation = next_presentation(&host)?;
            previous = paint(&mut engine.borrow_mut(), &mut vm, sender, &images, presentation, force_gc, floor, next, &host, Wait::Rendered, &mut paint_timings)?;
        }
        match result {
            Err(error) => reply(json!({"error":error,"presentation":last_presentation()}))?,
            Ok(result) => reply(json!({"result":result,"presentation":last_presentation()}))?,
        }
    }
    shutdown_editor(&mut vm, sender, exit::Exit::new("stdin-closed", "", 0))
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
fn close_window(vm: &mut Vm, host: &std::rc::Rc<std::cell::RefCell<prompt::Shared>>, lent: Lent<'_>) -> Result<bool, String> {
    terminate(vm, host, Lent { live: &mut *lent.live, width: &mut *lent.width, height: &mut *lent.height })?;
    // A server's window is a resource of the session: closing it ends the
    // session only when Lisp says so (lisp/session/romacs-server.el).
    if host.borrow().server.is_some() {
        return lent.run(host, |live| {
            live.changed();
            vm.eval_source("(romacs-server-window-closed)").map(|end| !end.is_nil())
        }).0;
    }
    lent.run(host, |live| live.window_close(vm)).0
}

/// emacs.c terminate_due_to_signal: a SIGTERM or SIGHUP the host noted runs
/// `(kill-emacs SIGNAL)', whose hook and auto-save run before the session
/// host exits with that status; a prompt it interrupted has unwound first.
fn terminate(vm: &mut Vm, host: &std::rc::Rc<std::cell::RefCell<prompt::Shared>>, lent: Lent<'_>) -> Result<(), String> {
    let Some(signal) = romacs_vm::host::termination_signal(false) else { return Ok(()) };
    lent.run(host, |_| vm.eval_source(&format!("(kill-emacs {signal})"))).0.map(|_| ())
}

/// The outer loop's presentation state, lent to the prompt host
/// (prompt.rs) for every piece of Lisp the loop runs (a command, a timer, a
/// background callback, a close), since any of them may read input mid-way:
/// the presentation in flight and the frame's size. The keys queued ahead
/// need no lending: the loop's one queue is the host's (`prompt::Shared`),
/// where any read, inside a lend or not, finds and leaves them. What the
/// reads settled or saw (a resize, a close) comes back; a read that presented a prompt leaves the frame to repaint, even
/// when the Lisp changed nothing else. A read that redisplays without the
/// in-flight presentation waits for one the renderer has already passed.
struct Lent<'a> {
    live: &'a mut presentation::Presentation,
    width: &'a mut u32,
    height: &'a mut u32,
}

impl Lent<'_> {
    /// RUN's value, and whether a read inside it saw the frame close.
    fn run<T>(self, host: &std::rc::Rc<std::cell::RefCell<prompt::Shared>>, run: impl FnOnce(&mut presentation::Presentation) -> T) -> (T, bool) {
        let before = {
            let mut shared = host.borrow_mut();
            shared.in_flight = self.live.in_flight();
            shared.presentation
        };
        let value = run(self.live);
        let mut shared = host.borrow_mut();
        // A server's window shown or hidden meanwhile: the new one is
        // painted whole under a fresh presentation; the old one's in-flight
        // paint will never draw.
        if std::mem::take(&mut shared.window_changed) {
            *self.live = presentation::Presentation::new();
            shared.presentation += 1;
        }
        if shared.presentation != before { self.live.changed(); }
        if let Some((settled, at)) = shared.settled.take() { self.live.completed(settled, at); }
        self.live.lent_back(shared.in_flight.take());
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

/// END, once the session's processes and window are shut down.
fn shutdown_editor(vm: &mut Vm, sender: &PaintSender, end: exit::Exit) -> Result<exit::Exit, String> {
    vm.shutdown_background_processes()?;
    sender.shutdown()?;
    Ok(end)
}

fn parse_geometry(value: &str) -> Result<(u32, u32), String> {
    let (w, h) = value.split_once('x').ok_or("geometry must be WIDTHxHEIGHT")?;
    let w = w.parse::<u32>().map_err(|e| e.to_string())?;
    let h = h.parse::<u32>().map_err(|e| e.to_string())?;
    if !(320..=16384).contains(&w) || !(200..=16384).contains(&h) { return Err("geometry out of range".into()); }
    Ok((w, h))
}
