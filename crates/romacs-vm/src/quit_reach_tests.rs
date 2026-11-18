//! C-g reaches each native loop (`Vm::native_poll`): a primitive Lisp calls,
//! entered with a press already counted, ends in `quit` at its first poll,
//! having changed nothing a quit there may not leave; under `inhibit-quit`
//! the same call runs to its end and the press stays pending.
//!
//! The primitive's body is called as the engine calls it once its entry poll
//! has passed (through `eval` the entry would take the press first), so each
//! test is deterministic: the poll, and nothing else, can have quit it.
use super::*;
use crate::quit::QuitSignal;
use std::sync::Arc;

/// A VM with the Lisp prelude, busy running a command, and the signal its
/// host's input thread holds.
fn busy_vm() -> (Vm, Arc<QuitSignal>) {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let signal = Arc::new(QuitSignal::new());
    vm.share_quit_signal(signal.clone());
    vm.enter_busy();
    (vm, signal)
}

fn eval(vm: &mut Vm, source: &str) -> Value {
    vm.eval_source_raw(source).unwrap_or_else(|e| panic!("{source}: {e}"))
}

/// NAME's primitive and the values ARGS (a Lisp form making a list)
/// evaluates to, made before any press: an evaluation would quit at once.
/// Nothing collects between this and the call.
fn prepare(vm: &mut Vm, name: &str, args: &str) -> (&'static PrimDesc, Vec<Value>) {
    let subr = eval(vm, &format!("(symbol-function '{name})"));
    let desc = generated(subr).unwrap_or_else(|| panic!("{name} is a generated primitive"));
    let list = eval(vm, args);
    let cx = Leaf::new(vm);
    let mut values = Vec::new();
    let mut tail = list;
    while let Some((car, cdr)) = cx.pair(tail) {
        values.push(car);
        tail = cdr;
    }
    (desc, values)
}

