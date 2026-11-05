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
    // started, the job itself running on a pooled worker meanwhile.
    let out = eval(&mut vm, "(progn (push \"/nonexistent/zygote-test\" load-path)
                               (let ((f (isolate-spawn '+ 1 2)))
                                 (list (aref (isolate-future-isolate f) 10) (isolate-await f 10))))");
    assert_eq!(out, "(pool 3)");
    assert!(settles(|| !process_exists(zygote)), "retired zygote {zygote} still runs");
    let fresh = ready_zygote(&mut vm);
    assert_ne!(fresh, zygote);
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
