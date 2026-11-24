//! Observable rows of O1a-2-lifecycle.md rev 2, SHA-256
//! f0f29acf3f6eec68636b30701aa9a68421b8efc9c42552ada3eea6a7deffaef8.
//! Row labels are suffixed by the state when the table groups states.
//! Boot preparation refuses missing features before a child can run a
//! failing start job. On a healthy child the start reply precedes every
//! caller reply. The public transport also cannot deliver invented pongs
//! or a second death for a worker that it has already forgotten.

use super::{
    Markers, Master, PUMP, REAP_BEFORE_DISPATCH, SETUP_TIMEOUT, literal, record_owner_events,
};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::OwnedFd;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

#[derive(Default)]
struct Receipt {
    included: BTreeSet<&'static str>,
    failures: Vec<String>,
}

impl Receipt {
    fn row(&mut self, row: &'static str, seen: &str, expected: &str) {
        assert!(self.included.insert(row), "duplicate row {row}");
        if seen == expected {
            eprintln!("TABLE {row} PASS {seen}");
        } else {
            self.failures
                .push(format!("{row}: got {seen}, expected {expected}"));
            eprintln!("TABLE {row} FAIL {seen}, expected {expected}");
        }
    }

    fn check(&mut self, label: &str, seen: &str, expected: &str) {
        if seen != expected {
            self.failures
                .push(format!("{label}: got {seen}, expected {expected}"));
        }
    }

    fn scene(&mut self, name: &str, run: fn(&mut Self)) {
        if let Err(failure) = catch_unwind(AssertUnwindSafe(|| run(self))) {
            let explanation = failure
                .downcast_ref::<String>()
                .map_or("fixture panic", String::as_str);
            self.failures.push(format!("{name}: {explanation}"));
        }
    }
}

struct Walk<'a> {
    master: Master,
    pids: BTreeMap<i32, OwnedFd>,
    receipt: &'a mut Receipt,
}