/// The prepared call: a `Step` body only to its first step into Lisp, past
/// its loop.
fn invoke(vm: &mut Vm, (desc, values): (&'static PrimDesc, Vec<Value>)) -> std::result::Result<Value, Fault> {
    let mut cx = Leaf::new(vm);
    match desc.body {
        Body::Leaf(f) => f(&mut cx, &values),
        Body::Call(f) => f(&mut cx, &values).map(|step| match step {
            Step::Done(value) => value,
            _ => Value::TRUE,
        }),
        Body::Reader(_) => unreachable!("no reading primitive here"),
    }
}

fn quit_signalled(vm: &mut Vm, outcome: &std::result::Result<Value, Fault>) -> bool {
    let quit = vm.intern("quit");
    matches!(outcome, Err(Fault::Signal(condition, _)) if *condition == quit)
}

/// SETUP, then NAME on ARGS with a press pending: it quits, and CHECK (a
/// Lisp form) is then t. Then, under `inhibit-quit`, it ends with a value
/// and the press waits for the next safepoint.
fn reaches(name: &str, setup: &str, args: &str, check: &str) {
    let (mut vm, signal) = busy_vm();
    eval(&mut vm, setup);
    let call = prepare(&mut vm, name, args);
    assert!(signal.interrupt(), "a busy evaluator takes the press");
    let outcome = invoke(&mut vm, call);
    let ended = match &outcome {
        Ok(_) => "a value".to_string(),
        Err(Fault::Signal(condition, _)) => format!("a signal {}", Leaf::new(&mut vm).symbol_name(*condition).unwrap_or_default()),
        Err(Fault::Error(message)) => format!("an error {message}"),
        Err(_) => "another fault".to_string(),
    };
    assert!(quit_signalled(&mut vm, &outcome), "{name} on {args}: no quit at its poll, but {ended}");
    assert!(!signal.due(), "{name}: the press was taken");
    vm.quit_unwinding = false;
    assert_eq!(vm.eval_source(check).unwrap().display().unwrap(), "t", "{name}: {check} after the quit");

    eval(&mut vm, "(setq inhibit-quit t)");
    let call = prepare(&mut vm, name, args);
    assert!(signal.interrupt());
    let outcome = invoke(&mut vm, call);
    assert!(outcome.is_ok(), "{name}: under inhibit-quit the call ends with a value");
    assert!(signal.due(), "{name}: inhibit-quit holds the press");
    eval(&mut vm, "(setq inhibit-quit nil)");
}

#[test]
fn json_parse_and_serialize_quit_between_values() {
    let setup = "(setq json-in (json-serialize (make-vector 5000 1)) json-v (make-vector 5000 1))";
    reaches("json-parse-string", setup, "(list json-in)", "t");
    reaches("json-serialize", setup, "(list json-v)", "t");
    reaches("json-parse-buffer", &format!("(progn {setup} (insert json-in) (goto-char 1))"), "nil", "(= (point) 1)");
}

#[test]
fn a_digest_quits_between_chunks() {
    let setup = "(setq big (make-string (* 3 1024 1024) ?a))";
    reaches("romacs--secure-hash", setup, "(list 'sha256 big)", "t");
    reaches("romacs--secure-hash", setup, "(list 'sha3-256 big)", "t");
    reaches("buffer-hash", &format!("(progn {setup} (insert big))"), "nil", "t");
}

#[test]
fn a_native_sort_quits_and_leaves_its_list() {
    let setup = "(setq l (let (l) (dotimes (i 20000) (push (% (* i 7919) 10007) l)) l) copy (copy-sequence l))";
    reaches("sort", setup, "(list l #'<)", "(equal l copy)");
    let strings = "(setq l (mapcar #'number-to-string (let (l) (dotimes (i 20000) (push (% (* i 7919) 10007) l)) l)) copy (copy-sequence l))";
    reaches("sort", strings, "(list l #'string-lessp)", "(equal l copy)");
}

/// A syntax table where `;` starts a comment a newline ends and `'` is an
/// expression prefix, as Lisp's are.
const SYNTAX: &str = "(let ((st (copy-syntax-table))) (modify-syntax-entry ?\\; \"<\" st) \
    (modify-syntax-entry ?\\n \">\" st) (modify-syntax-entry ?' \"'\" st) (set-syntax-table st))";

#[test]
fn syntax_scans_quit_before_point_moves() {
    let text = format!("(progn {SYNTAX} (insert \"(\" (make-string 50000 ?a) \")\") (goto-char 1))");
    reaches("scan-lists", &text, "(list 1 1 0)", "(= (point) 1)");
    reaches("scan-lists", &text, "(list (point-max) -1 0)", "(= (point) 1)");
    reaches("parse-partial-sexp", &text, "(list 1 (point-max) most-positive-fixnum)", "(= (point) 1)");
    // Lists, each step a change of depth the scanner reads one at a time.
    let lists = format!("(progn {SYNTAX} (insert \"(\") (dotimes (_ 25000) (insert \"()\")) (insert \")\") (goto-char 1))");
    reaches("parse-partial-sexp", &lists, "(list 1 (point-max) most-positive-fixnum)", "(= (point) 1)");
    let comments = format!("(progn {SYNTAX} (dotimes (_ 5000) (insert \";; c\\n\")) (goto-char 1))");
    reaches("forward-comment", &comments, "(list 100000)", "(= (point) 1)");
    reaches("forward-comment", &format!("(progn {comments} (goto-char (point-max)))"), "(list -100000)", "(= (point) (point-max))");
    let quotes = format!("(progn {SYNTAX} (insert (make-string 50000 ?')))");
    reaches("backward-prefix-chars", &quotes, "nil", "(= (point) (point-max))");
}

#[test]
fn text_loops_quit_before_they_change_anything() {
    let text = "(progn (insert (make-string 50000 ?a)) (goto-char 1))";
    reaches("skip-chars-forward", text, "(list \"a\")", "(= (point) 1)");
    reaches("string-width", "(setq s (make-string 50000 ?a))", "(list s)", "t");
    reaches("upcase", "(setq s (make-string 50000 ?a))", "(list s)", "(string= s (make-string 50000 ?a))");
    reaches("subst-char-in-region", text, "(list 1 (point-max) ?a ?b)", "(string= (buffer-string) (make-string 50000 ?a))");
    reaches("string-distance", "(setq a (make-string 1100 ?a) b (make-string 1100 ?b))", "(list a b)", "t");
}

#[test]
fn a_property_walk_lisp_calls_quits() {
    let runs = "(progn (insert (make-string 8000 ?x)) \
      (let ((i 1)) (while (< i 8000) (put-text-property i (1+ i) 'face (if (= 0 (% i 2)) 'bold 'italic)) (setq i (1+ i)))))";
    reaches("next-single-char-property-change", runs, "(list 1 'display)", "t");
}

#[test]
fn sqlite_interrupts_a_long_statement() {
    let setup = "(setq db (sqlite-open))";
    let query = "\"WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<200000) SELECT count(*) FROM c\"";
    reaches("sqlite-select", setup, &format!("(list db {query})"), "t");
    reaches("sqlite-execute", setup, &format!("(list db {query})"), "t");
}

#[test]
fn tree_sitter_parse_and_query_quit() {
    let (mut vm, _) = busy_vm();
    // The system C grammar, as `tests/treesit.rs` finds it.
    assert_ne!(eval(&mut vm, "(treesit-language-available-p 'c)"), Value::NIL, "the system C grammar is available");
    let c = "(progn (insert \"int a[] = {\" (let (parts) (dotimes (i 30000) (push (number-to-string i) parts)) (mapconcat #'identity parts \", \")) \"};\") \
             (setq parser (treesit-parser-create 'c)))";
    reaches("treesit-parser-root-node", c, "(list parser)", "t");
    let parsed = format!("(progn {c} (setq root (treesit-parser-root-node parser)))");
    reaches("treesit-query-capture", &parsed, "(list root '((number_literal) @n))", "t");
    // A query that matches nothing (the text has no comments) stops in the
    // cursor's walk, at its progress callback.
    reaches("treesit-query-capture", &parsed, "(list root '((comment) @c))", "t");
}

#[test]
fn call_process_s_wait_quits_and_interrupts_the_child() {
    let (mut vm, signal) = busy_vm();
    let call = prepare(&mut vm, "romacs--call-process-run", "(list '(\"sleep\" \"30\") nil \"/\" \"/dev/null\" nil nil t)");
    assert!(signal.interrupt());
    let started = std::time::Instant::now();
    let outcome = invoke(&mut vm, call);
    assert!(quit_signalled(&mut vm, &outcome), "the wait did not quit");
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "the wait outlasted the press");
    // Under inhibit-quit the wait goes on, the press held, without spinning.
    vm.quit_unwinding = false;
    eval(&mut vm, "(setq inhibit-quit t)");
    let call = prepare(&mut vm, "romacs--call-process-run", "(list '(\"sleep\" \"0.3\") nil \"/\" \"/dev/null\" nil nil t)");
    assert!(signal.interrupt());
    let outcome = invoke(&mut vm, call);
    assert!(outcome.is_ok(), "the wait ended with the child");
    assert!(signal.due(), "inhibit-quit holds the press");
}

