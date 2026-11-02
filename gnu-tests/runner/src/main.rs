//! VM-side adapter for the GNU test runner (`gnu-tests/run.py`).
//!
//! One process evaluates ONE GNU test file in ONE fresh VM. Real mode loads
//! pinned GNU ERT and the target with ordinary load semantics, optionally applies
//! the named Romacs batch adapter, then evaluates
//! an observer reporter which calls `ert-run-tests-batch`; it never enters the
//! reduced mini accessor path. Mini mode (the default) uses VM2's Lisp prelude,
//! the loadup bootstrap chain as far as it reaches, the test harness, then every top-level form of the test file, each
//! through `Vm::eval_source` so that one failing form does not hide the rest.
//! Then every test the harness registered runs, in registration order.
//!
//! Mini mode writes one JSON object per line to stdout and flushes after each.
//! Real mode streams GNU's messages and `ERT-SCOREBOARD` observer lines there,
//! plus JSON setup diagnostics. A killed run retains every earlier verdict. This
//! binary never decides pass/fail for the suite: it reports what the harness
//! returned. Classification, exclusions and the summary belong to `run.py`.
//!
//! Exit status: mini 0 when every phase ran (whatever the verdicts); real 0/1
//! from GNU's unexpected-result count; 2 usage, 3 setup/prelude failure,
//! 4 harness/test/reporter load failure, 70 a panic inside the VM.
use romacs_vm::Vm;
use serde_json::{Value as Json, json};
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

mod world;

/// Every allocation in the world arena, so a world image can be this
/// process's heap (`romacs_vm::world::arena`).
#[global_allocator]
static ALLOCATOR: romacs_vm::world::arena::Arena = romacs_vm::world::arena::Arena;

#[derive(Clone, Copy, PartialEq, Eq)]
enum HarnessKind {
    Mini,
    Real,
}

struct Args {
    kind: HarnessKind,
    measurement_regime: bool,
    ert_bootstrap: bool,
    ert_batch_adapter: bool,
    selector: String,
    reporter: Option<PathBuf>,
    load_path: Vec<PathBuf>,
    preload: Vec<PathBuf>,
    harness: PathBuf,
    plan: PathBuf,
    skip: Vec<String>,
    probe: Vec<String>,
    /// Mini mode: `romacs-mini-ert-audit.el`, loaded after the harness. Each
    /// result then carries `caught`: the should-errors absent machinery
    /// satisfied (run.py refuses to count a pass without it).
    audit: Option<PathBuf>,
    /// The Romacs executable the session names as `invocation-name' and
    /// `invocation-directory' (GNU's argv[0]): tests spawn it as their
    /// Emacs, and this runner is not one (romacs-vm-gui's `--batch' is).
    invocation: Option<PathBuf>,
    /// `--world-trace OUT`: write the recipe's world-build trace to OUT.
    world_trace: Option<PathBuf>,
    /// `--world-trace-child DIR`: set in the confined child that
    /// `--world-trace` runs, with the directory of its pinned table.
    world_child: Option<PathBuf>,
    /// `--world-store DIR`: boot from the world of the loadup chain in DIR,
    /// building it beside a source boot when it is missing or stale.
    world_store: Option<PathBuf>,
}

fn usage() -> ! {
    eprintln!(
        "usage: romacs-gnu-ert --harness FILE --plan PLAN.json [--load-path DIR]... \
         [--preload FILE]... [--skip-test NAME]... [--probe EXPR]... \
         [--harness-kind mini|real] [--ert-adapter none|romacs-batch] [--reporter FILE] [--audit FILE] \
         [--world-trace OUT.json] [--world-store DIR] [--invocation EMACS]"
    );
    std::process::exit(2)
}

