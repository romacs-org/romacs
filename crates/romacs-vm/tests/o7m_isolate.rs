//! O7m C3 through the master and worker Lisp over `SOCK_SEQPACKET`.
//! The proxy forwards the worker's hello as it is (it names the build,
//! which the launcher's worker shares), then replaces the frozen payload in
//! a selected local test frame.
#[path = "support/own_test.rs"]
mod own_test;

use romacs_vm::Vm;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    process::Command,
};

const WORKER: &str = env!("CARGO_BIN_EXE_romacs-vm-isolate");
const PROXY: &str = r#"import array
import os
import select
import socket
import subprocess
import sys

args = sys.argv[1:]
if '--parent' in args:
    args[args.index('--parent') + 1] = str(os.getpid())
master = socket.socket(fileno=0)
relay, peer = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
bad_message_sources = set()
with subprocess.Popen([worker, *args], stdin=peer) as process:
    peer.close()
    while True:
        ready, _, _ = select.select([master, relay], [], [], 30)
        if not ready:
            relay.close()
            raise SystemExit("o7m proxy timed out")
        for source in ready:
            packet, control, flags, _ = source.recvmsg(8192, socket.CMSG_SPACE(4))
            if not packet:
                relay.close()
                raise SystemExit(0)
            assert not flags
            kind = packet[0]
            generation = int.from_bytes(packet[1:9], 'little')
            fds = [fd for level, ty, data in control
                   if (level, ty) == (socket.SOL_SOCKET, socket.SCM_RIGHTS)
                   for fd in array.array('i', data)]
            try:
                if generation == 2 and ((mode == 'bad-request' and source is master and kind == 2)
                                    or (mode == 'bad-result' and source is relay and kind == 3)):
                    with open(log, 'a') as evidence:
                        print(kind, generation, file=evidence)
                    packet = packet[:9] + b'\x00BAD_MAGIC'
                    control = []
                if mode == 'missing-descriptor' and source is relay and kind == 3 and generation == 2:
                    assert packet[9] & 1 and len(fds) == 1
                    with open(log, 'a') as evidence:
                        print(kind, generation, 'sealed without fd', file=evidence)
                    control = []
                if mode == 'bad-message' and kind == 5 and source not in bad_message_sources:
                    bad_message_sources.add(source)
                    with open(log, 'a') as evidence:
                        print(kind, generation, 'to-worker' if source is master else 'to-master', file=evidence)
                    packet = packet[:9] + b'\x00BAD_MAGIC'
                    control = []
                if mode == 'observe-backing' and source is master and kind == 5:
                    assert len(fds) == 1
                    stat = os.fstat(fds[0])
                    magic = os.pread(fds[0], 8, 0).decode('ascii')
                    with open(log, 'a') as evidence:
                        print(stat.st_dev, stat.st_ino, magic, file=evidence)
                (relay if source is master else master).sendmsg([packet], control)
            finally:
                for fd in fds:
                    os.close(fd)
"#;

fn worker_program() -> std::path::PathBuf {
    std::env::var_os("ROMACS_O7M_WORKER").map_or_else(|| WORKER.into(), std::path::PathBuf::from)
}

fn worker_vm(program: &Path) -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let program = serde_json::to_string(program).unwrap();
    eval(
        &mut vm,
        &format!("(setq isolate-worker-program {program} isolate-zygotes nil)"),
    );
    vm
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source)
        .and_then(romacs_vm::Local::display)
        .unwrap()
}

