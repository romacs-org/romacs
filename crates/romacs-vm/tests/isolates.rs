//! Isolate lifecycle over the bare worker binary: spawn, results, lambda
//! forms with bindings, error propagation with condition ancestry,
//! messages both ways, interrupt, kill, pidfd child reporting and
//! retirement, no zombie and no orphan, a refused worker, the world
//! refusal.
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
    // An interrupt for a job not yet started cancels it before it runs.
    assert_eq!(eval(&mut vm, "(let* ((iso (isolate-create)) (a (isolate-spawn '(lambda () (romacs--isolate-wait 0.5) 'slow) :isolate iso)) (b (isolate-spawn 'list 'second :isolate iso))) (isolate-future-cancel b) (list (isolate-await a 5) (condition-case e (isolate-await b 5) (quit 'cancelled)) (aref b 3)))"), "(slow cancelled cancelled)");
    eval(&mut vm, "(isolate-kill-all)");
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

#[test]
fn a_worker_that_is_not_this_executable_is_refused_and_a_dead_one_reported() {
    let mut vm = vm();
    // The submission may already find the worker gone (isolate-died at the
    // spawn) or its death arrives at the await; either way the death is
    // reported once and the worker forgotten.
    assert_eq!(eval(&mut vm, "(let ((isolate-worker-program \"/bin/true\") (events nil)) (push (lambda (_iso event _data) (push event events)) isolate-error-functions) (list (condition-case e (isolate-await (isolate-spawn '+ 1 2) 5) (isolate-error (car e))) (progn (romacs--isolate-pump 0.5) events) (isolate-list)))"), "(isolate-died (isolate-died) nil)");
    assert_eq!(eval(&mut vm, "(let ((isolate-boot 'world) (isolate-world nil)) (condition-case e (isolate-spawn '+ 1 2) (isolate-world-unavailable 'unavailable)))"), "unavailable");
    assert_eq!(eval(&mut vm, "(let ((isolate-boot 'source)) (condition-case e (isolate-spawn '+ 1 2) (isolate-world-unavailable 'unavailable)))"), "unavailable");
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
    // A job that loads a feature retires its worker: the next job gets a fresh one.
    assert_eq!(eval(&mut vm, "(let ((isolate-pool-size 1)) (let ((iso (isolate-future-isolate (isolate-spawn '(lambda () (provide 'journey-feature)))))) (isolate-await (isolate-spawn 'list)) (list (aref iso 6) (eq iso (isolate-future-isolate (isolate-spawn 'list))) (length (isolate-list)))))"), "(t nil 1)");
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
    let out = eval(&mut vm, "(let ((isolate-pool-size 1)) (let* ((a (isolate-spawn '(lambda () (provide 'journey-feature-3) 'provided))) (b (isolate-spawn '(lambda () (featurep 'journey-feature-3)))) (c (isolate-spawn 'list 'third))) (list (isolate-await a) (isolate-await b 10) (isolate-await c 10) (length (isolate-list)))))");
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
