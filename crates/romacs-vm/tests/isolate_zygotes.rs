//! Zygotes (worker-vms design §2.2-2.4) over the bare worker binary: a
//! worker prepared once that forks a child per pooled job. Each child is
//! the zygote's, runs one job with its own random stream and none of an
//! earlier job's state, and exits; a spare is forked ahead; a changed
//! preparation input retires the zygote; the kill and death paths settle
//! futures and leave no process behind.
use romacs_vm::Vm;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(&format!("(setq isolate-worker-program {:?} isolate-pool-size 4 isolate-zygotes t)", env!("CARGO_BIN_EXE_romacs-vm-isolate"))).unwrap();
    vm
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("ERROR {e}"))
}

/// The zygote for no features, ready with a forked spare: its pid.
fn ready_zygote(vm: &mut Vm) -> i64 {
    let out = eval(vm, "(progn (isolate-zygote nil)
                         (let ((end (+ (float-time) 20)))
                           (while (and (not (and (eq (nth 1 (isolate-zygote nil)) 'ready) (nth 3 (isolate-zygote nil))))
                                       (< (float-time) end))
                             (romacs--isolate-pump 0.02)))
                         (isolate-zygote nil))");
    assert!(out.starts_with("(nil ready "), "{out}");
    out.split_whitespace().nth(2).unwrap().parse().unwrap()
}

fn stat_field(pid: i64, field: usize) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the name, which ends at the last ')'.
    stat[stat.rfind(')')? + 2..].split_whitespace().nth(field - 3).map(str::to_owned)
}

fn process_exists(pid: i64) -> bool {
    stat_field(pid, 3).is_some_and(|state| state != "Z")
}

fn comm(pid: i64) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default().trim().to_owned()
}

/// A job's memory death as its awaiter catches it with `error' and prints
/// (CONDITION DATA MESSAGE): the condition, the child's budget (16 MiB in
/// these tests) as data, and the message.
fn exhausted(out: &str) -> bool {
    out.starts_with("(isolate-memory-exhausted (16777216) \"Memory exhausted: the job grew past its budget")
}

/// Lisp: wait up to 30 s, pumping, until the death of ISO (bound around it)
/// has settled and it has left `romacs--isolates'; t if it has.
const AWAIT_GONE: &str = "(let ((end (+ (float-time) 30)))
                            (while (and (memq iso romacs--isolates) (< (float-time) end)) (romacs--isolate-pump 0.1))
                            (not (memq iso romacs--isolates)))";