fn in_proxy(test: &str, mode: &str, run: impl FnOnce(&mut Vm, &Path)) {
    const MARKER: &str = "ROMACS_O7M_PROXY_CHILD";
    if let Some(dir) = std::env::var_os(MARKER) {
        let dir = Path::new(&dir);
        let launch = dir.join("launch");
        let log = dir.join("frames");
        let worker_literal = serde_json::to_string(&worker_program()).unwrap();
        let log_literal = serde_json::to_string(&log).unwrap();
        let script = format!(
            "#!/usr/bin/python3\nworker = {worker_literal}\nmode = {mode:?}\nlog = {log_literal}\n{PROXY}"
        );
        fs::write(&launch, script).unwrap();
        fs::set_permissions(&launch, fs::Permissions::from_mode(0o700)).unwrap();
        let mut vm = worker_vm(&launch);
        run(&mut vm, &log);
        return;
    }
    let marker = std::env::temp_dir().join("romacs-o7m-isolate-XXXXXX");
    let (dir, file) = romacs_vm::host::temp_file(&marker).unwrap();
    drop(file);
    fs::remove_file(&dir).unwrap();
    fs::create_dir(&dir).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &own_test::name(module_path!(), test),
            "--nocapture",
        ])
        .env(MARKER, &dir)
        .output()
        .unwrap();
    if let Ok(log) = fs::read_to_string(dir.join("frames")) {
        eprint!("local proxy observed: {log}");
    }
    fs::remove_dir_all(dir).unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("running 1 test"), "{stdout}");
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn check_bad_frame(vm: &mut Vm, log: &Path, kind: u8) {
    let actual = vm.eval_source(r"
        (let ((iso (isolate-create)))
          (unwind-protect
              (let* ((bad (isolate-spawn '(lambda () (setq o7m-ran-bad t)) :isolate iso))
                     (good (isolate-spawn '(lambda () (list (+ 19 23) (boundp 'o7m-ran-bad))) :isolate iso))
                     (error-kind (condition-case err (isolate-await bad 5)
                                   (error (car err)))))
                (list (aref bad 2) error-kind (isolate-future-done-p bad)
                      (isolate-await good 5) (isolate-live-p iso)))
            (isolate-kill iso)))").and_then(romacs_vm::Local::display);
    let injected = fs::read_to_string(log)
        .unwrap_or_else(|error| panic!("proxy did not inject: {error}; VM result: {actual:?}"));
    assert_eq!(injected, format!("{kind} 2\n"));
    let original_job_ran = if kind == 2 { "nil" } else { "t" };
    assert_eq!(
        actual.unwrap(),
        format!("(2 isolate-frame-error t (42 {original_job_ran}) t)")
    );
}

// R4. A bad result settles its own Lisp future and the queued result still arrives.
#[test]
fn malformed_result_settles_future() {
    in_proxy(
        "malformed_result_settles_future",
        "bad-result",
        |vm, log| check_bad_frame(vm, log, 3),
    );
}

// R4. The real worker must reply under the bad request's generation and
// continue, without evaluating the original request that the proxy replaced.
#[test]
fn malformed_request_replied_next_frame_served() {
    in_proxy(
        "malformed_request_replied_next_frame_served",
        "bad-request",
        |vm, log| check_bad_frame(vm, log, 2),
    );
}

