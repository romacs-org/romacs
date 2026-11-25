use super::{Master, REAP_BEFORE_DISPATCH, SETUP_TIMEOUT, record_owner_events};
use std::process::Command;
use std::time::{Duration, Instant};

#[path = "own_test.rs"]
mod own_test;

const REEXEC: &str = "ROMACS_OWNER_ROUND_SIX_CHILD";

fn in_own_process(name: &str, test: impl FnOnce()) {
    if std::env::var_os(REEXEC).is_some_and(|value| value == name) {
        test();
        return;
    }
    let full_name = own_test::name(module_path!(), name);
    let status = Command::new("timeout")
        .args(["--kill-after=2s", "45s"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", &full_name, "--nocapture"])
        .env(REEXEC, name)
        .status()
        .unwrap();
    assert!(status.success(), "{full_name} in its own process: {status}");
}

fn process_state(pid: i32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(") ")?.1.chars().next()
}

fn shutdown_pids(master: &mut Master) -> Vec<i32> {
    master
        .eval("(mapcar (lambda (i) (romacs--isolate-pid (romacs--isolate-id i))) romacs--isolates)")
        .trim_matches(['(', ')'])
        .split_whitespace()
        .map(|pid| {
            pid.parse()
                .expect("shutdown fixture has reported every pid")
        })
        .collect()
}

fn stopped_zygote_shutdown() {
    let mut master = Master::new();
    master.run(
        "(setq isolate-zygotes t
               zygote (aref (gethash nil romacs--isolate-zygotes) 1)
               job (isolate-spawn '(lambda () (isolate-send 'parent 'job-running)
                                                (isolate-receive 20))))",
    );
    record_owner_events(&mut master);
    master.until(SETUP_TIMEOUT, "job running", |m| {
        m.eval("(memq 'job-running owner-test-events)") != "nil"
    });
    master.until(SETUP_TIMEOUT, "all isolate pids reported", |m| {
        m.eval("(memq nil (mapcar (lambda (i) (romacs--isolate-pid (romacs--isolate-id i))) romacs--isolates))")
            == "nil"
    });
    let pids = shutdown_pids(&mut master);
    let zpid: i32 = master
        .eval("(romacs--isolate-pid (romacs--isolate-id zygote))")
        .parse()
        .unwrap();
    master.run("(romacs--isolate-signal (romacs--isolate-id zygote) 19)");
    master.until(SETUP_TIMEOUT, "zygote stopped", |_| {
        process_state(zpid) == Some('T')
    });
    assert_eq!(
        master.eval("(isolate-retire zygote) (aref zygote 3)"),
        "retired"
    );
    master.run("(isolate-kill-all)");
    let deadline = Instant::now() + Duration::from_secs(2);
    let survivors = loop {
        let alive: Vec<_> = pids
            .iter()
            .filter_map(|&pid| {
                process_state(pid)
                    .filter(|&state| state != 'Z' && state != 'X')
                    .map(|state| (pid, state))
            })
            .collect();
        if alive.is_empty() || Instant::now() >= deadline {
            break alive;
        }
        // Read native reaps without dispatching deadlines that could
        // later kill a zygote that shutdown itself missed.
        master.run("(romacs--isolate-wait 0.01)");
    };
    master.run(
        "(condition-case nil (romacs--isolate-signal (romacs--isolate-id zygote) 9) (error nil))",
    );
    assert!(
        survivors.is_empty(),
        "shutdown left processes alive (zygote {zpid}): {survivors:?}"
    );
}

#[test]
fn shutdown_kills_stopped_retired_zygote_and_its_jobs() {
    in_own_process(
        "shutdown_kills_stopped_retired_zygote_and_its_jobs",
        stopped_zygote_shutdown,
    );
}

fn stale_admission() {
    let mut master = Master::new();
    assert_eq!(
        master.eval(
            "(let* ((record (gethash nil romacs--isolate-zygotes))
                    (zygote (aref record 1))
                    (members (copy-sequence romacs--isolates))
                    (outcome
                     (let ((isolate-process-limit (length romacs--isolates))
                           (isolate-job-memory-headroom (+ isolate-job-memory-headroom 4096)))
                       (condition-case e (progn (owner--spawn) 'admitted)
                         (isolate-error (car e))))))
               (list outcome (equal members romacs--isolates)
                     (eq record (gethash nil romacs--isolate-zygotes))
                     (aref zygote 3) (owner-state owner)
                     (eq (aref (owner--isolate owner) 11) record)))"
        ),
        "(isolate-process-limit t t live live t)",
        "admission refusal must leave the existing owner and its zygote unchanged"
    );
}

