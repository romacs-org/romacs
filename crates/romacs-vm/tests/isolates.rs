//! Isolate lifecycle over the bare worker binary: spawn, results, lambda
//! forms with bindings, error propagation with condition ancestry,
//! messages both ways, interrupt, kill, pidfd child reporting and
//! retirement, no zombie and no orphan, a refused worker, the world
//! refusal.
#[path = "support/tree_lisp.rs"]
mod tree_lisp;

use romacs_vm::Vm;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(&format!("(setq isolate-worker-program {:?} isolate-pool-size 2)", env!("CARGO_BIN_EXE_romacs-vm-isolate"))).unwrap();
    vm
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("ERROR {e}"))
}

fn process_exists(pid: i64) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "))
}

fn settles(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        if check() { return true; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

#[test]
fn a_job_runs_in_a_worker_and_answers_frozen() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(isolate-await (isolate-spawn '+ 1 2))"), "3");
    assert_eq!(eval(&mut vm, "(let ((v (isolate-await (isolate-spawn 'list 1 \"two\" 'three (vector 4))))) (list (frozen-p v) v))"), "(t (1 \"two\" three [4]))");
    assert_eq!(eval(&mut vm, "(isolate-await (isolate-spawn '(lambda (x) (* x y)) 4 :bindings '((y . 5))))"), "20");
    // Two jobs on one pooled worker, in order; a fresh worker when asked.
    eval(&mut vm, "(progn (isolate-kill-all) (setq isolate-pool-size 1))");
    assert_eq!(eval(&mut vm, "(let ((a (isolate-spawn 'list 'a)) (b (isolate-spawn 'list 'b))) (list (isolate-await a) (isolate-await b) (eq (isolate-future-isolate a) (isolate-future-isolate b))))"), "((a) (b) t)");
    assert_eq!(eval(&mut vm, "(let ((a (isolate-spawn 'list 'a)) (b (isolate-spawn 'list 'b :fresh t))) (list (isolate-await a) (isolate-await b) (eq (isolate-future-isolate a) (isolate-future-isolate b)) (length (isolate-list))))"), "((a) (b) nil 2)");
    // A worker keeps its own state between jobs of the same feature set.
    assert_eq!(eval(&mut vm, "(let ((iso (isolate-future-isolate (isolate-spawn 'list)))) (isolate-await (isolate-spawn '(lambda () (setq worker-state 42)) :isolate iso)) (isolate-await (isolate-spawn '(lambda () worker-state) :isolate iso)))"), "42");
    assert_eq!(eval(&mut vm, "(condition-case e (isolate-spawn (lambda () 1)) (error (car (cdr e))))"), "\"Cannot send closure\"");
    assert_eq!(eval(&mut vm, "(progn (isolate-kill-all) (isolate-list))"), "nil");
}

#[test]
fn a_workers_error_is_resignalled_with_its_ancestry() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(condition-case e (isolate-await (isolate-spawn 'signal 'error '(\"boom\" 1))) (error (list 'caught e)))"), "(caught (error \"boom\" 1))");
    assert_eq!(eval(&mut vm, "(condition-case e (isolate-await (isolate-spawn '(lambda () (define-error 'worker-only-error \"mine\" 'file-error) (signal 'worker-only-error '(7))))) (file-error (list 'caught e (get 'worker-only-error 'error-conditions))))"), "(caught (worker-only-error 7) (worker-only-error file-error error))");
    // Unfreezable error data travels printed.
    assert_eq!(eval(&mut vm, "(condition-case e (isolate-await (isolate-spawn '(lambda () (signal 'error (list \"data\" (current-buffer)))))) (error (list (nth 1 e) (car (nth 2 e)))))"), "(\"data\" isolate-unfreezable)");
    assert_eq!(eval(&mut vm, "(isolate-await (isolate-spawn '+ 2 2))"), "4");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn messages_travel_both_ways() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(let* ((iso (isolate-create)) (f (isolate-spawn '(lambda () (isolate-receive 5)) :isolate iso))) (isolate-send iso (list 'hello 1)) (isolate-await f))"), "(hello 1)");
    assert_eq!(eval(&mut vm, "(let ((got nil)) (push (lambda (sender message) (push (list (isolate-p sender) message) got)) isolate-message-functions) (isolate-await (isolate-spawn '(lambda () (isolate-send 'parent (list 'from-worker)) t))) (romacs--isolate-pump 0.5) got)"), "((t (from-worker)))");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn an_interrupt_quits_cpu_bound_lisp_and_the_worker_serves_on() {
    let mut vm = vm();
    // A booted worker (its seed job done) runs a tight loop; the interrupt
    // names that job, which quits; the worker serves on.
    assert_eq!(eval(&mut vm, "(let* ((iso (isolate-create)) (f (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn '(lambda () (while t)) :isolate iso)))) (romacs--isolate-wait 0.2) (isolate-interrupt iso) (list (condition-case e (isolate-await f 5) (quit 'interrupted)) (isolate-await (isolate-spawn '+ 1 1 :isolate iso))))"), "(interrupted 2)");
    // inhibit-quit holds the interrupt until it is unbound.
    assert_eq!(eval(&mut vm, "(let* ((iso (isolate-create)) (f (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn '(lambda () (let ((inhibit-quit t) (n 0)) (while (< n 3000000) (setq n (1+ n)))) (let ((n 0)) (while t (setq n (1+ n))))) :isolate iso)))) (romacs--isolate-wait 0.1) (isolate-interrupt iso) (condition-case e (isolate-await f 10) (quit 'interrupted)))"), "interrupted");
    // A future cancelled while queued never runs; one running is interrupted.
    assert_eq!(eval(&mut vm, "(let* ((iso (isolate-create)) (a (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn '(lambda () (while t)) :isolate iso))) (b (isolate-spawn 'list 'queued :isolate iso))) (romacs--isolate-wait 0.2) (isolate-future-cancel b) (isolate-future-cancel a) (list (condition-case e (isolate-await a 5) (quit 'interrupted)) (isolate-future-done-p b) (isolate-await (isolate-spawn 'list 'after :isolate iso))))"), "(interrupted t (after))");
    // A handler-bind callback is the job's own code: interruptible.
    assert_eq!(eval(&mut vm, "(let* ((iso (isolate-create)) (f (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn '(lambda () (handler-bind-1 (lambda () (error \"trigger\")) '(error) (lambda (_) (let ((n 0)) (while t (setq n (1+ n))))))) :isolate iso)))) (romacs--isolate-wait 0.2) (isolate-interrupt iso) (list (condition-case e (isolate-await f 5) (quit 'interrupted)) (isolate-await (isolate-spawn '+ 1 1 :isolate iso))))"), "(interrupted 2)");
    // An interrupt arriving while a finalizer runs (masked, even with
    // inhibit-quit unbound there) waits: the finalizer finishes, the resumed
    // job quits once, and the next job is not hit. The finalizer says it has
    // begun and waits to be released, so the interrupt is sent inside it, and
    // it counts only if it saw the release before holding on: one that gave up
    // leaves `ia-done' nil, however late the release comes. It then holds on,
    // polling, for the signal's handler to have run; the job's loop runs until
    // it quits, so a late handler cannot end it early.
    let entered = std::env::temp_dir().join(format!("romacs-finalizer-entered-{}", std::process::id()));
    let release = std::env::temp_dir().join(format!("romacs-finalizer-release-{}", std::process::id()));
    let _ = std::fs::remove_file(&entered);
    let _ = std::fs::remove_file(&release);
    let out = eval(&mut vm, &format!("(let* ((isolate-interrupt-timeout 60.0) (iso (isolate-create)) (f (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn '(lambda () (eval '(progn (make-finalizer (lambda () (let ((inhibit-quit nil) (end (+ (float-time) 30.0)) released) (write-region \"\" nil {entered} nil 'no-message) (while (and (not (file-exists-p {release})) (< (float-time) end))) (setq released (file-exists-p {release})) (let ((hold (+ (float-time) 0.25)) (n 0)) (while (or (< n 100000) (< (float-time) hold)) (setq n (1+ n)))) (setq ia-done released)))) nil) t) (garbage-collect) (let ((end (+ (float-time) 10.0))) (while (< (float-time) end))) 'resumed) :isolate iso)))) (let ((end (+ (float-time) 30.0))) (while (and (not (file-exists-p {entered})) (< (float-time) end)) (romacs--isolate-wait 0.01))) (isolate-interrupt iso) (write-region \"\" nil {release} nil 'no-message) (list (file-exists-p {entered}) (condition-case e (isolate-await f 30) (quit 'interrupted)) (isolate-await (isolate-spawn '(lambda () (and (boundp 'ia-done) ia-done)) :isolate iso)) (isolate-await (isolate-spawn '+ 1 1 :isolate iso))))", entered = serde_json::to_string(&entered).unwrap(), release = serde_json::to_string(&release).unwrap()));
    assert_eq!(out, "(t interrupted t 2)");
    let _ = std::fs::remove_file(&entered);
    let _ = std::fs::remove_file(&release);
    // An interrupt for a job not yet started cancels it before it runs.
    assert_eq!(eval(&mut vm, "(let* ((iso (isolate-create)) (a (isolate-spawn '(lambda () (romacs--isolate-wait 0.5) 'slow) :isolate iso)) (b (isolate-spawn 'list 'second :isolate iso))) (isolate-future-cancel b) (list (isolate-await a 5) (condition-case e (isolate-await b 5) (quit 'cancelled)) (aref b 3)))"), "(slow cancelled cancelled)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// Files a test and its worker's job meet at, in a directory of the
/// test's own: the job writes `ready' just before the wait the test is
/// about, the spawner writes `go' once it has signalled, a job writes
/// `mark' to show it ran. Each is given as a Lisp string.
struct Handshake(std::path::PathBuf);
impl Handshake {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("romacs-handshake-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Handshake(dir)
    }
    fn file(&self, name: &str) -> String {
        serde_json::to_string(&self.0.join(name)).unwrap()
    }
    /// The job's form that says it is about to wait.
    fn ready(&self) -> String {
        format!("(write-region \"\" nil {} nil 'no-message)", self.file("ready"))
    }
    /// The spawner's form that waits, up to 10 s, until the job is ready,
    /// and signals if it never is.
    fn await_ready(&self) -> String {
        let ready = self.file("ready");
        format!("(let ((end (+ (float-time) 10))) (while (and (not (file-exists-p {ready})) (< (float-time) end)) (romacs--isolate-wait 0.01)) \
                 (unless (file-exists-p {ready}) (error \"the job never reached its wait\")))")
    }
}
impl Drop for Handshake {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

/// A worker's job connected to its own server, which never writes: BODY
/// then waits on the client as `client'.
fn silent_socket_job(body: &str) -> String {
    format!("'(lambda () \
       (let* ((server (make-network-process :name \"s\" :server t :host \"127.0.0.1\" :service t :noquery t)) \
              (client (make-network-process :name \"c\" :host \"127.0.0.1\" :service (process-contact server :service) :noquery t))) \
         {body}))")
}

/// A worker waiting for a socket that never answers takes an interrupt as
/// a quit within the bound, not at its wait's timeout or the spawner's
/// kill: the wait polls the wake pipe the signal writes. The worker serves
/// on. The list's last element is the interrupt's milliseconds to quit.
#[test]
fn an_interrupt_ends_a_workers_wait_for_process_output() {
    let mut vm = vm();
    let hs = Handshake::new("interrupt");
    let job = silent_socket_job(&format!("{} (accept-process-output client 60) 'waited", hs.ready()));
    let out = eval(&mut vm, &format!("(let* ((iso (isolate-create)) \
        (f (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn {job} :isolate iso)))) \
        {} \
        (let* ((start (float-time)) \
               (result (progn (isolate-interrupt iso) (condition-case e (isolate-await f 10) (quit 'interrupted) (error (car e))))) \
               (ms (round (* 1000 (- (float-time) start))))) \
          (list result (< ms 500) (condition-case e (isolate-await (isolate-spawn '+ 1 1 :isolate iso)) (error (car e))) ms)))", hs.await_ready()));
    let (head, ms) = out.rsplit_once(' ').unwrap_or((&out, ""));
    eprintln!("interrupt to quit: {ms} ms");
    assert_eq!(head, "(interrupted t 2", "{out}");
    eval(&mut vm, "(isolate-kill-all)");
}

/// Under `inhibit-quit' the interrupt is held: the wait runs its full
/// 0.5 s without spinning on the drained wake pipe. Once `inhibit-quit'
/// ends, the next wait quits before it blocks, though no wake is left for
/// it. The worker keeps what the first wait measured.
#[test]
fn an_interrupt_held_by_inhibit_quit_waits_out_the_wait_without_spinning() {
    let mut vm = vm();
    let hs = Handshake::new("held");
    let job = silent_socket_job(&format!("(let ((inhibit-quit t) (t0 (float-time)) (c0 (float-time (get-internal-run-time)))) \
         {} (accept-process-output client 0.5) \
         (setq held (list (- (float-time) t0) (- (float-time (get-internal-run-time)) c0)))) \
       (accept-process-output client 60) 'unquit", hs.ready()));
    let out = eval(&mut vm, &format!("(let* ((iso (isolate-create)) \
        (f (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn {job} :isolate iso)))) \
        {} (isolate-interrupt iso) \
        (let* ((start (float-time)) \
               (result (condition-case e (isolate-await f 10) (quit 'interrupted) (error (car e)))) \
               (ms (round (* 1000 (- (float-time) start)))) \
               (held (isolate-await (isolate-spawn '(lambda () held) :isolate iso)))) \
          (list result (>= (car held) 0.45) (< (nth 1 held) 0.1) (< ms 1500) held)))", hs.await_ready()));
    eprintln!("held wait (wall cpu): {out}");
    assert!(out.starts_with("(interrupted t t t "), "{out}");
    eval(&mut vm, "(isolate-kill-all)");
}

/// A wake that raises no quit (the interrupt is held) keeps the events it
/// came with: output ready on the socket beside the wake byte is read, and
/// `(accept-process-output client 0)' says so.
#[test]
fn a_held_interrupt_leaves_ready_output_to_its_wait() {
    let mut vm = vm();
    let hs = Handshake::new("ready-output");
    let job = format!("'(lambda () \
       (let* ((server (make-network-process :name \"s\" :server t :host \"127.0.0.1\" :service t :noquery t)) \
              (client (make-network-process :name \"c\" :host \"127.0.0.1\" :service (process-contact server :service) :noquery t)) \
              (conn nil)) \
         (let ((end (+ (float-time) 5))) \
           (while (and (null conn) (< (float-time) end)) \
             (accept-process-output nil 0.05) \
             (dolist (p (process-list)) (unless (or (eq p server) (eq p client)) (setq conn p))))) \
         (let ((inhibit-quit t)) \
           (process-send-string conn \"x\") \
           {} (while (not (file-exists-p {}))) \
           (setq got (accept-process-output client 0))) \
         (accept-process-output client 60) 'unquit))", hs.ready(), hs.file("go"));
    let out = eval(&mut vm, &format!("(let* ((iso (isolate-create)) \
        (f (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn {job} :isolate iso)))) \
        {} (isolate-interrupt iso) (romacs--isolate-wait 0.1) (write-region \"\" nil {} nil 'no-message) \
        (list (condition-case e (isolate-await f 10) (quit 'interrupted) (error (car e))) \
              (isolate-await (isolate-spawn '(lambda () got) :isolate iso))))", hs.await_ready(), hs.file("go")));
    assert_eq!(out, "(interrupted t)", "{out}");
    eval(&mut vm, "(isolate-kill-all)");
}

/// SIGTERM writes the same wake pipe: `isolate-kill' of a worker waiting
/// on a silent socket ends it on the SIGTERM, before `isolate-kill-timeout'
/// would send SIGKILL. The list's last element is the kill's milliseconds.
#[test]
fn a_kill_ends_a_workers_wait_before_the_kill_timeout() {
    let mut vm = vm();
    let hs = Handshake::new("kill");
    let job = silent_socket_job(&format!("{} (accept-process-output client 60) 'waited", hs.ready()));
    let out = eval(&mut vm, &format!("(let* ((isolate-kill-timeout 1.0) (iso (isolate-create)) \
        (f (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn {job} :isolate iso)))) \
        {} \
        (let* ((start (float-time)) (_ (isolate-kill iso)) (ms (round (* 1000 (- (float-time) start))))) \
          (list (condition-case e (isolate-await f 1) (error (car e))) (< ms 900) ms)))", hs.await_ready()));
    let (head, ms) = out.rsplit_once(' ').unwrap_or((&out, ""));
    eprintln!("kill to end: {ms} ms");
    assert_eq!(head, "(isolate-killed t", "{out}");
}

/// A killed worker starts no job queued behind the one the kill ended.
#[test]
fn a_kill_starts_no_queued_job() {
    let mut vm = vm();
    let hs = Handshake::new("queued");
    let job = silent_socket_job(&format!("{} (accept-process-output client 60) 'waited", hs.ready()));
    let out = eval(&mut vm, &format!("(let* ((iso (isolate-create)) \
        (a (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn {job} :isolate iso))) \
        (b (isolate-spawn '(lambda () (write-region \"\" nil {} nil 'no-message)) :isolate iso))) \
        {} (isolate-kill iso) \
        (list (condition-case e (isolate-await b 1) (error (car e))) (file-exists-p {})))", hs.file("mark"), hs.await_ready(), hs.file("mark")));
    assert_eq!(out, "(isolate-killed nil)", "{out}");
}

/// An interrupt and a termination both pending: the termination's quit
/// supersedes the interrupt, so the cleanup it unwinds through, which waits
/// on the silent socket, is not cut short by the older interrupt.
#[test]
fn a_termination_supersedes_a_pending_interrupt() {
    let mut vm = vm();
    let hs = Handshake::new("supersede");
    let job = silent_socket_job(&format!("(unwind-protect \
         (progn (let ((inhibit-quit t)) {} (while (not (file-exists-p {})))) (accept-process-output client 60)) \
         (accept-process-output client 0) (write-region \"\" nil {} nil 'no-message))", hs.ready(), hs.file("go"), hs.file("mark")));
    let out = eval(&mut vm, &format!("(let* ((iso (isolate-create)) \
        (f (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn {job} :isolate iso)))) \
        {} (isolate-interrupt iso) (romacs--isolate-signal (romacs--isolate-id iso) 15) (romacs--isolate-wait 0.1) \
        (write-region \"\" nil {} nil 'no-message) \
        (let ((end (+ (float-time) 5))) (while (and (not (file-exists-p {})) (< (float-time) end)) (romacs--isolate-wait 0.01))) \
        (prog1 (file-exists-p {}) (isolate-kill iso)))", hs.await_ready(), hs.file("go"), hs.file("mark"), hs.file("mark")));
    assert_eq!(out, "t", "{out}");
}

/// A worker cancelled during a long `call-process' whose child runs the
/// shell PRELUDE, then writes its pid and the ready file (paths given as
/// arguments) and execs `sleep 30': the milliseconds from the cancel to the
/// quit, once the call quit, the worker served its next job, and the child's
/// /proc entry went (reaped, not a zombie) while the worker lives.
fn cancel_call_process(name: &str, prelude: &str) -> String {
    let mut vm = vm();
    let hs = Handshake::new(name);
    let pidfile = hs.0.join("pid");
    let script = format!("{prelude} echo $$ > \"$1\"; : > \"$2\"; exec sleep 30");
    // call-process's own primitive, whose wait this is: the bare VM's Lisp
    // call-process needs GNU's coding systems.
    let argv = [String::from("sh"), "-c".into(), script, "sh".into(), pidfile.display().to_string(), hs.0.join("ready").display().to_string()];
    let argv: Vec<String> = argv.iter().map(|a| serde_json::to_string(a).unwrap()).collect();
    let job = format!("'(lambda () (romacs--call-process-run '({}) nil \"/\" \"/dev/null\" nil nil t) 'waited)", argv.join(" "));
    let out = eval(&mut vm, &format!("(let* ((iso (isolate-create)) \
        (f (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn {job} :isolate iso)))) \
        (condition-case nil {} \
          (error (error \"the child never started: %S\" (condition-case e (isolate-await f 2) (error e))))) \
        (let* ((start (float-time)) \
               (result (progn (isolate-interrupt iso) (condition-case e (isolate-await f 10) (quit 'interrupted) (error (car e))))) \
               (ms (round (* 1000 (- (float-time) start))))) \
          (list result (condition-case e (isolate-await (isolate-spawn '+ 1 1 :isolate iso)) (error (car e))) ms)))", hs.await_ready()));
    let (head, ms) = out.rsplit_once(' ').unwrap_or((&out, ""));
    assert_eq!(head, "(interrupted 2", "{out}");
    let child: i64 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    let proc = std::path::PathBuf::from(format!("/proc/{child}"));
    assert!(settles(|| !proc.exists()), "child {child}'s /proc entry outlived the cancel, the worker still live");
    eval(&mut vm, "(isolate-kill-all)");
    ms.trim_end_matches(')').to_string()
}

/// A worker's cancel during a long `call-process' stops the child, as a
/// C-g's quit does (SIGINT to its group), and quits within the bound.
#[test]
fn an_interrupt_stops_a_workers_call_process() {
    let ms = cancel_call_process("call-process", "");
    eprintln!("call-process interrupt to quit: {ms} ms");
    assert!(ms.parse::<u64>().unwrap() < 500, "{ms} ms");
}

/// A child that ignores SIGINT gets no second C-g from a worker: the stop
/// sends SIGKILL itself after its grace, and the cancel still returns
/// within the grace and a margin.
#[test]
fn an_interrupt_kills_a_workers_child_that_ignores_sigint() {
    let ms = cancel_call_process("call-process-trap", "trap '' INT HUP;");
    eprintln!("call-process interrupt to quit, SIGINT ignored: {ms} ms");
    assert!(ms.parse::<u64>().unwrap() < 1000, "{ms} ms");
}

#[test]
fn kill_leaves_no_zombie_and_settles_futures() {
    let mut vm = vm();
    // SIGTERM ends a worker cleanly; its futures resolve as killed; it is reaped.
    let out = eval(&mut vm, "(let* ((f (isolate-spawn '(lambda () (while t)))) (iso (isolate-future-isolate f)) (pid (romacs--isolate-pid (romacs--isolate-id iso)))) (romacs--isolate-wait 0.2) (list pid (isolate-kill iso) (condition-case e (isolate-await f) (isolate-killed 'killed)) (isolate-live-p iso) (isolate-list)))");
    assert!(out.ends_with(" nil killed nil nil)"), "{out}");
    let worker: i64 = out.trim_start_matches('(').split_whitespace().next().unwrap().parse().unwrap();
    assert!(settles(|| !process_exists(worker)), "worker {worker} still exists: {out}");
    // A worker that ignores SIGTERM long enough is SIGKILLed after the timeout.
    let out = eval(&mut vm, "(let* ((isolate-kill-timeout 0.3) (f (isolate-spawn '(lambda () (let ((inhibit-quit t)) (while t))))) (iso (isolate-future-isolate f)) (pid (romacs--isolate-pid (romacs--isolate-id iso)))) (romacs--isolate-wait 0.2) (isolate-kill iso) (list pid (condition-case e (isolate-await f) (isolate-killed 'killed))))");
    let worker: i64 = out.trim_start_matches('(').split_whitespace().next().unwrap().parse().unwrap();
    assert!(out.ends_with(" killed)"), "{out}");
    assert!(settles(|| !process_exists(worker)), "worker {worker} still exists: {out}");
}

thread_local! {
    /// The forks this thread made since the handler was registered.
    static FORKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}
extern "C" fn count_fork() {
    FORKS.with(|forks| forks.set(forks.get() + 1));
}

/// A worker is spawned, never forked: fork runs every library's atfork
/// handlers, and a GPU driver unloading on another thread meanwhile
/// crashes in its own (World::start_builder). Only a fork runs them.
#[test]
fn a_worker_is_spawned_without_running_atfork_handlers() {
    use std::os::unix::process::CommandExt;
    // UNSAFE(no crate registers a fork handler safely: pthread_atfork is a C call)
    // SAFETY: `count_fork', run in the parent before each fork, only counts
    // in a thread-local and cannot unwind.
    assert_eq!(unsafe { libc::pthread_atfork(Some(count_fork), None, None) }, 0);
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(isolate-await (isolate-spawn '+ 1 2))"), "3");
    eval(&mut vm, "(isolate-kill-all)");
    assert_eq!(FORKS.with(std::cell::Cell::get), 0, "spawning the worker forked");
    // The handler sees a fork: a uid makes the standard library fork, which
    // posix_spawn cannot set; this process's own needs no privilege.
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").unwrap());
    let mut forked = std::process::Command::new("/bin/true");
    forked.uid(uid);
    assert!(forked.status().unwrap().success());
    assert_eq!(FORKS.with(std::cell::Cell::get), 1);
}

/// A worker's socket closes before its death is visible (exit closes its
/// descriptors first): one wait that sees the close goes on to the death.
/// This worker closes its socket at once and exits 0.3 s later.
#[test]
fn a_wait_that_sees_a_workers_socket_close_reports_its_death() {
    use std::os::unix::fs::PermissionsExt;
    let script = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("closes-then-exits-{}", std::process::id()));
    std::fs::write(&script, "#!/bin/sh\nexec 0<&-\nsleep 0.3\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut vm = vm();
    let out = eval(&mut vm, &format!("(let ((id (romacs--isolate-start {script:?} nil))) (list (romacs--isolate-wait 5) (romacs--isolate-take id) (romacs--isolate-close id)))"));
    assert_eq!(out, "(t (died 0 0) t)");
    std::fs::remove_file(&script).unwrap();
}

#[test]
fn a_worker_that_is_not_this_executable_is_refused_and_a_dead_one_reported() {
    let mut vm = vm();
    // A worker that exits 87 on its own (a program that only does that; Lisp
    // in a worker cannot `kill-emacs') is an ordinary death: no exit status
    // is read as a memory death, and `isolate-error-functions' hears of it.
    let script = std::env::temp_dir().join(format!("romacs-exit-87-{}", std::process::id()));
    std::fs::write(&script, "#!/bin/sh\nexit 87\n").unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    assert_eq!(eval(&mut vm, &format!("(let ((isolate-worker-program {}) (events nil)) (push (lambda (_iso event _data) (push event events)) isolate-error-functions) (list (condition-case e (isolate-await (isolate-spawn '+ 1 2) 5) (isolate-error (car e))) (progn (romacs--isolate-pump 0.5) events)))", serde_json::to_string(&script).unwrap())), "(isolate-died (isolate-died))");
    let _ = std::fs::remove_file(&script);
    eval(&mut vm, "(setq isolate-error-functions nil)");
    // The submission may already find the worker gone (isolate-died at the
    // spawn) or its death arrives at the await; either way the death is
    // reported once and the worker forgotten.
    assert_eq!(eval(&mut vm, "(let ((isolate-worker-program \"/bin/true\") (events nil)) (push (lambda (_iso event _data) (push event events)) isolate-error-functions) (list (condition-case e (isolate-await (isolate-spawn '+ 1 2) 5) (isolate-error (car e))) (progn (romacs--isolate-pump 0.5) events) (isolate-list)))"), "(isolate-died (isolate-died) nil)");
    // A worker always starts: with no world to adopt it boots from source,
    // never refused while its callers wait.
    assert_eq!(eval(&mut vm, "(let ((isolate-boot 'world) (isolate-world nil)) (romacs--isolate-boot-arguments))"), "(\"--source\")");
    assert_eq!(eval(&mut vm, "(let ((isolate-boot 'source)) (romacs--isolate-boot-arguments))"), "(\"--source\")");
    // A worker killed from outside resolves its futures as died.
    let out = eval(&mut vm, "(let* ((f (isolate-spawn '(lambda () (while t)))) (iso (isolate-future-isolate f))) (romacs--isolate-wait 0.2) (romacs--isolate-signal (romacs--isolate-id iso) 9) (list (condition-case e (isolate-await f 5) (isolate-died 'died)) (isolate-list)))");
    assert_eq!(out, "(died nil)");
}

#[test]
fn seeds_are_thawed_copies_the_worker_may_mutate() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(let ((isolate-inherited-variables '(seed-list))) (setq seed-list (list 1 2)) (isolate-await (isolate-spawn '(lambda () (list (frozen-p seed-list) (progn (setcar seed-list 9) seed-list))))))"), "(nil (9 2))");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn sends_to_a_full_queue_signal_instead_of_blocking() {
    // Sol r1 S2: the transport is nonblocking; a full socket buffer is
    // `isolate-queue-full', never a wait.
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((iso (isolate-create)) (busy (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn '(lambda () (while t)) :isolate iso))) (sent 0) (t0 (float-time)) (big (make-string 3000 ?x))) (condition-case e (while (< sent 100000) (isolate-send iso big) (setq sent (1+ sent))) (isolate-queue-full (setq sent (- sent)))) (isolate-kill iso) (list (< sent 0) (< (- (float-time) t0) 2.0)))");
    assert_eq!(out, "(t t)");
}

#[test]
fn cancelling_a_finished_future_does_not_hit_the_next_job() {
    // Sol r1 S3: a future whose result had arrived but was not yet delivered
    // must not interrupt the job running after it.
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((iso (isolate-create)) (a (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn 'list 'quick :isolate iso))) (b (isolate-spawn '(lambda () (let ((n 0)) (while (< n 4000000) (setq n (1+ n))) 'slow-done)) :isolate iso))) (romacs--isolate-wait 0.3) (isolate-future-cancel a) (list (isolate-await a) (isolate-await b 20)))");
    assert_eq!(out, "((quick) slow-done)");
}

#[test]
fn a_cancelled_queued_job_never_runs() {
    // Sol r1 S4: the worker classifies the cancel that arrived with the job
    // before running it.
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((iso (isolate-create)) (a (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn '(lambda () (romacs--isolate-wait 0.3) 'first) :isolate iso))) (b (isolate-spawn '(lambda () (setq journey-ran t)) :isolate iso))) (isolate-future-cancel b) (list (isolate-await a) (condition-case e (isolate-await b 5) (quit 'cancelled)) (isolate-await (isolate-spawn '(lambda () (boundp 'journey-ran)) :isolate iso))))");
    assert_eq!(out, "(first cancelled nil)");
}

#[test]
fn identical_jobs_share_one_pooled_worker_and_the_pool_is_bounded() {
    // Sol r1 S5: the preparation's requires are the worker's baseline, not a
    // taint; the pool bound holds, with isolate-queue-full past it.
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(let ((isolate-pool-size 1)) (isolate-await (isolate-spawn 'list 'a)) (isolate-await (isolate-spawn 'list 'b)) (length (isolate-list)))"), "1");
    assert_eq!(eval(&mut vm, "(let ((isolate-pool-size 1)) (let ((f (isolate-spawn '(lambda () (while t))))) (romacs--isolate-wait 0.2) (list (condition-case e (isolate-spawn 'list 'other :require '(other-feature)) (isolate-queue-full 'full)) (length (isolate-list)))))"), "(full 1)");
    eval(&mut vm, "(isolate-kill-all)");
    // A job that loads a feature retires its worker: the next job gets a
    // fresh one at once, the old one ending by itself, unwaited.
    let out = eval(&mut vm, "(let ((isolate-pool-size 1)) (let* ((iso (isolate-future-isolate (isolate-spawn '(lambda () (provide 'journey-feature))))) (first (isolate-spawn 'list)) (pid (romacs--isolate-pid (romacs--isolate-id iso)))) (isolate-await first) (let* ((t0 (float-time)) (next (isolate-spawn 'list)) (quick (< (- (float-time) t0) 0.1))) (list pid (aref iso 6) (eq iso (isolate-future-isolate next)) quick (aref iso 3) (let ((n 0)) (dolist (i (isolate-list) n) (when (isolate-live-p i) (setq n (1+ n)))))))))");
    let pid: i64 = out.trim_start_matches('(').split_whitespace().next().and_then(|p| p.parse().ok()).unwrap_or_else(|| panic!("{out}"));
    // Retired, not killed: nothing waited for its exit.
    assert!(out.ends_with(" t nil t retired 1)"), "{out}");
    assert!(settles(|| !process_exists(pid)), "retired worker {pid} survived");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn an_unsendable_job_signals_and_leaves_no_pending_future() {
    // Sol r1 S6.
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(let ((isolate-pool-size 1)) (list (condition-case e (isolate-spawn 'identity (current-buffer)) (error (car (cdr e)))) (progn (isolate-await (isolate-spawn 'list)) (length (aref (car (isolate-list)) 5)))))"), "(\"Cannot freeze\" 0)");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn a_workers_condition_ancestry_is_installed_whole() {
    // Sol r1 S10: a subtype of a worker-defined subtype matches every ancestor.
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(list (condition-case e (isolate-await (isolate-spawn '(lambda () (define-error 'journey-parent \"p\" 'file-error) (define-error 'journey-child \"c\" 'journey-parent) (signal 'journey-child '(1))))) (file-error (list 'file-error (car e)))) (condition-case e (isolate-await (isolate-spawn '(lambda () (signal 'journey-child '(2))))) (error (list 'error (car e)))) (get 'journey-child 'error-conditions))"), "((file-error journey-child) (error journey-child) (journey-child journey-parent file-error error))");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn a_cancel_behind_cpu_bound_work_still_cancels_the_queued_job() {
    // Sol r2 S4: the worker reads what arrived before each queued job.
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((iso (isolate-create)) (a (progn (isolate-await (isolate-spawn 'list :isolate iso)) (isolate-spawn '(lambda () (let ((n 0)) (while (< n 3000000) (setq n (1+ n)))) 'first) :isolate iso))) (b (isolate-spawn '(lambda () (setq journey-ran t)) :isolate iso))) (isolate-future-cancel b) (list (isolate-await a) (condition-case e (isolate-await b 5) (quit 'cancelled)) (isolate-await (isolate-spawn '(lambda () (boundp 'journey-ran)) :isolate iso))))");
    assert_eq!(out, "(first cancelled nil)");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn a_job_queued_behind_a_feature_change_runs_on_a_worker_without_it() {
    // Sol r2 S5: the retired worker hands its queued jobs back; they run on
    // a worker with the features they were queued for.
    let mut vm = vm();
    let out = eval(&mut vm, "(let ((isolate-pool-size 1)) (let* ((a (isolate-spawn '(lambda () (provide 'journey-feature) 'provided))) (first (isolate-future-isolate a)) (b (isolate-spawn '(lambda () (featurep 'journey-feature))))) (list (isolate-await a) (isolate-await b 10) (eq (isolate-future-isolate b) first) (aref first 6))))");
    assert_eq!(out, "(provided nil nil t)");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn every_reply_arrives_when_the_parent_is_slow_to_read() {
    // Sol r2 S13: a worker's replies wait for buffer space instead of being lost.
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((iso (isolate-create)) (futures nil)) (isolate-await (isolate-spawn 'list :isolate iso)) (dotimes (i 64) (push (isolate-spawn '(lambda (n) (make-string 3000 (+ ?a (% n 26)))) i :isolate iso) futures)) (romacs--isolate-wait 1.0) (let ((lengths (mapcar (lambda (f) (length (isolate-await f 20))) futures))) (list (length lengths) (apply #'min lengths) (apply #'max lengths))))");
    assert_eq!(out, "(64 3000 3000)");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn resubmitted_jobs_keep_their_snapshot_and_fit_the_pool() {
    // Sol r3 S14: the job frozen at submission is what a resubmission sends.
    let mut vm = vm();
    let out = eval(&mut vm, "(let ((isolate-pool-size 1)) (let* ((arg (list 1)) (a (isolate-spawn '(lambda () (provide 'journey-feature-2) 'provided))) (b (isolate-spawn 'identity arg))) (setcar arg 2) (list (isolate-await a) (isolate-await b 10))))");
    assert_eq!(out, "(provided (1))");
    eval(&mut vm, "(isolate-kill-all)");
    // Sol r3 S5: two jobs queued behind the change both run again, with pool size 1.
    let out = eval(&mut vm, "(let ((isolate-pool-size 1)) (let* ((a (isolate-spawn '(lambda () (provide 'journey-feature-3) 'provided))) (b (isolate-spawn '(lambda () (featurep 'journey-feature-3)))) (c (isolate-spawn 'list 'third))) (list (isolate-await a) (isolate-await b 10) (isolate-await c 10) (let ((n 0)) (dolist (i (isolate-list) n) (when (isolate-live-p i) (setq n (1+ n))))))))");
    assert_eq!(out, "(provided nil (third) 1)");
    eval(&mut vm, "(isolate-kill-all)");
    // Sol r3 S4 / r5 S15: cancelling a job the cancel's own pump queues
    // again reaches its new worker. Bare waits read frames without
    // delivering them, until the first worker's four (hello, preparation,
    // the retiring job's result, b's retirement) are held undelivered: b
    // is still on its first worker and generation when the cancel starts;
    // the cancel's pump then resubmits b, and the cancel must follow it.
    let out = eval(&mut vm, "(let ((isolate-pool-size 1)) (let* ((a (isolate-spawn '(lambda () (provide 'journey-feature-4) 'provided))) (b (isolate-spawn '(lambda () (while t)))) (first (isolate-future-isolate b)) (generation (aref b 2))) (let ((end (+ (float-time) 10))) (while (and (< (romacs--isolate-frames (romacs--isolate-id first)) 4) (< (float-time) end)) (romacs--isolate-wait 0.2))) (list (list (eq (isolate-future-isolate b) first) (= (aref b 2) generation) (aref b 3) (romacs--isolate-frames (romacs--isolate-id first))) (progn (isolate-future-cancel b) (list (not (eq (isolate-future-isolate b) first)) (> (aref b 2) generation))) (isolate-await a) (condition-case e (isolate-await b 10) (quit 'cancelled)) (isolate-await (isolate-spawn 'list 'after) 10))))");
    assert_eq!(out, "((t t pending 4) (t t) provided cancelled (after))");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn a_retired_worker_ends_without_a_wait_or_a_death_report() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((reports nil) (isolate-error-functions (list (lambda (&rest e) (push e reports))))
                                    (iso (isolate-create)) (pid (progn (isolate-await (isolate-spawn 'list :isolate iso)) (romacs--isolate-pid (romacs--isolate-id iso))))
                                    (busy (isolate-create)) (job (isolate-spawn '(lambda () (while t)) :isolate busy))
                                    (t0 (float-time)) (retired (isolate-retire iso)) (quick (< (- (float-time) t0) 0.1)))
                               (let ((end (+ (float-time) 5))) (while (and (memq iso romacs--isolates) (< (float-time) end)) (romacs--isolate-pump 0.05)))
                               (prog1 (list pid retired quick (isolate-live-p iso) (memq iso (isolate-list)) reports (isolate-retire busy))
                                 (isolate-kill busy)))");
    let pid: i64 = out.trim_start_matches('(').split_whitespace().next().unwrap().parse().unwrap();
    assert!(out.ends_with(" t t nil nil nil nil)"), "{out}");
    assert!(settles(|| !process_exists(pid)), "retired worker {pid} survived");
}

/// Every condition isolate.el settles a dead worker's jobs with, or signals
/// itself, is an `isolate-error`, so an awaiter's handler for it catches
/// each: one used and never defined reaches the awaiter as a peculiar error.
#[test]
fn every_condition_isolate_el_raises_is_an_isolate_error() {
    let source = tree_lisp::text("vm/isolate.el");
    // `(romacs--isolate-dead ISOLATE 'NAME ...)` and `(signal 'NAME ...)`.
    let quoted = |text: &str| text.strip_prefix('\'').map(|text| {
        text.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>()
    });
    let mut names: Vec<String> = source.match_indices("romacs--isolate-dead ")
        .filter_map(|(at, marker)| source[at + marker.len()..].split_once(' ').and_then(|(_, rest)| quoted(rest)))
        .chain(source.match_indices("(signal ").filter_map(|(at, marker)| quoted(&source[at + marker.len()..])))
        .filter(|name| name.starts_with("isolate-"))
        .collect();
    names.sort_unstable();
    names.dedup();
    assert!(names.iter().any(|n| n == "isolate-world-unavailable") && names.iter().any(|n| n == "isolate-died"), "{names:?}");
    let mut vm = vm();
    for name in names {
        assert_eq!(eval(&mut vm, &format!("(and (memq 'isolate-error (get '{name} 'error-conditions)) t)")), "t",
                   "{name} is no isolate-error");
    }
}
