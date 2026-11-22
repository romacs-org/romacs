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
    // A list quits in the walk that reads it, before the native sort.
    let setup = "(setq l (let (l) (dotimes (i 20000) (push (% (* i 7919) 10007) l)) l) copy (copy-sequence l))";
    reaches("sort", setup, "(list l #'<)", "(equal l copy)");
    let strings = "(setq l (mapcar #'number-to-string (let (l) (dotimes (i 20000) (push (% (* i 7919) 10007) l)) l)) copy (copy-sequence l))";
    reaches("sort", strings, "(list l #'string-lessp)", "(equal l copy)");
    // A vector has no such walk: it quits at the native sort's own poll,
    // after its run, and the vector is as it was.
    let setup = "(setq v (vconcat (let (l) (dotimes (i 20000) (push (% (* i 7919) 10007) l)) l)) copy (copy-sequence v))";
    reaches("sort", setup, "(list v #'<)", "(equal v copy)");
    let strings = "(setq v (vconcat (mapcar #'number-to-string (let (l) (dotimes (i 20000) (push (% (* i 7919) 10007) l)) l))) copy (copy-sequence v))";
    reaches("sort", strings, "(list v #'string-lessp)", "(equal v copy)");
}

/// Comparing 5 and 4 occurs only when the two runs merge.
#[test]
fn a_native_sort_quits_in_its_merge() {
    let (mut vm, signal) = busy_vm();
    // The scope the sort's own entry grants.
    vm.native_quit = true;
    let run = 4;
    let mut order = vec![4, 6, 8, 10, 5, 7, 9, 11];
    let mut pressed = false;
    let outcome = crate::prim::prims::leaf_sequences::sort_polled(&mut Leaf::new(&mut vm), &mut order, run, |_, a, b| {
        if (*a, *b) == (5, 4) { pressed = signal.interrupt(); }
        a.cmp(b)
    }).map(|()| Value::NIL);
    assert!(pressed, "the merge compared across runs");
    assert!(quit_signalled(&mut vm, &outcome), "the merge did not quit at its poll");
    vm.quit_unwinding = false;
}

#[test]
fn a_native_sort_quits_while_copying_a_merge_tail() {
    let (mut vm, signal) = busy_vm();
    vm.native_quit = true;
    let mut order = vec![4, 5, 6, 7, 8, 9, 10, 11, 2, 3, 12, 13, 0];
    let mut pressed = false;
    let outcome = crate::prim::prims::leaf_sequences::sort_polled(&mut Leaf::new(&mut vm), &mut order, 4, |_, a, b| {
        // Only the last merge compares 0 with 2. Its right run then ends.
        if (*a, *b) == (0, 2) { pressed = signal.interrupt(); }
        a.cmp(b)
    }).map(|()| Value::NIL);
    assert!(pressed, "the last merge compared its first pair");
    assert!(quit_signalled(&mut vm, &outcome), "the merge tail did not poll");
    assert_eq!(order, [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 0],
               "the interrupted merge published a partial level");
    vm.quit_unwinding = false;
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
    // A unibyte buffer's scan polls too.
    let bytes = "(progn (set-buffer-multibyte nil) (insert (make-string 200000 ?a)) (goto-char 1))";
    reaches("subst-char-in-region", bytes, "(list 1 (point-max) ?a ?b)", "(string= (buffer-string) (make-string 200000 ?a))");
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
    // A statement that writes and returns rows: its quit rolls the write
    // back, and the table holds none of its rows.
    let setup = "(progn (setq db (sqlite-open)) (sqlite-execute db \"CREATE TABLE t (x)\"))";
    let insert = "\"WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<200000) INSERT INTO t SELECT x FROM c RETURNING x\"";
    reaches("sqlite-execute", setup, &format!("(list db {insert})"), "(equal (sqlite-select db \"SELECT count(*) FROM t\") '((0)))");
    // The same through sqlite-select, whose rows are polled only for a
    // statement that writes nothing: it quits, and the table holds none of
    // its rows, never both a quit and a committed write.
    reaches("sqlite-select", setup, &format!("(list db {insert})"), "(equal (sqlite-select db \"SELECT count(*) FROM t\") '((0)))");
    // A set's interrupted step ends the set: the next call does not start
    // it over from its first row.
    let setup = format!("(setq db (sqlite-open) set (sqlite-select db {query} nil 'set))");
    reaches("sqlite-next", &setup, "(list set)", "(and (null (sqlite-next set)) (null (sqlite-more-p set)))");
}

/// sqlite-select polls between rows only a statement that writes nothing:
/// SQLite says which, once the statement is prepared.
#[test]
fn sqlite_select_tells_a_read_from_a_write() {
    let db = crate::sqlite::Connection::open(b":memory:", false, false, true).expect("an in-memory database");
    assert!(matches!(db.exec(b"CREATE TABLE t (x)", &crate::quit::NativeStop::never()), crate::sqlite::Step::Done));
    let readonly = |sql: &[u8]| db.prepare(sql).unwrap_or_else(|_| panic!("{} prepares", String::from_utf8_lossy(sql))).readonly();
    assert!(readonly(b"SELECT x FROM t"), "a select writes nothing");
    assert!(!readonly(b"INSERT INTO t VALUES (1) RETURNING x"), "an insert returning rows writes");
    assert!(readonly(b"EXPLAIN INSERT INTO t VALUES (1) RETURNING x"), "an EXPLAIN writes nothing");
}

