//! gnutls.c's client sessions against pinned GNU: both engines connect to
//! one loopback rustls server with a fixture authority (tests/fixtures/tls,
//! a test-only CA and localhost certificate) and must agree on gnutls-boot's
//! results and errors, the peer status and its certificate details, the
//! records read and written, a STARTTLS upgrade, a :nowait connection's
//! :tls-parameters and the error predicates.
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

#[test]
fn tls_sessions_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let port = start_server();
    let mut vm = bootstrapped();
    vm.eval_source("(progn (require 'gnutls) (setq default-process-coding-system '(utf-8-unix . utf-8-unix)))").unwrap();
    let mut failures = Vec::new();
    for (index, case) in cases(port).into_iter().enumerate() {
        eprintln!("gnutls case {index}");
        let source = format!("(append (prin1-to-string (condition-case e {case} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", "(require 'gnutls)", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        eprintln!("gnutls case {index}: GNU answered");
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{case}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
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
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let shape = "(list (func-arity 'gnutls-available-p) (subrp (symbol-function 'gnutls-available-p)) (special-variable-p 'libgnutls-version) (integerp libgnutls-version) (listp (gnutls-available-p)) gnutls-log-level)";
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {shape}))")])
        .stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(vm.eval_source(&format!("(prin1-to-string {shape})")).unwrap().display().unwrap(), format!("{gnu:?}"));
    assert_eq!(vm.eval_source("(list (gnutls-available-p) libgnutls-version)").unwrap().display().unwrap(), "((gnutls gnutls3) -1)");
}
