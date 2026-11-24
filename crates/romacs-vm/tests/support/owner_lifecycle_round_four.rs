use super::{Markers, Master, REAP_BEFORE_DISPATCH, SETUP_TIMEOUT, literal, record_owner_events};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

#[test]
fn shutdown_settles_reported_and_unreported_jobs_without_resubmitting() {
    let mut master = Master::bare();
    master.run(
        "(setq isolate-zygotes t isolate-pool-size 4 isolate-kill-timeout 0.2)
                (isolate-zygote nil)",
    );
    master.until(SETUP_TIMEOUT, "ready spare", |m| {
        m.eval("(integerp (nth 3 (isolate-zygote nil)))") == "t"
    });
    master.run(
        "(setq z (gethash nil romacs--isolate-zygotes) spare (aref z 4)
                      zid (romacs--isolate-id (aref z 1))
                      zpid (romacs--isolate-pid zid))
                (romacs--isolate-signal zid 19)",
    );
    master.until(
        SETUP_TIMEOUT,
        "zygote stopped before replacement fork",
        |m| m.eval("(cdr (assq 'state (process-attributes zpid)))") == r#""T""#,
    );
    master.run(
        "(setq began (isolate-spawn '(lambda () (while t)) :isolate spare)
               held (isolate-spawn 'identity 42 :isolate (aref z 4))
               began-worker (isolate-future-isolate began)
               held-worker (isolate-future-isolate held))",
    );
    assert_eq!(
        master.eval(
            "(list (integerp (romacs--isolate-pid (romacs--isolate-id began-worker)))
               (romacs--isolate-pid (romacs--isolate-id held-worker))
               (isolate-future-done-p began) (isolate-future-done-p held))"
        ),
        "(t nil nil nil)",
        "one reported and one accepted, unreported job"
    );
    assert_eq!(
        master.eval(
            "(isolate-kill-all)
         (list (isolate-future-done-p began) (isolate-future-done-p held)
               (let ((worker (isolate-future-isolate held)))
                 (and worker (not (eq worker held-worker))))
               (mapcar (lambda (i) (aref i 10))
                       (delq nil (mapcar (lambda (i) (and (isolate-live-p i) i))
                                         (isolate-list)))))"
        ),
        "(t t nil nil)",
        "both futures settle, the held job retains its identity, and no worker is live"
    );
}

fn events_after(master: &mut Master, count: usize) -> String {
    master.eval(&format!(
        "(let ((end (+ (float-time) 2)))
           (while (and (< (length owner-test-events) {count}) (< (float-time) end))
             (romacs--isolate-pump 0.02))
           (reverse owner-test-events))"
    ))
}