/// A press that comes as a writing statement halts, its write committed,
/// is not that statement's quit: the call returns, its rows kept, and the
/// press goes to the form after it. The progress handler's ask is placed
/// at the statement's last instruction (`sqlite::PROGRESS_PERIOD`), its
/// check after the halt, through sqlite-select, sqlite-execute and a batch.
/// One instruction earlier, before the halt, the press still stops the
/// statement, which SQLite rolls back: a quit, and no row kept.
#[test]
fn sqlite_never_quits_with_its_write_kept() {
    use crate::quit::NativeStop;
    use crate::sqlite::{Connection, PROGRESS_PERIOD, Step};
    let rows = 7;
    let make = format!("CREATE TABLE src (x); CREATE TABLE dst (x); \
        WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<{rows}) INSERT INTO src SELECT i FROM c");
    let (returning, clear, copy) = ("INSERT INTO dst SELECT x FROM src RETURNING x", "DELETE FROM dst", "INSERT INTO dst SELECT x FROM src");
    // Each statement's instructions, unwatched, on the same tables.
    let never = NativeStop::never();
    let db = Connection::open(b":memory:", false, false, true).expect("an in-memory database");
    assert!(matches!(db.exec(make.as_bytes(), &never), Step::Done));
    let instructions = |sql: &str| {
        let mut statement = db.prepare(sql.as_bytes()).unwrap_or_else(|_| panic!("{sql} prepares"));
        while matches!(statement.step(&never), Step::Row) {}
        i32::try_from(statement.instructions()).expect("a short statement")
    };
    // In the order each runs on an empty table: a plain insert counts the same over rows.
    let (cleared, whole, copied) = (instructions(clear), instructions(returning), instructions(copy));
    assert!(cleared < copied, "the batch's first statement ends before its last one's ask");
    let listed = format!("({})", (1..=rows).map(|i| format!("({i})")).collect::<Vec<_>>().join(" "));
    let batch = format!("\"{clear}; {copy}\"");
    let select = format!("(list db \"{returning}\")");
    for (name, args, period, value) in [
        ("sqlite-select", select.clone(), whole - 1, None),
        ("sqlite-execute", select.clone(), whole - 1, None),
        ("sqlite-select", select.clone(), whole, Some(listed.clone())),
        ("sqlite-execute", select.clone(), whole, Some(listed.clone())),
        ("sqlite-execute-batch", format!("(list db {batch})"), copied, Some("t".to_string())),
    ] {
        let (mut vm, signal) = busy_vm();
        eval(&mut vm, &format!("(progn (setq db (sqlite-open)) (sqlite-execute-batch db \"{make}\"))"));
        let call = prepare(&mut vm, name, &args);
        let period = PROGRESS_PERIOD.with(|p| p.replace(period));
        assert!(signal.interrupt(), "a busy evaluator takes the press");
        let outcome = invoke(&mut vm, call);
        PROGRESS_PERIOD.with(|p| p.set(period));
        let shown = outcome.as_ref().map(|v| vm.display(*v).unwrap_or_default()).map_err(|_| quit_signalled(&mut vm, &outcome));
        let Some(value) = value else {
            assert_eq!(shown, Err(true), "{name}, asked before the halt: it quits");
            assert!(!signal.due(), "{name}: the quit took the press");
            vm.quit_unwinding = false;
            vm.propagated_escape = None;
            assert_eq!(vm.eval_source("(equal (sqlite-select db \"SELECT count(*) FROM dst\") '((0)))").unwrap().display().unwrap(), "t", "{name}: the write was rolled back");
            continue;
        };
        assert_eq!(shown, Ok(value), "{name}: the call returns its rows (Err(true): it quit with them kept)");
        assert!(signal.due(), "{name}: the press waits for the next poll");
        assert!(vm.eval_source_raw("(sqlite-select db \"SELECT count(*) FROM dst\")").is_err(), "{name}: the form after the call quits");
        assert!(!signal.due(), "{name}: that form took the press");
        vm.quit_unwinding = false;
        vm.propagated_escape = None;
        assert_eq!(vm.eval_source(&format!("(equal (sqlite-select db \"SELECT count(*) FROM dst\") '(({rows})))")).unwrap().display().unwrap(), "t", "{name}: the write was kept");
    }
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
    let call = prepare(&mut vm, "romacs--call-process-run", "(list '(\"sleep\" \"30\") nil \"/\" \"/dev/null\" nil nil t (cons nil nil))");
    assert!(signal.interrupt());
    let started = std::time::Instant::now();
    let outcome = invoke(&mut vm, call);
    assert!(quit_signalled(&mut vm, &outcome), "the wait did not quit");
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "the wait outlasted the press");
    // Under inhibit-quit the wait goes on, the press held, without spinning.
    vm.quit_unwinding = false;
    eval(&mut vm, "(setq inhibit-quit t)");
    let call = prepare(&mut vm, "romacs--call-process-run", "(list '(\"sleep\" \"0.3\") nil \"/\" \"/dev/null\" nil nil t (cons nil nil))");
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
    // call-process's output, decoded before it is inserted.
    reaches("romacs--decode-output", setup, "(list crlf 'undecided)", "t");
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
        let call = prepare(&mut vm, "romacs--call-process-run", &format!("(list '(\"sh\" \"-c\" {script:?}) nil \"/\" \"/dev/null\" nil nil t (cons nil nil))"));
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
    let call = prepare(&mut vm, "romacs--call-process-run", &format!("(list '(\"sh\" \"-c\" {script:?}) nil \"/\" \"/dev/null\" nil nil t (cons nil nil))"));
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
    reaches("prin1-to-string", "(setq s (make-string 50000 ?é))", "(list s)", "t");
    reaches("prin1-to-string", "(setq l (make-list 20000 'a))", "(list l)", "t");
    // A render the quit stopped publishes none of its labels: the vector's
    // pending label 7 is still pending, so no later print writes a bare #7#.
    reaches("prin1-to-string", "(progn (setq print-circle t print-continuous-numbering t v (make-vector 20000 'a) \
                                  print-number-table (make-hash-table :test 'eq)) (puthash v -7 print-number-table))",
            "(list (list v))", "(eql (gethash v print-number-table) -7)");
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
    assert_eq!(count.ok().and_then(Value::integer_value), Some(3000), "the Lisp ran to its end");
    assert!(vm.native_quit, "the scope is the caller's again once the Lisp returns");
}