impl<'a> Walk<'a> {
    fn new(receipt: &'a mut Receipt) -> Self {
        let mut master = Master::bare();
        master.run(
            "(setq walk-futures nil)
             (defun walk-notify (f)
               (let ((entry (assq f walk-futures)))
                 (setcdr entry (1+ (cdr entry)))))
             (defun walk-track (f)
               (unless (assq f walk-futures)
                 (push (cons f 0) walk-futures)
                 (isolate-on-done f #'walk-notify))
               f)
             (defun walk-spawn (&optional features)
               (let ((new (owner--spawn features)))
                 (walk-track (owner--start-future new))
                 new))
             (defun walk-call (owner function &rest args)
               (walk-track (apply #'owner-call owner function args)))
             (defun walk-await (future)
               (condition-case e (isolate-await future 10)
                 (quit 'quit) (isolate-error (car e))))
             (defun walk-closed-call (owner)
               (condition-case e (progn (owner-call owner 'identity t) 'accepted)
                 (isolate-error (car e))))
             (defun walk-pool-count ()
               (- (length romacs--isolates) (romacs--isolate-owners)))",
        );
        Self {
            master,
            pids: BTreeMap::new(),
            receipt,
        }
    }

    fn run(&mut self, form: &str) {
        self.master.run(form);
    }

    fn eval(&mut self, form: &str) -> String {
        self.master.eval(form)
    }

    fn edge(&mut self, row: &'static str, form: &str, expected: &str) {
        let seen = self.eval(form);
        self.receipt.row(row, &seen, expected);
        self.check_invariants(row);
    }

    fn check_invariants(&mut self, row: &str) {
        let settled = self.eval(
            "(let ((ok t))
               (dolist (item walk-futures ok)
                 (unless (= (cdr item) (if (isolate-future-done-p (car item)) 1 0))
                   (setq ok nil))))",
        );
        self.receipt
            .check(&format!("{row} I1 callbacks"), &settled, "t");
        let accounting = self.eval(
            "(let ((ids nil) (pool 0) (unique t))
               (dolist (i romacs--isolates)
                 (let ((id (romacs--isolate-id i)))
                   (when (memq id ids) (setq unique nil))
                   (push id ids))
                 (unless (eq (aref i 10) 'owner) (setq pool (1+ pool))))
               (and unique (= pool (walk-pool-count))))",
        );
        self.receipt
            .check(&format!("{row} I3 pool census"), &accounting, "t");
        self.watch_pids();
        // Reaping is an eventual obligation. Keeping the pidfd at every edge
        // detects descendants that disappear from the registry prematurely.
    }

    fn watch_pids(&mut self) {
        let pids = self.eval(
            "(let ((pids nil))
               (dolist (i romacs--isolates pids)
                 (let* ((id (romacs--isolate-id i)) (pid (romacs--isolate-pid id)))
                   (when pid (push pid pids))
                   (setq pids (append (romacs--isolate-children id) pids)))))",
        );
        for pid in pids
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|word| word.parse::<i32>().ok())
        {
            if let std::collections::btree_map::Entry::Vacant(entry) = self.pids.entry(pid) {
                if let Some(fd) = romacs_vm::host::pidfd_open(pid) {
                    entry.insert(fd);
                }
            }
        }
    }

    fn until(&mut self, label: &str, form: &str) {
        self.master
            .until(SETUP_TIMEOUT, label, |m| m.eval(form) == "t");
    }

    fn passive(&mut self, label: &str, form: &str) {
        let start = std::time::Instant::now();
        while self.eval(form) == "nil" {
            assert!(start.elapsed() < SETUP_TIMEOUT, "no {label}");
            self.run("(romacs--isolate-wait 0.01)");
        }
    }

    fn stop(&mut self, isolate: &str) -> OwnedFd {
        let id = format!("(romacs--isolate-id {isolate})");
        let pid = self
            .eval(&format!("(romacs--isolate-pid {id})"))
            .parse::<i32>()
            .unwrap();
        let fd = romacs_vm::host::pidfd_open(pid).expect("test pidfd");
        self.run(&format!("(romacs--isolate-signal {id} 19)"));
        self.master.until(SETUP_TIMEOUT, "kernel stop", |_| {
            std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                stat.rsplit_once(") ")
                    .is_some_and(|(_, tail)| tail.starts_with("T "))
            })
        });
        fd
    }

    fn start(&mut self) {
        self.run("(setq owner (walk-spawn) iso (owner--isolate owner))");
    }

    fn live(&mut self) {
        self.start();
        self.until("live owner", "(eq (owner-state owner) 'live)");
        self.watch_pids();
    }

    fn finish(mut self, label: &str) {
        self.watch_pids();
        self.run("(condition-case nil (isolate-kill-all) (isolate-error nil))");
        let start = std::time::Instant::now();
        while self.eval("(and romacs--isolates t)") == "t"
            && start.elapsed() < Duration::from_secs(2)
        {
            self.run("(condition-case nil (romacs--isolate-pump 0.01) (isolate-error nil))");
        }
        self.check_invariants(label);
        let complete = self.eval("(let ((ok t)) (dolist (p walk-futures ok) (unless (isolate-future-done-p (car p)) (setq ok nil))))");
        self.receipt
            .check(&format!("{label} I1 terminal futures"), &complete, "t");
        for (pid, fd) in &self.pids {
            if !exited(fd, 1) {
                self.receipt
                    .failures
                    .push(format!("{label} I2 process {pid} survived shutdown"));
                // Only a pidfd captured from this fixture is ever signalled.
                let _ = rustix::process::pidfd_send_signal(fd, rustix::process::Signal::KILL);
            }
        }
    }
}

fn exited(fd: &OwnedFd, seconds: i64) -> bool {
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    poll(
        &mut fds,
        Some(&Timespec {
            tv_sec: seconds,
            tv_nsec: 0,
        }),
    )
    .unwrap()
        != 0
}

fn start_call_retire(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.start();
    w.edge(
        "1",
        "(list (owner-state owner) (plist-get (owner-describe owner) :pid))",
        "(starting nil)",
    );
    w.edge("Z1", "(nth 1 (isolate-zygote nil))", "preparing");
    w.edge(
        "3/unforked",
        "(progn (setq a (walk-call owner '+ 40 2)) (isolate-future-done-p a))",
        "nil",
    );
    w.passive("fork report", "(plist-get (owner-describe owner) :pid)");
    w.edge(
        "2",
        "(list (owner-state owner) (integerp (plist-get (owner-describe owner) :pid)))",
        "(starting t)",
    );
    w.edge(
        "3/forked",
        "(progn (setq b (walk-call owner '+ 2 3)) (isolate-future-done-p b))",
        "nil",
    );
    w.edge(
        "4/forked",
        "(list (walk-await a) (walk-await b) (owner-state owner))",
        "(42 5 live)",
    );
    w.until("spare reported", "(and (nth 3 (isolate-zygote nil)) t)");
    w.edge(
        "Z2",
        "(list (nth 1 (isolate-zygote nil)) (integerp (nth 3 (isolate-zygote nil))))",
        "(ready t)",
    );
    w.edge("7/live", "(progn (setq err (walk-call owner 'signal 'isolate-retired '(owner-data))) (owner-state owner))", "live");
    w.edge(
        "8/live",
        "(list (walk-await err) (eq (isolate-future-isolate err) iso) (owner-state owner))",
        "(isolate-retired t live)",
    );
    w.edge(
        "10/live",
        "(list (isolate-retire iso) (owner-state owner))",
        "(t closing)",
    );
    w.edge("21/closing", "(walk-closed-call owner)", "isolate-died");
    w.edge(
        "22/closing-kill",
        "(list (owner-kill owner) (owner-state owner))",
        "(nil closing)",
    );
    w.edge(
        "22/closing-retire",
        "(list (isolate-retire iso) (owner-state owner))",
        "(nil closing)",
    );
    w.edge(
        "22/closing-interrupt",
        "(list (isolate-interrupt iso) (owner-state owner))",
        "(nil closing)",
    );
    w.until("owner retired", "(eq (owner-state owner) 'dead)");
    w.edge("12", "(owner-state owner)", "dead");
    w.edge("21/dead", "(walk-closed-call owner)", "isolate-died");
    w.edge(
        "22/dead-kill",
        "(list (owner-kill owner) (owner-state owner))",
        "(nil dead)",
    );
    w.edge(
        "22/dead-retire",
        "(list (isolate-retire iso) (owner-state owner))",
        "(nil dead)",
    );
    w.edge(
        "22/dead-interrupt",
        "(list (isolate-interrupt iso) (owner-state owner))",
        "(nil dead)",
    );
    w.finish("start/retire");
}

fn early_kill(receipt: &mut Receipt, forked: bool, row: &'static str) {
    let mut w = Walk::new(receipt);
    w.start();
    if forked {
        w.passive("forked owner", "(plist-get (owner-describe owner) :pid)");
    }
    w.run("(setq pending (walk-call owner '(lambda () (while t))))");
    assert_eq!(w.eval("(owner-state owner)"), "starting");
    let timeout = "(setq isolate-kill-timeout 0.25)";
    w.run(timeout);
    w.edge(
        row,
        "(progn (owner-kill owner) (list (owner-state owner) (walk-await pending)))",
        "(closing owner-killed)",
    );
    w.until("early owner death", "(eq (owner-state owner) 'dead)");
    w.finish("early kill");
}

fn kill_unforked(receipt: &mut Receipt) {
    early_kill(receipt, false, "9/unforked");
}
fn kill_starting(receipt: &mut Receipt) {
    early_kill(receipt, true, "9/forked");
}

fn kill_live_and_deadline(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.live();
    let fd = w.stop("iso");
    w.run("(setq isolate-kill-timeout 1.0)");
    w.run("(setq pending (walk-call owner 'identity 9))");
    w.edge("9/live", "(progn (owner-kill owner) (list (owner-state owner) (walk-await pending) (cdr (aref iso 13))))", "(closing owner-killed 15)");
    w.receipt.check(
        "9/live I4 returned before forced kill",
        if exited(&fd, 0) { "exited" } else { "stopped" },
        "stopped",
    );
    w.until("forced owner kill", "(eq (cdr (aref iso 13)) 9)");
    w.edge("11/kill", "(and (eq (cdr (aref iso 13)) 9) t)", "t");
    w.run(PUMP);
    w.edge("11/sent", "(cdr (aref iso 13))", "9");
    w.until("forced owner death", "(eq (owner-state owner) 'dead)");
    w.finish("kill deadline");
}

fn prepare_wedged(w: &mut Walk<'_>, markers: &Markers) {
    w.live();
    w.run(&format!(
        "(setq owner-wedge-delay 0.2
               blocked (walk-call owner '(lambda ()
                          (write-region \"\" nil {} nil 'silent)
                          (while (not (file-exists-p {}))) 'released)))",
        literal(&markers.ready()),
        literal(&markers.go()),
    ));
    w.master.await_file(&markers.ready());
    w.until("wedged owner", "(eq (owner-state owner) 'wedged)");
}

fn wedge_cancel_and_recover(receipt: &mut Receipt) {
    let markers = Markers::new();
    let mut w = Walk::new(receipt);
    prepare_wedged(&mut w, &markers);
    w.edge(
        "19",
        "(list (owner-state owner) (isolate-future-done-p blocked))",
        "(wedged nil)",
    );
    w.edge(
        "20a",
        "(let ((owner-wedge-delay 20.0)) (owner-state owner))",
        "wedged",
    );
    w.edge("7/wedged", "(progn (setq queued (walk-call owner 'identity 7) cancelled (walk-call owner 'identity 77)) (owner-state owner))", "wedged");
    w.edge("17/wedged", "(progn (isolate-future-cancel cancelled) (list (owner-state owner) (walk-await cancelled)))", "(wedged quit)");
    std::fs::write(markers.go(), []).unwrap();
    let results = w.eval("(list (walk-await blocked) (walk-await queued) (walk-await cancelled))");
    w.receipt.check(
        "released and cancelled calls",
        &results,
        "(released 7 quit)",
    );
    w.until("pong recovery", "(eq (owner-state owner) 'live)");
    w.edge("20", "(owner-state owner)", "live");
    w.finish("wedge recovery");
}

fn idle_wedged_completion_and_retire(receipt: &mut Receipt) {
    let markers = Markers::new();
    let mut w = Walk::new(receipt);
    w.live();
    w.run(&format!(
        r#"(setq owner-wedge-delay 0.15
                 f (walk-call owner '(lambda ()
                     (let ((children (list
                             (make-process :name "table-wedge" :command '("/bin/sh" "-c" "printf x")
                               :connection-type 'pipe :noquery t
                               :filter (lambda (_ _) (write-region "" nil {} nil 'silent)
                                         (while (not (file-exists-p {}))))))))
                       {REAP_BEFORE_DISPATCH}
                       42))))"#,
        literal(&markers.ready()),
        literal(&markers.go()),
    ));
    w.master.await_file(&markers.ready());
    let began = std::time::Instant::now();
    while w.eval("(owner-state owner)") != "wedged" {
        assert!(
            began.elapsed() < SETUP_TIMEOUT,
            "pending reply never wedged"
        );
        // Send actual due pings while leaving the completed call's reply
        // in the transport inbox. The child is now looping in its filter.
        w.run("(romacs--isolate-check-deadlines) (romacs--isolate-wait 0.01)");
    }
    assert_eq!(w.eval("(isolate-future-done-p f)"), "nil");
    w.edge(
        "8/wedged",
        "(list (walk-await f) (owner-state owner) (aref iso 5))",
        "(42 wedged nil)",
    );
    w.edge(
        "10/wedged",
        "(list (isolate-retire iso) (owner-state owner))",
        "(t closing)",
    );
    std::fs::write(markers.go(), []).unwrap();
    w.until(
        "wedged owner retired once filter returned",
        "(eq (owner-state owner) 'dead)",
    );
    w.finish("idle wedged retirement");
}

fn interrupt_wedged_then_kill(receipt: &mut Receipt) {
    let markers = Markers::new();
    let mut w = Walk::new(receipt);
    prepare_wedged(&mut w, &markers);
    w.edge(
        "16/wedged",
        "(progn (isolate-interrupt iso) (list (walk-await blocked) (aref iso 7)))",
        "(quit nil)",
    );
    w.until("interrupt recovered", "(eq (owner-state owner) 'live)");
    let fd = w.stop("iso");
    w.run("(setq pending (walk-call owner 'identity 'blocked))");
    w.until("stopped owner wedged", "(eq (owner-state owner) 'wedged)");
    w.edge(
        "9/wedged",
        "(progn (owner-kill owner) (list (owner-state owner) (walk-await pending)))",
        "(closing owner-killed)",
    );
    w.receipt.check(
        "9/wedged I4 returned before forced kill",
        if exited(&fd, 0) { "exited" } else { "stopped" },
        "stopped",
    );
    w.finish("interrupt and kill");
}

fn live_wait_cancel_interrupt(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.live();
    record_owner_events(&mut w.master);
    w.run("(setq f (walk-call owner '(lambda () (isolate-send 'parent 'entered) (isolate-receive 15))))");
    w.until(
        "call entered wait",
        "(and (memq 'entered owner-test-events) t)",
    );
    w.edge("18", "(let ((sent (car (plist-get (owner-describe owner) :pings))) (end (+ (float-time) 3)))
                    (while (and (= sent (car (plist-get (owner-describe owner) :pings))) (< (float-time) end))
                      (romacs--isolate-pump 0.02))
                    (list (owner-state owner) (> (car (plist-get (owner-describe owner) :pings)) sent)))", "(live t)");
    w.run("(setq g (walk-call owner 'identity 77))");
    w.edge(
        "17/live",
        "(progn (isolate-future-cancel g) (list (walk-await g) (owner-state owner)))",
        "(quit live)",
    );
    w.run("(isolate-send iso 'release)");
    let finished = w.eval("(walk-await f)");
    w.receipt.check("wait call released", &finished, "release");
    let markers = Markers::new();
    w.run(&format!(
        "(setq owner-wedge-delay 20.0
               f (walk-call owner '(lambda ()
                     (write-region \"\" nil {} nil 'silent) (while t))))",
        literal(&markers.ready()),
    ));
    w.master.await_file(&markers.ready());
    assert_eq!(w.eval("(owner-state owner)"), "live");
    w.edge(
        "16/live",
        "(progn (isolate-interrupt iso) (list (walk-await f) (owner-state owner) (aref iso 7)))",
        "(quit live nil)",
    );
    w.finish("live wait");
}

fn preparing_retirement(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.start();
    w.run(
        "(setq z (gethash nil romacs--isolate-zygotes) zi (aref z 1)
                 pending (walk-call owner 'identity 42))",
    );
    assert_eq!(w.eval("(aref z 3)"), "preparing");
    w.edge(
        "Z5",
        "(progn (romacs--isolate-zygote-retire z)
                            (list (gethash nil romacs--isolate-zygotes) (owner-state owner)))",
        "(nil starting)",
    );
    w.edge(
        "Z3",
        "(list (walk-await pending) (owner-state owner) (aref zi 3))",
        "(42 live retired)",
    );
    w.finish("preparing retirement");
}

fn ready_retires_matching_idle_worker(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.run("(setq isolate-zygotes nil pool (isolate-warm nil))");
    w.until(
        "warm idle pool worker",
        "(and pool (eq pool (isolate-idle-worker nil)))",
    );
    assert_eq!(w.eval("(list (aref pool 10) (aref pool 3))"), "(pool live)");
    w.run("(setq isolate-zygotes t) (isolate-zygote nil)");
    w.until("ready zygote", "(eq (nth 1 (isolate-zygote nil)) 'ready)");
    w.edge("Z2/idle-worker", "(isolate-live-p pool)", "nil");
    w.finish("ready evicts matching idle worker");
}

fn retire_zygotes(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.run("(isolate-zygote nil) (setq z (gethash nil romacs--isolate-zygotes) zi (aref z 1))");
    assert_eq!(w.eval("(aref z 3)"), "preparing");
    w.edge("Z6", "(progn (romacs--isolate-zygote-retire z)
                             (list (gethash nil romacs--isolate-zygotes) (and (memq zi romacs--isolates) t)))", "(nil nil)");
    w.run("(isolate-zygote nil) (setq z (gethash nil romacs--isolate-zygotes) zi (aref z 1))");
    w.until("ready without owners", "(eq (aref z 3) 'ready)");
    w.edge(
        "Z7/ending",
        "(progn (romacs--isolate-zygote-retire z)
                                     (list (aref zi 3) (romacs--isolate-owners zi)))",
        "(retired 0)",
    );
    w.until("retired zygote gone", "(not (memq zi romacs--isolates))");
    w.edge("Z10/ending", "(and (memq zi romacs--isolates) t)", "nil");
    w.finish("zygote retire");
}

fn lingering_deadline(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.live();
    w.run(
        "(setq z (gethash nil romacs--isolate-zygotes) zi (aref z 1)
                 isolate-kill-timeout 0.25)",
    );
    w.edge("Z7/linger", "(progn (romacs--isolate-zygote-retire z)
                                     (list (aref zi 3) (romacs--isolate-owners zi) (owner-state owner)))", "(retired 1 live)");
    w.until(
        "spare gone and pings drained",
        "(let ((pings (plist-get (owner-describe owner) :pings)))
           (and (= (length romacs--isolates) 2) (= (car pings) (cdr pings))))",
    );
    w.edge(
        "Z8",
        "(progn (romacs--isolate-pump 0) (not romacs--isolate-next-deadline))",
        "t",
    );
    w.run("(setq isolate-kill-timeout 3)");
    let _stopped = w.stop("zi");
    w.run("(setq before (and (aref zi 13) (car (aref zi 13)))) (owner-kill owner)");
    w.until("last owner reaped", "(eq (owner-state owner) 'dead)");
    w.edge(
        "Z9",
        "(list (romacs--isolate-owners zi)
                        (and (aref zi 13) (not (cdr (aref zi 13)))
                             (or (null before) (> (car (aref zi 13)) before))
                             (numberp romacs--isolate-next-deadline)))",
        "(0 t)",
    );
    w.run("(romacs--isolate-signal (romacs--isolate-id zi) 18)");
    w.finish("lingering");
}

fn refusal(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.run(
        "(setq features-required '(romacs-table-walk-missing-feature)
               owner (walk-spawn features-required) iso (owner--isolate owner)
               other (walk-spawn features-required)
               z (gethash features-required romacs--isolate-zygotes) zi (aref z 1)
               a (walk-call owner 'identity 1) b (walk-call other 'identity 2))",
    );
    assert_eq!(
        w.eval("(list (owner-state owner) (owner-state other))"),
        "(starting starting)"
    );
    w.edge(
        "6",
        "(list (walk-await a) (walk-await b) (owner-state owner) (owner-state other))",
        "(isolate-zygote-refused isolate-zygote-refused refused refused)",
    );
    w.edge("Z4/registered", "(aref z 3)", "refused");
    w.until(
        "refused processes gone",
        "(and (not (memq zi romacs--isolates)) (not (memq iso romacs--isolates)))",
    );
    w.edge(
        "14",
        "(list (owner-state owner) (plist-get (owner-describe owner) :pid))",
        "(refused nil)",
    );
    w.edge(
        "Z10/refused",
        "(list (aref z 3) (and (memq zi romacs--isolates) t))",
        "(refused nil)",
    );
    w.edge("21/refused", "(walk-closed-call owner)", "isolate-died");
    w.edge(
        "23/refused",
        "(progn (isolate-kill-all) (owner-state owner))",
        "refused",
    );
    w.finish("refusal");
}

fn kill_zygote(receipt: &mut Receipt, unforked: bool) {
    let mut w = Walk::new(receipt);
    if unforked {
        w.run("(isolate-zygote nil) (setq zi (aref (gethash nil romacs--isolate-zygotes) 1))");
        w.stop("zi");
        w.start();
        assert_eq!(w.eval("(plist-get (owner-describe owner) :pid)"), "nil");
    } else {
        w.live();
        w.run("(setq zi (aref (gethash nil romacs--isolate-zygotes) 1))");
    }
    w.run("(setq pending (walk-call owner '(lambda () (while t))))");
    w.check_invariants("before zygote kill");
    w.run("(romacs--isolate-signal (romacs--isolate-id zi) 9)");
    let seen = w.eval("(list (walk-await pending) (owner-state owner))");
    let (zygote_row, death_row) = if unforked {
        ("15/unforked", "13/unforked")
    } else {
        ("15/live", "13/live")
    };
    for row in [zygote_row, death_row] {
        w.receipt.row(row, &seen, "(isolate-died dead)");
        w.check_invariants(row);
    }
    w.until(
        "dead zygote forgotten",
        "(and (not (memq zi romacs--isolates)) (not (gethash nil romacs--isolate-zygotes)))",
    );
    w.edge(
        if unforked {
            "Z10/preparing"
        } else {
            "Z10/ready"
        },
        "(and (memq zi romacs--isolates) t)",
        "nil",
    );
    w.finish("zygote death");
}

fn zygote_death_settles_and_resubmits_jobs(receipt: &mut Receipt) {
    let markers = Markers::new();
    let mut w = Walk::new(receipt);
    w.run(
        "(setq isolate-zygotes t isolate-pool-size 4)
           (isolate-zygote nil) (setq z (gethash nil romacs--isolate-zygotes) zi (aref z 1))",
    );
    w.until(
        "ready spare for a job",
        "(integerp (nth 3 (isolate-zygote nil)))",
    );
    w.stop("zi");
    w.run(&format!(
        r#"(setq busy (walk-track (isolate-spawn '(lambda ()
                                      (write-region "" nil {} nil 'silent) (while t))))
                 unforked (walk-track (isolate-spawn 'identity 42))
                 job (isolate-future-isolate unforked))"#,
        literal(&markers.ready())
    ));
    w.master.await_file(&markers.ready());
    assert_eq!(
        w.eval("(romacs--isolate-pid (romacs--isolate-id job))"),
        "nil"
    );
    w.check_invariants("zygote with busy and unforked jobs");
    w.run("(romacs--isolate-signal (romacs--isolate-id zi) 9)");
    w.edge("Z10/busy-job", "(walk-await busy)", "isolate-died");
    w.edge(
        "Z10/unforked-job",
        "(list (walk-await unforked)
                  (not (eq (isolate-future-isolate unforked) job)))",
        "(42 t)",
    );
    w.finish("zygote death resubmits unforked job");
}

fn last_job_reaps_under_lingering_zygote(receipt: &mut Receipt) {
    let markers = Markers::new();
    let mut w = Walk::new(receipt);
    w.run(
        "(setq isolate-zygotes t isolate-pool-size 4 isolate-kill-timeout 3)
           (isolate-zygote nil) (setq z (gethash nil romacs--isolate-zygotes) zi (aref z 1))",
    );
    w.until(
        "ready spare for a job",
        "(integerp (nth 3 (isolate-zygote nil)))",
    );
    w.run(&format!(
        r#"(setq busy (walk-track (isolate-spawn '(lambda ()
                                      (write-region "" nil {} nil 'silent)
                                      (while (not (file-exists-p {}))) 42)))
                 job (isolate-future-isolate busy))"#,
        literal(&markers.ready()),
        literal(&markers.go()),
    ));
    w.master.await_file(&markers.ready());
    w.stop("zi");
    w.edge(
        "Z7/linger-job",
        "(progn (romacs--isolate-zygote-retire z)
                   (list (aref zi 3) (romacs--isolate-busy-children zi)))",
        "(retired 1)",
    );
    w.run("(setq prior (and (aref zi 13) (car (aref zi 13))))");
    std::fs::write(markers.go(), []).unwrap();
    assert_eq!(w.eval("(walk-await busy)"), "42");
    w.until(
        "last job delivered dead",
        "(not (memq job romacs--isolates))",
    );
    w.edge(
        "Z9/last-job",
        "(list (romacs--isolate-busy-children zi)
                              (and (aref zi 13) (not (cdr (aref zi 13)))
                                   (or (null prior) (> (car (aref zi 13)) prior))))",
        "(0 t)",
    );
    w.run("(romacs--isolate-signal (romacs--isolate-id zi) 18)");
    w.finish("last busy job");
}