// R3. Ordinary job args and bindings still become writable copies.
#[test]
fn plain_job_args_still_thawed() {
    let mut vm = worker_vm(&worker_program());
    let actual = eval(
        &mut vm,
        r"
        (let ((iso (isolate-create))
              (arg (list 'unchanged))
              (binding (vector 'untouched)))
          (unwind-protect
              (list (isolate-await
                     (isolate-spawn '(lambda (data)
                                       (list (frozen-p data) (frozen-p o7m-binding)
                                             (setcar data 'changed)
                                             (aset o7m-binding 0 'changed)))
                                    arg :bindings (list (cons 'o7m-binding binding))
                                    :isolate iso) 5)
                    arg binding)
            (isolate-kill iso)))",
    );
    assert_eq!(
        actual,
        "((nil nil changed changed) (unchanged) [untouched])"
    );
}

fn sealed_backings() -> std::collections::BTreeSet<(u64, u64)> {
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            let name = fs::read_link(&path).ok()?;
            if !name.to_str()?.starts_with("/memfd:romacs-frozen") {
                return None;
            }
            let stat = fs::metadata(path).ok()?;
            Some((stat.dev(), stat.ino()))
        })
        .collect()
}

const TABLE: &str = "'(:incarnation (123 . 1) :world \"o7m-test-world\" :files nil :fbound nil :autoloads nil :features nil)";

fn backing(vm: &mut Vm) {
    eval(
        vm,
        r"(setq o7m-root (let ((s (make-string 8192 ?x)))
                                (lambda () (cons (length s) 'code))))",
    );
    let form = format!(
        "(setq o7m-backing (romacs--seed-build '((variable o7m-root)) {TABLE} 100000)) (cdr o7m-backing)"
    );
    assert_eq!(eval(vm, &form), "nil");
}

// R4. The same sealed VM2FRZ02 backing goes down the wire on both sends.
// Comparing the FD identity exposes an accidental decode/encode even if
// both encoders produce byte-identical output.
#[test]
fn backing_encoded_once() {
    in_proxy("backing_encoded_once", "observe-backing", |vm, log| {
        let before = sealed_backings();
        backing(vm);
        let after = sealed_backings();
        let new: Vec<_> = after.difference(&before).collect();
        let [(dev, inode)] = new.as_slice() else {
            panic!("expected one seal: {new:?}")
        };
        let actual = eval(
            vm,
            r"
          (let ((iso (isolate-create)))
            (unwind-protect
                (let ((probe '(lambda ()
                                (let* ((seed (isolate-receive 5))
                                       (entry (assq 'o7m-root (mapcar 'cdr (nth 2 seed)))))
                                  (list (frozen-p seed) (funcall (nth 1 entry)))))))
                  (isolate-send iso (car o7m-backing))
                  (isolate-send iso (car o7m-backing))
                  (list (isolate-await (isolate-spawn probe :isolate iso) 5)
                        (isolate-await (isolate-spawn probe :isolate iso) 5)))
              (isolate-kill iso)))",
        );
        assert_eq!(actual, "((nil (8192 . code)) (nil (8192 . code)))");
        assert_eq!(
            fs::read_to_string(log).unwrap(),
            format!("{dev} {inode} VM2FRZ02\n").repeat(2)
        );
    });
}

// S1. A received code graph survives a collection while held, then releases
// its private literals and request-owned function/cache slots when dropped.
#[test]
fn collection_after_drop() {
    let mut vm = worker_vm(&worker_program());
    eval(
        &mut vm,
        "(defun o7m-target (x) x) (setq o7m-iso (isolate-create)) nil",
    );
    let transfer = format!(
        r#"
      (isolate-await
       (isolate-spawn
        '(lambda ()
           (let ((built (progn (defun o7m-target (x) x) (setq o7m-root (let ((token (list 'live)))
                                              (lambda () (o7m-target token))))
                               (romacs--seed-build '((variable o7m-root)) {TABLE} 100000))))
             (unless (null (cdr built)) (error "seed refused" (cdr built)))
             (isolate-send 'parent (car built))
             t))
        :isolate o7m-iso)
       5)"#
    );
    eval(
        &mut vm,
        r"
       (setq o7m-weak (make-hash-table :test 'eq :weakness 'key)
             isolate-message-functions
             (list (lambda (_sender seed)
                     (setq o7m-received (nth 1 (assq 'o7m-root (mapcar 'cdr (nth 2 seed)))))
                     (puthash o7m-received t o7m-weak))))",
    );
    // Cache the exact submitted forms before counting their request code.
    vm.collect_compacting();
    receive_and_drop(&mut vm, &transfer);
    let before_code = request_code_counts(&mut vm);
    let held = receive_and_drop(&mut vm, &transfer);
    assert!(
        held.0 > before_code.0 && held.1 > before_code.1,
        "{before_code:?} -> {held:?}"
    );
    assert_eq!(request_code_counts(&mut vm), before_code);
    eval(&mut vm, "(isolate-kill o7m-iso)");
}

fn receive_and_drop(vm: &mut Vm, transfer: &str) -> (i64, i64) {
    assert_eq!(eval(vm, transfer), "t");
    vm.collect_compacting();
    assert_eq!(eval(vm, "(funcall o7m-received)"), "(live)");
    assert_eq!(eval(vm, "(hash-table-count o7m-weak)"), "1");
    let held = request_code_counts(vm);
    eval(vm, "(setq o7m-received nil)");
    vm.collect_compacting();
    assert_eq!(eval(vm, "(hash-table-count o7m-weak)"), "0");
    held
}

fn request_code_counts(vm: &mut Vm) -> (i64, i64) {
    let counts = vm.eval_source("(romacs--request-code-counts)").unwrap();
    let (functions, tail) = counts.cons_pair().unwrap();
    let (caches, end) = tail.cons_pair().unwrap();
    assert!(end.is_nil());
    (
        functions.as_integer().unwrap(),
        caches.as_integer().unwrap(),
    )
}

fn hold_seed(vm: &mut Vm, root: &str) -> String {
    eval(
        vm,
        &format!(
            "(setq o7m-held (romacs--seed-build '((variable {root})) {TABLE} 100000)) (cdr o7m-held)"
        ),
    )
}

fn send_held_seed(vm: &mut Vm, root: &str, probe: &str) -> String {
    eval(
        vm,
        &format!(
            r"(let ((iso (isolate-create)))
                (unwind-protect
                    (progn
                      (isolate-send iso (car o7m-held))
                      (setq o7m-held nil)
                      (isolate-await
                       (isolate-spawn
                        '(lambda ()
                           (let ((seed (isolate-receive 2)))
                             (if seed
                                 (let* ((entry (assq '{root} (mapcar 'cdr (nth 2 seed))))
                                        (value (nth 1 entry)))
                                   {probe})
                               'seed-missing)))
                        :isolate iso)
                       5))
                  (isolate-kill iso)))"
        ),
    )
}

// Sol S1 / Opus B1. The backing remains rooted across ordinary and compacting GC.
#[test]
fn review_held_seed_survives_collection_and_send() {
    for (length, compacting) in [(12, false), (8192, true)] {
        let mut vm = worker_vm(&worker_program());
        eval(
            &mut vm,
            &format!("(setq o7m-root (vector 'held (make-string {length} ?x))) nil"),
        );
        assert_eq!(hold_seed(&mut vm, "o7m-root"), "nil");
        if compacting {
            vm.collect_compacting();
        } else {
            vm.eval_source("(garbage-collect)").unwrap();
        }
        assert_eq!(
            send_held_seed(
                &mut vm,
                "o7m-root",
                &format!(
                    "(list (aref value 0) (string-equal (aref value 1) (make-string {length} ?x)))"
                )
            ),
            "(held t)"
        );
    }
}

// Sol S2. Ordinals refer to the graph tables, not the size of their record.
// Independent transfers ensure each index reader is exercised.
#[test]
fn review_graph_indices_can_exceed_record_size() {
    let mut vm = worker_vm(&worker_program());
    eval(
        &mut vm,
        r#"
        (setq o7m-functions (make-vector 42 nil) o7m-names (make-vector 12 nil))
        (dotimes (i 42) (aset o7m-functions i (eval (list 'lambda nil i) t)))
        (dotimes (i 12) (aset o7m-names i (make-symbol (format "o7m-g%d" i))))"#,
    );
    assert_eq!(hold_seed(&mut vm, "o7m-functions"), "nil");
    let functions = send_held_seed(
        &mut vm,
        "o7m-functions",
        r"(let ((valid (= (length value) 42)))
             (dotimes (i 42) (unless (= (funcall (aref value i)) i) (setq valid nil)))
             valid)",
    );
    assert_eq!(hold_seed(&mut vm, "o7m-names"), "nil");
    let names = send_held_seed(
        &mut vm,
        "o7m-names",
        r#"(let ((valid (= (length value) 12)))
              (dotimes (i 12)
                (let ((symbol (aref value i)))
                  (unless (and (equal (symbol-name symbol) (format "o7m-g%d" i))
                               (not (eq symbol (intern (symbol-name symbol)))))
                    (setq valid nil))))
              valid)"#,
    );
    assert_eq!((functions.as_str(), names.as_str()), ("t", "t"));
}

// Sol S5. A variable root names an alias but reads the base's default cell.
#[test]
fn review_alias_seed_carries_base_default() {
    let mut vm = worker_vm(&worker_program());
    assert_eq!(
        eval(
            &mut vm,
            r"(defvar o7m-base 73)
               (defvaralias 'o7m-alias 'o7m-base)
               (make-local-variable 'o7m-base)
               (setq o7m-base 99)
               (list (default-value 'o7m-alias) o7m-alias)"
        ),
        "(73 99)"
    );
    assert_eq!(
        hold_seed(&mut vm, "o7m-alias"),
        "nil",
        "seed refused an alias with a bound default"
    );
    assert_eq!(send_held_seed(&mut vm, "o7m-alias", "value"), "73");
}

// Sol S4. The original result header survives loss of its ancillary FD.
#[test]
fn review_sealed_result_without_fd_settles_future() {
    in_proxy(
        "review_sealed_result_without_fd_settles_future",
        "missing-descriptor",
        |vm, log| {
            let actual = eval(
                vm,
                r"(let ((iso (isolate-create)))
                     (unwind-protect
                         (let* ((bad (isolate-spawn
                                      '(lambda () (make-string 8192 ?x)) :isolate iso))
                                (reason (condition-case err (isolate-await bad 5)
                                          (error (car err))))
                                (settled (isolate-future-done-p bad))
                                (good (isolate-await (isolate-spawn '+ 19 23 :isolate iso) 5)))
                           (list reason settled good))
                       (isolate-kill iso)))",
            );
            assert_eq!(fs::read_to_string(log).unwrap(), "3 2 sealed without fd\n");
            assert_eq!(actual, "(isolate-frame-error t 42)");
        },
    );
}

// Opus S2. Reports reach both public receive paths, ahead of a valid message.
#[test]
fn review_malformed_message_reported_in_both_directions() {
    in_proxy(
        "review_malformed_message_reported_in_both_directions",
        "bad-message",
        |vm, log| {
            let actual = eval(
                vm,
                r"(let* ((iso (isolate-create))
                         (seen nil)
                        (isolate-message-functions
                         (list (lambda (_sender frame)
                                 (push (if (consp frame)
                                           (list (car frame) (nth 1 frame) (car (nth 2 frame)))
                                         frame)
                                       seen)))))
                     (unwind-protect
                         (progn
                           (isolate-send iso 'unreadable)
                           (isolate-send iso 'good)
                           (let ((worker
                                  (isolate-await
                                   (isolate-spawn
                                    '(lambda ()
                                       (let* ((frame (isolate-receive 2))
                                              (next (isolate-receive 0)))
                                         (isolate-send 'parent 'unreadable)
                                         (isolate-send 'parent 'good)
                                         (list (if (consp frame)
                                                   (list (car frame) (nth 1 frame) (car (nth 2 frame)))
                                                 frame)
                                               next)))
                                    :isolate iso)
                                   5)))
                             (list (nreverse seen) worker)))
                       (isolate-kill iso)))",
            );
            assert_eq!(
                fs::read_to_string(log).unwrap(),
                "5 0 to-worker\n5 0 to-master\n"
            );
            assert_eq!(
                actual,
                "(((frame-error 0 message) good) ((frame-error 0 message) good))"
            );
        },
    );
}

// Sol S6 / Opus R2-S1. Both explicit root kinds carry a single function and
// plist pair. Either half's refusal drops the whole definition.
#[test]
fn review_definition_roots_carry_atomic_pairs() {
    let mut vm = worker_vm(&worker_program());
    eval(
        &mut vm,
        "(defun o7m-helper () 41)
         (put 'o7m-helper 'o7m-prop 'kept)
         (setq o7m-caller (lambda () (1+ (o7m-helper)))) nil",
    );
    let cases = [
        ("((function o7m-helper))", "(t 1 1 kept 41)"),
        ("((plist o7m-helper))", "(t 1 1 kept 41)"),
        (
            "((plist o7m-helper) (variable o7m-caller))",
            "(t 1 1 kept 42)",
        ),
        (
            "((variable o7m-caller) (plist o7m-helper))",
            "(t 1 1 kept 42)",
        ),
    ];
    let outcomes: Vec<_> = cases
        .iter()
        .map(|(roots, _)| seed_definition_case(&mut vm, roots))
        .collect();

    eval(&mut vm, "(put 'o7m-helper 'o7m-prop (current-buffer)) nil");
    let bad_plist = seed_definition_case(&mut vm, "((function o7m-helper))");
    eval(
        &mut vm,
        "(put 'o7m-helper 'o7m-prop 'kept)
         (fset 'o7m-helper (let ((buffer (current-buffer))) (lambda () buffer))) nil",
    );
    let bad_function = seed_definition_case(&mut vm, "((plist o7m-helper))");

    let expected: Vec<_> = cases
        .iter()
        .map(|(_, result)| ("nil".to_owned(), (*result).to_owned()))
        .collect();
    assert_eq!(
        outcomes, expected,
        "function/plist roots and both caller orders"
    );
    for (refusal, installed) in [bad_plist, bad_function] {
        assert!(refusal.contains("cannot-freeze"), "{refusal}");
        assert_eq!(installed, "(t 0 0 nil void-function)", "partial pair");
    }
}

fn seed_definition_case(vm: &mut Vm, roots: &str) -> (String, String) {
    let refusal = eval(
        vm,
        &format!("(setq o7m-held (romacs--seed-build '{roots} {TABLE} 100000)) (cdr o7m-held)"),
    );
    // The codec pin predates owner startup; install exactly the received
    // function cell and plist in a fresh worker, then run the received caller.
    let installed = send_held_seed(
        vm,
        "o7m-caller",
        r"(let ((fresh (not (fboundp 'o7m-helper))) (functions 0) (plists 0))
             (dolist (item (nth 2 seed))
               (when (eq (nth 1 item) 'o7m-helper)
                 (cond ((eq (car item) 'function)
                        (fset 'o7m-helper (nth 2 item))
                        (setq functions (1+ functions)))
                       ((eq (car item) 'plist)
                        (setplist 'o7m-helper (nth 2 item))
                        (setq plists (1+ plists))))))
             (list fresh functions plists (get 'o7m-helper 'o7m-prop)
                   (condition-case err
                       (if value (funcall value) (o7m-helper))
                     (void-function (car err)))))",
    );
    (refusal, installed)
}