fn parse() -> Args {
    let mut it = std::env::args().skip(1);
    let (mut load_path, mut preload, mut skip, mut probe) = (vec![], vec![], vec![], vec![]);
    let (mut harness, mut plan, mut reporter, mut audit, mut invocation) = (None, None, None, None, None);
    let (mut world_trace, mut world_child, mut world_store) = (None, None, None);
    let mut kind = HarnessKind::Mini;
    let mut measurement_regime = false;
    let mut ert_bootstrap = false;
    let mut ert_batch_adapter = false;
    let mut selector = "t".to_owned();
    while let Some(flag) = it.next() {
        let Some(value) = it.next() else { usage() };
        match flag.as_str() {
            "--load-path" => load_path.push(PathBuf::from(value)),
            "--preload" => preload.push(PathBuf::from(value)),
            "--measurement-regime" if value == "retained-bindings-prefix" => measurement_regime = true,
            "--bootstrap-regime" if value == "gnu-ert" => ert_bootstrap = true,
            "--ert-adapter" if value == "romacs-batch" => ert_batch_adapter = true,
            "--ert-adapter" if value == "none" => ert_batch_adapter = false,
            "--selector" => selector = value,
            "--harness" => harness = Some(PathBuf::from(value)),
            "--plan" => plan = Some(PathBuf::from(value)),
            "--reporter" => reporter = Some(PathBuf::from(value)),
            "--audit" => audit = Some(PathBuf::from(value)),
            "--invocation" => invocation = Some(PathBuf::from(value)),
            "--harness-kind" => {
                kind = match value.as_str() {
                    "mini" => HarnessKind::Mini,
                    "real" => HarnessKind::Real,
                    _ => usage(),
                }
            }
            "--skip-test" => skip.push(value),
            "--probe" => probe.push(value),
            "--world-trace" => world_trace = Some(PathBuf::from(value)),
            "--world-trace-child" => world_child = Some(PathBuf::from(value)),
            "--world-store" => world_store = Some(PathBuf::from(value)),
            _ => usage(),
        }
    }
    let (Some(harness), Some(plan)) = (harness, plan) else {
        usage()
    };
    if ((measurement_regime || ert_batch_adapter || selector != "t") && kind != HarnessKind::Real)
        || (measurement_regime && ert_bootstrap)
        || (kind == HarnessKind::Real && (!skip.is_empty() || !probe.is_empty()))
        || (kind == HarnessKind::Mini && reporter.is_some())
        || (kind == HarnessKind::Real && audit.is_some())
        || (world_trace.is_some() && kind != HarnessKind::Real)
        || (world_child.is_some() && world_trace.is_none())
        || (world_store.is_some() && (kind != HarnessKind::Real || world_trace.is_some()))
    {
        usage();
    }
    Args {
        kind,
        measurement_regime,
        ert_bootstrap,
        ert_batch_adapter,
        selector,
        reporter,
        load_path,
        preload,
        harness,
        plan,
        skip,
        probe,
        audit,
        invocation,
        world_trace,
        world_child,
        world_store,
    }
}

fn retained_bindings_failure(enabled: bool, file: &std::path::Path, error: &str) -> bool {
    enabled && file.file_name().is_some_and(|name| name == "bindings.el")
        && error.starts_with(&format!("{}: line 1225 form 129: ", file.display()))
        && error.ends_with(": (void-function map-keymap)")
}

#[test]
fn retained_prefix_requires_exact_outer_failure() {
    let file = std::path::Path::new("/gnu/lisp/bindings.el");
    let exact = "/gnu/lisp/bindings.el: line 1225 form 129: (map-keymap ...): (void-function map-keymap)";
    assert!(retained_bindings_failure(true, file, exact));
    assert!(!retained_bindings_failure(false, file, exact));
    assert!(!retained_bindings_failure(true, file, &exact.replace("form 129:", "form 1290:")), "RETAIN_EXACT_FORM");
    assert!(!retained_bindings_failure(true, file, &format!("/gnu/lisp/other.el: line 1 form 1: (require 'bindings): {exact}")), "RETAIN_OUTER_FILE");
    assert!(!retained_bindings_failure(true, file, &exact.replace("(void-function map-keymap)", "(void-variable map-keymap)")), "RETAIN_EXACT_CONDITION");
}

fn emit(event: Json) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{event}").expect("stdout");
    out.flush().expect("stdout");
}

fn lisp_string(text: &str) -> String {
    let mut s = String::with_capacity(text.len() + 2);
    s.push('"');
    for c in text.chars() {
        if c == '"' || c == '\\' {
            s.push('\\');
        }
        s.push(c);
    }
    s.push('"');
    s
}