/// A kernel the engine runs for a primitive whose failures are text polls
/// per chunk inside the scope the engine's call target grants: `append`'s
/// new list, `min` and `max` over their arguments, `format`'s print. Its
/// quit waits in `propagated_escape` behind `quit::ESCAPED` for the engine
/// to raise, so a `throw-on-input` throw would cross it whole. Under
/// `inhibit-quit` each runs to its end; so does each without the scope, as
/// the source cache's direct tier, which has no escape channel, calls it.
#[test]
fn kernels_the_engine_runs_quit_per_chunk() {
    use crate::Primitive::{Append, DeepEqual, Format, Max, Min};
    fn call(vm: &mut Vm, p: crate::Primitive, args: &[Value], scoped: bool) -> crate::Result<Value> {
        vm.with_kernel_stack(args, |vm, n| {
            if scoped { vm.inside_quit_scope(|vm| vm.primitive(p, 0, n)) } else { vm.primitive(p, 0, n) }
        })
    }
    for (p, args) in [(Append, "(list (make-list 50000 1) nil)"), (Append, "(list (make-string 50000 ?a) nil)"),
                      (Min, "(make-list 50000 1)"), (Max, "(make-list 50000 1)"), (Format, "(list \"%S\" (make-list 50000 'a))"),
                      (Format, "(list \"%S\" (make-string 50000 ?a))"), (Append, "(list (make-bool-vector 50000 t) nil)"),
                      (DeepEqual, "(list (make-list 50000 1) (make-list 50000 1))")] {
        let (mut vm, signal) = busy_vm();
        let list = eval(&mut vm, args);
        let mut values = Vec::new();
        let mut tail = list;
        while let Some((car, cdr)) = Leaf::new(&mut vm).pair(tail) {
            values.push(car);
            tail = cdr;
        }
        assert!(signal.interrupt(), "a busy evaluator takes the press");
        assert!(call(&mut vm, p, &values, false).is_ok(), "{p:?}: without the scope the call ends with a value");
        assert!(signal.due(), "{p:?}: without the scope the press stays pending");
        let consed = vm.allocation.consed.conses;
        let outcome = call(&mut vm, p, &values, true);
        assert_eq!(outcome.err().as_deref(), Some(crate::quit::ESCAPED), "{p:?}: no quit at its poll");
        // `append' quits in its walk of an argument, before the new list's
        // first cons: the walk polls, not only the build after it.
        if p == Append { assert_eq!(vm.allocation.consed.conses, consed, "{args}: a cons made before the quit"); }
        let quit = vm.intern("quit");
        assert!(matches!(vm.propagated_escape, Some(crate::threaded::Escape::Signal(c, _)) if c == quit), "{p:?}: the quit is pending");
        vm.propagated_escape = None;
        vm.quit_unwinding = false;
        eval(&mut vm, "(setq inhibit-quit t)");
        assert!(signal.interrupt());
        assert!(call(&mut vm, p, &values, true).is_ok(), "{p:?}: under inhibit-quit the call ends with a value");
        assert!(signal.due(), "{p:?}: inhibit-quit holds the press");
    }
}

/// A native poll quits while a form compiles and a macro expands, as the
/// engine's safepoint quits the expander: the compiler defers that quit to
/// the form's reach. Here the printer runs inside an expansion as the
/// compiler sets one up; under `inhibit-quit` it runs to its end.
#[test]
fn a_native_poll_quits_inside_a_compiling_macro_expansion() {
    let (mut vm, signal) = busy_vm();
    eval(&mut vm, "(setq l (make-list 50000 'a))");
    let call = prepare(&mut vm, "prin1-to-string", "(list l)");
    vm.compile_depth += 1;
    vm.expanding.push(0);
    assert!(signal.interrupt(), "a busy evaluator takes the press");
    let outcome = invoke(&mut vm, call);
    assert!(quit_signalled(&mut vm, &outcome), "the printer's poll did not quit inside the expansion");
    vm.quit_unwinding = false;
    let inhibit = vm.slot("inhibit-quit");
    vm.slots[inhibit].value = Value::TRUE;
    let call = prepare(&mut vm, "prin1-to-string", "(list l)");
    assert!(signal.interrupt());
    assert!(invoke(&mut vm, call).is_ok(), "under inhibit-quit the print ends with a value");
    assert!(signal.due(), "inhibit-quit holds the press");
    vm.slots[inhibit].value = Value::NIL;
    vm.expanding.pop();
    vm.compile_depth -= 1;
}

