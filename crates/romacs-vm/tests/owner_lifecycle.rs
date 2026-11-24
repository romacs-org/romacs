//! O1a-2 acceptance through the Lisp interface, with a bare master and real
//! zygote, owner, and subprocesses. The VM's all target discovers this file.
//! Each test generates its process input and synchronization files.

use romacs_vm::Vm;
use rustix::time::{ClockId, clock_gettime};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SETUP_TIMEOUT: Duration = Duration::from_secs(30);
const PUMP: &str = "(romacs--isolate-pump 0.01)";

struct Master(Vm);

impl Master {
    fn bare() -> Self {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let mut master = Self(vm);
        master.run(&format!(
            "(setq isolate-worker-program {:?} isolate-boot 'bare)",
            env!("CARGO_BIN_EXE_romacs-vm-isolate")
        ));
        master
    }

    fn new() -> Self {
        let mut master = Self::bare();
        master.run("(setq owner (owner--spawn))");
        master.until(SETUP_TIMEOUT, "owner startup", |m| {
            m.eval("(owner-state owner)") != "starting"
        });
        assert_eq!(
            master.eval(
                "(list (owner-p owner) (owner-state owner) (aref (owner--isolate owner) 10)
                       (plist-get (owner-describe owner) :error))"
            ),
            "(t live owner nil)"
        );
        master
    }

    fn run(&mut self, source: &str) {
        self.0
            .eval_source(source)
            .unwrap_or_else(|error| panic!("{error}\n{source}"));
    }

    fn eval(&mut self, source: &str) -> String {
        self.0
            .eval_source(source)
            .and_then(romacs_vm::Local::display)
            .unwrap_or_else(|error| panic!("{error}\n{source}"))
    }

    fn until(&mut self, timeout: Duration, label: &str, mut ready: impl FnMut(&mut Self) -> bool) {
        let start = Instant::now();
        while !ready(self) {
            assert!(start.elapsed() < timeout, "timed out waiting for {label}");
            self.eval(PUMP);
        }
    }

    fn await_file(&mut self, path: &Path) {
        let start = Instant::now();
        while !path.exists() {
            assert!(start.elapsed() < SETUP_TIMEOUT, "no marker at {}", path.display());
            // Read transport traffic without dispatching master pings before
            // the owner has entered the intended turn.
            self.eval("(romacs--isolate-wait 0.01)");
        }
    }
}

impl Drop for Master {
    fn drop(&mut self) {
        let _ = self.0.eval_source("(isolate-kill-all)");
    }
}

struct Markers(PathBuf);

impl Markers {
    fn new() -> Self {
        let output = std::process::Command::new("mktemp")
            .args(["-d", "--tmpdir", "romacs-owner-test.XXXXXXXX"])
            .output()
            .unwrap();
        assert!(output.status.success(), "mktemp: {output:?}");
        Self(PathBuf::from(
            std::str::from_utf8(&output.stdout).unwrap().trim_end(),
        ))
    }

    fn ready(&self) -> PathBuf {
        self.0.join("ready")
    }

    fn go(&self) -> PathBuf {
        self.0.join("go")
    }
}

impl Drop for Markers {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.ready());
        let _ = std::fs::remove_file(self.go());
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn literal(path: &Path) -> String {
    serde_json::to_string(path).unwrap()
}

// Pace writes from a monotonic clock. Rebase when backpressure or scheduling
// delays a write, so a stalled producer does not later send a catch-up burst.
const STREAM: &str = r#"
import os
import time

period = 1 / 240
deadline = time.monotonic()
while True:
    time.sleep(max(0, deadline - time.monotonic()))
    deadline = time.monotonic() + period
    os.write(1, b"x" * 1024)
"#;

fn start_stream(master: &mut Master, markers: &Markers) -> String {
    master.eval(&format!(
        r#"(isolate-await
             (owner-call owner '(lambda ()
                (setq stream-dispatches 0)
                (let ((read-process-output-max 16))
                  (process-id
                   (make-process :name "owner stream" :connection-type 'pipe
                                 :command (list "/usr/bin/env" "python3" "-c" {script})
                                 :noquery t :coding 'no-conversion
                                 :filter (lambda (_proc _output)
                                           (setq stream-dispatches (1+ stream-dispatches))
                                           (when (= stream-dispatches 1)
                                             (write-region "" nil {ready} nil 'silent)
                                             (let ((end (+ (float-time) 30)))
                                               (while (and (not (file-exists-p {go}))
                                                           (< (float-time) end)))))))))))
             30)"#,
        script = serde_json::to_string(STREAM).unwrap(),
        ready = literal(&markers.ready()),
        go = literal(&markers.go())
    ))
}