fn zygote_death_unforked(receipt: &mut Receipt) {
    kill_zygote(receipt, true);
}
fn zygote_death_live(receipt: &mut Receipt) {
    kill_zygote(receipt, false);
}

fn kill_owned_process(receipt: &mut Receipt, shutdown: bool, row: &'static str) {
    let mut w = Walk::new(receipt);
    w.live();
    let pid = w.eval(
        r#"(walk-await (walk-call owner '(lambda ()
             (process-id (make-process :name "table-child" :command '("/bin/sh" "-c" "exec sleep 30")
                          :connection-type 'pipe :noquery t)))))"#,
    ).parse::<i32>().unwrap();
    let fd = romacs_vm::host::pidfd_open(pid).expect("test subprocess pidfd");
    w.until(
        "reported subprocess",
        &format!("(and (memq {pid} (romacs--isolate-children (romacs--isolate-id iso))) t)"),
    );
    if shutdown {
        w.run("(isolate-kill-all)");
    } else {
        w.run("(romacs--isolate-signal (romacs--isolate-id iso) 9)");
        w.until("owner death", "(eq (owner-state owner) 'dead)");
    }
    w.receipt.row(
        row,
        if exited(&fd, 1) {
            "child exited"
        } else {
            "child survived"
        },
        "child exited",
    );
    w.check_invariants(row);
    if !exited(&fd, 0) {
        let _ = rustix::process::pidfd_send_signal(&fd, rustix::process::Signal::KILL);
        exited(&fd, 1);
    }
    w.finish("owned child");
}

fn death_hup(receipt: &mut Receipt) {
    kill_owned_process(receipt, false, "13/live-direct");
}
fn shutdown_hup(receipt: &mut Receipt) {
    kill_owned_process(receipt, true, "23/live");
}

fn shutdown_callback_exit(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.live();
    w.run("(setq other (walk-spawn) oi (owner--isolate other))");
    w.until("second owner live", "(eq (owner-state other) 'live)");
    let first = w.stop("iso");
    let second = w.stop("oi");
    w.run(
        "(setq isolate-kill-timeout 10
               previous (walk-call owner 'identity 1)
               future (walk-call other 'identity 2)
               callback nil)
         (isolate-on-done future (lambda (f)
                                   (setq callback
                                         (list (walk-await f) (cdr (aref oi 13))
                                               (isolate-future-done-p previous)
                                               (cdr (aref iso 13))))
                                   (signal 'owner-killed nil)))",
    );
    w.run("(condition-case nil (isolate-kill-all) (owner-killed nil))");
    w.edge(
        "24/shutdown",
        "(list callback (walk-await future))",
        "((owner-killed 9 t 9) owner-killed)",
    );
    w.receipt.row(
        "23/callback-exit",
        if exited(&first, 1) && exited(&second, 1) {
            "both owners exited"
        } else {
            "an owner survived"
        },
        "both owners exited",
    );
    w.check_invariants("23/callback-exit");
    w.run(
        "(condition-case nil (romacs--isolate-signal (romacs--isolate-id oi) 9) (error nil))
           (condition-case nil (romacs--isolate-signal (romacs--isolate-id iso) 9) (error nil))",
    );
    w.finish("shutdown callback");
}

fn pool_full_with_closing_owner(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.live();
    w.until("spare reported", "(and (nth 3 (isolate-zygote nil)) t)");
    w.run(
        "(setq z (gethash nil romacs--isolate-zygotes) isolate-zygotes nil isolate-kill-timeout 10)
           (isolate-kill (aref z 4))
           (setq isolate-process-limit (walk-pool-count))",
    );
    let _owner = w.stop("iso");
    let count = w.eval("(walk-pool-count)");
    w.run("(owner-kill owner)");
    assert_eq!(w.eval("(owner-state owner)"), "closing");
    assert_eq!(w.eval("(walk-pool-count)"), count);
    w.edge(
        "P1/create",
        "(condition-case e (progn (isolate-create) 'admitted) (isolate-error (car e)))",
        "isolate-process-limit",
    );
    w.edge("P1/start", POOL_SPAWN, "isolate-process-limit");
    w.run("(setq isolate-zygotes t)");
    w.edge("P1/fork", POOL_SPAWN, "isolate-process-limit");
    w.run("(condition-case nil (romacs--isolate-signal (romacs--isolate-id iso) 9) (error nil))");
    w.finish("closing owner admission");
}

const POOL_SPAWN: &str = "(condition-case e
                           (let ((f (walk-track (isolate-spawn '+ 2 3))))
                             (list (isolate-future-done-p f) (and (isolate-future-isolate f) t)))
                           (isolate-error (car e)))";

fn full_owner_admission(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.live();
    w.run("(isolate-zygote '(isolate)) (setq victim (gethash '(isolate) romacs--isolate-zygotes))");
    w.until("idle unrelated zygote", "(eq (aref victim 3) 'ready)");
    w.run("(setq isolate-process-limit (length romacs--isolates))");
    w.edge(
        "P2",
        "(let ((admission (condition-case e (progn (walk-spawn) 'admitted)
                                    (isolate-process-limit (car e)))))
                    (list admission (eq victim (gethash '(isolate) romacs--isolate-zygotes))
                          (aref (aref victim 1) 3)))",
        "(isolate-process-limit t live)",
    );
    w.finish("owner admission");
}

fn reaped_process(receipt: &mut Receipt, filter_quits: bool, sentinel_quits: bool) {
    let mut w = Walk::new(receipt);
    w.live();
    record_owner_events(&mut w.master);
    let filter = if filter_quits {
        "(signal 'quit nil)"
    } else {
        "nil"
    };
    let drain = if !filter_quits && sentinel_quits {
        "(list (romacs--proc-ticks-pending)
               (romacs--isolate-run-output)
               (romacs--proc-ticks-pending) (reverse dispatches))"
    } else {
        "(romacs--proc-ticks-pending)"
    };
    let sentinel = if sentinel_quits {
        "(signal 'quit nil)"
    } else {
        "nil"
    };
    let expression = format!(
        r#"(walk-await
            (walk-call owner '(lambda ()
              (setq owner-output-batch 1 delete-exited-processes t)
              (let* ((dispatches nil)
                     (children (list
                       (make-process :name "table-child" :command '("/bin/sh" "-c" "printf x")
                           :connection-type 'pipe :noquery t
                           :filter (lambda (_ text)
                                     (push text dispatches)
                                     (isolate-send 'parent 'filter) {filter})
                           :sentinel (lambda (p _)
                                       (isolate-send 'parent 'sentinel)
                                       (isolate-send 'parent
                                                     (list 'status (process-status p)
                                                           (and (memq p (process-list)) t)))
                                       {sentinel})))))
                {REAP_BEFORE_DISPATCH}
                {drain}))))"#
    );
    let queued = w.eval(&expression);
    if !filter_quits && sentinel_quits {
        w.receipt
            .row("proc/budget-spent", &queued, r#"(t t t ("x"))"#);
    } else if !filter_quits {
        w.receipt.row("proc/clean+reap", &queued, "t");
    }
    w.check_invariants("reaped process");
    let events = w.eval(
        "(let ((end (+ (float-time) 2)))
           (while (and (< (length owner-test-events) 3) (< (float-time) end))
             (romacs--isolate-pump 0.02))
           (reverse owner-test-events))",
    );
    if filter_quits {
        w.receipt.row(
            "proc/filter-exit",
            &events,
            "(filter sentinel (status exit nil))",
        );
    } else if sentinel_quits {
        w.receipt
            .row("proc/changed+pass", &queued, r#"(t t t ("x"))"#);
        w.receipt.row(
            "proc/output-done",
            &events,
            "(filter sentinel (status exit nil))",
        );
    }
    w.check_invariants("process callbacks");
    let clean = w.eval("(walk-await (walk-call owner 'romacs--proc-ticks-pending))");
    if !filter_quits {
        let label = if sentinel_quits {
            "proc/sentinel-exit"
        } else {
            "proc/sentinel-return"
        };
        w.edge(
            label,
            "(reverse owner-test-events)",
            "(filter sentinel (status exit nil))",
        );
        w.receipt
            .check("sentinel status acknowledged", &clean, "nil");
    }
    w.finish("reaped process");
}

fn ordinary_status(receipt: &mut Receipt) {
    reaped_process(receipt, false, false);
}
fn quitting_sentinel(receipt: &mut Receipt) {
    reaped_process(receipt, false, true);
}
fn aborted_status(receipt: &mut Receipt) {
    reaped_process(receipt, true, false);
}

fn retire_stopped_owner(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.live();
    let fd = w.stop("iso");
    w.run("(setq isolate-kill-timeout 0.5)");
    assert_eq!(w.eval("(isolate-retire iso)"), "t");
    w.until("retirement SIGTERM", "(eq (cdr (aref iso 13)) 15)");
    w.edge(
        "11/retire",
        "(list (owner-state owner) (cdr (aref iso 13)))",
        "(closing 15)",
    );
    w.receipt.check(
        "11/retire I4 no synchronous reap",
        if exited(&fd, 0) { "exited" } else { "stopped" },
        "stopped",
    );
    w.until("retired owner reaped", "(eq (owner-state owner) 'dead)");
    w.finish("stopped retire");
}

fn kill_held_through_fork(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.run("(isolate-zygote nil) (setq zi (aref (gethash nil romacs--isolate-zygotes) 1))");
    w.stop("zi");
    w.start();
    assert_eq!(w.eval("(plist-get (owner-describe owner) :pid)"), "nil");
    w.run(
        "(setq f (walk-call owner '(lambda () (while t))) isolate-kill-timeout 0.1)
           (owner-kill owner)",
    );
    w.until("held SIGKILL", "(eq (cdr (aref iso 13)) 9)");
    w.edge(
        "11/held",
        "(list (walk-await f) (owner-state owner) (plist-get (owner-describe owner) :pid))",
        "(owner-killed closing nil)",
    );
    w.run("(romacs--isolate-signal (romacs--isolate-id zi) 18)");
    w.until("held kill delivered", "(eq (owner-state owner) 'dead)");
    w.finish("held owner kill");
}

fn shutdown_before_fork(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.start();
    w.run("(setq pending (walk-call owner '(lambda () (while t))))");
    assert_eq!(
        w.eval("(list (owner-state owner) (plist-get (owner-describe owner) :pid))"),
        "(starting nil)"
    );
    w.edge(
        "23/unforked",
        "(progn (isolate-kill-all) (walk-await pending))",
        "owner-killed",
    );
    w.until("shutdown owner gone", "(eq (owner-state owner) 'dead)");
    w.edge(
        "23/dead",
        "(progn (isolate-kill-all) (owner-state owner))",
        "dead",
    );
    w.finish("shutdown before fork");
}

fn full_pool_with_dying_unforked_job(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.live();
    w.until(
        "initial spare reported",
        "(and (nth 3 (isolate-zygote nil)) t)",
    );
    w.run(
        "(setq z (gethash nil romacs--isolate-zygotes) zi (aref z 1)
                 isolate-pool-size 4 isolate-zygotes t)",
    );
    w.stop("zi");
    w.run("(setq other (walk-spawn))");
    w.until(
        "other owner took the spare",
        "(eq (owner-state other) 'live)",
    );
    w.run("(setq f (walk-track (isolate-spawn 'identity 11)) target (isolate-future-isolate f))");
    assert_eq!(
        w.eval("(romacs--isolate-pid (romacs--isolate-id target))"),
        "nil"
    );
    w.run(
        "(isolate-kill target)
           (setq isolate-zygotes nil isolate-process-limit (walk-pool-count))",
    );
    w.edge(
        "P3",
        "(progn (setq next (walk-track (isolate-spawn 'identity 42)))
                          (list (walk-await f) (isolate-future-done-p next)
                                (and (isolate-future-isolate next) t)))",
        "(isolate-killed nil nil)",
    );
    w.run("(romacs--isolate-signal (romacs--isolate-id zi) 18)");
    let got = w.eval("(walk-await next)");
    w.receipt
        .check("P3 slot becomes usable after the fork report", &got, "42");
    w.finish("unforked pool slot");
}

// Stop a real spare before promoting it, so a known PID cannot get a
// start reply before the event being tested. For starting₀ stop its zygote.
fn hold_start(w: &mut Walk<'_>, forked: bool) {
    w.run("(isolate-zygote nil) (setq z (gethash nil romacs--isolate-zygotes) zi (aref z 1))");
    if forked {
        w.until(
            "forked spare",
            "(and (integerp (nth 3 (isolate-zygote nil))) t)",
        );
        w.run("(setq spare (aref z 4))");
        w.stop("spare");
    } else {
        w.stop("zi");
    }
    w.start();
    let expected = if forked {
        "(starting t)"
    } else {
        "(starting nil)"
    };
    assert_eq!(
        w.eval("(list (owner-state owner) (integerp (plist-get (owner-describe owner) :pid)))"),
        expected
    );
}

fn starting_terminal_events(receipt: &mut Receipt, forked: bool, label: &'static str) {
    let mut w = Walk::new(receipt);
    hold_start(&mut w, forked);
    w.run(
        "(setq cancelled (walk-call owner 'identity 1)
                 pending (walk-call owner 'identity 2))",
    );
    let cancel = "(progn (isolate-future-cancel cancelled)
                     (list (walk-await cancelled) (owner-state owner)
                           (isolate-future-done-p pending)))";
    w.edge(label, cancel, "(quit starting nil)");
    if forked {
        w.run("(romacs--isolate-signal (romacs--isolate-id iso) 9)");
        w.edge(
            "13/starting",
            "(list (walk-await pending) (owner-state owner))",
            "(isolate-died dead)",
        );
    } else {
        w.run("(romacs--isolate-signal (romacs--isolate-id zi) 18)");
        w.until(
            "start completed after queued cancellation",
            "(eq (owner-state owner) 'live)",
        );
        let got = w.eval("(walk-await pending)");
        w.receipt
            .check("cancel left following call runnable", &got, "2");
    }
    w.finish("starting");
}

fn cancel_starting(receipt: &mut Receipt) {
    starting_terminal_events(receipt, true, "17/starting");
}

fn cancel_unforked(receipt: &mut Receipt) {
    starting_terminal_events(receipt, false, "17/unforked");
}

fn zygote_death_during_owner_start(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    hold_start(&mut w, true);
    w.run("(setq pending (walk-call owner 'identity 2))");
    w.run("(romacs--isolate-signal (romacs--isolate-id zi) 9)");
    w.until(
        "zygote died before stopped owner",
        "(not (memq zi romacs--isolates))",
    );
    w.edge(
        "15/starting",
        "(list (owner-state owner) (isolate-future-done-p pending))",
        "(starting nil)",
    );
    w.run("(romacs--isolate-signal (romacs--isolate-id iso) 18)");
    let outcome = w.eval("(list (walk-await pending) (owner-state owner))");
    w.receipt
        .check("15/starting PDEATHSIG", &outcome, "(isolate-died dead)");
    w.finish("zygote dies during owner start");
}

fn lingering_zygote_death(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.live();
    w.run(
        "(setq z (gethash nil romacs--isolate-zygotes) zi (aref z 1))
           (romacs--isolate-zygote-retire z)
           (setq owner-wedge-delay 0.15 isolate-kill-timeout 10)",
    );
    let _stopped = w.stop("iso");
    w.run("(setq f (walk-call owner 'identity 42))");
    w.until("stopped owner wedged", "(eq (owner-state owner) 'wedged)");
    w.run("(romacs--isolate-signal (romacs--isolate-id zi) 9)");
    w.until("lingering zygote gone", "(not (memq zi romacs--isolates))");
    w.edge(
        "Z10/lingering",
        "(list (gethash nil romacs--isolate-zygotes)
                                    (and (memq zi romacs--isolates) t))",
        "(nil nil)",
    );
    w.edge(
        "15/wedged",
        "(list (owner-state owner) (isolate-future-done-p f))",
        "(wedged nil)",
    );
    w.run("(romacs--isolate-signal (romacs--isolate-id iso) 18)");
    w.edge(
        "13/wedged",
        "(list (walk-await f) (owner-state owner))",
        "(isolate-died dead)",
    );
    w.finish("lingering zygote death");
}

fn detached_preparing_death(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    hold_start(&mut w, false);
    w.run(
        "(setq pending (walk-call owner 'identity 1))
           (romacs--isolate-zygote-retire z)",
    );
    assert_eq!(
        w.eval("(list (aref z 3) (gethash nil romacs--isolate-zygotes))"),
        "(preparing nil)"
    );
    w.run("(romacs--isolate-signal (romacs--isolate-id zi) 9)");
    w.edge(
        "Z10/detached",
        "(list (walk-await pending) (owner-state owner)
                  (and (memq zi romacs--isolates) t))",
        "(isolate-died dead nil)",
    );
    w.finish("detached zygote death");
}

fn detached_refusal(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.run(
        "(setq required '(romacs-table-walk-missing-feature)
                 owner (walk-spawn required) iso (owner--isolate owner)
                 pending (walk-call owner 'identity 1)
                 z (gethash required romacs--isolate-zygotes) zi (aref z 1))
           (romacs--isolate-zygote-retire z)",
    );
    assert_eq!(
        w.eval("(list (aref z 3) (gethash required romacs--isolate-zygotes))"),
        "(preparing nil)"
    );
    w.edge(
        "Z4/detached",
        "(list (walk-await pending) (owner-state owner)
                        (gethash required romacs--isolate-zygotes))",
        "(isolate-zygote-refused refused nil)",
    );
    w.until(
        "detached refusal ended zygote",
        "(not (memq zi romacs--isolates))",
    );
    w.finish("detached refusal");
}