/// A C-g an `inhibit-quit` held, released before the call, leaves
/// `quit-flag` set and no press: a native poll quits for it at once, as GNU's
/// `maybe_quit` reads `Vquit_flag`, through the mutable poll (`append`'s
/// kernel) and the borrowed probe (base64's decoder) alike. Under
/// `inhibit-quit` the call runs to its end.
#[test]
fn a_native_poll_quits_for_a_latched_quit_flag() {
    use crate::Primitive::Append;
    let (mut vm, signal) = busy_vm();
    let list = eval(&mut vm, "(make-list 50000 1)");
    let (flag, inhibit) = (vm.slot("quit-flag"), vm.slot("inhibit-quit"));
    let append = |vm: &mut Vm| {
        vm.with_kernel_stack(&[list, Value::NIL], |vm, n| vm.inside_quit_scope(|vm| vm.primitive(Append, 0, n)))
    };
    vm.slots[flag].value = Value::TRUE;
    assert!(!signal.due(), "no press, only the flag");
    assert_eq!(append(&mut vm).err().as_deref(), Some(crate::quit::ESCAPED), "append did not quit for quit-flag");
    assert_eq!(vm.slots[flag].value, Value::NIL, "the quit took the flag");
    vm.propagated_escape = None;
    vm.quit_unwinding = false;
    let call = prepare(&mut vm, "base64-decode-string", "(list (base64-encode-string (make-string 100000 ?a) t))");
    vm.slots[flag].value = Value::TRUE;
    let outcome = invoke(&mut vm, call);
    assert!(quit_signalled(&mut vm, &outcome), "base64's probe did not stop for quit-flag");
    vm.quit_unwinding = false;
    vm.slots[inhibit].value = Value::TRUE;
    vm.slots[flag].value = Value::TRUE;
    assert!(append(&mut vm).is_ok(), "under inhibit-quit append ends with a value");
    vm.slots[inhibit].value = Value::NIL;
    vm.slots[flag].value = Value::NIL;
}

/// A C-g that lands in a kernel's poll reaches Lisp as `quit`, never as an
/// error made of the poll's text: through `append` the engine runs, `apply`'s
/// spread, and the loader's read of one long form.
#[test]
fn a_kernel_s_or_the_loader_s_quit_reaches_lisp_as_quit() {
    let dir = std::env::temp_dir().join(format!("quit-kernel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("long.el");
    std::fs::write(&file, format!("(setq long '({}))\n", (0..400_000).map(|i| i.to_string()).collect::<Vec<_>>().join(" "))).unwrap();
    for form in ["(progn (setq big (make-list 2000000 1)) (condition-case nil (while t (append big nil)) (quit 'q)))".to_string(),
                 "(progn (setq big (make-list 3000000 1)) (condition-case nil (while t (apply #'max big)) (quit 'q)))".to_string(),
                 format!("(condition-case nil (while t (load {:?} nil t t)) (quit 'q))", file.display().to_string())] {
        let (mut vm, signal) = busy_vm();
        let presser = { let signal = signal.clone(); std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_millis(300)); signal.interrupt() }) };
        let outcome = vm.eval_source_raw(&form).map(|v| vm.display(v).unwrap());
        assert!(presser.join().unwrap(), "a busy evaluator takes the press");
        assert_eq!(outcome.as_deref(), Ok("q"), "{form}: the quit did not reach Lisp as `quit`");
    }
}

/// call-process's stop without a pidfd (as when `pidfd_open` fails): it asks
/// the child's end every tick, so a second C-g still kills a child that
/// ignores SIGINT, and the child is reaped, not left running; one that dies
/// of SIGINT is reaped with no press.
#[test]
fn call_process_s_stop_without_a_pidfd_hears_the_second_press() {
    use std::os::unix::process::CommandExt;
    for (script, presses) in [("trap '' INT; exec sleep 30", 1), ("exec sleep 30", 0)] {
        let (mut vm, signal) = busy_vm();
        let child = std::process::Command::new("sh").args(["-c", script]).process_group(0).spawn().unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        // The shell's trap in place before the SIGINT.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let presser = { let signal = signal.clone(); std::thread::spawn(move || {
            for _ in 0..presses { std::thread::sleep(std::time::Duration::from_millis(300)); assert!(signal.interrupt()); }
        }) };
        let started = std::time::Instant::now();
        Leaf::new(&mut vm).call_process_stop(pid, None, None);
        presser.join().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{script}: the stop outlasted the press");
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists(), "{script}: child {pid} still exists after the stop");
        assert!(vm.subprocesses.deleted.is_empty(), "{script}: the child was reaped, not handed on");
        drop(child);
    }
}

/// call-process's reap after its quit's SIGKILL hands a child still alive at
/// the deadline to the reaper of deleted children, with no status made up
/// for it (`ChildEnd::HandedOff` carries none). The reaper then owns it
/// alone, and reports its death by no longer owning it once it is reaped.
#[test]
fn call_process_s_reap_hands_a_live_child_on_at_its_deadline() {
    use std::os::unix::process::CommandExt;
    let (mut vm, _signal) = busy_vm();
    let child = std::process::Command::new("sleep").arg("30").process_group(0).spawn().unwrap();
    let pid = i32::try_from(child.id()).unwrap();
    let end = Leaf::new(&mut vm).call_process_reap(pid, &[], std::time::Duration::from_millis(50));
    assert_eq!(end, super::leaf::ChildEnd::HandedOff, "a live child at the deadline is handed on");
    assert!(vm.subprocesses.owns(pid), "the reaper owns the handed-off child");
    assert!(std::path::Path::new(&format!("/proc/{pid}")).exists(), "the child still runs");
    let _ = crate::host::kill(pid, libc::SIGKILL);
    let started = std::time::Instant::now();
    while vm.subprocesses.owns(pid) {
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "the reaper never took the child");
        std::thread::sleep(std::time::Duration::from_millis(10));
        vm.subprocesses.reap_deleted();
    }
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists(), "the child was reaped, its death noticed");
    drop(child);
}