fn settles(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..300 {
        if check() { return true; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

fn numbers(out: &str) -> Vec<i64> {
    out.split(|c: char| !c.is_ascii_digit() && c != '-').filter_map(|n| n.parse().ok()).collect()
}

#[test]
fn pooled_jobs_run_in_children_forked_for_one_job() {
    let mut vm = vm();
    let zygote = ready_zygote(&mut vm);
    assert_eq!(comm(zygote), "romacs-zygote");
    eval(&mut vm, "(setq events nil) (push (lambda (_iso event _data) (push event events)) isolate-error-functions)");
    let job = "(let* ((f (isolate-spawn '(lambda () (prog1 (list (emacs-pid) (random 1000000000) (boundp 'job-mark)) (setq job-mark t)))))
                      (role (aref (isolate-future-isolate f) 10)))
                 (list role (isolate-await f 10)))";
    let first = eval(&mut vm, job);
    let second = eval(&mut vm, job);
    assert!(first.starts_with("(job (") && first.ends_with(" nil))"), "{first}");
    assert!(second.starts_with("(job (") && second.ends_with(" nil))"), "a job saw an earlier job's state: {second}");
    let (a, b) = (numbers(&first), numbers(&second));
    assert_ne!(a[0], b[0], "one child per job: {first} {second}");
    assert_ne!(a[1], b[1], "children share the zygote's random stream: {first} {second}");
    for child in [a[0], b[0]] {
        assert!(settles(|| !process_exists(child)), "child {child} outlived its job");
    }
    // Each child was the zygote's, named a worker; the next spare is too.
    let spare = eval(&mut vm, "(progn (romacs--isolate-pump 0.2) (nth 3 (isolate-zygote nil)))");
    let spare: i64 = spare.parse().unwrap_or_else(|_| panic!("no spare: {spare}"));
    assert_eq!(stat_field(spare, 4).as_deref(), Some(zygote.to_string().as_str()));
    assert_eq!(comm(spare), "romacs-worker");
    // A job that cannot be sent leaves the spare where it was: no child
    // is taken or forked for it.
    let out = eval(&mut vm, "(let ((spare (aref (gethash nil romacs--isolate-zygotes) 4)) (refused 0))
                               (dotimes (_ 5)
                                 (condition-case nil (isolate-spawn 'identity (make-marker)) (error (setq refused (1+ refused)))))
                               (romacs--isolate-pump 0.2)
                               (list refused (eq spare (aref (gethash nil romacs--isolate-zygotes) 4))
                                     (let ((n 0)) (dolist (i romacs--isolates n) (when (eq (aref i 10) 'job) (setq n (1+ n)))))))");
    assert_eq!(out, "(5 t 1)");
    // A child's end after its job is no news.
    assert_eq!(eval(&mut vm, "(progn (romacs--isolate-pump 0.3) events)"), "nil");
    eval(&mut vm, "(isolate-kill-all)");
    assert!(settles(|| !process_exists(zygote) && !process_exists(spare)));
}

#[test]
fn a_changed_preparation_input_retires_the_zygote() {
    let mut vm = vm();
    let zygote = ready_zygote(&mut vm);
    // The job's directory is the spawner's, not an input.
    let out = eval(&mut vm, "(let ((default-directory \"/tmp/\")) (isolate-await (isolate-spawn '(lambda () default-directory)) 10))");
    assert_eq!(out, "\"/tmp/\"");
    assert_eq!(eval(&mut vm, "(nth 2 (isolate-zygote nil))"), zygote.to_string());
    // `load-path' is: the next job finds the zygote retired and a new one
    // started, and runs on its first child.
    let out = eval(&mut vm, "(progn (push \"/nonexistent/zygote-test\" load-path)
                               (let ((f (isolate-spawn '+ 1 2)))
                                 (list (aref (isolate-future-isolate f) 10) (isolate-await f 10))))");
    assert_eq!(out, "(job 3)");
    assert!(settles(|| !process_exists(zygote)), "retired zygote {zygote} still runs");
    let fresh = ready_zygote(&mut vm);
    assert_ne!(fresh, zygote);
    // So is the budget its children are forked with.
    eval(&mut vm, "(setq isolate-job-memory-headroom (* 32 1024 1024))");
    assert_eq!(eval(&mut vm, "(isolate-await (isolate-spawn '+ 2 2) 10)"), "4");
    assert!(settles(|| !process_exists(fresh)), "zygote {fresh} kept an old budget");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn kills_and_deaths_settle_and_leave_no_process() {
    let mut vm = vm();
    let zygote = ready_zygote(&mut vm);
    // A running child killed: its future is killed, its process gone.
    let out = eval(&mut vm, "(let* ((f (isolate-spawn '(lambda () (while t)))) (iso (isolate-future-isolate f)))
                               (romacs--isolate-wait 0.3)
                               (let ((pid (romacs--isolate-pid (romacs--isolate-id iso))))
                                 (isolate-kill iso)
                                 (list pid (condition-case e (isolate-await f 5) (isolate-killed 'killed)))))");
    let child = *numbers(&out).first().unwrap_or_else(|| panic!("{out}"));
    assert!(out.ends_with(" killed)"), "{out}");
    assert!(settles(|| !process_exists(child)), "killed child {child} still runs");
    // The zygote killed while a child runs a job: the job settles as died,
    // the zygote is forgotten, the child is gone.
    let out = eval(&mut vm, "(let* ((f (isolate-spawn '(lambda () (while t)))) (iso (isolate-future-isolate f)))
                               (romacs--isolate-wait 0.3)
                               (let ((pid (romacs--isolate-pid (romacs--isolate-id iso))))
                                 (isolate-kill (aref (gethash nil romacs--isolate-zygotes) 1))
                                 (list pid (condition-case e (isolate-await f 5) (isolate-died 'died)) (gethash nil romacs--isolate-zygotes))))");
    let child = *numbers(&out).first().unwrap_or_else(|| panic!("{out}"));
    assert!(out.ends_with(" died nil)"), "{out}");
    assert!(settles(|| !process_exists(child) && !process_exists(zygote)), "child {child} or zygote {zygote} still runs");
    // A zygote that dies before forking a child that has a job: the job,
    // never started, runs elsewhere.
    ready_zygote(&mut vm);
    // (Stopped first, so it cannot fork the child before it dies.)
    let out = eval(&mut vm, "(let* ((z (gethash nil romacs--isolate-zygotes))
                                    (zid (romacs--isolate-id (aref z 1))))
                               (romacs--isolate-signal zid 19)
                               (let* ((iso (romacs--isolate-zygote-fork z))
                                      (f (romacs--isolate-submit iso '+ '(2 3) nil nil nil)))
                                 (romacs--isolate-signal zid 9)
                                 (list (isolate-await f 10) (romacs--isolate-pid (romacs--isolate-id iso)))))");
    assert_eq!(out, "(5 nil)");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn a_child_its_zygote_never_reported_runs_nothing() {
    let mut vm = vm();
    let file = std::env::temp_dir().join(format!("zygote-effect-{}", std::process::id()));
    let _ = std::fs::remove_file(&file);
    // The zygote reports its spare as usual, then stalls between forking
    // the next child and reporting it, and is killed there; that child
    // already holds a job whose effect is a line appended to FILE. The job
    // must run exactly once, elsewhere: never in the unreported child.
    let out = eval(&mut vm, &format!("(let* ((z (romacs--isolate-zygote-make nil))
                                            (_ (isolate-await (romacs--isolate-submit (aref z 1)
                                                 '(lambda ()
                                                    (let ((real (symbol-function 'romacs--isolate-release)) (n 0))
                                                      (fset 'romacs--isolate-release
                                                            (lambda (generation)
                                                              (setq n (1+ n))
                                                              (when (= n 2)
                                                                (let ((stall (+ (float-time) 20))) (while (< (float-time) stall))))
                                                              (funcall real generation))))
                                                    t)
                                                 nil nil nil nil) 10))
                                            (end (+ (float-time) 20)))
                                       (while (and (not (and (eq (aref z 3) 'ready) (aref z 4) (romacs--isolate-pid (romacs--isolate-id (aref z 4)))))
                                                   (< (float-time) end))
                                         (romacs--isolate-pump 0.02))
                                       (let* ((child (romacs--isolate-zygote-fork z))
                                              (f (romacs--isolate-submit child '(lambda () (write-region \"x\\n\" nil {file:?} t 'silent) 'wrote) nil nil nil nil))
                                              (stalled (+ (float-time) 0.5)))
                                         (while (< (float-time) stalled) (romacs--isolate-pump 0.05))
                                         (romacs--isolate-signal (romacs--isolate-id (aref z 1)) 9)
                                         (list (isolate-await f 10) (romacs--isolate-pid (romacs--isolate-id child)) (eq (isolate-future-isolate f) child))))"));
    assert_eq!(out, "(wrote nil nil)");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(std::fs::read_to_string(&file).unwrap_or_default(), "x\n", "the job ran other than once");
    let _ = std::fs::remove_file(&file);
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn a_zygotes_warm_functions_fill_what_its_children_inherit() {
    let mut vm = vm();
    // The zygote calls the warm functions of its features before it forks;
    // a child reads the value they left instead of computing it.
    let out = eval(&mut vm, "(let ((isolate-zygote-warm-functions '((isolate . (lambda () (setq zygote-warmed (emacs-pid))))
                                                                    (absent . (lambda () (setq zygote-absent t)))))
                                   (end (+ (float-time) 20)))
                               (isolate-zygote '(isolate))
                               (while (and (not (eq (nth 1 (isolate-zygote '(isolate))) 'ready)) (< (float-time) end))
                                 (romacs--isolate-pump 0.02))
                               (let ((zygote (nth 2 (isolate-zygote '(isolate))))
                                     (child (isolate-await (isolate-spawn '(lambda () (list zygote-warmed (boundp 'zygote-absent)))
                                                                          :require '(isolate)) 10)))
                                 (list (equal (car child) zygote) (nth 1 child))))");
    assert_eq!(out, "(t nil)");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn a_job_for_a_zygote_still_preparing_waits_for_it_not_the_spawner() {
    let mut vm = vm();
    // Handed out while its zygote prepares, the child is forked once the
    // zygote is ready, and runs the job then.
    let out = eval(&mut vm, "(let* ((z (romacs--isolate-zygote-make nil))
                                    (f (isolate-spawn '(lambda () (emacs-pid)))))
                               (list (aref z 3) (aref (isolate-future-isolate f) 10) (integerp (isolate-await f 20)) (aref z 3)))");
    assert_eq!(out, "(preparing job t ready)");
    eval(&mut vm, "(isolate-kill-all)");
    // A zygote refused (its preparation left a timer) never forks the
    // child: the job, never started, runs on a pooled worker instead.
    let out = eval(&mut vm, "(let* ((isolate-zygote-warm-functions '((isolate . (lambda () (setq timer-list (cons 'zygote-test-timer timer-list))))))
                                    (z (romacs--isolate-zygote-make '(isolate)))
                                    (f (isolate-spawn '(lambda () 'ran) :require '(isolate)))
                                    (child (isolate-future-isolate f))
                                    (result (isolate-await f 20)))
                               (list (aref child 10) result (aref z 3) (aref (isolate-future-isolate f) 10)))");
    assert_eq!(out, "(job ran refused pool)");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn a_job_collects_only_past_its_budget_and_live_growth_past_it_is_memory_full() {
    let mut vm = vm();
    // 1 M objects at the collector's 16 bytes each.
    eval(&mut vm, "(setq isolate-job-memory-headroom (* 16 1024 1024))");
    ready_zygote(&mut vm);
    let job = |count: u32, keep: bool| format!(
        "(condition-case e
           (isolate-await
            (isolate-spawn '(lambda ()
                              (let ((i 0) kept)
                                (while (< i {count}) {action} (setq i (1+ i)))
                                (list 'done gcs-done)))) 30)
           (error (list (car e) (cdr e) (error-message-string e))))",
        action = if keep { "(setq kept (cons i kept))" } else { "(cons i i)" });
    // Garbage within the budget: no collection at all, where the default
    // pacing would run many on a heap this small.
    assert_eq!(eval(&mut vm, &job(500_000, false)), "(done 0)");
    // Garbage past it: collected, and the job goes on.
    let out = eval(&mut vm, &job(3_000_000, false));
    let collections: i64 = numbers(&out)[0];
    assert!(out.starts_with("(done ") && (1..=4).contains(&collections), "{out}");
    // Growth that survives past the budget is memory exhaustion in the job.
    let out = eval(&mut vm, &job(3_000_000, true));
    assert!(exhausted(&out), "{out}");
    // Payload counts as much as objects: strings and vectors that survive
    // collections exhaust the budget too, across several of them.
    for item in ["(make-string 1000000 ?x)", "(make-vector 100000 0)", "(make-bool-vector 8000000 nil)"] {
        let out = eval(&mut vm, &format!("(condition-case e
                                            (isolate-await (isolate-spawn '(lambda ()
                                                                             (let ((i 0) kept)
                                                                               (while (< i 100) (setq kept (cons {item} kept)) (setq i (1+ i)))
                                                                               'survived))) 30)
                                            (error (list (car e) (cdr e) (error-message-string e))))"));
        assert!(exhausted(&out), "{item}: {out}");
    }
    // The same inside protected bodies, whose normal end enters a cleanup:
    // wherever the collection runs that finds the excess, the child ends.
    let out = eval(&mut vm, "(condition-case e
                               (isolate-await (isolate-spawn '(lambda ()
                                                                (let ((i 0) (cleaned 0) kept)
                                                                  (while (< i 100)
                                                                    (unwind-protect (setq kept (cons (make-string 1000000 ?x) kept))
                                                                      (setq cleaned (1+ cleaned)))
                                                                    (setq i (1+ i)))
                                                                  'survived))) 30)
                               (error (list (car e) (cdr e) (error-message-string e))))");
    assert!(exhausted(&out), "protected bodies: {out}");
    // A finite job with no back-jump at all, its growth in one protected
    // body: its end collects, before it replies.
    let out = eval(&mut vm, "(condition-case e
                               (isolate-await (isolate-spawn '(lambda ()
                                                                (unwind-protect (progn (setq jb-kept (make-string 40000000 ?x)) 'survived)
                                                                  nil))) 30)
                               (error (list (car e) (cdr e) (error-message-string e))))");
    assert!(exhausted(&out), "one protected body: {out}");
    // The job's own code never sees it: a handler, a cleanup, a handler-bind
    // callback, a loop inside a cleanup, an explicit collection, a finalizer
    // or `post-gc-hook' that grows past the budget ends the child there, and
    // nothing after (each marker file the job would write stays absent). The
    // reply is the spawner's, an ordinary error.
    let marker = std::env::temp_dir().join(format!("romacs-memory-{}", std::process::id()));
    let quoted = serde_json::to_string(&marker).unwrap();
    let grow = "(let ((i 0) kept) (while (< i 100) (setq kept (cons (make-string 1000000 ?x) kept)) (setq i (1+ i))))";
    for (name, body) in [
        ("handler and cleanup", format!("(condition-case nil (unwind-protect {grow} (write-region \"cleanup\" nil MARKER)) (error (write-region \"handler\" nil MARKER)))")),
        ("a loop inside a cleanup", format!("(condition-case nil (unwind-protect nil {grow}) (error (write-region \"handler\" nil MARKER)))")),
        ("handler-bind callback", format!("(condition-case nil (handler-bind-1 (lambda () (error \"trigger\")) '(error) (lambda (_) {grow})) (error (write-region \"handler\" nil MARKER)))")),
        ("explicit collection", "(setq jb-kept (make-string 40000000 ?x)) (garbage-collect) (write-region \"after\" nil MARKER)".to_owned()),
        ("finalizer", "(romacs--safepoint) (eval '(progn (make-finalizer (lambda () (setq jb-late (make-string 40000000 ?x)) (let ((i 0)) (while (< i 30000) (setq i (1+ i)))) (write-region \"finalizer\" nil MARKER))) nil) t) (eval '(progn (make-list 2000000 nil) nil) t)".to_owned()),
        ("post-gc-hook", "(romacs--safepoint) (setq post-gc-hook (list (lambda () (setq post-gc-hook nil jb-late (make-string 40000000 ?x)) (let ((i 0)) (while (< i 30000) (setq i (1+ i)))) (write-region \"hook\" nil MARKER)))) (eval '(progn (make-list 2000000 nil) nil) t)".to_owned()),
    ] {
        let _ = std::fs::remove_file(&marker);
        let body = body.replace("MARKER", &quoted);
        let out = eval(&mut vm, &format!("(condition-case e
                                            (isolate-await (isolate-spawn '(lambda () {body} 'finished)) 30)
                                            (error (list (car e) (cdr e) (error-message-string e))))"));
        assert!(exhausted(&out), "{name}: the spawner replies: {out}");
        assert!(!marker.exists(), "{name}: nothing of the job ran after");
    }
    // A cleanup that signals after its entry: the entry's collection
    // measured the excess, so the child ended before the cleanup's signal;
    // likewise with an unlet watcher that would signal as that escape
    // unwinds.
    let out = eval(&mut vm, "(condition-case e
                               (isolate-await (isolate-spawn '(lambda ()
                                                                (unwind-protect (setq jb-kept (make-string 40000000 ?x))
                                                                  (signal 'error '(\"cleanup failed\"))))) 30)
                               (error (list (car e) (cdr e) (error-message-string e))))");
    assert!(exhausted(&out), "a cleanup's signal past the budget: {out}");
    let out = eval(&mut vm, "(condition-case e
                               (isolate-await (isolate-spawn '(lambda ()
                                                                (eval '(progn (defvar jb-watched nil)
                                                                              (add-variable-watcher 'jb-watched (lambda (_s _n operation _w)
                                                                                                                 (when (eq operation 'unlet) (error \"unlet watcher\")))))
                                                                      t)
                                                                (eval '(let ((jb-watched 1))
                                                                         (unwind-protect (setq jb-kept (make-string 40000000 ?x))
                                                                           (signal 'error '(\"cleanup failed\"))))
                                                                      t))) 30)
                               (error (list (car e) (cdr e) (error-message-string e))))");
    assert!(exhausted(&out), "an unlet watcher on the cleanup's way out: {out}");
    // An argument past the budget ends the child too, replied as the job's error.
    let out = eval(&mut vm, "(condition-case e
                               (isolate-await (isolate-spawn '(lambda (text)
                                                                (let ((i 0)) (while (< i 200000) (setq i (1+ i))))
                                                                (length text))
                                                             (make-string (* 40 1024 1024) ?x)) 30)
                               (error (list (car e) (cdr e) (error-message-string e))))");
    assert!(exhausted(&out), "{out}");
    // The reply is a condition with the budget the child ran under as its
    // data, whatever the spawner's variable says when it is read; an
    // `error' handler catches it.
    let out = eval(&mut vm, "(let ((f (isolate-spawn '(lambda () (let ((i 0) kept) (while (< i 100) (setq kept (cons (make-string 1000000 ?x) kept)) (setq i (1+ i))))))))
                               (let ((isolate-job-memory-headroom 12345))
                                 (condition-case e (isolate-await f 30)
                                   (error (list (car e) (cdr e) (get (car e) 'error-conditions)
                                                (equal (substring (error-message-string e) 0 16) \"Memory exhausted\"))))))");
    assert_eq!(out, "(isolate-memory-exhausted (16777216) (isolate-memory-exhausted error) t)", "the child's budget, as data");
    // A long-lived worker keeps the default pacing.
    let out = eval(&mut vm, "(let ((iso (isolate-create)))
                               (prog1 (isolate-await (isolate-spawn '(lambda ()
                                                                       (let ((i 0)) (while (< i 500000) (cons i i) (setq i (1+ i))))
                                                                       (> gcs-done 0))
                                                                    :isolate iso) 30)
                                 (isolate-kill iso)))");
    assert_eq!(out, "t");
    eval(&mut vm, "(isolate-kill-all)");
}

fn signal_process(pid: i64, signal: &str) {
    assert!(std::process::Command::new("kill").arg(format!("-{signal}")).arg(pid.to_string()).status().unwrap().success());
}

/// A result accepted before a memory death stays the reply: the child grows
/// past its budget after replying, and its end, once it has settled, is a
/// retired child's (no `isolate-error-functions' report). The files and the
/// events are judged only after that death (`AWAIT_GONE').
#[test]
fn a_result_accepted_before_a_memory_death_stays_the_reply() {
    let mut vm = vm();
    eval(&mut vm, "(setq isolate-job-memory-headroom (* 16 1024 1024))");
    ready_zygote(&mut vm);
    let replied = std::env::temp_dir().join(format!("romacs-replied-{}", std::process::id()));
    let survived = std::env::temp_dir().join(format!("romacs-survived-{}", std::process::id()));
    let _ = std::fs::remove_file(&replied);
    let _ = std::fs::remove_file(&survived);
    let out = eval(&mut vm, &format!("(let ((events nil))
                                        (let* ((isolate-error-functions (list (lambda (_iso event _data) (push event events))))
                                               (f (isolate-spawn '(lambda ()
                                                                    (let ((reply (symbol-function 'romacs--isolate-reply)))
                                                                      (fset 'romacs--isolate-reply
                                                                            (lambda (&rest args)
                                                                              (apply reply args)
                                                                              (write-region \"replied\" nil {replied})
                                                                              (let ((i 0) kept) (while (< i 100) (setq kept (cons (make-string 1000000 ?x) kept)) (setq i (1+ i))))
                                                                              (write-region \"survived\" nil {survived}))))
                                                                    'finished)))
                                               (value (isolate-await f 30))
                                               (iso (isolate-future-isolate f)))
                                          (list value {AWAIT_GONE} events)))",
        replied = serde_json::to_string(&replied).unwrap(), survived = serde_json::to_string(&survived).unwrap()));
    let files = (replied.exists(), survived.exists());
    let _ = std::fs::remove_file(&replied);
    let _ = std::fs::remove_file(&survived);
    assert_eq!(out, "(finished t nil)", "the accepted result stays; the child's death settled within 30 s, and no news");
    assert_eq!(files, (true, false), "the child replied, then ended for its memory");
    eval(&mut vm, "(isolate-kill-all)");
}

/// A forked child's death settles at once, its zygote stopped: a memory
/// death through the child's own `memory' frame (the job's Memory exhausted
/// reply), a crash as `isolate-died' (its status unknown), each settled once
/// and the child gone before the zygote runs again. Killing a reported child
/// returns promptly too.
#[test]
fn a_childs_death_settles_at_once_with_its_zygote_stopped() {
    let mut vm = vm();
    eval(&mut vm, "(setq isolate-job-memory-headroom (* 16 1024 1024))");
    let zygote = ready_zygote(&mut vm);
    let reported = "(let ((end (+ (float-time) 10)) pid)
                      (while (and (null (setq pid (romacs--isolate-pid (romacs--isolate-id (isolate-future-isolate od-f))))) (< (float-time) end))
                        (romacs--isolate-pump 0.05))
                      pid)";
    for (name, body, expected) in [
        ("memory", "(let ((i 0) kept) (while (< i 100) (setq kept (cons (make-string 1000000 ?x) kept)) (setq i (1+ i))))", "isolate-memory-exhausted"),
        ("crash", "(signal-process (emacs-pid) 9)", "isolate-died"),
    ] {
        eval(&mut vm, &format!("(progn (setq od-settled 0 od-f (isolate-spawn '(lambda () (sleep-for 0.6) {body} 'survived)))
                                       (isolate-on-done od-f (lambda (_) (setq od-settled (1+ od-settled)))) t)"));
        let pid: i64 = eval(&mut vm, reported).parse().expect("the child was reported");
        signal_process(zygote, "STOP");
        let out = eval(&mut vm, "(condition-case e (isolate-await od-f 10) (error (car e)))");
        // Its frame can arrive a moment before its `_exit'.
        let gone = settles(|| !process_exists(pid));
        let settled = eval(&mut vm, "(progn (romacs--isolate-pump 0.3) od-settled)");
        signal_process(zygote, "CONT");
        assert_eq!(out, expected, "{name}");
        assert!(gone, "{name}: the child is gone before its zygote runs again");
        assert_eq!(settled, "1", "{name}: settled once");
    }
    // A reported child killed while its zygote is stopped.
    eval(&mut vm, "(setq od-f (isolate-spawn '(lambda () (while t))))");
    let _: i64 = eval(&mut vm, reported).parse().expect("the child was reported");
    signal_process(zygote, "STOP");
    let took = eval(&mut vm, "(let ((start (float-time))) (isolate-kill (isolate-future-isolate od-f)) (< (- (float-time) start) 3))");
    signal_process(zygote, "CONT");
    assert_eq!(took, "t", "isolate-kill returns promptly");
}

/// A job child's memory death ends the commands its job started: they are
/// reported to the spawner as they start, which signals their groups as for
/// any worker death.
#[test]
fn a_memory_death_ends_the_commands_its_job_started() {
    let mut vm = vm();
    eval(&mut vm, "(setq isolate-job-memory-headroom (* 16 1024 1024))");
    ready_zygote(&mut vm);
    let pidfile = std::env::temp_dir().join(format!("romacs-memory-command-{}", std::process::id()));
    let _ = std::fs::remove_file(&pidfile);
    let quoted = serde_json::to_string(&pidfile).unwrap();
    let script = serde_json::to_string(&format!("echo $$ > {}; exec sleep 60", pidfile.display())).unwrap();
    let out = eval(&mut vm, &format!("(let* ((f (isolate-spawn '(lambda ()
                                                                         (romacs--process-start (list \"sh\" \"-c\" {script}) \"\" default-directory 120)
                                                                         (let ((end (+ (float-time) 5)))
                                                                           (while (and (not (file-exists-p {quoted})) (< (float-time) end)) (sleep-for 0.05)))
                                                                         (let ((i 0) kept) (while (< i 100) (setq kept (cons (make-string 1000000 ?x) kept)) (setq i (1+ i)))))))
                                             (out (condition-case e (isolate-await f 30) (error (list (car e) (cdr e) (error-message-string e)))))
                                             (iso (isolate-future-isolate f)))
                                        (list {AWAIT_GONE} out))"));
    // The reply comes with the child's `memory' frame; its death, which
    // signals the groups, settles at a later wait.
    assert!(out.starts_with("(t ") && exhausted(&out[3..]), "{out}");
    let pid: i64 = std::fs::read_to_string(&pidfile).expect("the command started").trim().parse().unwrap();
    let ended = settles(|| !process_exists(pid));
    if !ended { let _ = std::process::Command::new("kill").arg("-9").arg(pid.to_string()).status(); }
    let _ = std::fs::remove_file(&pidfile);
    assert!(ended, "the command's group ended with the job");
}

/// An interrupt the child never read makes its death reset the socket ahead
/// of its last frames, and the memory reply still arrives: `inhibit-quit'
/// holds the frame unread while the job, running by then, grows past its
/// budget, and the spawner reads nothing until the child is gone, so it meets
/// the reset first.
#[test]
fn an_interrupted_job_past_its_budget_gets_the_memory_reply() {
    let mut vm = vm();
    eval(&mut vm, "(setq isolate-job-memory-headroom (* 16 1024 1024))");
    ready_zygote(&mut vm);
    let name = |what: &str| std::env::temp_dir().join(format!("romacs-interrupted-{what}-{}", std::process::id()));
    let (started, go) = (name("started"), name("go"));
    let _ = std::fs::remove_file(&started);
    let _ = std::fs::remove_file(&go);
    let (started_name, go_name) = (serde_json::to_string(&started).unwrap(), serde_json::to_string(&go).unwrap());
    let out = eval(&mut vm, &format!("(progn
                                        (setq ij-f (isolate-spawn '(lambda ()
                                                                     (let ((inhibit-quit t) (end (+ (float-time) 20)))
                                                                       (write-region \"started\" nil {started_name})
                                                                       (while (and (not (file-exists-p {go_name})) (< (float-time) end)) (sleep-for 0.02))
                                                                       (let ((i 0) kept) (while (< i 100) (setq kept (cons (make-string 1000000 ?x) kept)) (setq i (1+ i))))
                                                                       'survived))))
                                        (let ((end (+ (float-time) 20)) pid)
                                          (while (and (not (and (file-exists-p {started_name})
                                                                (setq pid (romacs--isolate-pid (romacs--isolate-id (isolate-future-isolate ij-f))))))
                                                      (< (float-time) end))
                                            (romacs--isolate-pump 0.02))
                                          (unless (isolate-interrupt (isolate-future-isolate ij-f)) (error \"no job to interrupt\"))
                                          (write-region \"go\" nil {go_name})
                                          pid))"));
    let pid: i64 = out.parse().unwrap_or_else(|_| panic!("{out}"));
    // The spawner's Lisp is not running, so nothing reads the socket meanwhile.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while process_exists(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!process_exists(pid), "the child ended");
    let out = eval(&mut vm, "(condition-case e (isolate-await ij-f 30) (error (car e)))");
    let _ = std::fs::remove_file(&started);
    let _ = std::fs::remove_file(&go);
    assert_eq!(out, "isolate-memory-exhausted", "the reply behind the reset");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn a_child_blocked_in_a_primitive_ends_within_seconds_of_sigterm() {
    let mut vm = vm();
    ready_zygote(&mut vm);
    // A job blocked in a primitive (the bare VM's `sleep-for' is a plain
    // sleep): no safepoint sees the termination, and no SIGKILL follows
    // here. The alarm the handler armed ends it.
    let out = eval(&mut vm, "(let* ((f (isolate-spawn '(lambda () (sleep-for 60))))
                                    (id (romacs--isolate-id (isolate-future-isolate f)))
                                    (end (+ (float-time) 10)))
                               (while (and (null (romacs--isolate-pid id)) (< (float-time) end)) (romacs--isolate-wait 0.05))
                               (let ((settle (+ (float-time) 0.3))) (while (< (float-time) settle) (romacs--isolate-wait 0.05)))
                               (romacs--isolate-signal id 15)
                               (romacs--isolate-pid id))");
    let child: i64 = out.parse().unwrap_or_else(|_| panic!("{out}"));
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(process_exists(child), "the child ended on the flag, not the alarm");
    assert!(settles(|| !process_exists(child)), "child {child} outlived its termination limit");
    eval(&mut vm, "(isolate-kill-all)");
}

/// Spawn N pooled jobs that each sleep and report (START . END): the most
/// that ran at once, and how many had no isolate when spawned.
const OVERLAP_JOBS: &str = "(let* ((futures nil) (waited 0) (i 0))
    (while (< i 6)
      (let ((f (isolate-spawn '(lambda () (let ((start (float-time))) (sleep-for 0.15) (cons start (float-time)))))))
        (unless (isolate-future-isolate f) (setq waited (1+ waited)))
        (setq futures (cons f futures)))
      (setq i (1+ i)))
    (let ((spans (mapcar (lambda (f) (isolate-await f 30)) futures)) (peak 0))
      (dolist (span spans)
        (let ((running 0))
          (dolist (other spans)
            (when (and (<= (car other) (car span)) (< (car span) (cdr other))) (setq running (1+ running))))
          (when (> running peak) (setq peak running))))
      (list peak waited (length spans))))";

#[test]
fn jobs_past_the_pools_width_wait_and_run_in_order_of_room() {
    let mut vm = vm();
    eval(&mut vm, "(setq isolate-pool-size 2)");
    ready_zygote(&mut vm);
    let out = eval(&mut vm, OVERLAP_JOBS);
    let [peak, waited, done] = numbers(&out)[..] else { panic!("{out}") };
    assert!(peak <= 2 && waited >= 3 && done == 6, "{out}");
    // A job cancelled while it waits never runs; the one behind it does.
    let out = eval(&mut vm, "(let* ((a (isolate-spawn '(lambda () (sleep-for 0.3) 'a)))
                                    (b (isolate-spawn '(lambda () (sleep-for 0.3) 'b)))
                                    (c (isolate-spawn '(lambda () 'c)))
                                    (d (isolate-spawn '(lambda () 'd))))
                               (isolate-future-cancel c)
                               (list (isolate-future-isolate c) (condition-case nil (isolate-await c 5) (quit 'cancelled))
                                     (isolate-await a 10) (isolate-await b 10) (isolate-await d 10)))");
    assert_eq!(out, "(nil cancelled a b d)");
    // A job sent back to wait leaves its old worker: cancelling it there
    // settles it, and the drain does not run it.
    let out = eval(&mut vm, "(let* ((busy (list (isolate-spawn '(lambda () (sleep-for 0.3) 'a))
                                                (isolate-spawn '(lambda () (sleep-for 0.3) 'b))))
                                    (old (isolate-future-isolate (car busy)))
                                    (f (romacs--isolate-future old 0 (freeze (list '(lambda () (setq never-ran nil)) nil nil nil nil)))))
                               (romacs--isolate-resubmit f)
                               (let ((queued (list (isolate-future-isolate f) (length romacs--isolate-waiting))))
                                 (isolate-future-cancel f)
                                 (dolist (b busy) (isolate-await b 10))
                                 (romacs--isolate-pump 0.1)
                                 (list queued (aref f 3) romacs--isolate-waiting)))");
    assert_eq!(out, "((nil 1) cancelled nil)");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn the_pools_memory_limit_holds_jobs_back_and_a_large_zygote_is_refused() {
    let mut vm = vm();
    eval(&mut vm, "(setq isolate-pool-size 8 isolate-job-memory-headroom (* 64 1024 1024))");
    ready_zygote(&mut vm);
    // Room for one reservation over what the pool holds now, and not two
    // unless the pool's PSS falls by half a reservation (32 MiB) while
    // the first runs: a few MiB of drift between samples is ordinary.
    eval(&mut vm, "(setq isolate-pool-memory-limit (+ (* 1024 (romacs--isolate-pss)) (* 96 1024 1024)))");
    let out = eval(&mut vm, OVERLAP_JOBS);
    let [peak, _, done] = numbers(&out)[..] else { panic!("{out}") };
    assert!(peak == 1 && done == 6, "{out}");
    // A limit below what the idle pool holds: one job at a time still runs.
    eval(&mut vm, "(setq isolate-pool-memory-limit 1)");
    let out = eval(&mut vm, OVERLAP_JOBS);
    let [peak, _, done] = numbers(&out)[..] else { panic!("{out}") };
    assert!(peak == 1 && done == 6, "{out}");
    eval(&mut vm, "(isolate-kill-all)");
    // Reservations are the budgets jobs were forked with, whatever the
    // setting says since.
    eval(&mut vm, "(setq isolate-pool-memory-limit (* 1024 1024 1024) isolate-job-memory-headroom (* 8 1024 1024))");
    ready_zygote(&mut vm);
    let out = eval(&mut vm, "(let ((f (isolate-spawn '(lambda () (sleep-for 0.3) t))))
                               (setq isolate-job-memory-headroom (* 1024 1024))
                               (prog1 (/ (romacs--isolate-reserved) (* 1024 1024)) (isolate-await f 10)))");
    assert_eq!(out, "8");
    eval(&mut vm, "(isolate-kill-all)");
    // A zygote over its limit: its features go to pooled workers.
    let out = eval(&mut vm, "(let ((isolate-zygote-memory-limit 1) (end (+ (float-time) 20)))
                               (setq isolate-pool-memory-limit (* 1024 1024 1024))
                               (isolate-zygote '(isolate))
                               (while (and (eq (nth 1 (isolate-zygote '(isolate))) 'preparing) (< (float-time) end))
                                 (romacs--isolate-pump 0.02))
                               (let ((f (isolate-spawn '+ 1 2 :require '(isolate))))
                                 (list (nth 1 (isolate-zygote '(isolate))) (aref (isolate-future-isolate f) 10) (isolate-await f 10))))");
    assert_eq!(out, "(refused pool 3)");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn the_pools_width_follows_the_processors_its_zygote_is_pinned_to() {
    let mut vm = vm();
    // The spare is asked for while the zygote prepares.
    eval(&mut vm, "(progn (setq isolate-pool-size nil) (romacs--isolate-zygote-make nil) (isolate-idle-worker nil))");
    let zygote = ready_zygote(&mut vm);
    let cpus = |pid: i64| std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap()
        .lines().find_map(|l| l.strip_prefix("Cpus_allowed_list:").map(|v| v.trim().to_owned())).unwrap();
    let spare: i64 = eval(&mut vm, "(nth 3 (isolate-zygote nil))").parse().unwrap();
    let wide = eval(&mut vm, "(isolate--pool-size)");
    // Pin the zygote's threads, as a CPU policy would, to two processors.
    let allowed = cpus(zygote);
    let first = allowed.split(['-', ',']).next().unwrap().parse::<u32>().unwrap();
    let pin = format!("{first},{}", first + 1);
    assert!(std::process::Command::new("taskset").args(["-a", "-c", "-p", &pin, &zygote.to_string()]).stdout(std::process::Stdio::null()).status().unwrap().success());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(eval(&mut vm, "(isolate--pool-size)"), "1", "width was {wide}");
    // The spare keeps its old processors, so the next dispatch replaces it.
    let out = eval(&mut vm, "(let ((f (isolate-spawn '(lambda () (emacs-pid))))) (isolate-await f 10))");
    let child: i64 = out.parse().unwrap_or_else(|_| panic!("{out}"));
    assert_ne!(child, spare, "the job ran on the spare from before the pin");
    let fresh: i64 = eval(&mut vm, "(progn (romacs--isolate-pump 0.3) (nth 3 (isolate-zygote nil)))").parse().unwrap();
    assert_eq!(cpus(fresh), pin.replace(',', "-"));
    // A second zygote, not pinned, keeps its own width and spare.
    let other = eval(&mut vm, "(let ((end (+ (float-time) 20)))
                                 (isolate-zygote '(isolate))
                                 (while (and (not (and (eq (nth 1 (isolate-zygote '(isolate))) 'ready) (nth 3 (isolate-zygote '(isolate)))))
                                             (< (float-time) end))
                                   (romacs--isolate-pump 0.02))
                                 (isolate--pool-size (gethash '(isolate) romacs--isolate-zygotes)))");
    assert_eq!(other, wide, "the second zygote took the first one's width");
    eval(&mut vm, "(isolate-kill-all)");
}

#[test]
fn a_waiting_job_is_judged_by_its_own_zygotes_width() {
    let mut vm = vm();
    eval(&mut vm, "(setq isolate-pool-size nil)");
    let narrow = ready_zygote(&mut vm);
    let wider: i64 = eval(&mut vm, "(let ((end (+ (float-time) 20)))
                                      (isolate-zygote '(isolate))
                                      (while (and (not (eq (nth 1 (isolate-zygote '(isolate))) 'ready)) (< (float-time) end))
                                        (romacs--isolate-pump 0.02))
                                      (nth 2 (isolate-zygote '(isolate))))").parse().unwrap();
    // The first zygote runs one job at a time, the second two.
    let pin = |cpus: &str, pid: i64| assert!(std::process::Command::new("taskset").args(["-a", "-c", "-p", cpus, &pid.to_string()])
        .stdout(std::process::Stdio::null()).status().unwrap().success());
    pin("0,1", narrow);
    pin("0,1,2", wider);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    // A short job and a long one fill the second zygote's width; the third
    // waits, and takes the short one's slot while the long one still runs.
    let out = eval(&mut vm, "(let* ((job '(lambda (seconds) (let ((start (float-time))) (sleep-for seconds) (cons start (float-time)))))
                                    (short (isolate-spawn job 0.1 :require '(isolate)))
                                    (long (isolate-spawn job 0.8 :require '(isolate)))
                                    (third (isolate-spawn job 0.1 :require '(isolate)))
                                    (waited (null (isolate-future-isolate third)))
                                    (third-span (isolate-await third 10))
                                    (long-span (isolate-await long 10)))
                               (isolate-await short 10)
                               (list waited (< (car third-span) (cdr long-span))))");
    assert_eq!(out, "(t t)");
    eval(&mut vm, "(isolate-kill-all)");
}