fn normal_callback_exit(receipt: &mut Receipt) {
    let mut w = Walk::new(receipt);
    w.live();
    w.run(
        "(setq f (walk-call owner 'identity 41) g (walk-call owner 'identity 42))
           (isolate-on-done f (lambda (_f) (signal 'quit nil)))",
    );
    w.edge(
        "24/completion",
        "(progn
              (condition-case nil (isolate-await f 10) (quit nil))
              (list (walk-await f) (walk-await g) (owner-state owner)))",
        "(41 42 live)",
    );
    w.finish("ordinary callback exit");
}

fn shutdown_held_owner(receipt: &mut Receipt, state: &'static str) {
    let mut w = Walk::new(receipt);
    if state == "starting" {
        hold_start(&mut w, true);
    } else {
        w.live();
        w.stop("iso");
    }
    let pid = w
        .eval("(plist-get (owner-describe owner) :pid)")
        .parse()
        .unwrap();
    let fd = romacs_vm::host::pidfd_open(pid).unwrap();
    w.run("(setq pending (walk-call owner 'identity 42))");
    if state == "wedged" {
        w.run("(setq owner-wedge-delay 0.15)");
        w.until("stopped owner wedged", "(eq (owner-state owner) 'wedged)");
    } else if state == "closing" {
        w.run("(setq isolate-kill-timeout 0.25) (owner-kill owner)");
    }
    assert_eq!(w.eval("(owner-state owner)"), state);
    w.run("(isolate-kill-all)");
    let seen = format!(
        "({} {})",
        w.eval("(list (walk-await pending) (cdr (aref iso 13)))"),
        if exited(&fd, 1) { "exited" } else { "survived" },
    );
    let row = match state {
        "starting" => "23/starting",
        "closing" => "23/closing",
        "wedged" => "23/wedged",
        _ => unreachable!(),
    };
    w.receipt.row(row, &seen, "((owner-killed 9) exited)");
    w.check_invariants(row);
    w.finish(row);
}

fn shutdown_starting(receipt: &mut Receipt) {
    shutdown_held_owner(receipt, "starting");
}
fn shutdown_closing(receipt: &mut Receipt) {
    shutdown_held_owner(receipt, "closing");
}
fn shutdown_wedged(receipt: &mut Receipt) {
    shutdown_held_owner(receipt, "wedged");
}

#[test]
fn walks_the_observable_owner_lifecycle_table() {
    let mut receipt = Receipt::default();
    let scenes: &[(&str, fn(&mut Receipt))] = &[
        ("start/call/retire", start_call_retire),
        ("kill before report", kill_unforked),
        ("kill while starting", kill_starting),
        ("cancel while starting", cancel_starting),
        ("cancel before fork", cancel_unforked),
        ("zygote dies during start", zygote_death_during_owner_start),
        ("kill/deadline", kill_live_and_deadline),
        ("retire stopped owner", retire_stopped_owner),
        ("kill held through fork", kill_held_through_fork),
        ("shutdown before fork", shutdown_before_fork),
        ("wedge/cancel/pong", wedge_cancel_and_recover),
        ("idle wedged retirement", idle_wedged_completion_and_retire),
        ("wedge interrupt", interrupt_wedged_then_kill),
        ("live wait", live_wait_cancel_interrupt),
        ("preparing retirement", preparing_retirement),
        ("zygote retirement", retire_zygotes),
        (
            "zygote ready evicts matching idle worker",
            ready_retires_matching_idle_worker,
        ),
        ("lingering deadlines", lingering_deadline),
        ("refusal", refusal),
        ("detached refusal", detached_refusal),
        ("detached preparing death", detached_preparing_death),
        ("lingering zygote death", lingering_zygote_death),
        ("unforked zygote death", zygote_death_unforked),
        ("live zygote death", zygote_death_live),
        (
            "zygote death with busy and unforked jobs",
            zygote_death_settles_and_resubmits_jobs,
        ),
        (
            "last job reaped under lingering zygote",
            last_job_reaps_under_lingering_zygote,
        ),
        ("death HUP", death_hup),
        ("shutdown HUP", shutdown_hup),
        ("ordinary callback exit", normal_callback_exit),
        ("shutdown callback exit", shutdown_callback_exit),
        ("shutdown starting owner", shutdown_starting),
        ("shutdown closing owner", shutdown_closing),
        ("shutdown wedged owner", shutdown_wedged),
        ("closing-owner pool admission", pool_full_with_closing_owner),
        ("full owner admission", full_owner_admission),
        (
            "full pool with dying unforked job",
            full_pool_with_dying_unforked_job,
        ),
        ("process status", ordinary_status),
        ("quit inside an acknowledged sentinel", quitting_sentinel),
        ("filter quit in status drain", aborted_status),
    ];
    for (name, run) in scenes {
        receipt.scene(name, *run);
    }
    eprintln!("TABLE WALK {} observed cases", receipt.included.len());
    assert!(
        receipt.failures.is_empty(),
        "{}",
        receipt.failures.join("\n")
    );
}