/// `equal`'s walk polls only for `equal` itself: at its engine site it
/// stops with a press pending (the full call then polls and quits), while a
/// hash table's `equal` lookups, run inside a primitive's quit scope with a
/// press pending, walk to their end, so an insert that rehashes never stops
/// halfway.
/// A walk of many short vectors, none a chunk long (512 of 512), asks too:
/// its steps count over all of them.
#[test]
fn equal_polls_over_many_short_vectors() {
    let (mut vm, signal) = busy_vm();
    eval(&mut vm, "(setq va (let (l) (dotimes (_ 512) (push (make-vector 512 1) l)) (vconcat l)) \
                         vb (let (l) (dotimes (_ 512) (push (make-vector 512 1) l)) (vconcat l)))");
    let [va, vb] = ["va", "vb"].map(|name| eval(&mut vm, name));
    assert_eq!(vm.equal_site_kernel(va, vb), Value::TRUE, "with no press the walk ends");
    assert!(signal.interrupt(), "a busy evaluator takes the press");
    assert_eq!(vm.equal_site_kernel(va, vb), Value::UNBOUND, "the walk stops at its poll");
}

#[test]
fn equal_polls_for_itself_and_never_inside_a_hash_lookup() {
    use crate::Primitive::Puthash;
    let (mut vm, signal) = busy_vm();
    // Keys that differ only at their end and hash alike: each insert walks
    // the bucket's keys with `equal`, and the table grows and rehashes.
    // Rooted in variables, read back once nothing more is evaluated.
    eval(&mut vm, "(setq table (make-hash-table :test 'equal) \
                         keys (let (keys) (dotimes (i 120) (push (append (make-list 2000 0) (list i)) keys)) keys) \
                         a (make-list 50000 1) b (make-list 50000 1))");
    let [table, keys, a, b] = ["table", "keys", "a", "b"].map(|name| eval(&mut vm, name));
    assert_eq!(vm.equal_site_kernel(a, b), Value::TRUE, "with no press the walk ends");
    assert!(signal.interrupt(), "a busy evaluator takes the press");
    assert_eq!(vm.equal_site_kernel(a, b), Value::UNBOUND, "equal's site stops at its poll");
    let mut tail = keys;
    let mut entries = 0;
    while let Some((key, rest)) = Leaf::new(&mut vm).pair(tail) {
        let put = vm.with_kernel_stack(&[key, Value::TRUE, table], |vm, n| vm.inside_quit_scope(|vm| vm.primitive(Puthash, 0, n)));
        assert!(put.is_ok(), "an insert inside a quit scope ran to its end");
        entries += 1;
        tail = rest;
    }
    assert!(signal.due(), "the hash lookups left the press pending");
    let count = vm.with_kernel_stack(&[table], |vm, n| vm.primitive(crate::Primitive::HashTableCount, 0, n));
    assert_eq!(count.ok().and_then(Value::integer_value), Some(entries), "every key went in");
}

/// `vconcat`'s list walk polls inside its own scope, the vector made only
/// after it; `copy-sequence` and `nreverse` poll inside the scope their call
/// target grants. Each quit leaves the list as it was: `nreverse` polls in
/// its first walk, which writes nothing.
#[test]
fn list_walks_quit_inside_their_own_scope() {
    use crate::Primitive::{CopySequence, Nreverse};
    reaches("vconcat", "(setq l (make-list 50000 1))", "(list l)", "(= (length l) 50000)");
    reaches("vconcat", "(setq s (make-string 50000 ?a))", "(list s)", "t");
    reaches("vconcat", "(setq b (make-bool-vector 50000 t))", "(list b)", "t");
    for p in [CopySequence, Nreverse] {
        let (mut vm, signal) = busy_vm();
        let list = eval(&mut vm, "(setq l (make-list 50000 1))");
        assert!(signal.interrupt(), "a busy evaluator takes the press");
        let outcome = vm.with_kernel_stack(&[list], |vm, n| vm.inside_quit_scope(|vm| vm.primitive(p, 0, n)));
        assert_eq!(outcome.err().as_deref(), Some(crate::quit::ESCAPED), "{p:?}: no quit at its poll");
        vm.propagated_escape = None;
        vm.quit_unwinding = false;
        assert_eq!(eval(&mut vm, "(length l)").integer_value(), Some(50000), "{p:?}: the list is whole");
        // Under inhibit-quit it runs to its end and the press waits.
        eval(&mut vm, "(setq inhibit-quit t)");
        assert!(signal.interrupt());
        let outcome = vm.with_kernel_stack(&[list], |vm, n| vm.inside_quit_scope(|vm| vm.primitive(p, 0, n)));
        assert!(outcome.is_ok(), "{p:?}: under inhibit-quit the call ends with a value");
        assert!(signal.due(), "{p:?}: inhibit-quit holds the press");
    }
}

/// Step 7's rows: each polls per chunk inside its own scope, before it
/// makes anything, and ends in `quit` with a press counted.
#[test]
fn chunked_rows_quit_before_they_make_anything() {
    reaches("base64-decode-string", "(setq s (base64-encode-string (make-string 100000 ?a) t))", "(list s)", "t");
    reaches("current-column", "(progn (insert (make-string 50000 ?a)) (goto-char (point-max)))", "nil", "(= (point) (point-max))");
    reaches("make-string", "nil", "(list 50000 ?a)", "t");
    reaches("string-to-multibyte", "(setq s (string-to-unibyte (make-string 50000 ?a)))", "(list s)", "(not (multibyte-string-p s))");
    reaches("find-coding-systems-region-internal", "(setq s (make-string 50000 ?é))", "(list s nil)", "t");
}

/// Base64 encoding polls between 3-byte-aligned slices, before anything is
/// made; its text matches an unpolled encode across the slice edges.
#[test]
fn base64_encoding_quits_between_slices() {
    reaches("base64-encode-string", "(setq s (make-string 100000 ?a))", "(list s)", "t");
    reaches("base64url-encode-string", "(setq s (make-string 100000 ?a))", "(list s)", "t");
}

