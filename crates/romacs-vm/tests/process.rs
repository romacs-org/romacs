//! Subprocesses against pinned GNU (process.c, callproc.c): process objects
//! and their accessors, make-process with pipes and a pty, pipe processes as
//! stderr, filters, sentinels and their default insertion, call-process and
//! call-process-region destinations and statuses, signals, and loopback
//! network servers and clients (blocking and :nowait). Every case
//! starts its own children and waits for them with accept-process-output;
//! none sleeps to synchronize.
use romacs_vm::Vm;
use std::path::Path;

/// One expression per line, each with `default-directory' set to a private
/// directory. Romacs runs them in order in one VM, so each leaves no process.
const CASES: &str = r###"
(list (processp 1) (process-list) (get-process "nope") (get-buffer-process "nope") (process-status "nope") (condition-case e (get-process 1) (error e)) (condition-case e (process-name 1) (error e)) (condition-case e (process-status nil) (error e)))
(list (make-process) (make-pipe-process) (make-network-process) (condition-case e (make-process :command (list "true")) (error e)) (condition-case e (make-process :name 1 :command (list "true")) (error e)) (condition-case e (make-process :name "x" :command (list "true") :stop t) (error e)) (condition-case e (make-process :name "x" :foo) (error e)) (condition-case e (make-process :name "x" :command (list "romacs-no-such-program")) (error e)) (process-list))
(let ((p (make-process :name "q" :command (list "sh" "-c" "exit 7") :connection-type 'pipe))) (romacs-test-wait p) (list (process-status p) (process-exit-status p) (integerp (process-id p)) (memq p (process-list)) (process-command p) (process-type p) (process-contact p) (process-contact p t) (process-tty-name p) (process-query-on-exit-flag p) (process-plist p) (process-sentinel p) (process-filter p) (process-buffer p) (format "%S|%s" p p) (type-of p)))
(let* ((out nil) (p (make-process :name "f" :command (list "printf" "a\nb") :connection-type 'pipe :filter (lambda (_p s) (garbage-collect) (setq out (cons s out))) :sentinel (lambda (p m) (setq out (cons (list 'sentinel m (process-status p)) out)))))) (romacs-test-wait p) (accept-process-output nil 0.05) (list (apply #'concat (mapcar (lambda (x) (if (stringp x) x "")) (reverse out))) (car out)))
(let* ((b (get-buffer-create "romacs-kbuf")) (p (make-process :name "k" :command (list "sleep" "10") :connection-type 'pipe :buffer b))) (kill-process p) (romacs-test-wait p) (list (process-status p) (process-exit-status p) (with-current-buffer b (buffer-string)) (marker-position (process-mark p))))
(let* ((b (get-buffer-create "romacs-dbuf")) (p (make-process :name "d" :command (list "sleep" "10") :connection-type 'pipe :buffer b))) (delete-process p) (list (process-status p) (process-exit-status p) (with-current-buffer b (buffer-string)) (memq p (process-list))))
(let* ((b (get-buffer-create "romacs-sbuf")) (p (make-process :name "s" :command (list "cat") :connection-type 'pipe :buffer b))) (with-current-buffer b (insert "pre") (goto-char 2)) (process-send-string p "hello\n") (process-send-eof p) (romacs-test-wait p) (with-current-buffer b (list (buffer-string) (point) (marker-position (process-mark p)))))
(let* ((b (get-buffer-create "romacs-ubuf")) (p (make-process :name "u" :command (list "cat") :connection-type 'pipe :buffer b :coding 'utf-8))) (process-send-string p "λ→😀\n") (process-send-eof p) (romacs-test-wait p) (list (with-current-buffer b (buffer-string)) (process-coding-system p)))
(let* ((chunks nil) (p (make-process :name "split" :command (list "sh" "-c" "printf '\\316'; sleep 0.05; printf '\\273\\n'") :connection-type 'pipe :coding 'utf-8 :filter (lambda (_p s) (setq chunks (cons s chunks)))))) (romacs-test-wait p) (reverse chunks))
(let ((p (make-process :name "tty" :command (list "sh" "-c" "tty >/dev/null && echo tty || echo notty") :buffer (get-buffer-create "romacs-ttybuf")))) (romacs-test-wait p) (list (with-current-buffer "romacs-ttybuf" (buffer-string)) (stringp (process-tty-name p)) (process-tty-name p 'stderr)))
(let ((p (make-process :name "sg" :command (list "sleep" "10") :connection-type 'pipe))) (list (signal-process p 'TERM) (progn (romacs-test-wait p) (process-status p)) (process-exit-status p) (condition-case e (signal-process 0 'NOSUCHSIG) (error e)) (signal-process "romacs-nope" 0)))
(let ((p (make-pipe-process :name "pp" :noquery t))) (list (process-status p) (process-id p) (process-contact p) (process-contact p t) (process-type p) (buffer-name (process-buffer p)) (process-query-on-exit-flag p) (progn (stop-process p) (process-status p)) (progn (continue-process p) (process-status p)) (progn (delete-process p) (process-status p)) (process-list)))
(let* ((buf (get-buffer-create "romacs-pipebuf")) (pipe (make-pipe-process :name "errpipe" :buffer buf :noquery t)) (p (make-process :name "w" :command (list "sh" "-c" "echo e1 >&2; echo o1") :connection-type 'pipe :stderr pipe :buffer (get-buffer-create "romacs-wbuf")))) (romacs-test-wait p) (romacs-test-wait pipe) (list (with-current-buffer buf (buffer-string)) (with-current-buffer "romacs-wbuf" (buffer-string)) (process-status pipe)))
(let ((p (make-process :name "st" :command (list "true") :connection-type 'pipe :stderr "romacs-stderr"))) (romacs-test-wait p) (list (mapcar #'process-name (process-list)) (process-name (get-buffer-process "romacs-stderr")) (progn (delete-process (get-buffer-process "romacs-stderr")) (process-list))))
(list (process-name (make-pipe-process :name "dup")) (process-name (make-pipe-process :name "dup")) (process-name (make-pipe-process :name "dup")) (progn (mapc #'delete-process (process-list)) (process-list)))
(let ((p (make-pipe-process :name "pl" :plist (list 'a 1)))) (prog1 (list (process-plist p) (set-process-plist p (list 3 4)) (condition-case e (set-process-plist p 5) (error e)) (set-process-query-on-exit-flag p nil) (process-query-on-exit-flag p)) (delete-process p)))
(with-temp-buffer (let ((p (make-pipe-process :name "gbp" :buffer (current-buffer)))) (prog1 (list (eq (get-buffer-process (current-buffer)) p) (eq (process-type nil) 'pipe) (eq (get-process "gbp") p) (eq (get-process p) p)) (delete-process p))))
(let* ((b (get-buffer-create "romacs-zbuf")) (p (make-pipe-process :name "z" :buffer b))) (process-send-string p "abc") (prog1 (list (accept-process-output p 0.05) (with-current-buffer b (buffer-string)) (marker-position (process-mark p))) (delete-process p)))
(let* ((b (get-buffer-create "romacs-ffbuf")) (p (make-process :name "ff" :command (list "printf" "xy") :connection-type 'pipe :filter t :buffer b))) (romacs-test-wait p) (list (process-filter p) (with-current-buffer b (buffer-string)) (process-status p)))
(let* ((r nil) (p (make-process :name "md" :command (list "sh" "-c" "echo a; exit 1") :connection-type 'pipe :sentinel (lambda (_p m) (string-match "x" "xyz") (setq r m))))) (string-match "b" "abc") (romacs-test-wait p) (list r (match-data)))
(let ((p (make-process :name "late" :command (list "sh" "-c" "sleep 0.05; echo late; read x") :connection-type 'pipe :buffer (get-buffer-create "romacs-late")))) (list (accept-process-output p) (with-current-buffer "romacs-late" (buffer-string)) (progn (process-send-eof p) (romacs-test-wait p) (process-status p))))
(list (accept-process-output) (accept-process-output nil 0) (accept-process-output nil 0.01) (condition-case e (accept-process-output 1) (error e)) (condition-case e (accept-process-output nil "x") (error e)))
(let ((p (make-process :name "e" :command (list "true") :connection-type 'pipe))) (romacs-test-wait p) (list (condition-case e (process-send-string p "x") (error e)) (condition-case e (process-send-eof p) (error e)) (condition-case e (process-send-string p 3) (error e))))
(with-temp-buffer (list (call-process "sh" nil t nil "-c" "echo out; echo err >&2; exit 3") (buffer-string)))
(with-temp-buffer (list (call-process "sh" nil (list t nil) nil "-c" "echo out; echo err >&2") (call-process "sh" nil (list (current-buffer) nil) nil "-c" "echo out2; echo err2 >&2") (buffer-string)))
(list (call-process "sh" nil nil nil "-c" "exit 4") (call-process "sh" nil 0 nil "-c" "exit 4") (call-process "sh" nil nil nil "-c" "kill -9 $$") (call-process "sh" nil nil nil "-c" "kill -15 $$"))
(progn (call-process "sh" nil (list :file "out.txt") nil "-c" "echo filed") (with-temp-buffer (list (call-process "cat" "out.txt" t) (buffer-string))))
(progn (call-process "sh" nil (list nil "err.txt") nil "-c" "echo hidden; echo shown >&2") (with-temp-buffer (call-process "cat" "err.txt" t) (buffer-string)))
(with-temp-buffer (insert "abc\ndef\n") (list (call-process-region (point-min) (point-max) "tr" t t nil "a-z" "A-Z") (buffer-string) (call-process-region "xyz" nil "cat" nil t) (buffer-string) (call-process-region nil nil "wc" t t nil "-c") (buffer-string)))
(list (condition-case e (call-process "romacs-no-such-program") (error e)) (condition-case e (call-process "true" "/romacs/no/such/infile") (error e)) (progn (setq default-directory "/romacs/no/such/dir/") (condition-case e (call-process "true") (error e))))
(with-temp-buffer (list (call-process "printf" nil t nil "λ") (buffer-string) (multibyte-string-p (buffer-string)) last-coding-system-used))
(with-temp-buffer (list (call-process "pwd" nil t) (equal (buffer-string) (concat (directory-file-name default-directory) "\n"))))
(let* ((log nil) (srv (make-network-process :name "srv" :server t :host "127.0.0.1" :service t :noquery t :family 'ipv4 :log (lambda (s c m) (setq log (cons (list (process-name s) (string-match "\\`srv <127.0.0.1:[0-9]+>\\'" (process-name c)) m) log))) :filter (lambda (p s) (setq log (cons (list 'got s) log)) (process-send-string p (upcase s))) :sentinel (lambda (p m) (setq log (cons (list 'sentinel (process-status p) m) log))))) (port (process-contact srv :service)) (got "") (c (make-network-process :name "cli" :host "127.0.0.1" :service port :noquery t :filter (lambda (_p s) (setq got (concat got s)))))) (process-send-string c "hello") (while (string= got "") (accept-process-output c 1)) (delete-process c) (accept-process-output nil 0.05) (prog1 (list got (reverse log) (process-status c) (process-status srv) (length (process-list)) (process-buffer srv) (integerp port) (process-contact srv :host)) (delete-process srv)))
(let* ((srv (make-network-process :name "s2" :server t :host "127.0.0.1" :service t :noquery t :family 'ipv4)) (c (make-network-process :name "c2" :host "127.0.0.1" :service (process-contact srv :service) :noquery t :buffer (get-buffer-create "romacs-c2")))) (while (< (length (process-list)) 3) (accept-process-output nil 1)) (let ((peer (car (process-list)))) (process-send-string peer "data") (process-send-eof peer) (romacs-test-wait c) (prog1 (list (process-status c) (with-current-buffer "romacs-c2" (buffer-string)) (string-match "\\`s2 <127.0.0.1:[0-9]+>\\'" (buffer-name (process-buffer peer))) (process-status peer) (process-type peer)) (delete-process srv))))
(let* ((events nil) (srv (make-network-process :name "s3" :server t :host "127.0.0.1" :service t :noquery t :family 'ipv4)) (c (make-network-process :name "c3" :host "127.0.0.1" :service (process-contact srv :service) :nowait t :noquery t :sentinel (lambda (p m) (setq events (cons (list (process-status p) m) events)))))) (let ((first (process-status c))) (while (null events) (accept-process-output nil 1)) (prog1 (list first events (process-status c)) (delete-process c) (delete-process srv))))
(let* ((events nil) (c (make-network-process :name "c4" :host "127.0.0.1" :service 1 :nowait t :noquery t :sentinel (lambda (p m) (setq events (cons (list (process-status p) m) events)))))) (let ((first (process-status c))) (while (null events) (accept-process-output nil 1)) (prog1 (list first events (process-status c) (process-exit-status c)) (delete-process c))))
(list (condition-case e (make-network-process :name "bad" :host "127.0.0.1" :service 1 :noquery t) (error e)) (process-list))
(let ((srv (make-network-process :name "s5" :server t :host "127.0.0.1" :service t :noquery t :family 'ipv4 :buffer "romacs-s5"))) (prog1 (list (process-status srv) (buffer-name (process-buffer srv)) (process-id srv) (process-command srv) (process-type srv) (process-query-on-exit-flag srv) (car (process-contact srv)) (plist-get (process-contact srv t) :server)) (delete-process srv)))
(let ((p (make-pipe-process :name "kb" :buffer "romacs-kb" :noquery t))) (kill-buffer "romacs-kb") (list (process-status p) (process-list)))
(let ((p (make-process :name "kb2" :noquery t :buffer "romacs-kb2" :command (list "sleep" "10") :connection-type 'pipe))) (kill-buffer "romacs-kb2") (romacs-test-wait p) (list (process-status p) (process-exit-status p)))
"###;

/// The prologue both sides run: GNU's batch UTF-8 process coding and a
/// wait for a process to finish.
const PROLOGUE: &str = r#"(progn
  (setq default-process-coding-system '(utf-8-unix . utf-8-unix))
  (defun romacs-test-wait (p)
    (while (memq (process-status p) '(run open listen connect stop))
      (accept-process-output p 1))
    (accept-process-output nil 0.01)))"#;

/// The loadup chain the daily-driver consumers run on (real_ert_load.rs).
fn bootstrapped() -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    let lisp = root.join("gnu-lisp");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('='))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    paths.push(root.join("rlisp"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("rlisp/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

#[test]
fn processes_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let base = std::env::temp_dir().join(format!("romacs-process-test-{}", std::process::id()));
    let mut vm = bootstrapped();
    vm.eval_source(PROLOGUE).unwrap();
    let mut failures = Vec::new();
    for (index, expression) in CASES.lines().filter(|line| !line.is_empty()).enumerate() {
        let directory = |side: &str| {
            let directory = base.join(format!("{index}-{side}"));
            std::fs::create_dir_all(&directory).unwrap();
            format!("{}/", directory.display())
        };
        // As character codes, as tests/json.rs compares.
        let source = |side: &str| format!(
            "(progn (setq default-directory {:?}) (append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil))",
            directory(side));
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", PROLOGUE, "--eval", &format!("(prin1 {})", source("gnu"))])
            .output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source("romacs")).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    drop(vm);
    std::fs::remove_dir_all(&base).unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