/// Evaluate SOURCE, converting a panic into an error tagged `panic`.
fn eval<'vm>(vm: &'vm mut Vm, source: &str) -> Result<romacs_vm::Local<'vm>, (bool, String)> {
    match catch_unwind(AssertUnwindSafe(move || {
        let vm = vm;
        vm.eval_source(source)
    })) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err((false, e)),
        Err(p) => Err((
            true,
            p.downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic".into()),
        )),
    }
}

fn shown(v: romacs_vm::Local<'_>) -> String {
    v.display()
        .unwrap_or_else(|e| format!("#<undisplayable: {e}>"))
}

fn panic_exit(phase: &str, detail: &str, test: Option<&str>) -> ! {
    emit(json!({"event": "panic", "phase": phase, "test": test, "detail": detail}));
    std::process::exit(70)
}

/// Read a harness accessor that must return a Lisp string.
fn string_of(vm: &mut Vm, source: &str) -> Result<String, String> {
    let v = eval(vm, source).map_err(|(p, e)| {
        if p {
            panic_exit("harness-query", &e, None)
        } else {
            e
        }
    })?;
    let text = v.display()?;
    if let Some(inner) = text.strip_prefix("#u8(").and_then(|t| t.strip_suffix(')')) {
        // A unibyte string (VM2 returns `symbol-name' of a pure-ASCII or
        // UTF-8 name this way); decode its octets, refusing non-UTF-8.
        let bytes = inner
            .split_whitespace()
            .map(|n| n.parse::<u8>())
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|_| format!("harness returned malformed octets {text}"))?;
        return String::from_utf8(bytes).map_err(|_| format!("harness returned non-UTF-8 {text}"));
    }
    serde_json::from_str::<String>(&text).map_err(|_| format!("harness returned non-string {text}"))
}

fn integer_of(vm: &mut Vm, source: &str) -> Result<usize, String> {
    let v = eval(vm, source).map_err(|(p, e)| {
        if p {
            panic_exit("harness-query", &e, None)
        } else {
            e
        }
    })?;
    let text = v.display()?;
    text.parse()
        .map_err(|_| format!("harness returned non-integer {text}"))
}

/// VM `message` writes directly to fd 2 (lib.rs), not an embedder callback.
/// Merge it into fd 1 before VM startup: native summaries and observer messages
/// stay ordered and are streamed even if the supervisor later kills a test.
#[cfg(unix)]
fn native_output_to_stdout() -> std::io::Result<()> {
    unsafe extern "C" {
        fn dup2(oldfd: std::ffi::c_int, newfd: std::ffi::c_int) -> std::ffi::c_int;
    }
    // SAFETY: dup2 takes integer descriptors, no pointers; stdout/stderr are
    // process-owned and this single-threaded adapter calls it before VM startup.
    if unsafe { dup2(1, 2) } == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(unix))]
fn native_output_to_stdout() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "real ERT output requires Unix fd redirection",
    ))
}

/// Real mode uses the VM's ordinary loader, never plan-form recovery or mini
/// accessors. A reporter observes GNU's runner and returns its 0/1 exit status;
/// it must not call kill-emacs, which is not an embedder process-exit API.
fn run_real(vm: &mut Vm, args: &Args, file: &str) -> ! {
    match catch_unwind(AssertUnwindSafe(|| {
        vm.load_file(std::path::Path::new(file)).map(|_| ())
    })) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let report = vm.load_report();
            let failed_forms = report["steps"].as_array().map(|steps| steps.iter()
                .filter(|step| step["status"] != "ok").collect::<Vec<_>>());
            emit(
                json!({"event": "setup", "ok": false, "phase": "test-load", "file": file, "error": e, "failed_forms": failed_forms}),
            );
            std::process::exit(4);
        }
        Err(_) => panic_exit("test-load", "panic", None),
    }
    if let Err((panic, error)) = eval(vm, &format!("(setq ert-scoreboard-selector {})", args.selector)) {
        if panic { panic_exit("selector", &error, None); }
        emit(json!({"event": "setup", "ok": false, "phase": "selector", "error": error}));
        std::process::exit(4);
    }
    let result = if let Some(path) = &args.reporter {
        match catch_unwind(AssertUnwindSafe(|| vm.load_file(path).map(shown))) {
            Ok(r) => r.map_err(|e| (false, e)),
            Err(_) => Err((true, "panic".into())),
        }
    } else {
        eval(
            vm,
            include_str!("../../harness/romacs-real-ert-reporter.el"),
        )
        .map(shown)
    };
    match result {
        Ok(status) if status == "0" || status == "1" => {
            std::process::exit(if status == "0" { 0 } else { 1 });
        }
        Ok(value) => {
            emit(
                json!({"event": "setup", "ok": false, "phase": "reporter", "error": format!("reporter must return 0 or 1, got {value}")}),
            );
            std::process::exit(4);
        }
        Err((true, e)) => panic_exit("reporter", &e, None),
        Err((false, e)) => {
            emit(json!({"event": "setup", "ok": false, "phase": "reporter", "error": e}));
            std::process::exit(4);
        }
    }
}