/// The values the list FORM evaluates to, made before any press.
fn values_of(vm: &mut Vm, form: &str) -> Vec<Value> {
    let list = eval(vm, form);
    let cx = Leaf::new(vm);
    let mut values = Vec::new();
    let mut tail = list;
    while let Some((car, cdr)) = cx.pair(tail) {
        values.push(car);
        tail = cdr;
    }
    values
}

/// P called on ARGS through the engine's call target for it when ENGINE,
/// else through `Vm::primitive`, which grants no scope, as the source
/// cache's direct tier calls it.
fn call_kernel(vm: &mut Vm, p: crate::Primitive, args: &[Value], engine: bool) -> crate::Result<Value> {
    vm.with_kernel_stack(args, |vm, n| {
        if engine { crate::threaded::primitive_target(p)(vm, p, 0, n) } else { vm.primitive(p, 0, n) }
    })
}

/// The kernels gated at their first chunk (`Vm::kernel_poll`), each with
/// the form making its arguments, N elements long. A press counted: through
/// the engine's call target three chunks' worth quits at a poll, and one
/// short of a chunk ends with a value, never polled, the press pending;
/// through `Vm::primitive`, outside any scope, three chunks end with a
/// value too.
#[test]
fn gated_kernels_take_their_scope_at_their_first_chunk() {
    use crate::Primitive::{Append, CopySequence, Max, Min, Nreverse};
    const K: usize = crate::quit::KERNEL_CHUNK;
    // Each kernel, with the form making its arguments, N elements long.
    type Kernel = (crate::Primitive, fn(usize) -> String);
    let kernels: [Kernel; 7] = [
        (Nreverse, |n| format!("(list (make-list {n} 1))")),
        (CopySequence, |n| format!("(list (make-list {n} 1))")),
        (Append, |n| format!("(list (make-list {n} 1) nil)")),
        (Append, |n| format!("(list (make-string {n} ?a) nil)")),
        (Append, |n| format!("(list (make-bool-vector {n} t) nil)")),
        (Min, |n| format!("(make-list {n} 1)")),
        (Max, |n| format!("(make-list {n} 1)")),
    ];
    for (p, form) in kernels {
        for (n, engine) in [(3 * K, true), (K - 1, true), (3 * K, false)] {
            let (mut vm, signal) = busy_vm();
            let args = values_of(&mut vm, &form(n));
            assert!(signal.interrupt(), "a busy evaluator takes the press");
            let outcome = call_kernel(&mut vm, p, &args, engine);
            if engine && n > K {
                assert_eq!(outcome.err().as_deref(), Some(crate::quit::ESCAPED), "{p:?} of {n}: no quit at its poll");
                let quit = vm.intern("quit");
                assert!(matches!(vm.propagated_escape, Some(crate::threaded::Escape::Signal(c, _)) if c == quit), "{p:?}: the quit is pending");
            } else {
                assert!(outcome.is_ok(), "{p:?} of {n} (engine {engine}): the call ends with a value");
                assert!(signal.due(), "{p:?} of {n} (engine {engine}): the press stays pending");
            }
        }
    }
}

/// `nreverse`'s first poll comes at `KERNEL_CHUNK` steps of its walk, as
/// every gated kernel's does: a list between one chunk and two, with a
/// press pending, quits through the engine's call target and is left as it
/// was (the walk writes nothing).
#[test]
fn nreverse_polls_at_its_first_chunk() {
    let (mut vm, signal) = busy_vm();
    let args = values_of(&mut vm, "(list (setq l (let (l) (dotimes (i 20000) (push (- 20000 i) l)) l)))");
    assert!(signal.interrupt(), "a busy evaluator takes the press");
    let outcome = call_kernel(&mut vm, crate::Primitive::Nreverse, &args, true);
    assert_eq!(outcome.err().as_deref(), Some(crate::quit::ESCAPED), "no quit at its first chunk");
    vm.propagated_escape = None;
    vm.quit_unwinding = false;
    assert_eq!(eval(&mut vm, "(equal (list (length l) (car l) (nth 19999 l)) '(20000 1 20000))"), Value::TRUE, "the list is whole");
}

/// `nreverse`'s polled walk asks before it checks for the cycle: a list
/// whose last cdr is itself, the cycle found at the step that polls, quits
/// with a press waiting, the list as it was.
#[test]
fn nreverse_polls_at_the_step_that_finds_its_cycle() {
    let (mut vm, signal) = busy_vm();
    let args = values_of(&mut vm, "(list (setq l (let (l) (dotimes (i 16384) (push (- 16384 i) l)) (let ((c (nthcdr 16383 l))) (setcdr c c)) l)))");
    assert!(signal.interrupt(), "a busy evaluator takes the press");
    let outcome = call_kernel(&mut vm, crate::Primitive::Nreverse, &args, true);
    assert_eq!(outcome.err().as_deref(), Some(crate::quit::ESCAPED), "no quit at the step that finds the cycle");
    vm.propagated_escape = None;
    vm.quit_unwinding = false;
    let shape = eval(&mut vm, "(let ((c (nthcdr 16383 l))) (list (car l) (car c) (eq (cdr c) c) (car (nthcdr 100 l))))");
    assert_eq!(vm.display(shape).unwrap(), "(1 16384 t 101)", "the list is unmodified");
}