#[test]
fn a_queued_call_runs_within_one_round_of_stream_output() {
    let markers = Markers::new();
    let mut master = Master::new();
    let batch: u64 = master.eval("owner-output-batch").parse().unwrap();
    let pid = start_stream(&mut master, &markers).parse::<i64>().unwrap();
    assert!(pid > 0);
    master.await_file(&markers.ready());
    // A 1024-byte pipe write and a 16-byte read limit guarantee at least
    // 63 more dispatches' worth of ready bytes after the first filter.
    // Queue the call before releasing that filter. At most BATCH total
    // dispatches (including the held one) can run in the round it entered.
    // Counting dispatches gives scheduler preemption no timing penalty.
    assert!(
        batch < 64,
        "fixture needs more than one batch of ready reads"
    );
    master.run("(setq probe (owner-call owner '(lambda () stream-dispatches)))");
    std::fs::write(markers.go(), []).unwrap();
    let dispatches: u64 = master.eval("(isolate-await probe 30)").parse().unwrap();
    assert!(
        (1..=batch).contains(&dispatches),
        "the queued call saw {dispatches} dispatches, one round permits {batch}"
    );
    assert_eq!(
        master.eval(&format!(
            "(and (memq {pid} (romacs--isolate-children (romacs--isolate-id (owner--isolate owner)))) t)"
        )),
        "t",
        "the owner never reported its streaming child {pid}"
    );
}