/// The VM the chain starts in: made, in batch mode on GNU's batch frame,
/// with the load path and the prelude. A failure exits with status 3.
fn fresh_vm(args: &Args) -> Vm {
    let mut vm = Vm::new();
    vm.world_begin();
    if let Err((_, error)) = eval(&mut vm, "(setq noninteractive t)") {
        emit(json!({"event": "setup", "ok": false, "phase": "batch-mode", "error": error}));
        std::process::exit(3);
    }
    if let Some(emacs) = &args.invocation {
        let (Some(name), Some(directory)) = (emacs.file_name(), emacs.parent()) else { usage() };
        let quote = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
        let set = format!("(setq invocation-name {} invocation-directory {})",
                          quote(&name.to_string_lossy()), quote(&format!("{}/", directory.display())));
        if let Err((_, error)) = eval(&mut vm, &set) {
            emit(json!({"event": "setup", "ok": false, "phase": "invocation", "error": error}));
            std::process::exit(3);
        }
    }
    // GNU batch runs on its initial terminal frame: 80 columns by 25 lines
    // of 1x1 characters (DIVERGENCES.md VM2-WIN5).
    if let Err(error) = vm.set_frame_geometry(80, 25, 1, 1) {
        emit(json!({"event": "setup", "ok": false, "phase": "batch-geometry", "error": error.to_string()}));
        std::process::exit(3);
    }
    if let Err(e) = vm.set_load_path(&args.load_path) {
        emit(json!({"event": "setup", "ok": false, "phase": "load-path", "error": e}));
        std::process::exit(3);
    }
    match catch_unwind(AssertUnwindSafe(|| vm.install_lisp_prelude().map(|_| ()))) {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            emit(json!({"event": "setup", "ok": false, "phase": "prelude", "error": e}));
            std::process::exit(3);
        }
        Err(_) => panic_exit("prelude", "panic", None),
    }
    vm
}

/// Load the preloads FROM..TO, each entry onto CHAIN; false once one failed
/// and stopped the chain.
fn load_chain(vm: &mut Vm, args: &Args, from: usize, to: usize, chain: &mut Vec<Json>) -> bool {
    for link in &args.preload[from..to] {
        let result = match catch_unwind(AssertUnwindSafe(|| vm.load_file(link).map(|_| ()))) {
            Ok(r) => r.map(|_| ()),
            Err(_) => panic_exit("chain", &link.display().to_string(), None),
        };
        let failed = result.is_err();
        let retained = result.as_ref().is_err_and(|error| retained_bindings_failure(args.measurement_regime, link, error));
        chain.push(json!({"file": link, "loaded": !failed, "retained_failed_prefix": retained,
                          "error": result.err()}));
        if failed && !retained {
            return false; // the chain as far as it reaches, exactly as the warm census
        }
    }
    true
}

/// The evaluator's stack, romacs-vm-gui's `EVALUATOR_STACK_SIZE` for its
/// `--batch`: a test file sees the stack Romacs's batch gives Lisp, not
/// whatever `ulimit -s` the invoking shell had.
const EVALUATOR_STACK_SIZE: usize = 64 * 1024 * 1024;

fn main() {
    if let Err(error) = romacs_vm::host::catch_user_signals() {
        emit(json!({"event": "setup", "ok": false, "phase": "user-signals", "error": error.to_string()}));
        std::process::exit(3);
    }
    let evaluator = std::thread::Builder::new()
        .name("romacs-evaluator".into())
        .stack_size(EVALUATOR_STACK_SIZE)
        .spawn(evaluate);
    match evaluator.map(|thread| thread.join()) {
        Ok(Ok(())) => {}
        Ok(Err(_)) => panic_exit("evaluator", "panic", None),
        Err(error) => {
            emit(json!({"event": "setup", "ok": false, "phase": "evaluator", "error": error.to_string()}));
            std::process::exit(3);
        }
    }
}