/// `nreverse`'s walk hands off to its polled continuation once Brent's
/// power reaches `KERNEL_CHUNK`, after `KERNEL_CHUNK` - 1 steps: on either side
/// of that, and across it, a proper list comes back reversed, and a list
/// whose cycle the walk finds before the hand-off or after it signals
/// `circular-list` with the list itself, as fns.c's `Fnreverse` does.
#[test]
fn nreverse_is_one_walk_across_its_hand_off() {
    const K: usize = crate::quit::KERNEL_CHUNK;
    let (mut vm, _signal) = busy_vm();
    for n in [1, 2, K - 2, K - 1, K, K + 1, 2 * K - 1, 2 * K, 3 * K + 5] {
        let form = format!("(let (l r) (dotimes (i {n}) (push (- {n} i) l) (push (1+ i) r)) (equal (nreverse l) r))");
        assert_eq!(eval(&mut vm, &form), Value::TRUE, "{form}");
    }
    // (LENGTH, BACK): the last cons's cdr is the BACKth.
    for (length, back) in [(1, 0), (100, 50), (K - 1, 0), (K, 3), (K + 1, K), (3 * K, 0), (3 * K, K + 7)] {
        let form = format!("(let (l) (dotimes (i {length}) (push i l)) (setcdr (nthcdr (1- {length}) l) (nthcdr {back} l)) \
                            (condition-case e (progn (nreverse l) 'returned) (circular-list (eq (car (cdr e)) l))))");
        assert_eq!(eval(&mut vm, &form), Value::TRUE, "{form}");
    }
}

/// `romacs--call-process-quit` raises C-g's quit only for its wait's own
/// token, outstanding, which it takes: a fresh cons known by `eq`, so no
/// number or look-alike cons stands for it. Called with none, as from a
/// macro, or with any other value, it signals a plain quit, which an
/// expansion treats as any error: the form in the branch not taken is never
/// reached. `romacs--call-process-retire` ends a token not raised.
#[test]
fn call_process_quit_is_c_g_s_only_for_its_wait_s_token() {
    let (mut vm, _signal) = busy_vm();
    eval(&mut vm, "(setq quit-test-mine (cons nil nil) quit-test-other (cons nil nil))");
    let (slot_mine, slot_other) = (vm.slot("quit-test-mine"), vm.slot("quit-test-other"));
    let (mine, other) = (vm.slots[slot_mine].value, vm.slots[slot_other].value);
    { let mut cx = Leaf::new(&mut vm); cx.issue_requit(mine); cx.issue_requit(other); }
    for call in ["(romacs--call-process-quit)", "(romacs--call-process-quit 1)", "(romacs--call-process-quit 2)",
                 "(romacs--call-process-quit (cons nil nil))", "(romacs--call-process-quit (cons (car quit-test-mine) (cdr quit-test-mine)))"] {
        eval(&mut vm, &format!("(defmacro m () {call})"));
        assert_eq!(vm.eval_source("(eval '(if nil (m) 'else) t)").unwrap().display().unwrap(), "else", "{call}");
    }
    let outstanding = "(length (symbol-value (intern \"#:vm2-call-process-quits\")))";
    assert_eq!(vm.eval_source(outstanding).unwrap().display().unwrap(), "2", "no forged call took a token");
    assert_eq!(vm.eval_source("(condition-case nil (romacs--call-process-quit quit-test-mine) (quit 'quit))").unwrap().display().unwrap(), "quit");
    vm.quit_unwinding = false;
    assert_eq!(vm.eval_source(outstanding).unwrap().display().unwrap(), "1", "the quit took its own token, and only it");
    eval(&mut vm, "(romacs--call-process-retire quit-test-other)");
    assert_eq!(vm.eval_source(outstanding).unwrap().display().unwrap(), "0", "a retired token is gone");
}

/// A step that fails for its own reason with a press waiting is that error,
/// not the quit. On an empty `t (x UNIQUE)`, `INSERT OR FAIL ... (1), (1)`
/// keeps its first row, committed at the halt, and ends with the
/// constraint; the progress handler's ask is placed at the halt
/// (`sqlite::PROGRESS_PERIOD`). sqlite-execute signals the constraint,
/// sqlite-select ends its rows there and a batch returns nil: the row kept,
/// and the next form takes the press.
#[test]
fn a_sqlite_error_with_a_press_waiting_is_itself() {
    use crate::quit::NativeStop;
    use crate::sqlite::{Connection, PROGRESS_PERIOD, Step};
    let (make, insert) = ("CREATE TABLE t (x UNIQUE)", "INSERT OR FAIL INTO t VALUES (1), (1)");
    let never = NativeStop::never();
    let db = Connection::open(b":memory:", false, false, true).expect("an in-memory database");
    assert!(matches!(db.exec(make.as_bytes(), &never), Step::Done));
    let mut statement = db.prepare(insert.as_bytes()).unwrap_or_else(|_| panic!("{insert} prepares"));
    assert!(matches!(statement.step(&never), Step::Error(_)), "the second row conflicts");
    let whole = i32::try_from(statement.instructions()).expect("a short statement");
    drop(statement);
    for (name, ends) in [("sqlite-execute", None), ("sqlite-select", Some("nil")), ("sqlite-execute-batch", Some("nil"))] {
        let (mut vm, signal) = busy_vm();
        eval(&mut vm, &format!("(progn (setq db (sqlite-open)) (sqlite-execute db \"{make}\"))"));
        let call = prepare(&mut vm, name, &format!("(list db \"{insert}\")"));
        let period = PROGRESS_PERIOD.with(|p| p.replace(whole));
        assert!(signal.interrupt(), "a busy evaluator takes the press");
        let outcome = invoke(&mut vm, call);
        PROGRESS_PERIOD.with(|p| p.set(period));
        assert!(!quit_signalled(&mut vm, &outcome), "{name}: the constraint, not a quit");
        match ends {
            Some(value) => assert_eq!(outcome.as_ref().map(|v| vm.display(*v).unwrap_or_default()).ok().as_deref(), Some(value), "{name}"),
            None => assert!(matches!(outcome, Err(Fault::Signal(..))), "{name}: the constraint is signaled"),
        }
        assert!(signal.due(), "{name}: the press waits for the next poll");
        assert!(vm.eval_source_raw("(sqlite-select db \"SELECT count(*) FROM t\")").is_err(), "{name}: the form after the call quits");
        assert!(!signal.due(), "{name}: that form took the press");
        vm.quit_unwinding = false;
        vm.propagated_escape = None;
        assert_eq!(vm.eval_source("(equal (sqlite-select db \"SELECT count(*) FROM t\") '((1)))").unwrap().display().unwrap(), "t", "{name}: the first row was kept");
    }
}