#[test]
fn calls_from_two_callers_keep_order_and_return_remote_errors() {
    let markers = Markers::new();
    let mut master = Master::new();
    master.run(
        r#"
        (defun caller-a (target operation &rest args)
          (apply 'owner-call target operation args))
        (defun caller-b (target operation &rest args)
          (apply 'owner-call target operation args))
        (isolate-await
         (owner-call owner '(lambda ()
            (setq call-log nil)
            (defun record-owner-call (value)
              (setq call-log (append call-log (list value))))
            (defun record-after-gate (ready go)
              (write-region "" nil ready nil 'silent)
              (let ((end (+ (float-time) 30)))
                (while (and (not (file-exists-p go)) (< (float-time) end))
                  (sleep-for 0.01)))
              (unless (file-exists-p go) (error "caller gate timed out"))
              (record-owner-call 'one))
            (defun fail-inside-owner ()
              (define-error 'owner-test-condition "Remote condition")
              (signal 'owner-test-condition '("remote data" 17 (a b))))))
         30)"#,
    );
    master.run(&format!(
        "(setq first (caller-a owner 'record-after-gate {} {}))",
        literal(&markers.ready()),
        literal(&markers.go())
    ));
    master.await_file(&markers.ready());
    master.run(
        "
        (setq second (caller-b owner 'record-owner-call 'two)
              third (caller-a owner 'record-owner-call 'three))",
    );
    std::fs::write(markers.go(), []).unwrap();
    // Await in reverse, so the observation cannot serialize admission.
    assert_eq!(
        master.eval(
            "(list (isolate-await third 30) (isolate-await second 30) (isolate-await first 30))"
        ),
        "((one two three) (one two) (one))"
    );
    // The signal hook runs before the error unwinds the backtrace binding.
    assert_eq!(
        master.eval(
            r#"
            (let* ((trace nil)
                   (signal-hook-function
                    (lambda (condition _data)
                      (when (eq condition 'owner-test-condition)
                        (setq trace isolate-error-backtrace))))
                   (caught (condition-case err
                               (isolate-await (caller-b owner 'fail-inside-owner) 30)
                             (error err))))
              (list caught
                    (and (stringp trace)
                         (numberp (string-match "fail-inside-owner" trace)))))"#
        ),
        r#"((owner-test-condition "remote data" 17 (a b)) t)"#
    );
}

#[test]
fn an_idle_owner_survives_retirement_of_its_zygote() {
    let mut master = Master::new();
    let pair = "(list (plist-get (owner-describe owner) :zygote)
                        (isolate-await (owner-call owner 'emacs-pid) 30))";
    let before = master.eval(pair);
    master.run(
        r#"
        (let ((zygote (gethash nil romacs--isolate-zygotes)))
          (unless (and zygote
                       (equal (nth 2 (isolate-zygote nil))
                              (plist-get (owner-describe owner) :zygote)))
            (error "owner did not name its original zygote"))
          (romacs--isolate-zygote-retire zygote))"#,
    );
    assert_eq!(master.eval("(gethash nil romacs--isolate-zygotes)"), "nil");
    // Keep pumping past the retired zygote's normal kill deadline, then
    // require the same owner and parent. A single immediate call can race
    // a scheduled SIGKILL and pass before it reaches either process.
    assert_eq!(
        master.eval(
            "
            (isolate-await (owner-call owner '(lambda (seconds) (sleep-for seconds) t)
                                       (+ 1 isolate-kill-timeout))
                           30)"
        ),
        "t"
    );
    assert_eq!(master.eval(pair), before);
    assert_eq!(master.eval("(owner-state owner)"), "live");
}

fn thread_cpu() -> Duration {
    let time = clock_gettime(ClockId::ThreadCPUTime);
    Duration::new(
        time.tv_sec.try_into().unwrap(),
        time.tv_nsec.try_into().unwrap(),
    )
}

fn load1() -> String {
    let load = std::fs::read_to_string("/proc/loadavg").unwrap();
    load.split_whitespace().next().unwrap().to_owned()
}

#[test]
fn killing_a_blocked_owner_uses_under_50ms_of_master_cpu() {
    let markers = Markers::new();
    let mut master = Master::new();
    master.run(&format!(
        "(setq blocked (owner-call owner 'romacs--test-block 12 {}))",
        literal(&markers.ready())
    ));
    master.await_file(&markers.ready());
    let load = load1();
    let start = Instant::now();
    let cpu_start = thread_cpu();
    let result = master.0.eval_source("(owner-kill owner)");
    let cpu = thread_cpu().checked_sub(cpu_start).unwrap();
    let wall = start.elapsed();
    eprintln!(
        "[measured] owner-kill master CPU {cpu:?}, wall {wall:?}, load1 {load}; \
         executable {:?}; (owner-kill owner)",
        std::env::current_exe().unwrap()
    );
    assert_eq!(result.unwrap().display().unwrap(), "nil");
    // CPU includes evaluating the call but excludes scheduler delay. The
    // closing and pending-call assertions catch a wait that consumes no CPU.
    assert!(cpu < Duration::from_millis(50), "{cpu:?} at load1 {load}");
    assert_eq!(
        master.eval(
            "
            (list (owner-state owner) (isolate-future-done-p blocked)
                  (condition-case err (isolate-await blocked 0)
                    (error (car err))))"
        ),
        "(closing t owner-killed)"
    );
    master.until(SETUP_TIMEOUT, "owner death", |m| {
        m.eval("(owner-state owner)") == "dead"
    });
}

fn timed_wait(expression: &str) -> String {
    format!("(let ((begin (float-time))) {expression} (- (float-time) begin))")
}

fn check_wait(master: &mut Master, name: &str, body: &str) {
    master.run(&format!(
        "(setq start-pings (plist-get (owner-describe owner) :pings)
               wait-future (owner-call owner '(lambda () {body})))"
    ));
    let load = load1();
    let mut answered_while_pending = false;
    master.until(SETUP_TIMEOUT, name, |m| {
        assert_eq!(
            m.eval("(owner-state owner)"),
            "live",
            "{name} at load1 {load}"
        );
        let done = m.eval("(isolate-future-done-p wait-future)") == "t";
        if !done && !answered_while_pending {
            answered_while_pending = m.eval(
                "(> (cdr (plist-get (owner-describe owner) :pings))
                                                  (cdr start-pings))",
            ) == "t";
        }
        done
    });
    assert!(
        answered_while_pending,
        "{name} answered no ping during the wait at load1 {load}"
    );
    let seconds: f64 = master
        .eval("(isolate-await wait-future 0)")
        .parse()
        .unwrap();
    assert!(
        seconds >= 2.9,
        "{name} only waited {seconds}s at load1 {load}"
    );
    // A pong racing the result can still be in transit. A subsequent
    // ping can retire every earlier outstanding ping, even after the call.
    master.until(Duration::from_secs(2), name, |m| {
        assert_eq!(
            m.eval("(owner-state owner)"),
            "live",
            "{name} at load1 {load}"
        );
        m.eval(
            "(let ((now (plist-get (owner-describe owner) :pings)))
                    (and (= (car now) (cdr now))
                         (>= (- (cdr now) (cdr start-pings)) 2)))",
        ) == "t"
    });
    eprintln!(
        "{name}: owner waited {seconds}s, pings {:?}, load1 {load}",
        master.eval("(list start-pings (plist-get (owner-describe owner) :pings))")
    );
}

#[test]
fn three_second_owner_waits_answer_every_ping() {
    let mut master = Master::new();
    let silent_process_wait = timed_wait("(accept-process-output process 3)");
    let accept = format!(
        r#"(let ((process (make-process :name "silent peer" :connection-type 'pipe
                                        :command '("/bin/sh" "-c" "exec sleep 30") :noquery t)))
              (unwind-protect {silent_process_wait} (delete-process process)))"#
    );
    // Three seconds gives six nominal half-second ping opportunities.
    // Requiring two leaves scheduling margin at load 30–100 on 32 CPUs,
    // while checking each sample and every sent ping for liveness.
    for (name, body) in [
        ("accept-process-output", accept),
        ("sleep-for", timed_wait("(sleep-for 3)")),
        ("read-event", timed_wait("(read-event nil nil 3)")),
        (
            "call-process",
            timed_wait(
                r#"
            (let ((exit (romacs--call-process-run '("/bin/sh" "-c" "exec sleep 3")
                                                 nil "/" "/dev/null" nil nil t (cons nil nil))))
              (unless (equal (car exit) 0) (error "call-process failed: %S" exit)))"#,
            ),
        ),
        (
            "isolate-await",
            format!(
                "
            (let ((future (isolate-spawn '(lambda () (sleep-for 3) 'done) :fresh t)))
              {})",
                timed_wait(
                    r#"(unless (eq (isolate-await future 30) 'done)
                             (error "nested isolate did not finish"))"#
                )
            ),
        ),
    ] {
        check_wait(&mut master, name, &body);
    }
}

#[expect(clippy::float_cmp, reason = "the option's default is exactly 1.0, read back from Lisp")]
fn check_wedge(name: &str, body: impl FnOnce(&str) -> String) {
    let markers = Markers::new();
    let mut master = Master::new();
    let delay: f64 = master.eval("owner-wedge-delay").parse().unwrap();
    assert_eq!(delay, 1.0);
    master.run(&format!(
        "(setq hung (owner-call owner '(lambda () {})))",
        body(&literal(&markers.ready()))
    ));
    master.await_file(&markers.ready());
    let load = load1();
    let start = Instant::now();
    let limit = Duration::from_secs_f64(delay + delay / 2.0);
    // One 10 ms pump step plus 240 ms of scheduling/observation margin on
    // the shared 32-CPU host. Measure from the first pump after entry.
    let margin = Duration::from_millis(250);
    master.until(limit + margin, name, |m| {
        match m.eval("(owner-state owner)").as_str() {
            "live" => false,
            "wedged" => true,
            state => panic!("{name}: expected live or wedged, got {state} at load1 {load}"),
        }
    });
    let elapsed = start.elapsed();
    eprintln!("[measured] {name}: wedge after {elapsed:?}, load1 {load}");
    assert!(
        elapsed <= limit + margin,
        "{name}: {elapsed:?} at load1 {load}"
    );
    assert_eq!(
        master.eval(
            "(let ((pings (plist-get (owner-describe owner) :pings)))
                        (list (> (car pings) (cdr pings)) (isolate-future-done-p hung)))"
        ),
        "(t nil)"
    );
}

#[test]
fn a_lisp_loop_and_a_native_hang_miss_the_wedge_deadline() {
    check_wedge("Lisp loop", |ready| {
        format!("(write-region \"\" nil {ready} nil 'silent) (while t)")
    });
    check_wedge("native hang", |ready| {
        format!("(romacs--test-block 12 {ready})")
    });
}

#[test]
fn preparing_zygote_retirement_preserves_its_accepted_owner() {
    let mut master = Master::bare();
    master.run(
        "(setq isolate-zygote-limit 1
               first (owner--spawn)
               pending (owner-call first '+ 1 1))",
    );
    assert_eq!(
        master.eval("(list (nth 1 (isolate-zygote nil)) (owner-state first))"),
        "(preparing starting)"
    );
    // Admission may refuse the replacement while the original is starting.
    master.run(
        "(condition-case nil (owner--spawn '(isolate))
           (isolate-process-limit nil))",
    );
    assert_eq!(
        master.eval(
            "(list (condition-case e (isolate-await pending 10)
                     (isolate-error (car e)))
                   (owner-state first))"
        ),
        "(2 live)"
    );
}

fn record_owner_events(master: &mut Master) {
    master.run(
        "(setq owner-test-events nil
               isolate-message-functions
               (list (lambda (_sender event) (push event owner-test-events))))",
    );
}

// Reap the children while still in the call, without dispatching filters
// or sentinels. The next idle turn cannot rely on a live child's pidfd.
const REAP_BEFORE_DISPATCH: &str = "
    (let ((end (+ (float-time) 10)))
      (while (and (memq 'run (mapcar #'process-status children))
                  (< (float-time) end))
        (romacs--isolate-wait 0.01)
        (romacs--proc-reap)))
    (unless (equal (mapcar #'process-status children)
                   (make-list (length children) 'exit))
      (error \"children did not exit before the idle pass\"))";

#[test]
fn idle_owner_delivers_reaped_child_sentinels_without_another_frame() {
    let mut master = Master::new();
    record_owner_events(&mut master);
    assert_eq!(
        master.eval(&format!(
            r#"
            (isolate-await
             (owner-call owner '(lambda ()
                (setq owner-output-batch 1)
                (let (children)
                  (dotimes (i 4)
                    (push (make-process
                           :name (format "p%d" i) :command '("/bin/sh" "-c" "printf x")
                           :connection-type 'pipe :noquery t
                           :filter (lambda (p _)
                                     (isolate-send 'parent (list 'filter (process-name p))))
                           :sentinel (lambda (p _)
                                       (isolate-send 'parent (list 'sentinel (process-name p)))))
                          children))
                  {REAP_BEFORE_DISPATCH}
                  'reaped)))
             15)"#,
        )),
        "reaped"
    );
    assert_eq!(
        master.eval(
            "(let ((end (+ (float-time) 3)))
               (while (and (< (length owner-test-events) 8) (< (float-time) end))
                 (romacs--isolate-pump 0.05))
               (let ((events (reverse owner-test-events)) (complete t))
                 (dotimes (i 4)
                   (let* ((name (format \"p%d\" i))
                          (child (delq nil
                                       (mapcar (lambda (e) (and (equal (nth 1 e) name) (car e)))
                                               events))))
                     (unless (equal child '(filter sentinel))
                       (setq complete nil))))
                 (list (length events) complete)))"
        ),
        "(8 t)",
        "reaped output must reach its sentinel in the real idle loop"
    );
}

fn prepare_connection_batch(master: &mut Master, port: u16, ready: &Path) {
    let ready = literal(ready);
    master.run(&format!(
        r#"(setq pending
             (owner-call owner '(lambda (port)
                (setq owner-test-observed nil)
                (let ((children
                       (list (make-process
                              :name "exited-child" :command '("/bin/sh" "-c" "printf x")
                              :connection-type 'pipe :noquery t
                              :filter (lambda (_ _) (push 'child-filter owner-test-observed))
                              :sentinel (lambda (_ _) (push 'child-end owner-test-observed))))))
                  {REAP_BEFORE_DISPATCH})
                (make-network-process :name "owner-test-client" :host "127.0.0.1"
                   :service port :family 'ipv4 :nowait t :noquery t
                   :sentinel (lambda (_ _) (push 'connected owner-test-observed)))
                (let ((end (+ (float-time) 10)) (owner-output-batch 1))
                  (while (and (not (file-exists-p {ready})) (< (float-time) end))
                    (romacs--isolate-wait 0.01))
                  (unless (file-exists-p {ready}) (error "server never accepted the connection"))
                  (romacs--isolate-run-between)
                  (list (length owner-test-observed) (reverse owner-test-observed))))
              {port}))"#,
    ));
}

#[test]
fn ready_connection_sentinel_respects_the_batch_bound() {
    let markers = Markers::new();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut master = Master::new();
    prepare_connection_batch(
        &mut master,
        listener.local_addr().unwrap().port(),
        &markers.ready(),
    );
    let mut peer = None;
    // Accept confirms the handshake without consuming the owner's pending
    // connect event. Keep the accepted socket open until the pass returns.
    master.until(
        SETUP_TIMEOUT,
        "loopback TCP handshake",
        |_| match listener.accept() {
            Ok((socket, _)) => {
                peer = Some(socket);
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(error) => panic!("accepting the test connection: {error}"),
        },
    );
    std::fs::write(markers.ready(), []).unwrap();
    let out = master.eval("(isolate-await pending 15)");
    drop(peer);
    assert!(
        out.starts_with("(1 ("),
        "expected exactly one callback in the first idle pass: {out}"
    );
}

#[test]
fn a_subnanosecond_ping_delay_cannot_become_an_immediate_wedge() {
    let mut master = Master::new();
    let out = master.eval(
        "(let* ((owner-wedge-delay 1e-100)
                (id (romacs--isolate-id (owner--isolate owner)))
                (native (condition-case nil (romacs--owner-ping id 1e-100 t)
                          (args-out-of-range 'rejected))))
           (list (romacs--owner-wedge-delay)
                 (or (eq native 'rejected)
                     (and (not (car native)) (> (nth 1 native) 0)))
                 native))",
    );
    assert!(
        out.starts_with("(1.0 t "),
        "tiny values must fall back in Lisp and be rejected or safely normalized in Rust: {out}"
    );
}

#[test]
fn a_signalling_refusal_callback_leaves_no_owner_or_zygote_pending() {
    let mut master = Master::bare();
    master.run(
        "(setq req '(romacs-owner-test-missing-feature)
               first (owner--spawn req)
               fa (owner-call first '+ 1 1)
               second (owner--spawn req)
               fb (owner-call second '+ 2 2)
               zygote (aref (gethash req romacs--isolate-zygotes) 1)
               callback-count 0)
         (isolate-on-done fb (lambda (f)
                             (setq callback-count (1+ callback-count))
                             (isolate-await f)))",
    );
    assert_eq!(
        master.eval("(list (owner-state first) (owner-state second))"),
        "(starting starting)"
    );
    let got = master.eval(
        "(let ((end (+ (float-time) 5)))
           (while (and (< (float-time) end)
                       (or (not (isolate-future-done-p fa)) (not (isolate-future-done-p fb))
                           (memq zygote romacs--isolates)))
             (condition-case nil (romacs--isolate-pump 0.05) (isolate-zygote-refused nil)))
           (list (owner-state first) (owner-state second)
                 (condition-case e (isolate-await fa 0) (isolate-error (car e)))
                 (condition-case e (isolate-await fb 0) (isolate-error (car e)))
                 (and (memq zygote romacs--isolates) t)
                 callback-count))",
    );
    assert_eq!(
        got,
        "(refused refused isolate-zygote-refused isolate-zygote-refused nil 1)"
    );
}

#[test]
fn admitting_owners_leaves_room_for_a_pooled_job() {
    let mut master = Master::bare();
    master.run(
        "(setq isolate-pool-size 1 isolate-zygote-limit 1
               isolate-process-limit 4 isolate-zygotes t admitted nil)
         (catch 'full
           (dotimes (_ 4)
             (condition-case nil
                 (let ((candidate (owner--spawn)))
                   (isolate-await (owner-call candidate 'identity t) 10)
                   (push candidate admitted))
               (isolate-process-limit (throw 'full nil)))))",
    );
    assert_eq!(master.eval("(and admitted t)"), "t");
    // Either cap owner admission or budget the pool separately. Already
    // admitted owners must stay live when the ordinary pooled job runs.
    let out = master.eval(
        "(list (condition-case e (isolate-await (isolate-spawn '+ 20 22) 10)
                     (isolate-error (car e)))
                   (mapcar #'owner-state admitted))",
    );
    assert!(
        out.starts_with("(42 "),
        "admitting owners consumed the pool's process allowance: {out}"
    );
    assert_eq!(
        master.eval("(delq 'live (mapcar #'owner-state admitted))"),
        "nil"
    );
}

#[test]
fn killing_an_unforked_job_preserves_its_zygote_and_owner() {
    let mut master = Master::new();
    master.run(
        "(setq zygote (gethash nil romacs--isolate-zygotes)
               zid (romacs--isolate-id (aref zygote 1)))
         (romacs--isolate-signal zid 19)",
    );
    let pid = master.eval("(romacs--isolate-pid zid)");
    // Wait for the kernel stop, so queuing the fork cannot race SIGSTOP.
    master.until(SETUP_TIMEOUT, "zygote SIGSTOP", |_| {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(") ")
                .is_some_and(|(_, tail)| tail.starts_with("T "))
        })
    });
    assert_eq!(
        master.eval(
            "(unwind-protect
                 (let* ((job (romacs--isolate-zygote-fork zygote))
                        (pending (romacs--isolate-submit job 'identity '(unwanted) nil nil nil))
                        (isolate-kill-timeout 0.15))
                   (when (romacs--isolate-pid (romacs--isolate-id job))
                     (error \"test job already forked\"))
                   (isolate-kill job)
                   (condition-case e (isolate-await pending 0) (isolate-error (car e))))
               (condition-case nil (romacs--isolate-signal zid 18) (error nil)))"
        ),
        "isolate-killed"
    );
    assert_eq!(
        master.eval(
            "(list (condition-case nil (romacs--isolate-pid zid) (error nil))
                   (condition-case e (isolate-await (owner-call owner '+ 1 2) 10)
                     (isolate-error (car e))))"
        ),
        format!("({pid} 3)"),
        "killing the unforked job killed or disconnected its zygote or owner"
    );
}

fn owner_survives_idle_quit(turn: &str) {
    let mut master = Master::new();
    record_owner_events(&mut master);
    assert_eq!(
        master.eval(&format!("(isolate-await (owner-call owner '{turn}) 10)")),
        "armed"
    );
    master.until(SETUP_TIMEOUT, "idle callback entry", |m| {
        m.eval("(memq 'entered owner-test-events)") != "nil"
    });
    assert_eq!(
        master.eval(
            "(condition-case e (isolate-await (owner-call owner '+ 4 5) 10)
               (isolate-error (car e)))"
        ),
        "9",
        "the idle callback quit must leave the same owner serving"
    );
}

#[test]
fn an_idle_filter_quit_ends_its_turn_without_ending_the_owner() {
    owner_survives_idle_quit(
        r#"(lambda ()
           (make-process :name "quitting-filter" :command '("/bin/sh" "-c" "printf x")
             :connection-type 'pipe :noquery t
             :filter (lambda (_ _) (isolate-send 'parent 'entered) (signal 'quit nil)))
           'armed)"#,
    );
}

#[test]
fn an_idle_timer_quit_ends_its_turn_without_ending_the_owner() {
    // The bare worker has no GNU timer library. Use its timer vector ABI
    // and a one-shot handler so the real between-call timer pass runs it.
    owner_survives_idle_quit(
        "(lambda ()
           (defun timer-event-handler (timer)
             (setq timer-list (delq timer timer-list))
             (apply (aref timer 5) (aref timer 6)))
           (setq timer-list
                 (list (vector nil 0 0 0 nil
                               (lambda () (isolate-send 'parent 'entered) (signal 'quit nil))
                               nil nil 0 nil)))
           'armed)",
    );
}

fn stop_owner(master: &mut Master) -> std::os::fd::OwnedFd {
    master.run("(setq stopped-id (romacs--isolate-id (owner--isolate owner)))");
    let pid = master
        .eval("(romacs--isolate-pid stopped-id)")
        .parse::<i32>()
        .unwrap();
    let pidfd = romacs_vm::host::pidfd_open(pid).expect("open test owner's pidfd");
    master.run("(romacs--isolate-signal stopped-id 19)");
    master.until(SETUP_TIMEOUT, "owner SIGSTOP", |_| {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(") ")
                .is_some_and(|(_, tail)| tail.starts_with("T "))
        })
    });
    pidfd
}

#[test]
fn shutdown_kills_a_stopped_owner_even_if_its_callback_signals() {
    let mut master = Master::new();
    master.run(
        "(setq isolate-kill-timeout 10
                      pending (owner-call owner '(lambda () (while t)))
                      shutdown-callbacks 0)",
    );
    let pidfd = stop_owner(&mut master);
    let outcome = master.eval(
        "(isolate-on-done pending (lambda (future)
                                    (setq shutdown-callbacks (1+ shutdown-callbacks))
                                    (isolate-await future)))
         (let ((shutdown (condition-case e (progn (isolate-kill-all) 'returned)
                           (owner-killed (car e)))))
           (list shutdown shutdown-callbacks
                 (condition-case e (isolate-await pending 0) (isolate-error (car e)))))",
    );

    // Wait on the process identity only. Pumping deadlines could send the
    // missing SIGKILL and hide a shutdown that leaves the owner stopped.
    let mut watch = [rustix::event::PollFd::new(
        &pidfd,
        rustix::event::PollFlags::IN,
    )];
    let grace = rustix::event::Timespec {
        tv_sec: 1,
        tv_nsec: 0,
    };
    let exited = rustix::event::poll(&mut watch, Some(&grace)).unwrap() == 1;
    master.run("(condition-case nil (romacs--isolate-signal stopped-id 9) (error nil))");

    assert!(
        matches!(
            outcome.as_str(),
            "(returned 1 owner-killed)" | "(owner-killed 1 owner-killed)"
        ),
        "expected one signalling completion of the killed future: {outcome}"
    );
    assert!(exited, "shutdown left the stopped owner alive: {outcome}");
}

#[test]
fn a_quitting_filter_does_not_lose_the_reaped_childs_sentinel() {
    let mut master = Master::new();
    record_owner_events(&mut master);
    assert_eq!(
        master.eval(&format!(
            r#"(isolate-await
                 (owner-call owner '(lambda ()
                    (let ((children (list
                           (make-process :name "quit-filter" :command '("/bin/sh" "-c" "printf x")
                             :connection-type 'pipe :noquery t
                             :filter (lambda (_ _) (isolate-send 'parent 'filter) (signal 'quit nil))
                             :sentinel (lambda (_ _) (isolate-send 'parent 'sentinel))))))
                      {REAP_BEFORE_DISPATCH}
                      'reaped)))
                 15)"#,
        )),
        "reaped"
    );
    let events = master.eval(
        "(let ((end (+ (float-time) 3)))
           (while (and (not (memq 'sentinel owner-test-events)) (< (float-time) end))
             (romacs--isolate-pump 0.05))
           (reverse owner-test-events))",
    );
    // Inspect the owner after the idle window. The call is deliberately
    // too late to wake a sentinel before the recorded observation.
    let follow_up = master.eval(
        "(isolate-await
          (owner-call owner '(lambda ()
              (list (mapcar #'process-name (process-list)) (romacs--proc-ticks-pending))))
          10)",
    );
    assert_eq!(
        events, "(filter sentinel)",
        "owner's later status: {follow_up}"
    );
}

#[test]
fn closing_an_owner_does_not_make_a_full_job_pool_queue_work() {
    let mut master = Master::new();
    master.until(SETUP_TIMEOUT, "zygote preparation", |m| {
        m.eval("(nth 1 (isolate-zygote nil))") == "ready"
    });
    master.run(
        "(setq isolate-zygotes nil isolate-kill-timeout 10
               isolate-process-limit (- (length romacs--isolates) (romacs--isolate-owners)))
         (defun probe-pool-admission ()
           (condition-case e
               (let ((future (isolate-spawn '+ 1 2)))
                 (list 'queued (isolate-future-done-p future) (and (isolate-future-isolate future) t)))
             (isolate-process-limit (car e))))",
    );
    assert_eq!(
        master.eval("(probe-pool-admission)"),
        "isolate-process-limit"
    );

    let _stopped = stop_owner(&mut master);
    master.run("(owner-kill owner)");
    let actual = master.eval("(list (owner-state owner) (probe-pool-admission))");
    master.run("(condition-case nil (romacs--isolate-signal stopped-id 9) (error nil))");
    assert_eq!(actual, "(closing isolate-process-limit)");
}

#[path = "support/owner_lifecycle_table.rs"]
mod table_walk;