/// The native sort, in runs merged a level at a time, orders as the sort
/// through a Lisp predicate does, ties kept in order: over several merge
/// levels and many equal keys.
#[test]
fn a_native_sort_in_runs_is_stable_and_ordered() {
    let (mut vm, _) = busy_vm();
    let form = "(let (l) (dotimes (i 100000) (push (cons (% (* i 7919) 97) i) l)) \
                  (equal (sort (copy-sequence l) :key #'car) \
                         (sort (copy-sequence l) :key #'car :lessp (lambda (a b) (< a b)))))";
    assert_eq!(vm.eval_source(form).unwrap().display().unwrap(), "t");
}

/// Coding conversion stops between the chunks of its passes, before the
/// new string is made: a long non-ASCII encode, and a decode that carriage
/// returns keep off its fast path.
#[test]
fn coding_conversion_quits_between_chunks() {
    // `undecided', which the runtime defines itself: the bare VM loads no
    // coding systems of GNU's Lisp.
    let setup = "(progn (encode-coding-string \"é\" 'undecided) (decode-coding-string \"a\" 'undecided) \
                 (setq s (make-string 50000 ?é)) (setq crlf (string-to-unibyte (make-string 50000 ?\\r))))";
    reaches("encode-coding-string", setup, "(list s 'undecided)", "t");
    reaches("decode-coding-string", setup, "(list crlf 'undecided)", "t");
}

#[test]
fn compare_buffer_substrings_quits_between_chunks() {
    let setup = "(progn (setq b1 (get-buffer-create \"cmp-1\") b2 (get-buffer-create \"cmp-2\")) \
                 (set-buffer b1) (insert (make-string 50000 ?a)) (set-buffer b2) (insert (make-string 50000 ?a)))";
    reaches("compare-buffer-substrings", setup, "(list b1 nil nil b2 nil nil)", "t");
}

