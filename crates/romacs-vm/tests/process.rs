//! Subprocesses against pinned GNU (process.c, callproc.c): process objects
//! and their accessors, make-process with pipes and a pty, pipe processes as
//! stderr, filters, sentinels and their default insertion, call-process and
//! call-process-region destinations and statuses, signals, and loopback
//! network servers and clients (blocking and :nowait), decoding carried
//! across reads, waits for children that already exited, sleep-for's
//! deadline and reaping after the VM is gone. Every case starts its own
//! children and waits for them with accept-process-output; none sleeps to
//! synchronize. A child that must split its output across reads waits for a
//! line the test sends once the filter has seen the first part; a part has
//! a newline only at its end, since sh's printf may write before one.
use romacs_vm::Vm;
use std::os::fd::{FromRawFd, OwnedFd};
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
(romacs-test-chunks "printf 'a\\316'; read x; printf '\\273\\n'" 'utf-8 1)
(romacs-test-chunks "printf 'x\\r'; read x; printf '\\n'" 'utf-8-dos 1)
(romacs-test-chunks "printf 'x\\r'; read x; printf 'y\\r'; read x; printf 'z'" 'utf-8-dos 2)
(romacs-test-chunks "printf 'x\\r'; read x; printf '\\n'" 'undecided-dos 1)
(romacs-test-chunks "printf 'x\\r'; read x; printf '\\n'" 'utf-8-unix 1)
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
(let ((p (make-process :name "z1" :command (list "sh" "-c" "echo hi") :connection-type 'pipe :buffer (get-buffer-create "romacs-z1")))) (while (not (romacs-test-exited p))) (let* ((t0 (float-time)) (got (accept-process-output p 2))) (list got (< (- (float-time) t0) 1) (progn (romacs-test-wait p) (process-status p)) (with-current-buffer "romacs-z1" (buffer-string)))))
(let ((p (make-process :name "slp" :command (list "sh" "-c" "echo hi") :connection-type 'pipe :buffer (get-buffer-create "romacs-slp"))) (t0 nil)) (while (not (romacs-test-exited p))) (setq t0 (float-time)) (sleep-for 0.3) (list (>= (- (float-time) t0) 0.3) (progn (romacs-test-wait p) (with-current-buffer "romacs-slp" (buffer-string)))))
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
(let* ((events nil) (srv (make-network-process :name "s6" :server t :host "127.0.0.1" :service t :noquery t :family 'ipv4)) (c (make-network-process :name "c6" :host "localhost" :service (process-contact srv :service) :nowait t :noquery t :sentinel (lambda (p m) (setq events (cons (list (process-status p) m) events)))))) (while (null events) (accept-process-output nil 1)) (prog1 (list events (butlast (append (process-contact c :remote) nil)) (equal (process-contact c :remote) (vconcat [127 0 0 1] (list (process-contact srv :service))))) (mapc #'delete-process (process-list))))
(list (condition-case e (make-network-process :name "bad" :host "127.0.0.1" :service 1 :noquery t) (error e)) (process-list))
(let ((srv (make-network-process :name "s5" :server t :host "127.0.0.1" :service t :noquery t :family 'ipv4 :buffer "romacs-s5"))) (prog1 (list (process-status srv) (buffer-name (process-buffer srv)) (process-id srv) (process-command srv) (process-type srv) (process-query-on-exit-flag srv) (car (process-contact srv)) (plist-get (process-contact srv t) :server)) (delete-process srv)))
(let ((p (make-pipe-process :name "kb" :buffer "romacs-kb" :noquery t))) (kill-buffer "romacs-kb") (list (process-status p) (process-list)))
(let ((p (make-process :name "kb2" :noquery t :buffer "romacs-kb2" :command (list "sleep" "10") :connection-type 'pipe))) (kill-buffer "romacs-kb2") (romacs-test-wait p) (list (process-status p) (process-exit-status p)))
"###;

/// The prologue both sides run: GNU's batch UTF-8 process coding, a wait
/// for a process to finish, whether its child has exited (Linux drops a
/// process's exe link once it is a zombie), and the chunks a filter sees
/// from a child that reads a line before each further part of its output.
const PROLOGUE: &str = r#"(progn
  (setq default-process-coding-system '(utf-8-unix . utf-8-unix))
  (defun romacs-test-wait (p)
    (while (memq (process-status p) '(run open listen connect stop))
      (accept-process-output p 1))
    (accept-process-output nil 0.01))
  (defun romacs-test-note (_p m) (setq romacs-test-events (cons m romacs-test-events)))
  (defun romacs-test-exited (p)
    (not (file-symlink-p (format "/proc/%d/exe" (process-id p)))))
  (defun romacs-test-chunks (command coding phases)
    (let* ((chunks nil)
           (p (make-process :name "chunks" :command (list "sh" "-c" command) :connection-type 'pipe
                            :coding coding :filter (lambda (_p s) (setq chunks (cons s chunks))))))
      (dotimes (i phases)
        (while (<= (length chunks) i) (accept-process-output p 1))
        (process-send-string p "go\n"))
      (romacs-test-wait p)
      (reverse chunks))))"#;

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

/// Run each of EXPRESSIONS in Romacs's VM and in pinned GNU, each in a
/// private `default-directory' under BASE; the mismatches.
fn mismatches(vm: &mut Vm, expressions: &[&str], base: &Path) -> Vec<String> {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for (index, expression) in expressions.iter().enumerate() {
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
    failures
}

fn scratch(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("romacs-process-{name}-{}", std::process::id()))
}

#[test]
fn processes_match_pinned_gnu() {
    let base = scratch("test");
    let mut vm = bootstrapped();
    vm.eval_source(PROLOGUE).unwrap();
    let cases: Vec<&str> = CASES.lines().filter(|line| !line.is_empty()).collect();
    let failures = mismatches(&mut vm, &cases, &base);
    drop(vm);
    std::fs::remove_dir_all(&base).unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A loopback listener with a full accept queue: a further connection's SYN
/// is dropped, so it stays in progress. Returns the listener, the queued
/// connections and the port.
fn saturated_listener() -> (OwnedFd, Vec<std::net::TcpStream>, u16) {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    assert!(fd >= 0);
    let listener = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut sin: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sin.sin_family = libc::AF_INET as libc::sa_family_t;
    sin.sin_addr = libc::in_addr { s_addr: u32::from_ne_bytes([127, 0, 0, 1]) };
    let mut len = size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let sa = (&mut sin as *mut libc::sockaddr_in).cast::<libc::sockaddr>();
    assert_eq!(unsafe { libc::bind(fd, sa, len) }, 0);
    assert_eq!(unsafe { libc::listen(fd, 0) }, 0);
    assert_eq!(unsafe { libc::getsockname(fd, sa, &mut len) }, 0);
    let port = u16::from_be(sin.sin_port);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    // Fill the queue until a connection is no longer answered.
    let mut queued = Vec::new();
    loop {
        match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)) {
            Ok(stream) => queued.push(stream),
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => break,
            Err(error) => panic!("filling the accept queue: {error}"),
        }
        assert!(queued.len() < 64, "the accept queue never filled");
    }
    (listener, queued, port)
}

// GNU's :nowait connect is nonblocking: it returns in `connect' status even
// when the peer never answers, and stays there while the wait polls it.
#[test]
fn nowait_connect_does_not_block_like_gnu() {
    let base = scratch("nowait");
    let (_listener, _queued, port) = saturated_listener();
    let mut vm = bootstrapped();
    vm.eval_source(PROLOGUE).unwrap();
    let case = format!("(let* ((t0 (float-time)) (c (make-network-process :name \"sat\" :host \"127.0.0.1\" :service {port} :nowait t :noquery t))) (prog1 (list (process-status c) (< (- (float-time) t0) 1) (progn (accept-process-output nil 0.05) (process-status c)) (process-contact c :remote)) (delete-process c)))");
    let failures = mismatches(&mut vm, &[&case], &base);
    drop(vm);
    std::fs::remove_dir_all(&base).unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Local (Unix domain) sockets against pinned GNU: a server's contact and
/// its socket file (which `delete-process' leaves, as GNU does), an echo
/// round trip with the accepted connection's name, contact and sentinels,
/// :nowait connections that open or fail, GNU's errors, and the :local and
/// :remote string address forms. @DIR@ is a short directory for the socket
/// files (sun_path holds 108 bytes).
const LOCAL_CASES: &str = r###"
(let* ((f "@DIR@/a") (p (make-network-process :name "srv" :server t :family 'local :service f :noquery t))) (unwind-protect (list (process-contact p) (process-contact p :local) (process-contact p :service) (process-contact p :host) (process-contact p t) (file-exists-p f) (process-status p) (process-type p) (progn (delete-process p) (list (process-status p) (file-exists-p f) (process-list)))) (delete-file f)))
(let* ((f "@DIR@/b") (log nil) (srv (make-network-process :name "srv" :server t :family 'local :service f :noquery t :log (lambda (s c m) (setq log (cons (list (process-name s) (string-match "\\`srv <[0-9]+>\\'" (process-name c)) m) log))) :filter (lambda (p s) (setq log (cons (list 'got s) log)) (process-send-string p (upcase s))) :sentinel (lambda (p m) (setq log (cons (list 'sentinel (process-status p) m) log))))) (got "") (c (make-network-process :name "cli" :family 'local :service f :noquery t :filter (lambda (_p s) (setq got (concat got s)))))) (unwind-protect (progn (process-send-string c "hello") (while (string= got "") (accept-process-output c 1)) (let ((peer (car (process-list)))) (delete-process c) (accept-process-output nil 0.05) (list got (reverse log) (process-status c) (process-status srv) (length (process-list)) (process-contact c) (process-contact c :remote) (process-contact c :local) (process-contact peer :host) (process-contact peer :service) (process-contact peer :remote) (process-contact peer :local) (process-contact peer :server) (process-buffer srv) (process-status peer)))) (mapc #'delete-process (process-list)) (delete-file f)))
(let* ((f "@DIR@/c") (events nil) (srv (make-network-process :name "s3" :server t :family 'local :service f :noquery t)) (c (make-network-process :name "c3" :family 'local :service f :nowait t :noquery t :sentinel (lambda (p m) (setq events (cons (list (process-status p) m) events)))))) (unwind-protect (let ((first (process-status c))) (while (null events) (accept-process-output nil 1)) (list first events (process-status c) (process-contact c :remote) (process-contact c :local))) (mapc #'delete-process (process-list)) (delete-file f)))
(let* ((c (progn (setq romacs-test-events nil) (make-network-process :name "c4" :family 'local :service "@DIR@/nope" :nowait t :noquery t :sentinel #'romacs-test-note)))) (let ((first (process-status c))) (accept-process-output nil 0.2) (prog1 (list first romacs-test-events (process-status c) (process-exit-status c) (process-contact c :remote) (process-contact c :local) (condition-case e (process-send-string c "x") (error e))) (delete-process c) (accept-process-output nil 0.05) romacs-test-events)))
(list (format-network-address [127 0 0 1 80]) (format-network-address [127 0 0 1 80] t) (format-network-address [256 0 0 1]) (format-network-address [0 0 0 0 0 0 0 1 8080]) (format-network-address [0 0 0 0 0 0 0 1 8080] t) (format-network-address [1 2 3]) (format-network-address '(42 . [1 2])) (format-network-address [1 2 3 70000 1]) (format-network-address "x") (format-network-address nil))
(let ((f "@DIR@/e")) (list (condition-case e (make-network-process :name "x" :family 'local :service f :noquery t) (error e)) (condition-case e (make-network-process :name "x" :family 'local :service 7) (error e)) (condition-case e (make-network-process :name "x" :family 'local :service (make-string 108 ?a)) (error e)) (condition-case e (make-network-process :name "x" :family 'local :service "@DIR@/nope" :server t :nowait t) (error e)) (let ((p (make-network-process :name "srv" :server t :family 'local :service f :noquery t))) (unwind-protect (list (condition-case e (make-network-process :name "x" :family 'local :service f :server t :noquery t) (error e)) (process-contact (make-network-process :name "h" :family 'local :host "h" :service f :noquery t) t) (process-contact (make-network-process :name "l" :family 'local :host 'local :service f :noquery t) t)) (mapc #'delete-process (process-list)) (delete-file f))) (process-list)))
(let* ((f "@DIR@/f") (srv (make-network-process :name "srv" :server t :local f :noquery t :host "ignored" :service 99)) (c (make-network-process :name "cli" :remote f :noquery t))) (unwind-protect (list (process-contact srv t) (process-contact c t) (featurep 'make-network-process '(:family local)) (process-status c) (process-status srv) (format-network-address f)) (mapc #'delete-process (process-list)) (delete-file f)))
"###;

#[test]
fn local_sockets_match_pinned_gnu() {
    let base = scratch("local");
    // Short paths: sun_path holds 108 bytes.
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let sockets = runtime.join(format!("romacs-ls-{}", std::process::id()));
    std::fs::create_dir_all(&sockets).unwrap();
    let cases: Vec<String> = LOCAL_CASES.lines().filter(|line| !line.is_empty())
        .map(|line| line.replace("@DIR@", sockets.to_str().unwrap())).collect();
    let cases: Vec<&str> = cases.iter().map(String::as_str).collect();
    let mut vm = bootstrapped();
    vm.eval_source(PROLOGUE).unwrap();
    let failures = mismatches(&mut vm, &cases, &base);
    drop(vm);
    std::fs::remove_dir_all(&base).unwrap();
    std::fs::remove_dir_all(&sockets).unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// A host outlives its VM: children the VM leaves, running or deleted, are
// still reaped once they exit, not left as zombies.
#[test]
fn dropped_vm_children_are_reaped() {
    let mut vm = bootstrapped();
    let pids = vm.eval_source(r#"(progn (setq default-directory "/") (list (process-id (make-process :name "live" :command (list "sleep" "10") :connection-type 'pipe :noquery t))
                                       (let ((p (make-process :name "dead" :command (list "sleep" "10") :connection-type 'pipe :noquery t))) (delete-process p) (process-id p))))"#)
        .and_then(|v| v.display()).unwrap();
    let pids: Vec<u32> = pids.trim_matches(|c| c == '(' || c == ')').split_whitespace().map(|n| n.parse().unwrap()).collect();
    assert_eq!(pids.len(), 2);
    drop(vm);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    for pid in pids {
        let entry = format!("/proc/{pid}");
        while Path::new(&entry).exists() {
            assert!(std::time::Instant::now() < deadline, "child {pid} was not reaped: {}",
                    std::fs::read_to_string(format!("{entry}/stat")).unwrap_or_default());
            std::thread::yield_now();
        }
    }
}

fn reaper_threads() -> usize {
    std::fs::read_dir("/proc/self/task").unwrap().flatten()
        .filter(|task| std::fs::read_to_string(task.path().join("comm")).is_ok_and(|comm| comm == "romacs-reaper\n"))
        .count()
}

// Children that outlive their VM's hangup share the host's one reaper
// thread, however many there are, and are each reaped once they exit.
#[test]
fn dropped_vm_children_share_one_reaper() {
    let base = scratch("reaper");
    std::fs::create_dir_all(&base).unwrap();
    let fifo = base.join("release");
    let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    // Held open for writing, so each child's open of the FIFO returns.
    let release = std::fs::OpenOptions::new().read(true).write(true).open(&fifo).unwrap();
    let mut vm = bootstrapped();
    let script = format!("trap '' HUP; echo ready; read x < {}", fifo.display());
    let pids = vm.eval_source(&format!(r#"(progn (setq default-directory "/") (let* ((out "")
            (ps (mapcar (lambda (_) (make-process :name "hup" :command (list "sh" "-c" {script:?}) :connection-type 'pipe :noquery t
                                                  :filter (lambda (_p s) (setq out (concat out s)))))
                        (number-sequence 1 8))))
       (while (< (length out) (* 8 (length "ready\n"))) (accept-process-output nil 1))
       (mapcar #'process-id ps)))"#))
        .and_then(|v| v.display()).unwrap();
    let pids: Vec<u32> = pids.trim_matches(|c| c == '(' || c == ')').split_whitespace().map(|n| n.parse().unwrap()).collect();
    assert_eq!(pids.len(), 8);
    drop(vm);
    for pid in &pids {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        assert!(!stat.contains(") Z "), "child {pid} did not survive the hangup: {stat}");
    }
    assert_eq!(reaper_threads(), 1);
    use std::io::Write;
    (&release).write_all(&b"\n".repeat(pids.len())).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    for pid in &pids {
        let entry = format!("/proc/{pid}");
        while Path::new(&entry).exists() {
            assert!(std::time::Instant::now() < deadline, "child {pid} was not reaped: {}",
                    std::fs::read_to_string(format!("{entry}/stat")).unwrap_or_default());
            std::thread::yield_now();
        }
    }
    assert_eq!(reaper_threads(), 1);
    drop(release);
    std::fs::remove_dir_all(&base).unwrap();
}