#[test]
fn quitting_sentinel_keeps_the_later_reaped_child_pending() {
    let mut master = Master::new();
    record_owner_events(&mut master);
    assert_eq!(master.eval(&format!(
        r#"(isolate-await
             (owner-call owner '(lambda ()
               (setq owner-output-batch 4)
               (let ((children
                       (list (make-process :name "one" :command '("/bin/sh" "-c" "printf x")
                                           :connection-type 'pipe :noquery t)
                             (make-process :name "two" :command '("/bin/sh" "-c" "printf x")
                                           :connection-type 'pipe :noquery t))))
                 {REAP_BEFORE_DISPATCH}
                 (let* ((ordered (process-list)) (first (car ordered)) (second (nth 1 ordered)))
                   (set-process-filter first (lambda (_ _) (isolate-send 'parent 'first-filter)))
                   (set-process-filter second (lambda (_ _) (isolate-send 'parent 'second-filter)))
                   (set-process-sentinel first (lambda (_ _) (isolate-send 'parent 'first-sentinel)
                                                           (signal 'quit nil)))
                   (set-process-sentinel second (lambda (_ _) (isolate-send 'parent 'second-sentinel))))
                 'both-reaped)))
             10)"#
    )), "both-reaped");
    let idle_events = events_after(&mut master, 4);
    let follow_up = master.eval("(isolate-await (owner-call owner 'identity 'still-live) 10)");
    assert_eq!(follow_up, "still-live");
    assert_eq!(
        idle_events,
        "(first-filter first-sentinel second-filter second-sentinel)"
    );
    master.run("(romacs--isolate-pump 0.05)");
    assert_eq!(
        master.eval("(reverse owner-test-events)"),
        idle_events,
        "no sentinel replay"
    );
}

struct WorkerLink {
    files: Markers,
    path: PathBuf,
}

impl WorkerLink {
    fn new() -> Self {
        let files = Markers::new();
        let source = env!("CARGO_BIN_EXE_romacs-vm-isolate");
        fs::copy(source, files.ready()).unwrap();
        fs::copy(source, files.go()).unwrap();
        let path = files.0.join("worker");
        symlink(files.ready(), &path).unwrap();
        Self { files, path }
    }

    fn retarget(&self) {
        fs::remove_file(&self.path).unwrap();
        symlink(self.files.go(), &self.path).unwrap();
    }
}

impl Drop for WorkerLink {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn observe_owner_hello_waits(master: &mut Master) {
    // Observe the actual rejection path. Every wrapper delegates unchanged,
    // and the test enters the handler only through a real forked owner's hello.
    master.run(
        "(defvar test-owner-hello nil)
         (setq test-hello-waits nil test-hello-count 0
               test-accept (symbol-function 'romacs--isolate-accept-hello)
               test-wait (symbol-function 'romacs--isolate-wait)
               test-reap (symbol-function 'romacs--isolate-reap))
         (fset 'romacs--isolate-accept-hello
               (lambda (isolate text)
                 (let ((test-owner-hello (eq (aref isolate 10) 'owner)))
                   (when test-owner-hello (setq test-hello-count (1+ test-hello-count)))
                   (funcall test-accept isolate text))))
         (fset 'romacs--isolate-wait
               (lambda (&rest args)
                 (when test-owner-hello (push 'wait test-hello-waits))
                 (apply test-wait args)))
         (fset 'romacs--isolate-reap
               (lambda (id)
                 (when test-owner-hello (push 'reap test-hello-waits))
                 (funcall test-reap id)))",
    );
}

#[test]
fn worker_path_retarget_never_reaps_synchronously_in_owner_hello() {
    let worker = WorkerLink::new();
    let mut master = Master::bare();
    master.run(&format!(
        "(setq isolate-worker-program {} first (owner--spawn))",
        literal(&worker.path)
    ));
    assert_eq!(
        master.eval("(isolate-await (owner-call first 'identity 1) 10)"),
        "1"
    );
    observe_owner_hello_waits(&mut master);
    worker.retarget();
    master.run("(setq second (owner--spawn) third (owner--spawn))");
    let result = master.eval(
        "(mapcar (lambda (o)
                   (let ((reply (condition-case e (isolate-await (owner-call o 'identity 42) 10)
                                  (isolate-error (car e)))))
                     (list reply (owner-state o))))
                 (list second third))",
    );
    let hellos = master.eval("test-hello-count").parse::<usize>().unwrap();
    assert!(
        hellos > 0 || result.contains("refused"),
        "no owner hello or refusal: {result}"
    );
    assert_eq!(
        master.eval("(reverse test-hello-waits)"),
        "nil",
        "synchronous owner hello reap/wait; outcomes {result}"
    );
    assert_eq!(
        master.eval(
            "(let ((ok t))
           (dolist (o (list second third) ok)
             (unless (memq (owner-state o) '(live refused)) (setq ok nil))))"
        ),
        "t",
        "each owner starts or is refused: {result}"
    );
}

#[test]
fn cold_owner_refusal_leaves_the_warm_pool_worker_live() {
    let mut master = Master::bare();
    master.run("(setq isolate-zygotes nil isolate-pool-size 2 worker (isolate-warm nil))");
    master.until(SETUP_TIMEOUT, "warm ordinary worker", |m| {
        m.eval("(and worker (eq worker (isolate-idle-worker nil)))") == "t"
    });
    assert_eq!(master.eval("(isolate-zygote-list)"), "nil");
    assert_eq!(
        master.eval(
            "(setq isolate-process-limit 1)
         (let ((outcome (condition-case e (progn (owner--spawn '(isolate)) 'admitted)
                          (isolate-error (car e)))))
           (list outcome (isolate-live-p worker) (isolate-zygote-list)))"
        ),
        "(isolate-process-limit t nil)",
        "refusal leaves the ordinary worker and zygote registry alone"
    );
}

#[test]
fn spent_pool_worker_is_retired_before_result_callback() {
    let mut master = Master::bare();
    assert_eq!(
        master.eval(
            "(setq isolate-zygotes nil isolate-pool-size 1
               future (isolate-spawn '+ 1 2)
               worker (isolate-future-isolate future)
               seen nil)
         (isolate-on-done future
           (lambda (done)
             (setq seen (list (isolate-live-p worker) (isolate-pending worker)
                              (isolate-future-done-p done)))))
         (list (isolate-await future 10) seen (isolate-live-p worker))"
        ),
        "(3 (nil 0 t) nil)",
        "result callback sees the spent worker retired"
    );
}

#[test]
fn ready_output_runs_before_a_call_arriving_during_the_previous_call() {
    let mut master = Master::new();
    record_owner_events(&mut master);
    // A must wake the idle worker from its native wait. That is the path
    // whose post-wait snapshot can bypass the next output batch.
    master.until(SETUP_TIMEOUT, "owner parked in its idle wait", |m| {
        m.eval("(cdr (assq 'state (process-attributes (plist-get (owner-describe owner) :pid))))")
            == r#""S""#
    });
    master.run(&format!(
        r#"(setq a (owner-call owner '(lambda ()
                    (let ((children
                           (list (make-process :name "ready-output" :command '("/bin/sh" "-c" "printf x")
                                   :connection-type 'pipe :noquery t
                                   :filter (lambda (_ _) (isolate-send 'parent 'filter))))))
                      {REAP_BEFORE_DISPATCH}
                      (isolate-send 'parent 'a-enter)
                      (unless (eq (isolate-receive 10) 'release) (error "missing release"))
                      (isolate-send 'parent 'a-end)))))"#
    ));
    master.until(SETUP_TIMEOUT, "A waiting and output ready", |m| {
        m.eval("(equal owner-test-events '(a-enter))") == "t"
    });
    master.run(
        "(setq b (owner-call owner '(lambda () (isolate-send 'parent 'b))))
                (isolate-send (owner--isolate owner) 'release)
                (isolate-await a 10)
                (isolate-await b 10)",
    );
    assert_eq!(events_after(&mut master, 4), "(a-enter a-end filter b)");
}
