//! Owners (lisp/vm/owner.el, ownership design O1 §2) over the bare worker
//! binary: their loop (lisp/vm/isolate-worker.el), calls, kills and their
//! zygote's end.
use romacs_vm::Vm;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(&format!("(setq isolate-worker-program {:?} isolate-pool-size 2)", env!("CARGO_BIN_EXE_romacs-vm-isolate"))).unwrap();
    vm
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(romacs_vm::Local::display).unwrap_or_else(|e| format!("ERROR {e}"))
}

/// A call sent while the owner's call waits is read by that wait, which
/// leaves it queued in Rust; no socket wakes the loop for it after, so the
/// loop takes it before it waits again.
#[test]
fn a_call_sent_during_a_wait_runs_after_it() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((owner (owner--spawn))
                                    (first (progn (isolate-await (owner-call owner '+ 1 1) 60)
                                                  (owner-call owner '(lambda () (sleep-for 1) 'slept))))
                                    (second (progn (romacs--isolate-pump 0.3) (owner-call owner '(lambda () 'second)))))
                               (list (isolate-await first 10) (isolate-await second 10)))");
    assert_eq!(out, "(slept second)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// Calls run one at a time in the order they were sent: the second, sent
/// while the first runs, finds the first's entry in the owner's own log.
#[test]
fn calls_run_in_the_order_sent() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((owner (owner--spawn)) (seen nil)
                                    (isolate-message-functions (list (lambda (_ message) (push message seen)))))
                               (isolate-await (owner-call owner '(lambda () (defvar order-log nil) (setq order-log nil))) 60)
                               (let ((first (owner-call owner '(lambda () (isolate-send 'parent 'first-running)
                                                                 (sleep-for 0.3) (push 'first order-log))))
                                     (end (+ (float-time) 10)))
                                 (while (and (not (memq 'first-running seen)) (< (float-time) end))
                                   (romacs--isolate-pump 0.05))
                                 (let* ((second (owner-call owner '(lambda () (push 'second order-log))))
                                        (log (owner-call owner '(lambda () (reverse order-log)))))
                                   (list (isolate-await log 10) (aref first 3) (aref second 3)))))");
    assert_eq!(out, "((first second) done done)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// A kill settles the owner's pending calls at once with `owner-killed`,
/// which an `isolate-killed` handler catches: their outcome is unknown.
/// The death that follows settles nothing again.
#[test]
fn a_kill_settles_pending_calls_with_owner_killed_once() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((owner (owner--spawn)) (settled 0)
                                    (call (progn (isolate-await (owner-call owner '+ 1 1) 60)
                                                 (owner-call owner '(lambda () (sleep-for 30))))))
                               (isolate-on-done call (lambda (_) (setq settled (1+ settled))))
                               (owner-kill owner)
                               (let ((state (owner-state owner))
                                     (error (condition-case e (isolate-await call 1) (isolate-killed (car e))))
                                     (end (+ (float-time) 10)))
                                 (while (and (not (eq (owner-state owner) 'dead)) (< (float-time) end))
                                   (romacs--isolate-pump 0.05))
                                 (list state error (owner-state owner) settled)))");
    assert_eq!(out, "(closing owner-killed dead 1)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// The zygote stopped, a third owner is asked for and left unforked; then
/// ENDING (a form over THIRD and ZYGOTE) runs, the zygote resumes, and
/// FINALLY (a form over THIRD and FUTURE, a call sent to it) is evaluated
/// until it is not `wait`: (UNFORKED RESULT), UNFORKED whether THIRD had
/// no pid at ENDING.
fn with_an_unforked_owner(vm: &mut Vm, ending: &str, finally: &str) -> String {
    eval(vm, &format!("(let* ((isolate-kill-timeout 0.08) (first (owner--spawn))
                                (record (gethash nil romacs--isolate-zygotes)) (zygote (aref record 1))
                                (id (romacs--isolate-id zygote)) (end (+ (float-time) 20)) (out 'wait))
                           (isolate-await (owner-call first '+ 1 1) 60)
                           (while (not (and (aref record 4) (romacs--isolate-pid (romacs--isolate-id (aref record 4)))))
                             (romacs--isolate-pump 0.05))
                           (romacs--isolate-signal id 19)
                           (unwind-protect
                               (progn
                                 (owner--spawn)
                                 (let* ((third (owner--spawn))
                                        (future (owner-call third '(lambda () 'ran)))
                                        (unforked (null (romacs--isolate-pid (romacs--isolate-id (owner--isolate third))))))
                                   {ending}
                                   (dotimes (_ 6) (romacs--isolate-pump 0.04))
                                   (romacs--isolate-signal id 18)
                                   (while (and (eq out 'wait) (< (float-time) end))
                                     (romacs--isolate-pump 0.05)
                                     (setq out {finally}))
                                   (list unforked out)))
                             (romacs--isolate-signal id 18)))"))
}

/// An owner killed before its zygote reported it is signalled once it is,
/// whatever both deadlines did meanwhile: it ends.
#[test]
fn an_owner_killed_before_its_fork_is_reported_ends_once_it_is() {
    let mut vm = vm();
    let out = with_an_unforked_owner(&mut vm, "(owner-kill third)",
                                     "(if (eq (owner-state third) 'dead) (list 'dead (aref future 3)) 'wait)");
    assert_eq!(out, "(t (dead owner-killed))");
    eval(&mut vm, "(isolate-kill-all)");
}

/// A zygote retired behind a fork it had not read yet forks that child
/// first: the owner lives and runs its call.
#[test]
fn a_zygote_retired_behind_an_owners_fork_forks_it_first() {
    let mut vm = vm();
    let out = with_an_unforked_owner(&mut vm, "(isolate-retire zygote)",
                                     "(if (isolate-future-done-p future) (list (owner-state third) (isolate-await future)) 'wait)");
    assert_eq!(out, "(t (live ran))");
    eval(&mut vm, "(isolate-kill-all)");
}

/// An owner whose zygote dies dies with it: a call it was running settles
/// with `isolate-died`, not as a quit.
#[test]
fn an_owners_call_settles_with_isolate_died_when_its_zygote_dies() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((owner (owner--spawn)) (seen nil)
                                    (isolate-message-functions (list (lambda (_ message) (push message seen))))
                                    (end (+ (float-time) 10)))
                               (isolate-await (owner-call owner '+ 1 1) 60)
                               (let ((call (owner-call owner '(lambda () (isolate-send 'parent 'looping) (while t))))
                                     (zygote (aref (gethash nil romacs--isolate-zygotes) 1)))
                                 (while (and (not (memq 'looping seen)) (< (float-time) end))
                                   (romacs--isolate-pump 0.05))
                                 (romacs--isolate-signal (romacs--isolate-id zygote) 9)
                                 (condition-case e (isolate-await call 10) (isolate-error (car e)) (quit 'quit))))");
    assert_eq!(out, "isolate-died");
    eval(&mut vm, "(isolate-kill-all)");
}

/// An owner's call that signals `isolate-retired` is its error, never a
/// worker's retirement: it is not run again elsewhere.
#[test]
fn an_owners_isolate_retired_error_is_its_calls_error() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((owner (owner--spawn))
                                    (future (owner-call owner '(lambda () (if romacs--isolate-owner (signal 'isolate-retired '(owner-data))
                                                                         'ran-outside-owner)))))
                               (list (condition-case err (isolate-await future 10) (isolate-error err))
                                     (eq (isolate-future-isolate future) (owner--isolate owner))))");
    assert_eq!(out, "((isolate-retired owner-data) t)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// One batch of the loop runs at most `owner-output-batch` filter and
/// sentinel calls, however many processes ended at once; the rest run in
/// later batches, each process's output before its sentinel.
#[test]
fn a_batch_runs_at_most_owner_output_batch_calls() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let ((owner (owner--spawn)))
                               (isolate-await
                                (owner-call owner '(lambda ()
                                  (defvar dispatched nil)
                                  (dotimes (i 24)
                                    (make-process :name (format \"p%d\" i) :command '(\"/bin/sh\" \"-c\" \"printf x\")
                                                  :connection-type 'pipe :noquery t
                                                  :filter (lambda (p _) (push (cons 'output (process-name p)) dispatched))
                                                  :sentinel (lambda (p _) (push (cons 'sentinel (process-name p)) dispatched))))
                                  (let ((end (+ (float-time) 1))) (while (< (float-time) end) (romacs--isolate-wait 0.05)))
                                  (setq dispatched nil)
                                  (let ((owner-output-batch 16)) (romacs--isolate-run-between))
                                  (let ((one (length dispatched)) (end (+ (float-time) 10)))
                                    (while (and (process-list) (< (float-time) end)) (romacs--isolate-run-between))
                                    (list one (length dispatched)
                                          (let ((ordered t) (ended nil))
                                            (dolist (entry (reverse dispatched) ordered)
                                              (if (eq (car entry) 'sentinel)
                                                  (push (cdr entry) ended)
                                                (when (member (cdr entry) ended) (setq ordered nil)))))))))
                                30))");
    assert_eq!(out, "(16 48 t)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// An owner never serves without the features it was asked for: a zygote
/// whose preparation failed is refused, the owner asked of it meanwhile is
/// `refused` and its calls settle with the reason, and the next ask is
/// refused at once.
#[test]
fn an_owner_whose_features_fail_to_load_is_refused() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((owner (owner--spawn '(owner-test-absent-feature)))
                                    (call (owner-call owner '+ 1 1))
                                    (error (condition-case e (isolate-await call 30) (isolate-error e)))
                                    (again (condition-case e (progn (owner--spawn '(owner-test-absent-feature)) 'spawned)
                                             (isolate-zygote-refused (car e)))))
                               (list (car error) (and (string-match \"owner-test-absent-feature\" (format \"%S\" (cdr error))) t)
                                     (owner-state owner) (car (plist-get (owner-describe owner) :error)) again))");
    assert_eq!(out, "(isolate-zygote-refused t refused isolate-zygote-refused isolate-zygote-refused)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// A zygote still preparing, retired for another's room, forks the owner
/// asked of it before it ends: the owner lives and runs its call.
#[test]
fn a_preparing_zygote_retired_keeps_the_owner_asked_of_it() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((isolate-zygote-limit 1) (a (owner--spawn)) (fa (owner-call a '+ 1 1))
                                    (preparing (aref (gethash nil romacs--isolate-zygotes) 3)))
                               (owner--spawn '(isolate))
                               (list preparing (condition-case e (isolate-await fa 30) (isolate-error (car e))) (owner-state a)))");
    assert_eq!(out, "(preparing 2 live)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// An idle owner runs every filter and sentinel a batch of one leaves
/// behind, with nothing else to wake it: a reaped process's sentinel has
/// no descriptor of its own.
#[test]
fn an_idle_owner_runs_the_sentinels_a_spent_batch_left() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((owner (owner--spawn)) (seen nil)
                                    (isolate-message-functions (list (lambda (_ message) (push message seen))))
                                    (end (+ (float-time) 15)))
                               (isolate-await
                                (owner-call owner '(lambda ()
                                  (setq owner-output-batch 1)
                                  (dotimes (i 4)
                                    (make-process :name (format \"p%d\" i) :command '(\"/bin/sh\" \"-c\" \"printf x\")
                                                  :connection-type 'pipe :noquery t
                                                  :filter (lambda (p _) (isolate-send 'parent 'output))
                                                  :sentinel (lambda (p _) (isolate-send 'parent 'sentinel))))
                                  (let ((end (+ (float-time) 0.5))) (while (< (float-time) end) (romacs--isolate-wait 0.025)))
                                  t))
                                10)
                               (while (and (< (length seen) 8) (< (float-time) end)) (romacs--isolate-pump 0.05))
                               (list (length seen) (let ((n 0)) (dolist (m seen n) (when (eq m 'sentinel) (setq n (1+ n)))))))");
    assert_eq!(out, "(8 4)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// A batch of one that a status notification spent runs no connection
/// sentinel in the same pass.
#[test]
fn a_spent_batch_ends_its_pass_before_select() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((server (make-network-process :name \"server\" :server t :host \"127.0.0.1\" :service t
                                                                  :family 'ipv4 :noquery t))
                                    (port (process-contact server :service))
                                    (owner (owner--spawn)))
                               (prog1
                                   (isolate-await
                                    (owner-call owner '(lambda (port)
                                      (defvar observed nil)
                                      (make-process :name \"child\" :command '(\"/bin/sh\" \"-c\" \"printf x\")
                                                    :connection-type 'pipe :noquery t
                                                    :filter (lambda (_ _) (push 'child-output observed))
                                                    :sentinel (lambda (_ _) (push 'child-end observed)))
                                      (let ((end (+ (float-time) 0.2))) (while (< (float-time) end) (romacs--isolate-wait 0.01)))
                                      (romacs--proc-select nil nil 0 nil)
                                      (make-network-process :name \"client\" :host \"127.0.0.1\" :service port :family 'ipv4
                                                            :nowait t :noquery t :sentinel (lambda (_ _) (push 'connected observed)))
                                      (let ((owner-output-batch 1)) (romacs--isolate-run-between))
                                      (length observed))
                                                port)
                                    10)
                                 (delete-process server)))");
    assert_eq!(out, "1");
    eval(&mut vm, "(isolate-kill-all)");
}

/// A refusal whose callback signals still settles every owner asked of
/// the zygote, and ends the zygote.
#[test]
fn a_refusal_callback_that_signals_strands_no_owner() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((require '(owner-test-absent-feature))
                                    (a (owner--spawn require)) (fa (owner-call a '+ 1 1))
                                    (b (owner--spawn require)) (fb (owner-call b '+ 2 2))
                                    (zygote (aref (gethash require romacs--isolate-zygotes) 1)))
                               (isolate-on-done fb (lambda (f) (isolate-await f)))
                               (let ((end (+ (float-time) 15)))
                                 (while (and (or (eq (aref fa 3) 'pending) (eq (aref fb 3) 'pending)) (< (float-time) end))
                                   (condition-case nil (romacs--isolate-pump 0.05) (isolate-error nil))))
                               (list (owner-state a) (aref fa 3) (owner-state b) (aref fb 3) (isolate-live-p zygote)))");
    assert_eq!(out, "(refused error refused error nil)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// The wedge delay and the batch take their defaults when set out of
/// range, a tiny positive delay too.
#[test]
fn owner_options_out_of_range_act_as_their_defaults() {
    let mut vm = vm();
    let out = eval(&mut vm, "(list (let ((owner-wedge-delay 1e-100)) (romacs--owner-wedge-delay))
                                   (let ((owner-wedge-delay 0.0e+NaN)) (romacs--owner-wedge-delay))
                                   (let ((owner-wedge-delay 0.01)) (romacs--owner-wedge-delay))
                                   (let ((owner-output-batch 0)) (romacs--owner-output-batch)))");
    assert_eq!(out, "(1.0 1.0 0.01 16)");
}

/// Killing a job child its zygote has not reported yet waits for nothing
/// and spares the zygote and its owners.
#[test]
fn killing_an_unforked_job_child_spares_its_zygote_and_owners() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let* ((owner (owner--spawn)) (record (gethash nil romacs--isolate-zygotes))
                                    (id (romacs--isolate-id (aref record 1))))
                               (isolate-await (owner-call owner '+ 1 1) 60)
                               (romacs--isolate-signal id 19)
                               (let ((killed (unwind-protect
                                                 (let* ((child (romacs--isolate-zygote-fork record))
                                                        (f (romacs--isolate-submit child '+ '(2 3) nil nil nil))
                                                        (start (float-time)))
                                                   (isolate-kill child)
                                                   (list (< (- (float-time) start) 0.5)
                                                         (condition-case e (isolate-await f 1) (isolate-killed (car e)))))
                                               (romacs--isolate-signal id 18))))
                                 (list killed (isolate-await (owner-call owner '+ 2 2) 10) (owner-state owner))))");
    assert_eq!(out, "((t isolate-killed) 4 live)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// A quit or an error a filter or sentinel lets out between calls ends
/// that turn, never the owner.
#[test]
fn a_quit_in_a_filter_between_calls_spares_the_owner() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let ((owner (owner--spawn)))
                               (isolate-await (owner-call owner '(lambda ()
                                                (make-process :name \"q\" :command '(\"/bin/sh\" \"-c\" \"printf x\")
                                                              :connection-type 'pipe :noquery t
                                                              :filter (lambda (_ _) (signal 'quit nil))
                                                              :sentinel (lambda (_ _) (signal 'quit nil)))
                                                t))
                                              10)
                               (dotimes (_ 10) (romacs--isolate-pump 0.05))
                               (list (isolate-await (owner-call owner '+ 2 2) 10) (owner-state owner)))");
    assert_eq!(out, "(4 live)");
    eval(&mut vm, "(isolate-kill-all)");
}

/// Owners count against `isolate-process-limit` among themselves, never
/// against the pool's room: owners at the limit leave pooled jobs running.
#[test]
fn owners_at_the_process_limit_leave_pooled_jobs_room() {
    let mut vm = vm();
    let out = eval(&mut vm, "(let ((isolate-process-limit 4) (owners 0) (refused nil))
                               (while (and (not refused) (< owners 10))
                                 (condition-case nil (progn (owner--spawn) (setq owners (1+ owners)))
                                   (isolate-process-limit (setq refused t))))
                               (list refused (> owners 0) (isolate-await (isolate-spawn '+ 1 2) 30)))");
    assert_eq!(out, "(t t 3)");
    eval(&mut vm, "(isolate-kill-all)");
}
