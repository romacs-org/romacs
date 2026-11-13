use romacs_vm::Vm;
#[path = "support/own_test.rs"]
mod own_test;

fn check(source: &str, expected: &str, marker: &str) {
    let mut vm = Vm::new();
    let got = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(got, Ok(expected.to_owned()), "{marker}: GNU expected {expected}, VM {got:?}");
}

#[test]
fn native_stack_probe() {
    let limit = std::env::var("ROMACS_DEPTH_PROBE_LIMIT").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(4200);
    let calls = std::env::var("ROMACS_DEPTH_PROBE_CALLS").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(limit + 20);
    let stack = std::env::var("ROMACS_DEPTH_PROBE_STACK").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(512 * 1024);
    std::thread::Builder::new().stack_size(stack).spawn(move || {
        let expected = if calls * 2 + 2 > limit { format!("(excessive-lisp-nesting {})", limit + 1) } else { "survived".to_owned() };
        check(&format!("(defun depth-native (n) (if (= n 0) 'survived (eval (list 'depth-native (1- n)))))
            (let ((max-lisp-eval-depth {limit})) (condition-case e (depth-native {calls}) (excessive-lisp-nesting e)))"),
            &expected, "DEPTH_NATIVE_SIGNAL");
        println!("DEPTH_NATIVE_OK native_stack_bytes={stack} limit={limit} calls={calls} result={expected}");
    }).unwrap().join().expect("DEPTH_NATIVE_CHILD");
}

#[test]
fn signal_hook_borrows_reserve_and_restores_on_throw() {
    check("(defvar depth-hook-seen nil)
        (defun depth-hook (symbol data)
          (setq depth-hook-seen (vector max-lisp-eval-depth lisp-eval-depth-reserve))
          (setq max-lisp-eval-depth (+ max-lisp-eval-depth 5))
          (throw 'depth-hook-exit 'hook-threw))
        (defun depth-recur (n) (if (= n 0) 'ok (depth-recur (1- n))))
        (let ((max-lisp-eval-depth 100) (lisp-eval-depth-reserve 200)
              (signal-hook-function #'depth-hook))
          (vector (catch 'depth-hook-exit (depth-recur 200))
                  depth-hook-seen max-lisp-eval-depth lisp-eval-depth-reserve))",
        "[hook-threw [121 179] 100 205]", "DEPTH_RESERVE_RUNTIME");
}

#[test]
fn growing_registers_keep_captures_rest_apply_and_reentry_intact() {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(|| {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let got = vm.eval_source("(let ((f nil) (payload (list 42)))
            (setq f (lambda (n &optional wanted &rest tail)
              (if (= n 0) (vector (eq wanted payload) tail
                           (eval '(let ((a 1) (b 2) (c 3) (d 4) (e 5)) (+ a b c d e))))
                (apply f (list (1- n) wanted 'kept)))))
            (let ((max-lisp-eval-depth 4200)) (funcall f 1200 payload 'kept)))").and_then(|v| v.display());
        assert_eq!(got, Ok("[t (kept) 15]".into()), "DEPTH_GROW_ROOTS: {got:?}");
        assert!(vm.heap_stats().collections > 0, "DEPTH_GROW_COLLECTED");
    }).unwrap().join().unwrap();
}

#[test]
fn growing_t2_frames_preserve_hook_callbacks() {
    let small = std::env::var_os("ROMACS_DEPTH_T2_SMALL").is_some();
    let stack = if small { 512 * 1024 } else { 64 * 1024 * 1024 };
    std::thread::Builder::new().stack_size(stack).spawn(move || {
        let mut vm = Vm::new();
        let got = vm.eval_source("(defvar depth-h nil)
            (defvar depth-native-entered 0)
            (defun depth-hook-n (n) (setq depth-native-entered (1+ depth-native-entered)) (if (= n 0) 'finished
              (let ((depth-h (list (lambda () (depth-hook-n (1- n))))))
                (run-hooks 'depth-h) 'finished)))
            (let ((max-lisp-eval-depth 4200)) (condition-case e (depth-hook-n 700) (error (car (cdr e)))))").and_then(|v| v.display());
        if small {
            assert!(got == Ok("finished".into()) || got == Ok("\"native stack exhausted during Lisp evaluation\"".into()), "DEPTH_T2_GAP: {got:?}");
        } else {
            assert_eq!(got, Ok("finished".into()), "DEPTH_GROW_T2: {got:?}");
        }
        let entered = vm.eval_source("depth-native-entered").unwrap().display().unwrap();
        println!("DEPTH_T2_RESULT stack={stack} limit=4200 entered={entered} result={got:?}");
    }).unwrap().join().unwrap();
}

#[test]
fn signal_hook_runs_once_across_eval_and_unwind_cleanup() {
    check("(defvar depth-hook-count 0)
      (defun depth-hook-counted (symbol data) (setq depth-hook-count (1+ depth-hook-count)))
      (let ((signal-hook-function #'depth-hook-counted))
        (condition-case e (unwind-protect (eval '(signal 'arith-error '(17))) (+ 1 2))
          (arith-error (vector e depth-hook-count))))",
      "[(arith-error 17) 1]", "DEPTH_HOOK_ONCE");
}

/// A loop over the threaded buffer fast paths (point, goto-char,
/// char-after, point-min, point-max, buffer-size, and a quiet insert) must
/// not grow the native stack per iteration: each handler continues by a
/// tail call.
#[test]
fn buffer_read_handlers_run_in_constant_native_stack() {
    std::thread::Builder::new().stack_size(2 * 1024 * 1024).spawn(|| {
        check("(with-temp-buffer (insert \"abcdef\") (let ((n 0)) (while (< n 200000) (goto-char (1+ (% n 6))) (setq n (+ n (point) (- (point-min)) (- (point-max) (buffer-size)) (if (char-after) 0 0) (if (char-after 2) 0 0)))) n))",
            "200001", "BUFFER_READ_TAIL_CALL");
        check("(with-temp-buffer (let ((n 0)) (while (< n 200000) (insert \"a\") (setq n (1+ n))) (buffer-size)))",
            "200000", "INSERT_STRING_TAIL_CALL");
    }).unwrap().join().expect("BUFFER_READ_TAIL_CALL_CHILD");
}

/// `eval', `load' and the other primitives that run Lisp with the caller
/// suspended return to their loop by a tail call too: a loop of evals, and
/// eval-buffer's readevalloop, which evals once per top-level form, run in
/// constant native stack whatever the count.
#[test]
fn eval_in_a_loop_runs_in_constant_native_stack() {
    std::thread::Builder::new().stack_size(2 * 1024 * 1024).spawn(|| {
        check("(let ((n 0)) (while (< n 200000) (setq n (+ n (eval '(+ 0 1) t)))) n)", "200000", "EVAL_LOOP_TAIL_CALL");
    }).unwrap().join().expect("EVAL_LOOP_TAIL_CALL_CHILD");
}

/// A generated file of 20,000 top-level defuns, each with a body worth
/// expanding, loads and evaluates as a buffer on the evaluator's stack. (A
/// buffer-local `lexical-binding' names the dialect, as eval-buffer allows,
/// since the prelude alone does not parse the cookie.)
#[test]
fn a_file_of_20000_forms_loads_and_evaluates_as_a_buffer() {
    let dir = std::env::temp_dir().join(format!("romacs-forms-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("forms.el");
    let forms: String = (0..20_000).map(|i| format!(
        "(defun gen-f{i} (x) (let ((a x) (b {i})) (setq a (if (> a 0) (* b 2) (- b 1))) (list a \"s{i}\" '(q . {i}))))\n")).collect();
    std::fs::write(&file, format!(";;; -*- lexical-binding: t -*-\n{forms}")).unwrap();
    let file = file.display().to_string();
    std::thread::Builder::new().stack_size(8 * 1024 * 1024).spawn(move || {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let mut eval = |source: String| vm.eval_source(&source).and_then(|v| v.display());
        assert_eq!(eval(format!(r#"(progn (load "{file}" nil t) (list (gen-f19999 1) (fboundp 'gen-f0)))"#)),
            Ok("((39998 \"s19999\" (q . 19999)) t)".into()), "LOAD_20000_FORMS");
        assert_eq!(eval(format!(r#"(progn (fmakunbound 'gen-f19999) (with-temp-buffer (set (make-local-variable (quote lexical-binding)) t) (insert-file-contents "{file}") (eval-buffer)) (gen-f19999 0))"#)),
            Ok("(19998 \"s19999\" (q . 19999))".into()), "EVAL_BUFFER_20000_FORMS");
    }).unwrap().join().expect("FORMS_20000_CHILD");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn t2_native_dispatch_is_bounded_without_lto() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "growing_t2_frames_preserve_hook_callbacks"), "--nocapture"])
        .env("ROMACS_DEPTH_T2_SMALL", "1").output().unwrap();
    print!("{}", String::from_utf8_lossy(&out.stdout));
    assert!(out.status.code() == Some(0), "DEPTH_T2_NATIVE_PROCESS: expected normal result or catchable native guard, numeric_code={:?}: {} {}", out.status.code(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

#[test]
fn native_limit_is_a_lisp_signal_not_process_abort() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "native_stack_probe"), "--nocapture"])
        .env("ROMACS_DEPTH_PROBE_LIMIT", "4200")
        .env("ROMACS_DEPTH_PROBE_CALLS", "4220")
        .env("ROMACS_DEPTH_PROBE_STACK", "524288").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && stdout.contains("result=(excessive-lisp-nesting 4201)"),
        "DEPTH_NATIVE_PROCESS: GNU expected clean excessive-lisp-nesting, child {:?}: {} {}",
        out.status, stdout, String::from_utf8_lossy(&out.stderr));
}

fn native_probe_child(limit: usize, calls: usize, stack: usize) -> (Option<i32>, String, String) {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "native_stack_probe"), "--nocapture"])
        .env("ROMACS_DEPTH_PROBE_LIMIT", limit.to_string())
        .env("ROMACS_DEPTH_PROBE_CALLS", calls.to_string())
        .env("ROMACS_DEPTH_PROBE_STACK", stack.to_string()).output().unwrap();
    (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// GNU loadup.el:106 raises the limit to 4200 and :156 doubles it. Each
/// probe level is a call plus an `eval` reentry (two depth units), on a
/// 512 KiB native stack: both raised limits must be reachable, and the
/// first level past them must be GNU's signal, never a process abort.
#[test]
fn gnu_loadup_limits_are_sustained_and_then_signal() {
    for (limit, calls, expected) in [
        (4200, 2099, "result=survived".to_owned()),
        (8400, 4199, "result=survived".to_owned()),
        (8400, 4220, "result=(excessive-lisp-nesting 8401)".to_owned()),
    ] {
        let (code, stdout, stderr) = native_probe_child(limit, calls, 512 * 1024);
        assert!(code == Some(0) && stdout.contains(&expected),
            "DEPTH_LOADUP_LIMIT: limit={limit} calls={calls} expected {expected}, numeric_code={code:?}: {stdout} {stderr}");
    }
}

#[test]
fn compiler_moderate_nesting_compiles_in_child() {
    // Compiler recursion is refused on remaining bytes (see
    // compiler_remaining_stack_guard_is_recoverable), not grown.
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "compiler_native_stack_worker"), "--nocapture"])
        .env("ROMACS_DEPTH_COMPILER_CHILD", "1").output().unwrap();
    assert!(out.status.success(), "DEPTH_COMPILER_STACK: GNU expected normal compilation, child {:?}: {}", out.status, String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("DEPTH_COMPILER_WORKER_OK"), "DEPTH_COMPILER_RAN");
}

#[test]
fn compiler_native_stack_worker() {
    if std::env::var_os("ROMACS_DEPTH_COMPILER_CHILD").is_none() { return; }
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(|| {
        let source = format!("{}42{}", "(if t ".repeat(100), " nil)".repeat(100));
        check(&source, "42", "DEPTH_COMPILER_VALUE");
        println!("DEPTH_COMPILER_WORKER_OK");
    }).unwrap().join().unwrap();
}

#[test]
fn signal_data_is_not_internal_depth_state() {
    check("(defvar depth-hook-seen nil)
      (defun depth-hook-forged (symbol data)
        (setq depth-hook-seen (vector max-lisp-eval-depth lisp-eval-depth-reserve))
        (throw 'done depth-hook-seen))
      (let ((max-lisp-eval-depth 100) (lisp-eval-depth-reserve 200)
            (signal-hook-function #'depth-hook-forged))
        (catch 'done (signal 'excessive-lisp-nesting '(50000))))",
      "[100 200]", "DEPTH_SIGNAL_DATA");
}

#[inline(never)]
fn compile_with_low_native_headroom(vm: &mut Vm) -> Result<usize, String> {
    let mut ballast = [0u8; 16 * 1024];
    std::hint::black_box(&mut ballast);
    // Recurse until less than the VM's 128 KiB native headroom remains.
    let result = if stacker::remaining_stack().expect("known native stack") > 96 * 1024 {
        compile_with_low_native_headroom(vm)
    } else {
        vm.compile_source("42")
    };
    std::hint::black_box(&ballast);
    result
}

#[test]
fn compiler_remaining_stack_guard_is_recoverable() {
    if std::env::var_os("ROMACS_DEPTH_NATIVE_GAP_CHILD").is_some() {
        std::thread::Builder::new().stack_size(2 * 1024 * 1024).spawn(|| {
            let mut vm = Vm::new();
            let got = compile_with_low_native_headroom(&mut vm);
            assert_eq!(got, Err("native stack exhausted while compiling Lisp".into()), "DEPTH_NATIVE_GAP: {got:?}");
            assert_eq!(vm.eval_source("42").unwrap().display().unwrap(), "42", "DEPTH_NATIVE_GAP_REUSE");
            println!("DEPTH_NATIVE_GAP_OK stack_bytes=2097152");
        }).unwrap().join().unwrap();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "compiler_remaining_stack_guard_is_recoverable"), "--nocapture"])
        .env("ROMACS_DEPTH_NATIVE_GAP_CHILD", "1").output().unwrap();
    assert!(out.status.code() == Some(0) && String::from_utf8_lossy(&out.stdout).contains("DEPTH_NATIVE_GAP_OK"),
      "DEPTH_NATIVE_GAP_PROCESS: code {:?}: {} {}", out.status.code(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

#[test]
fn gnu_real_compiler_fixtures_survive_on_two_mib() {
    if std::env::var_os("ROMACS_DEPTH_GNU_FIXTURE_CHILD").is_some() {
        std::thread::Builder::new().stack_size(2 * 1024 * 1024).spawn(|| {
            for (origin, source, line, prefix) in [
                ("comp-common.el:71", include_str!("../../../lisp/gnu/emacs-lisp/comp-common.el"), 71, "(defconst comp-primitive-type-specifiers"),
                ("cus-start.el:44", include_str!("../../../lisp/gnu/cus-start.el"), 44, "(let* ((quoter"),
            ] {
                let offset: usize = source.split_inclusive('\n').take(line - 1).map(str::len).sum();
                let tail = &source[offset..];
                assert!(tail.starts_with(prefix), "DEPTH_GNU_FIXTURE_ID: {origin}");
                let mut vm = Vm::new();
                let end = vm.read_source(tail).unwrap().unwrap().1;
                let got = vm.compile_source(&tail[..end]);
                assert!(got.is_ok() || got == Err("native stack exhausted while compiling Lisp".into()),
                    "DEPTH_GNU_FIXTURE_RESULT: {origin}: {got:?}");
                println!("DEPTH_GNU_FIXTURE_OK {origin} stack_bytes=2097152 result={got:?}");
                assert_eq!(vm.eval_source("42").unwrap().display().unwrap(), "42", "DEPTH_GNU_FIXTURE_REUSE");
            }
        }).unwrap().join().unwrap();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "gnu_real_compiler_fixtures_survive_on_two_mib"), "--nocapture"])
        .env("ROMACS_DEPTH_GNU_FIXTURE_CHILD", "1").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.code() == Some(0) && stdout.matches("DEPTH_GNU_FIXTURE_OK").count() == 2,
        "DEPTH_GNU_FIXTURE_PROCESS: numeric_code={:?}: {} {}", out.status.code(), stdout, String::from_utf8_lossy(&out.stderr));
    print!("{stdout}");
}

#[test]
fn authoritative_limit_signals_existing_condition() {
    check("(defun depth-recur (n) (if (= n 0) 'ok (depth-recur (1- n))))
        (let ((max-lisp-eval-depth 100))
          (condition-case e (depth-recur 200) (excessive-lisp-nesting e)))",
        "(excessive-lisp-nesting 101)", "DEPTH_AUTHORITATIVE");
}

#[test]
fn dynamic_raise_restores_effective_limit() {
    check("(defun depth-recur (n) (if (= n 0) 'ok (depth-recur (1- n))))
        (let ((max-lisp-eval-depth 100))
          (vector (let ((max-lisp-eval-depth 4200)) (depth-recur 2000))
                  max-lisp-eval-depth
                  (condition-case e (depth-recur 200) (excessive-lisp-nesting e))))",
        "[ok 100 (excessive-lisp-nesting 101)]", "DEPTH_DYNAMIC_RESTORE");
}

#[test]
fn nonlocal_exit_and_cleanup_restore_depth() {
    check("(defvar depth-clean nil)
        (defun depth-recur (n) (if (= n 0) 'ok (depth-recur (1- n))))
        (defun depth-throw (n) (if (= n 0) (throw 'done 'caught) (depth-throw (1- n))))
        (let ((max-lisp-eval-depth 100))
          (vector (catch 'done (depth-throw 20))
            (condition-case e
                (unwind-protect (depth-recur 200) (setq depth-clean (depth-recur 20)))
              (excessive-lisp-nesting (depth-recur 20)))
            depth-clean (depth-recur 20)))",
        "[caught ok ok ok]", "DEPTH_NONLOCAL_RESTORE");
}

#[test]
fn nested_eval_is_not_a_new_depth_budget() {
    // The pre-fix evaluator has no reentry bound. Give the bounded RED fixture
    // enough native stack to reach its behavioural assertion, not an abort.
    std::thread::Builder::new().stack_size(128 * 1024 * 1024).spawn(|| {
        check("(defun depth-eval (n) (if (= n 0) 'escaped (eval (list 'depth-eval (1- n)))))
            (let ((max-lisp-eval-depth 100))
              (condition-case e (depth-eval 150) (excessive-lisp-nesting e)))",
            "(excessive-lisp-nesting 101)", "DEPTH_REENTRY");
    }).unwrap().join().unwrap();
}

#[test]
fn macro_expansion_depth_signals_without_native_overflow() {
    check("(defun depth-expand (n) (if (= n 0) 42 (depth-expand (1- n))))
        (defmacro depth-macro (n) (depth-expand n))
        (let ((max-lisp-eval-depth 100))
          (condition-case e (eval '(depth-macro 300)) (excessive-lisp-nesting e)))",
        "(excessive-lisp-nesting 101)", "DEPTH_MACRO_SIGNAL");
}

/// Per-level depth accounting, pinned against GNU at 45b0f7699df with
/// `max-lisp-eval-depth` 100. VM2 compiles everything, so its regime is GNU
/// BYTECODE's: GNU byte-compiled reaches 48 eval-reentry levels and 96 call
/// levels (interpreted: 30 and 45); VM2 reaches 50 and 99 because GNU's
/// batch top level holds a few frames (command-line, load, byte-code) that
/// a fresh VM request does not. An `eval` reentry costs two units in both.
#[test]
fn reentry_and_call_levels_follow_gnu_bytecode_accounting() {
    check("(defvar seen 0)
        (defun depth-eval (n) (setq seen (1+ seen)) (if (= n 0) 'escaped (eval (list 'depth-eval (1- n)))))
        (defun depth-call (n) (setq seen (1+ seen)) (if (= n 0) 'ok (depth-call (1- n))))
        (let ((max-lisp-eval-depth 100))
          (vector (condition-case e (depth-eval 150) (excessive-lisp-nesting e))
                  (prog1 seen (setq seen 0))
                  (condition-case e (depth-call 150) (excessive-lisp-nesting e))
                  seen))",
        "[(excessive-lisp-nesting 101) 50 (excessive-lisp-nesting 101) 99]", "DEPTH_LEVEL_ACCOUNTING");
}

/// A long loop has no Lisp activation, but each 10,000-instruction poll may
/// keep a native frame when LLVM does not make `poll_resume`'s dispatch a
/// sibling call. That was observed (74,600 frames, SIGABRT) in the gate-built
/// `romacs-gnu-ert` adapter, NOT in this test binary, whose codegen tail-calls
/// it: here the loop completes, so this test guards against an abort if the
/// leak reaches this binary and does not by itself prove the poll guard.
#[test]
fn long_loop_polls_never_abort_on_a_small_stack() {
    if std::env::var_os("ROMACS_DEPTH_POLL_CHILD").is_some() {
        std::thread::Builder::new().stack_size(384 * 1024).spawn(|| {
            let mut vm = Vm::new();
            let got = vm.eval_source("(condition-case e (let ((i 0)) (while (< i 6000000) (setq i (1+ i))) i) (error (car (cdr e))))")
                .and_then(|v| v.display());
            assert!(got == Ok("6000000".into()) || got == Ok("\"native stack exhausted during Lisp evaluation\"".into()),
                "DEPTH_POLL_RESULT: {got:?}");
            println!("DEPTH_POLL_OK stack_bytes=393216 result={got:?}");
        }).unwrap().join().unwrap();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "long_loop_polls_never_abort_on_a_small_stack"), "--nocapture"])
        .env("ROMACS_DEPTH_POLL_CHILD", "1").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.code() == Some(0) && stdout.contains("DEPTH_POLL_OK"),
        "DEPTH_POLL_PROCESS: numeric_code={:?}: {} {}", out.status.code(), stdout, String::from_utf8_lossy(&out.stderr));
    print!("{stdout}");
}
