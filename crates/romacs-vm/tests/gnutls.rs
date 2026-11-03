//! gnutls.c's client sessions against pinned GNU: both engines connect to
//! one loopback rustls server with a fixture authority (tests/fixtures/tls,
//! a test-only CA and localhost certificate) and must agree on gnutls-boot's
//! results and errors, the peer status and its certificate details, the
//! records read and written, a STARTTLS upgrade, a :nowait connection's
//! :tls-parameters and the error predicates. GNU's answers are pinned in
//! tests/fixtures/tls/gnu-answers.json with the build that gave them, so
//! the gate never runs GNU; ROMACS_GNUTLS_LIVE=1 asks GNU live instead and
//! ROMACS_GNUTLS_RECORD=1 rewrites the pins (tests/fixtures/tls/regenerate.sh),
//! each GNU call bounded in time and retried a few times.
use romacs_vm::Vm;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls").join(name)
}

/// The server's TLS configuration: the fixture certificate and key.
fn server_config() -> Arc<rustls::ServerConfig> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs = CertificateDer::pem_file_iter(fixture("server.pem")).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    let key = PrivateKeyDer::from_pem_file(fixture("server-key.pem")).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(rustls::ServerConfig::builder_with_provider(provider).with_safe_default_protocol_versions().unwrap()
        .with_no_client_auth().with_single_cert(certs, key).unwrap())
}

/// Each line a client sends comes back as "echo: LINE" until it sends
/// "bye"; a connection that first says STARTTLS in plain text is answered
/// "220 go" and continues in TLS.
fn serve(stream: TcpStream, config: Arc<rustls::ServerConfig>) {
    let mut plain = BufReader::new(stream.try_clone().unwrap());
    let mut first = [0u8; 1];
    if stream.peek(&mut first).is_err() { return; }
    if first[0] == b'S' {
        let mut line = String::new();
        if plain.read_line(&mut line).is_err() { return; }
        (&stream).write_all(b"220 go\r\n").unwrap();
    }
    let conn = rustls::ServerConnection::new(config).unwrap();
    let mut tls = BufReader::new(rustls::StreamOwned::new(conn, stream));
    let mut line = String::new();
    while tls.read_line(&mut line).is_ok_and(|n| n > 0) {
        if line.trim() == "bye" { break; }
        let reply = format!("echo: {line}");
        if tls.get_mut().write_all(reply.as_bytes()).is_err() { return; }
        let _ = tls.get_mut().flush();
        line.clear();
    }
    tls.get_mut().conn.send_close_notify();
    let _ = tls.get_mut().flush();
    let _ = tls.get_mut().sock.read(&mut [0; 1]);
}

fn start_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = server_config();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let config = config.clone();
            std::thread::spawn(move || serve(stream, config));
        }
    });
    port
}