/// The child's pid, as it wrote it to FILE.
fn child_pid(file: &std::path::Path) -> i32 {
    let started = std::time::Instant::now();
    loop {
        if let Some(pid) = std::fs::read_to_string(file).ok().and_then(|text| text.trim().parse().ok()) { return pid; }
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "the child never wrote its pid");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// GNU's call-process: "If you quit, the process is killed with SIGINT, or
/// SIGKILL if you quit again." After the quit no child is left running: one
/// that dies of SIGINT, and one that ignores it and takes a second C-g.
#[test]
fn a_quit_in_call_process_leaves_no_child_running() {
    let dir = std::env::temp_dir().join(format!("quit-call-process-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, script, presses) in [("plain", "echo $$ > {F}; exec sleep 30", 1), ("ignoring", "trap '' INT; echo $$ > {F}; exec sleep 30", 2)] {
        let file = dir.join(name);
        let script = script.replace("{F}", &file.display().to_string());
        let (mut vm, signal) = busy_vm();
        let call = prepare(&mut vm, "romacs--call-process-run", &format!("(list '(\"sh\" \"-c\" {script:?}) nil \"/\" \"/dev/null\" nil nil t)"));
        let presser = {
            let (signal, file) = (signal.clone(), file.clone());
            std::thread::spawn(move || {
                let pid = child_pid(&file);
                for _ in 0..presses {
                    assert!(signal.interrupt());
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
                pid
            })
        };
        let outcome = invoke(&mut vm, call);
        let pid = presser.join().unwrap();
        assert!(quit_signalled(&mut vm, &outcome), "{name}: the wait did not quit");
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists(), "{name}: child {pid} still exists after the quit");
    }
}

/// The process group of /proc's PID: the third field after its name's
/// closing parenthesis.
fn process_group(pid: &str) -> i32 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let after = &stat[stat.rfind(')').expect("a stat names its process") + 1..];
    after.split_whitespace().nth(2).and_then(|f| f.parse().ok()).expect("a stat gives its group")
}

/// call-process's child leads a session, so a group of its own: the quit's
/// SIGINT goes to that group alone. A process in this one's group, as a
/// sibling Romacs started, lives on through the quit.
#[test]
fn a_quit_in_call_process_signals_only_the_child_s_group() {
    let dir = std::env::temp_dir().join(format!("quit-call-process-group-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ids");
    let mut sibling = std::process::Command::new("sleep").arg("30").spawn().unwrap();
    let ours = process_group("self");
    assert_eq!(process_group(&sibling.id().to_string()), ours, "the sibling shares this process's group");
    // The child's pid and its process group (field 5 of its stat; sh's name has no space).
    let script = format!("read -r _ _ _ _ group _ < /proc/$$/stat; echo $$ $group > {}; exec sleep 30", file.display());
    let (mut vm, signal) = busy_vm();
    let call = prepare(&mut vm, "romacs--call-process-run", &format!("(list '(\"sh\" \"-c\" {script:?}) nil \"/\" \"/dev/null\" nil nil t)"));
    let presser = {
        let (signal, file) = (signal.clone(), file.clone());
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let ids: Vec<i32> = loop {
                let ids: Vec<i32> = std::fs::read_to_string(&file).unwrap_or_default().split_whitespace().filter_map(|w| w.parse().ok()).collect();
                if ids.len() == 2 { break ids; }
                assert!(started.elapsed() < std::time::Duration::from_secs(10), "the child never wrote its ids");
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            assert!(signal.interrupt());
            ids
        })
    };
    let outcome = invoke(&mut vm, call);
    let ids = presser.join().unwrap();
    assert!(quit_signalled(&mut vm, &outcome), "the wait did not quit");
    assert_eq!(ids[1], ids[0], "the child leads its own group");
    assert_ne!(ids[1], ours, "the child's group is not this process's");
    let alive = sibling.try_wait().unwrap().is_none();
    let _ = sibling.kill();
    let _ = sibling.wait();
    assert!(alive, "the sibling in this process's group died with the quit");
}

/// The printer stops between objects, as GNU's `print_object` polls at
/// each, before any text is made; `delq` polls as GNU's `FOR_EACH_TAIL` does.
#[test]
fn the_printer_and_delq_quit_in_their_walks() {
    reaches("prin1-to-string", "(setq l (make-list 20000 'a))", "(list l)", "t");
    reaches("delq", "(setq l (make-list 20000 'a))", "(list 'b l)", "(= (length l) 20000)");
}

/// A quit scope ends at any entry below the primitive that declared it.
/// Lisp entered from inside one (a predicate, a filter, a watcher) runs
/// outside it, so the primitives it calls start outside it too: a `puthash`
/// that rehashes an `equal` table under them cannot quit halfway. Each
/// generated primitive's entry asserts it starts outside a caller's scope;
/// `make-list` is one. The scope is the caller's again on return.
#[test]
fn a_quit_scope_ends_at_lisp_entry() {
    let (mut vm, _signal) = busy_vm();
    let witness = eval(&mut vm, "(symbol-function 'make-list)");
    assert!(generated(witness).is_some(), "make-list is a generated primitive");
    let predicate = eval(&mut vm, "(lambda ()
      (let ((h (make-hash-table :test 'equal)))
        (dotimes (i 3000) (puthash (make-list 40 i) i h))
        (hash-table-count h)))");
    // Inside a primitive's scope, as `Leaf::quittable` grants it.
    vm.native_quit = true;
    let count = vm.reader_call(predicate, &[]);
    assert_eq!(count.ok().and_then(|v| v.integer_value()), Some(3000), "the Lisp ran to its end");
    assert!(vm.native_quit, "the scope is the caller's again once the Lisp returns");
}