fn evaluate() {
    let args = parse();
    let world_out = args.world_trace.clone().zip(args.world_child.clone());
    if let Some(out) = &args.world_trace && args.world_child.is_none() {
        world::trace(out);
    }
    if args.kind == HarnessKind::Real {
        if let Err(e) = native_output_to_stdout() {
            emit(json!({"event": "setup", "ok": false, "phase": "output", "error": e.to_string()}));
            std::process::exit(3);
        }
    }
    let plan: Json = romacs_vm::host::read_to_string(&args.plan)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| {
            eprintln!("romacs-gnu-ert: unreadable plan {}", args.plan.display());
            std::process::exit(2)
        });
    let file = plan["file"].as_str().unwrap_or_else(|| usage()).to_owned();
    let forms = plan["forms"].as_array().cloned().unwrap_or_default();

    if let Some((out, dir)) = &world_out {
        let recipe = args.load_path.iter().chain(&args.preload).chain([&args.harness, &args.plan]).chain(&args.reporter);
        world::begin(recipe.cloned().collect(), out, dir);
    }
    let (mut vm, mut chain, from) = world::boot(&args);
    load_chain(&mut vm, &args, from, args.preload.len(), &mut chain);
    emit(json!({"event": "chain", "entries": chain, "regime": if args.measurement_regime { "MEASUREMENT REGIME: retained failed bindings prefix" } else if args.ert_bootstrap { "gnu-ert" } else { "standard" }}));
    if (args.kind == HarnessKind::Real || args.ert_bootstrap) && chain.iter().any(|entry| entry["loaded"] == false && entry["retained_failed_prefix"] != true) {
        std::process::exit(3);
    }

    match catch_unwind(AssertUnwindSafe(|| vm.load_file(&args.harness).map(|_| ()))) {
        Ok(Ok(_)) => emit(json!({"event": "harness", "ok": true, "file": args.harness})),
        Ok(Err(e)) => {
            emit(json!({"event": "harness", "ok": false, "file": args.harness, "error": e}));
            std::process::exit(4);
        }
        Err(_) => panic_exit("harness", "panic", None),
    }
    if args.ert_batch_adapter {
        match eval(&mut vm, "(require 'romacs-ert-batch)") {
            Ok(_) => emit(json!({"event": "ert-adapter", "name": "romacs-batch", "automatic_window_isolation": false, "condition_format": "native-prin1"})),
            Err((true, error)) => panic_exit("ert-adapter", &error, None),
            Err((false, error)) => {
                emit(json!({"event": "setup", "ok": false, "phase": "ert-adapter", "error": error}));
                std::process::exit(4);
            }
        }
    }
    if let Some((out, _)) = &world_out {
        world::finish(&mut vm, out);
    }
    if let Some(audit) = &args.audit {
        match catch_unwind(AssertUnwindSafe(|| vm.load_file(audit).map(|_| ()))) {
            Ok(Ok(_)) => emit(json!({"event": "audit", "ok": true, "file": audit})),
            Ok(Err(e)) => {
                emit(json!({"event": "audit", "ok": false, "file": audit, "error": e}));
                std::process::exit(4);
            }
            Err(_) => panic_exit("audit", "panic", None),
        }
    }
    for expr in &args.probe {
        let r = eval(&mut vm, expr);
        let shown = match r {
            Ok(v) => Ok(shown(v)),
            Err((_, e)) => Err(e),
        };
        emit(
            json!({"event": "probe", "expr": expr, "value": shown.as_ref().ok(), "error": shown.as_ref().err()}),
        );
    }

    if args.kind == HarnessKind::Real {
        run_real(&mut vm, &args, &file);
    }

    // GNU `load` binds these while the file's forms run (lread.c; the
    // load history's `current-load-list' starts as (FILE), which
    // `macroexp-file-name' and so `ert-resource-directory' read); ERT runs
    // the tests later, after `load` has returned and unbound them.
    let bind = format!(
        "(setq load-file-name {0} load-true-file-name {0} current-load-list (list {0}) load-in-progress t)",
        lisp_string(&file)
    );
    if let Err((_, e)) = eval(&mut vm, &bind) {
        emit(json!({"event": "setup", "ok": false, "phase": "load-file-name", "error": e}));
    }
    for form in &forms {
        let ordinal = form["ordinal"].clone();
        let Some(source) = form["source"].as_str() else {
            emit(
                json!({"event": "form", "ordinal": ordinal, "line": form["line"], "ok": false,
                        "error": format!("not extractable: {}", form["unextractable"])}),
            );
            continue;
        };
        let result = eval(&mut vm, source).map(|_| ());
        let registered = integer_of(&mut vm, "(romacs-gnu-tests--count)");
        match result {
            Ok(_) => emit(
                json!({"event": "form", "ordinal": ordinal, "line": form["line"], "ok": true,
                                 "registered": registered.ok()}),
            ),
            Err((true, e)) => panic_exit("form", &e, None),
            Err((false, e)) => emit(
                json!({"event": "form", "ordinal": ordinal, "line": form["line"],
                                           "ok": false, "error": e.chars().take(2000).collect::<String>(),
                                           "registered": registered.ok()}),
            ),
        }
    }
    if let Err((_, e)) = eval(&mut vm, "(setq load-file-name nil load-true-file-name nil current-load-list nil load-in-progress nil)") {
        emit(json!({"event": "setup", "ok": false, "phase": "unbind-load-file-name", "error": e}));
    }

    let count = match integer_of(&mut vm, "(romacs-gnu-tests--count)") {
        Ok(n) => n,
        Err(e) => {
            emit(json!({"event": "setup", "ok": false, "phase": "count", "error": e}));
            std::process::exit(4);
        }
    };
    for i in 0..count {
        let name = match string_of(&mut vm, &format!("(romacs-gnu-tests--name {i})")) {
            Ok(n) => n,
            Err(e) => {
                emit(
                    json!({"event": "setup", "ok": false, "phase": "name", "index": i, "error": e}),
                );
                continue;
            }
        };
        if args.skip.contains(&name) {
            continue;
        }
        emit(json!({"event": "start", "test": name}));
        if args.audit.is_some() {
            // Whatever top-level forms caught belongs to no test.
            let _ = string_of(&mut vm, "(romacs-gnu-tests--audit-take)");
        }
        let outcome = match eval(&mut vm, &format!("(romacs-gnu-tests--run {i})")) {
            Err((true, e)) => panic_exit("test", &e, Some(&name)),
            Err((false, e)) => {
                json!({"outcome": "vm-fault", "detail": e.chars().take(2000).collect::<String>()})
            }
            Ok(_) => {
                let kind = string_of(&mut vm, "(symbol-name (car romacs-gnu-tests--last))");
                let detail = eval(&mut vm, "(cdr romacs-gnu-tests--last)")
                    .map(|v| shown(v))
                    .unwrap_or_else(|(_, e)| format!("#<detail error: {e}>"));
                match kind {
                    Ok(k) => {
                        json!({"outcome": k, "detail": detail.chars().take(2000).collect::<String>()})
                    }
                    Err(e) => json!({"outcome": "harness-error", "detail": e}),
                }
            }
        };
        let expected = string_of(&mut vm, &format!("(romacs-gnu-tests--expected {i})"))
            .unwrap_or_else(|e| format!("#<error: {e}>"));
        // Absent: no audit was asked for, or it could not be read. run.py
        // then counts no pass for this test (fail-closed).
        let caught = args.audit.as_ref().and_then(|_| {
            string_of(&mut vm, "(romacs-gnu-tests--audit-take)")
                .ok()
                .map(|lines| {
                    lines
                        .lines()
                        .map(|l| {
                            let (condition, datum) = l.split_once(' ').unwrap_or((l, ""));
                            json!([condition, datum])
                        })
                        .collect::<Vec<_>>()
                })
        });
        let mut result = json!({"event": "result", "test": name, "expected": expected,
                                "outcome": outcome["outcome"], "detail": outcome["detail"]});
        if let Some(caught) = caught {
            result["caught"] = Json::Array(caught);
        }
        emit(result);
    }
    emit(json!({"event": "done", "registered": count}));
}