/// A batch is prepared statement by statement from one buffer ended by a
/// NUL, which the library copies none of: running 20,000 statements raises
/// the most memory it has in use far less than the batch's size, where a
/// prepare given a length with no NUL would copy the rest of the batch for
/// each statement.
#[test]
fn a_sqlite_batch_is_not_copied_per_statement() {
    use crate::sqlite::{Connection, memory_highwater};
    let db = Connection::open(b":memory:", false, false, true).expect("an in-memory database");
    let batch = "SELECT 1;".repeat(20_000);
    memory_highwater();
    let before = memory_highwater();
    assert!(matches!(db.exec(batch.as_bytes(), &crate::quit::NativeStop::never()), crate::sqlite::Step::Done));
    let peak = memory_highwater() - before;
    let bound = i64::try_from(batch.len() / 4).expect("a short batch");
    assert!(peak < bound, "SQLite's memory rose {peak} bytes over a batch of {}", batch.len());
}

/// An `SQLITE_INTERRUPT` the progress handler did not ask for (an SQL
/// function's own result code) is the statement's error, not the quit, with
/// a press waiting: sqlite-execute signals it and a batch returns nil, and
/// the next form takes the press.
#[test]
fn a_sqlite_interrupt_not_the_handler_s_is_an_error() {
    for (name, args, ends) in [("sqlite-execute", "(list db \"SELECT interrupt_itself()\")", None),
                               ("sqlite-execute-batch", "(list db \"SELECT 1; SELECT interrupt_itself();\")", Some("nil"))] {
        let (mut vm, signal) = busy_vm();
        let db = eval(&mut vm, "(setq db (sqlite-open))");
        Leaf::new(&mut vm).sqlite_connection_of(db).unwrap_or_else(|_| panic!("a database")).define_interrupt_itself();
        let call = prepare(&mut vm, name, args);
        assert!(signal.interrupt(), "a busy evaluator takes the press");
        let outcome = invoke(&mut vm, call);
        assert!(!quit_signalled(&mut vm, &outcome), "{name}: the function's interrupt, not a quit");
        match ends {
            Some(value) => assert_eq!(outcome.as_ref().map(|v| vm.display(*v).unwrap_or_default()).ok().as_deref(), Some(value), "{name}"),
            None => assert!(matches!(outcome, Err(Fault::Signal(..))), "{name}: the error is signaled"),
        }
        assert!(signal.due(), "{name}: the press waits for the next poll");
        assert!(vm.eval_source_raw("(sqlite-select db \"SELECT 1\")").is_err(), "{name}: the form after the call quits");
        vm.quit_unwinding = false;
        vm.propagated_escape = None;
    }
}

/// A batch steps past the blanks `sqlite3_exec` skips after a statement,
/// a vertical tab among them, before it prepares the next: both statements
/// run, through sqlite-execute-batch and sqlite-pragma.
#[test]
fn a_sqlite_batch_skips_the_blanks_sqlite3_exec_skips() {
    let (mut vm, _signal) = busy_vm();
    let form = "(let ((db (sqlite-open)) (vt (string 11)))
                  (list (sqlite-execute-batch db (concat \"SELECT 1;\" vt \"SELECT 2;\"))
                        (sqlite-pragma db (concat \"user_version = 7;\" vt \"SELECT 2;\"))
                        (sqlite-execute-batch db (concat \"CREATE TABLE a (x);\" vt \"CREATE TABLE b (x);\" vt))
                        (sqlite-select db \"SELECT count(*) FROM sqlite_master\")
                        (sqlite-select db \"PRAGMA user_version\")))";
    assert_eq!(vm.eval_source(form).unwrap().display().unwrap(), "(t t t ((2)) ((7)))");
}

/// A stop in a nested query recovered by the first step cannot turn a
/// later step's own SQLITE_INTERRUPT into a quit. The batch reports failure;
/// callers materializing a SELECT retain the row already delivered.
#[test]
fn a_sqlite_batch_s_stop_is_each_step_s_own() {
    let rows = "SELECT recover_nested() UNION ALL SELECT interrupt_itself()";
    let recovered = format!("(({}))", crate::sqlite::INTERRUPT);
    for (name, sql, expected) in [
        ("sqlite-execute-batch", "SELECT recover_nested(); SELECT interrupt_itself();", "nil"),
        ("sqlite-execute", rows, recovered.as_str()),
        ("sqlite-select", rows, recovered.as_str()),
    ] {
        let (mut vm, signal) = busy_vm();
        let db = eval(&mut vm, "(setq db (sqlite-open))");
        let connection = Leaf::new(&mut vm).sqlite_connection_of(db).unwrap_or_else(|_| panic!("a database"));
        connection.define_recover_nested();
        connection.define_interrupt_itself();
        let call = prepare(&mut vm, name, &format!(r#"(list db "{sql}")"#));
        assert!(signal.interrupt(), "a busy evaluator takes the press");
        let outcome = invoke(&mut vm, call);
        assert!(!quit_signalled(&mut vm, &outcome), "{name}: the later step's own interrupt");
        assert_eq!(outcome.as_ref().map(|v| vm.display(*v).unwrap_or_default()).ok().as_deref(), Some(expected), "{name}");
        assert!(signal.due(), "{name}: the press waits for the next form");
        vm.quit_unwinding = false;
    }
}