#[test]
fn full_owner_admission_leaves_stale_zygote_untouched() {
    in_own_process(
        "full_owner_admission_leaves_stale_zygote_untouched",
        stale_admission,
    );
}

pub(super) fn initial_call(master: &mut Master) {
    master.run(&format!(
        r#"(setq owner (owner--spawn)
                 first (owner-call owner '(lambda ()
                   ;; A bare owner has no timer.el. Give the real scheduler
                   ;; a one-shot timer in GNU's representation.
                   (unless (fboundp 'timer-event-handler)
                     (defun timer-event-handler (timer)
                       (setq timer-list (delq timer timer-list))
                       (apply (aref timer 5) (aref timer 6))))
                   (let ((now (time-convert (current-time) 'list)))
                     (push (vector nil (nth 0 now) (nth 1 now) (nth 2 now) nil
                                   (lambda () (isolate-send 'parent 'timer)) nil nil
                                   (nth 3 now) nil)
                           timer-list))
                   (let ((children (list (make-process
                           :name "first-child" :command '("/bin/sh" "-c" "exit 0")
                           :connection-type 'pipe :noquery t
                           :sentinel (lambda (_p _) (isolate-send 'parent 'sentinel))))))
                     {REAP_BEFORE_DISPATCH})
                   'armed)))"#
    ));
}

#[test]
fn first_owner_call_delivers_timer_and_sentinel_without_another_frame() {
    let mut master = Master::bare();
    record_owner_events(&mut master);
    master.run("(setq owner-wedge-delay 300.0)");
    initial_call(&mut master);
    assert_eq!(master.eval("(isolate-await first 15)"), "armed");
    assert_eq!(
        initial_call_events(&mut master),
        "(t t)",
        "first call's (timer, sentinel) with no later frame"
    );
}

pub(super) fn initial_call_events(master: &mut Master) -> String {
    master.eval(
        "(let ((end (+ (float-time) 2)))
           (while (and (< (length owner-test-events) 2) (< (float-time) end))
             (romacs--isolate-pump 0.01)))
         (list (and (memq 'timer owner-test-events) t)
               (and (memq 'sentinel owner-test-events) t))",
    )
}

pub(super) fn delete_last_process_form() -> String {
    format!(
        r#"(let ((children (list (make-process
                  :name "last-child" :command '("/bin/sh" "-c" "printf x")
                  :connection-type 'pipe :noquery t
                  :filter (lambda (p _)
                            (delete-process p)
                            (isolate-send 'parent 'deleted)
                            (signal 'quit nil))))))
             {REAP_BEFORE_DISPATCH})
           (let ((quit-seen (condition-case nil
                               (progn (romacs--isolate-run-between) nil)
                             (quit t))))
             (list quit-seen (process-list)
                   (romacs--isolate-run-between) (romacs--proc-ticks-pending)))"#
    )
}

#[test]
fn deleting_last_process_and_quitting_acknowledges_the_tick() {
    let mut master = Master::new();
    record_owner_events(&mut master);
    let outcome = master.eval(&format!(
        "(isolate-await (owner-call owner '(lambda () {})) 15)",
        delete_last_process_form()
    ));
    assert_eq!(master.eval("owner-test-events"), "(deleted)");
    assert_eq!(
        outcome, "(t nil nil nil)",
        "after deleting the last process (quit seen, processes, wait timeout, tick pending)"
    );
}

fn refused_hello() {
    // Same build-identity entry point as GUI startup, with a bare worker
    // of a different build. The real protocol check sees the real hello.
    romacs_vm::isolate_set_build_id("owner-test-different-master-build");
    let mut master = Master::bare();
    master.run(include_str!("owner_lifecycle_refused_hello.el"));
    assert_eq!(
        master.eval("(list (aref future 3) (aref (isolate-future-isolate future) 10))"),
        "(pending job)",
        "a real job is queued behind the preparing zygote"
    );
    let outcome = master.eval(
        "(let ((end (+ (float-time) 2)))
           (while (and (or romacs--isolates (not (isolate-future-done-p future))) (< (float-time) end))
             (romacs--isolate-pump 0.01)))
         (list (condition-case e (isolate-await future 0) (isolate-error (car e)))
               callbacks resubmits zygotes refusal-seen)",
    );
    assert_eq!(
        outcome, "(isolate-refused 1 0 1 t)",
        "refused hello (future result, settlements, resubmits, zygotes started, refusal observed)"
    );
}

#[test]
fn refused_zygote_hello_fails_queued_job_once_without_resubmitting() {
    in_own_process(
        "refused_zygote_hello_fails_queued_job_once_without_resubmitting",
        refused_hello,
    );
}