/// The cases, with PORT and CA substituted; each leaves no process.
fn cases(port: u16) -> Vec<String> {
    let ca = fixture("ca.pem").display().to_string();
    let open = |name: &str| format!(r#"(make-network-process :name "{name}" :host "127.0.0.1" :service {port} :buffer (get-buffer-create "{name}") :coding 'utf-8 :noquery t)"#);
    let status = r#"(let ((s (gnutls-peer-status p)))
      (list (plist-get s :warnings) (plist-get s :protocol) (plist-get s :cipher) (plist-get s :key-exchange) (plist-get s :mac)
            (plist-member s :encrypt-then-mac) (plist-member s :safe-renegotiation) (equal (plist-get s :certificate) (car (plist-get s :certificates)))
            (mapcar (lambda (c) (list (plist-get c :version) (plist-get c :serial-number) (plist-get c :issuer) (plist-get c :valid-from)
                                      (plist-get c :valid-to) (plist-get c :subject) (plist-get c :public-key-algorithm)
                                      (plist-get c :certificate-security-level) (plist-get c :signature-algorithm) (plist-get c :public-key-id)
                                      (plist-get c :public-key-id-sha256) (plist-get c :certificate-id) (plist-get c :pem)))
                    (plist-get s :certificates))))"#;
    let echo = |name: &str| format!(r#"(progn (process-send-string p "hello λ\n") (while (not (string-search "\n" (with-current-buffer "{name}" (buffer-string)))) (accept-process-output p 1)) (with-current-buffer "{name}" (buffer-string)))"#);
    vec![
        format!(r#"(let ((p {})) (prog1 (list (gnutls-get-initstage p) (gnutls-boot p 'gnutls-x509pki (list :hostname "localhost" :trustfiles (list {ca:?}) :verify-error t)) (gnutls-get-initstage p) {} {status}) (delete-process p)))"#, open("t1"), echo("t1")),
        format!(r#"(let ((p {})) (prog1 (list (gnutls-boot p 'gnutls-x509pki (list :hostname "localhost" :trustfiles nil)) {status}) (delete-process p)))"#, open("t2")),
        format!(r#"(let ((p {})) (prog1 (condition-case e (gnutls-boot p 'gnutls-x509pki (list :hostname "localhost" :verify-error t)) (error (list 'signal e (gnutls-get-initstage p)))) (delete-process p)))"#, open("t3")),
        format!(r#"(let ((p {})) (prog1 (list (gnutls-boot p 'gnutls-x509pki (list :hostname "127.0.0.1" :trustfiles (list {ca:?}) :verify-error t)) {status}) (delete-process p)))"#, open("t4")),
        format!(r#"(let ((p {})) (prog1 (list (gnutls-boot p 'gnutls-x509pki (list :hostname "wrong.example" :trustfiles (list {ca:?}) :verify-error '(:trustfiles))) (plist-get (gnutls-peer-status p) :warnings)) (delete-process p)))"#, open("t5")),
        format!(r#"(let ((p {})) (prog1 (condition-case e (gnutls-boot p 'gnutls-x509pki (list :hostname "wrong.example" :trustfiles (list {ca:?}) :verify-error '(:hostname))) (error (list 'signal e))) (delete-process p)))"#, open("t6")),
        format!(r#"(let ((p {})) (prog1 (list (gnutls-boot p 'gnutls-x509pki (list :hostname "localhost" :trustfiles (list "/romacs/no/such/ca.pem"))) (condition-case e (gnutls-boot p 'gnutls-bogus nil) (error e)) (condition-case e (gnutls-boot p 'gnutls-x509pki (list :hostname 7)) (error e)) (condition-case e (gnutls-boot p 'gnutls-x509pki (list :hostname "localhost" :trustfiles (list 7))) (error e))) (delete-process p)))"#, open("t7")),
        format!(r#"(let ((p {})) (process-send-string p "STARTTLS\r\n") (while (not (string-search "220 go" (with-current-buffer "t8" (buffer-string)))) (accept-process-output p 1)) (with-current-buffer "t8" (erase-buffer)) (prog1 (list (gnutls-boot p 'gnutls-x509pki (list :hostname "localhost" :trustfiles (list {ca:?}) :verify-error t)) {} (progn (process-send-string p "bye\n") (while (eq (process-status p) 'open) (accept-process-output p 1)) (list (process-status p) (with-current-buffer "t8" (buffer-string))))) (delete-process p)))"#, open("t8"), echo("t8")),
        format!(r#"(let* ((events nil) (p (make-network-process :name "t9" :host "127.0.0.1" :service {port} :buffer (get-buffer-create "t9") :coding 'utf-8 :noquery t :nowait t :sentinel (lambda (p m) (setq events (cons (list (process-status p) m) events))) :tls-parameters (cons 'gnutls-x509pki (gnutls-boot-parameters :hostname "localhost" :trustfiles (list {ca:?})))))) (prog1 (list (process-status p) (progn (while (null events) (accept-process-output nil 1)) events) (gnutls-get-initstage p) (plist-get (gnutls-peer-status p) :warnings) {}) (delete-process p)))"#, echo("t9")),
        format!(r#"(let ((p {})) (prog1 (list (gnutls-boot p 'gnutls-x509pki (list :hostname "localhost" :trustfiles (list {ca:?}) :verify-error t)) (progn (process-send-string p (concat (make-string 200000 ?a) "\n")) (while (< (with-current-buffer "t10" (buffer-size)) 200007) (accept-process-output p 1)) (with-current-buffer "t10" (list (buffer-size) (buffer-substring 1 9) (char-before))))) (delete-process p)))"#, open("t10")),
        r#"(mapcar (lambda (e) (list e (gnutls-errorp e) (gnutls-error-fatalp e) (gnutls-error-string e))) '(t gnutls-e-again gnutls-e-interrupted gnutls-e-invalid-session -8 -9 -12 -15 -24 -28 -32 -43 -50 -52 -53 -54 -59 -64 -87 -110 -326 5))"#.into(),
        r#"(mapcar #'gnutls-peer-status-warning-describe '(:invalid :revoked :self-signed :unknown-ca :not-ca :insecure :not-activated :expired :no-host-match :signature-failure :purpose-mismatch :bogus))"#.into(),
    ]
}

/// The loadup chain the daily-driver consumers run on (process.rs's).
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

/// How long one GNU oracle call may take, and how many times a call that
/// ran out of it is tried.
const GNU_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const GNU_ATTEMPTS: usize = 3;

/// GNU's batch output for ARGS: bounded in time, stdin closed, a call that
/// runs out of time killed and tried again; Err after GNU_ATTEMPTS.
fn gnu(args: &[&str]) -> Result<String, String> {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    for attempt in 1..=GNU_ATTEMPTS {
        let mut child = std::process::Command::new(&binary).args(args)
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
            .spawn().map_err(|e| e.to_string())?;
        let start = std::time::Instant::now();
        while child.try_wait().map_err(|e| e.to_string())?.is_none() && start.elapsed() < GNU_TIMEOUT {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_none() {
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("GNU oracle ran out of {GNU_TIMEOUT:?} (attempt {attempt} of {GNU_ATTEMPTS})");
            continue;
        }
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        return String::from_utf8(output.stdout).map_err(|e| e.to_string());
    }
    Err(format!("GNU oracle gave no answer in {GNU_ATTEMPTS} tries of {GNU_TIMEOUT:?}"))
}

/// CASE as the pins name it: the server's port and the authority's path
/// as placeholders, so a changed case shows as a stale pin.
fn template(case: &str, port: u16) -> String {
    case.replace(&fixture("ca.pem").display().to_string(), "CA").replace(&format!(":service {port}"), ":service PORT")
}

/// The case's printed value as character codes, the form GNU prints it in.
fn as_codes(case: &str) -> String {
    format!("(append (prin1-to-string (condition-case e {case} (error (list 'signal e)))) nil)")
}

/// The oracle's identity: its repository revision and its binary's SHA-256.
fn gnu_identity() -> (String, String) {
    use sha2::Digest;
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let revision = gnu(&["-Q", "--batch", "--eval", "(princ emacs-repository-version)"]).expect("GNU revision");
    let digest = sha2::Sha256::digest(std::fs::read(&binary).expect("GNU binary"));
    (revision, digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn pins_path() -> PathBuf {
    fixture("gnu-answers.json")
}

/// The pins' answer under KEY, or GNU's live to ARGS under ROMACS_GNUTLS_LIVE
/// or ROMACS_GNUTLS_RECORD (which writes it to a pins file of its own,
/// merged by regenerate.sh).
fn pinned_or_live(key: &str, args: &[&str]) -> String {
    let live = std::env::var_os("ROMACS_GNUTLS_LIVE").is_some();
    let record = std::env::var_os("ROMACS_GNUTLS_RECORD").is_some();
    if !live && !record {
        let pins: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(pins_path()).unwrap()).unwrap();
        return pins[key].as_str().unwrap_or_else(|| panic!("no pin {key}: run tests/fixtures/tls/regenerate.sh")).to_owned();
    }
    let answer = gnu(args).unwrap_or_else(|e| panic!("GNU on {key}: {e}"));
    if record {
        std::fs::write(fixture(&format!("gnu-{key}.json")), serde_json::to_string(&serde_json::json!({key: answer})).unwrap()).unwrap();
    }
    answer
}

/// GNU's answer to each case: the pins, or GNU itself under
/// ROMACS_GNUTLS_LIVE or ROMACS_GNUTLS_RECORD (which also rewrites the pins).
fn gnu_answers(cases: &[String], port: u16) -> Vec<String> {
    let live = std::env::var_os("ROMACS_GNUTLS_LIVE").is_some();
    let record = std::env::var_os("ROMACS_GNUTLS_RECORD").is_some();
    if !live && !record {
        let pins: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(pins_path()).unwrap()).unwrap();
        let pinned = pins["cases"].as_array().unwrap();
        assert_eq!(pinned.len(), cases.len(), "stale pins: run tests/fixtures/tls/regenerate.sh");
        return cases.iter().zip(pinned).map(|(case, pin)| {
            assert_eq!(pin["source"].as_str().unwrap(), template(case, port), "stale pin: run tests/fixtures/tls/regenerate.sh");
            pin["gnu"].as_str().unwrap().to_owned()
        }).collect();
    }
    let answers: Vec<String> = cases.iter().enumerate().map(|(index, case)| {
        eprintln!("gnutls case {index}");
        let answer = gnu(&["-Q", "--batch", "--eval", "(require 'gnutls)", "--eval", &format!("(prin1 {})", as_codes(case))])
            .unwrap_or_else(|e| panic!("GNU on case {index}: {e}"));
        eprintln!("gnutls case {index}: GNU answered");
        answer
    }).collect();
    if record {
        let (revision, sha256) = gnu_identity();
        let pins = serde_json::json!({
            "revision": revision, "gnu_binary_sha256": sha256, "regime": "-Q --batch; (require 'gnutls)",
            "cases": cases.iter().zip(&answers).map(|(case, gnu)| serde_json::json!({"source": template(case, port), "gnu": gnu})).collect::<Vec<_>>(),
        });
        std::fs::write(pins_path(), serde_json::to_string_pretty(&pins).unwrap() + "\n").unwrap();
    }
    answers
}

#[test]
fn tls_sessions_match_pinned_gnu() {
    let port = start_server();
    let mut vm = bootstrapped();
    vm.eval_source("(progn (require 'gnutls) (setq default-process-coding-system '(utf-8-unix . utf-8-unix)))").unwrap();
    let cases = cases(port);
    let answers = gnu_answers(&cases, port);
    let mut failures = Vec::new();
    for (index, (case, expected)) in cases.iter().zip(&answers).enumerate() {
        eprintln!("gnutls case {index}: Romacs");
        let actual = vm.eval_source(&as_codes(case)).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{case}\n  GNU    {}\n  Romacs {:?}", text(expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A :nowait connection whose peer its :verify-error refuses fails, closed,
/// its sentinel told why. GNU's wait ignores that verdict (process.c:5466
/// discards gnutls_verify_boot's result) and opens the socket plain, with
/// the session already freed; Romacs does not open it (VM2-TLS1).
#[test]
fn nowait_verification_refusal_fails_the_connection() {
    let port = start_server();
    let mut vm = bootstrapped();
    vm.eval_source("(require 'gnutls)").unwrap();
    let case = format!(r#"(let* ((events nil) (p (make-network-process :name "t11" :host "127.0.0.1" :service {port} :buffer (get-buffer-create "t11") :noquery t :nowait t :sentinel (lambda (p m) (setq events (cons (list (process-status p) m) events))) :tls-parameters (cons 'gnutls-x509pki (gnutls-boot-parameters :hostname "localhost" :trustfiles nil :verify-error t))))) (prog1 (equal (list (progn (while (null events) (accept-process-output nil 1)) events) (process-status p) (process-live-p p)) '(((failed "failed with code Certificate validation failed localhost, verification code 42\n")) failed nil)) (delete-process p)))"#);
    assert!(vm.eval_source(&case).unwrap().is_t());
}

/// Romacs offers TLS but not GnuTLS's crypto primitives, so it claims only
/// the session capability, with GNU's shape for the queries (VM2-TLS1).
#[test]
fn tls_capability_claims_sessions_only() {
    let shape = "(list (func-arity 'gnutls-available-p) (subrp (symbol-function 'gnutls-available-p)) (special-variable-p 'libgnutls-version) (integerp libgnutls-version) (listp (gnutls-available-p)) gnutls-log-level)";
    let gnu = pinned_or_live("capability", &["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {shape}))")]);
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(vm.eval_source(&format!("(prin1-to-string {shape})")).unwrap().display().unwrap(), format!("{gnu:?}"));
    assert_eq!(vm.eval_source("(list (gnutls-available-p) libgnutls-version)").unwrap().display().unwrap(), "((gnutls gnutls3) -1)");
}

/// A 200 KB line sent over TLS comes back whole, every time: twenty
/// echoes, each bounded, so a stall reports how far the echo got instead
/// of hanging the suite.
#[test]
fn a_large_echo_completes_every_time() {
    let port = start_server();
    let mut vm = bootstrapped();
    vm.eval_source("(progn (require 'gnutls) (setq default-process-coding-system '(utf-8-unix . utf-8-unix)))").unwrap();
    let ca = fixture("ca.pem").display().to_string();
    for round in 0..20 {
        let case = format!(r#"(let* ((b (get-buffer-create "echo-{round}"))
                 (p (make-network-process :name "echo-{round}" :host "127.0.0.1" :service {port} :buffer b :coding 'utf-8 :noquery t))
                 (deadline (+ (float-time) 10)))
            (gnutls-boot p 'gnutls-x509pki (list :hostname "localhost" :trustfiles (list {ca:?}) :verify-error t))
            (process-send-string p (concat (make-string 200000 ?a) "\n"))
            (while (and (< (buffer-size b) 200007) (< (float-time) deadline))
              (accept-process-output p 1))
            (prog1 (buffer-size b) (delete-process p) (kill-buffer b)))"#);
        let got = vm.eval_source(&case).unwrap().display().unwrap();
        assert_eq!(got, "200007", "round {round}: the echo stopped at {got} of 200007 bytes");
    }
}


/// A :nowait connection that boots TLS through :tls-parameters opens and
/// tells its sentinel, every time: twenty, each bounded, so a lost event
/// fails the round instead of waiting for ever.
#[test]
fn a_nowait_tls_connection_opens_every_time() {
    let port = start_server();
    let mut vm = bootstrapped();
    vm.eval_source("(require 'gnutls)").unwrap();
    let ca = fixture("ca.pem").display().to_string();
    for round in 0..20 {
        let case = format!(r#"(let* ((events nil)
                  (p (make-network-process :name "nowait-{round}" :host "127.0.0.1" :service {port} :noquery t :nowait t
                       :sentinel (lambda (_p m) (push m events))
                       :tls-parameters (cons 'gnutls-x509pki (gnutls-boot-parameters :hostname "localhost" :trustfiles (list {ca:?})))))
                  (deadline (+ (float-time) 10)))
            (while (and (null events) (< (float-time) deadline))
              (accept-process-output nil 1))
            (prog1 (let ((got (list (process-status p) events)))
                     (if (equal got '(open ("open\n"))) t (prin1-to-string got)))
              (delete-process p)))"#);
        let got = vm.eval_source(&case).unwrap().display().unwrap();
        assert_eq!(got, "t", "round {round}: {got}");
    }
}
