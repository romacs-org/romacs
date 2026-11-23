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
    fn new() -> Self {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let mut master = Self(vm);
        master.run(&format!(
            "(setq isolate-worker-program {:?} owner (owner--spawn))",
            env!("CARGO_BIN_EXE_romacs-vm-isolate")
        ));
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
            .and_then(|value| value.display())
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
            assert!(start.elapsed() < SETUP_TIMEOUT, "no marker at {path:?}");
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
    let cpu = thread_cpu() - cpu_start;
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
