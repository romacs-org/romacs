//! A busy session stays inspectable (debugger.md §11.1): its evaluator spins
//! in Lisp under `inhibit-quit', and a client of `inspect.sock', on no thread
//! of the evaluator, still gets the call stack; the spin goes on unchanged.
#[path = "support/installed.rs"]
mod installed;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Session { child: Child, dir: PathBuf }

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A session that listens, then runs BODY, which calls `inspect-ready` where
/// the test may begin: from inside the frame the test asserts on
/// (`inspect-spin` calls it), or just before a native call, which a test
/// then polls for (`inspect_until`). The marker exists while it is being
/// written, so only the frames around it are known when it appears.
fn session(name: &str, body: &str) -> Session {
    let dir = std::env::temp_dir().join(format!("ri-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(r#"(setq debug-on-event nil)
(defun inspect-ready () (with-temp-file (expand-file-name "ready" (getenv "MARK")) (insert "ready")))
(defun inspect-spin (tag) (let ((inhibit-quit t) (n 0)) (inspect-ready) (while t (setq n (1+ n)))))
(romacs--inspect-listen (concat (getenv "MARK") "/") (concat (getenv "MARK") "/store"))
{body}
"#);
    std::fs::write(dir.join("busy.el"), source).unwrap();
    let child = Command::new(installed::romacs())
        .args(["-Q", "--batch", "-l"]).arg(dir.join("busy.el")).current_dir(&dir)
        .env("HOME", &dir).env("MARK", &dir)
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let start = Instant::now();
    while !dir.join("ready").exists() {
        assert!(start.elapsed() < Duration::from_secs(60), "{name}: the session never began");
        std::thread::sleep(Duration::from_millis(20));
    }
    Session { child, dir }
}

fn inspect(dir: &Path) -> (serde_json::Value, Duration) {
    let start = Instant::now();
    let mut stream = UnixStream::connect(dir.join("inspect.sock")).unwrap();
    stream.write_all(b"inspect\n").unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    (serde_json::from_str(&answer).unwrap_or_else(|error| panic!("{error}: {answer}")), start.elapsed())
}

/// Inspect until DONE holds of the answer, or a minute has passed: the
/// last answer and how long it took.
fn inspect_until(dir: &Path, done: impl Fn(&serde_json::Value) -> bool) -> (serde_json::Value, Duration) {
    let start = Instant::now();
    loop {
        let (answer, took) = inspect(dir);
        if done(&answer) || start.elapsed() > Duration::from_secs(60) { return (answer, took); }
    }
}

fn calls(answer: &serde_json::Value, name: &str) -> bool {
    answer["snapshot"]["frames"].as_array().is_some_and(|frames| frames.iter().any(|frame| frame["fn"] == name))
}

fn spins(answer: &serde_json::Value) -> bool {
    calls(answer, "inspect-spin")
}

#[test]
fn a_spinning_session_answers_and_keeps_spinning() {
    let mut session = session("spin", r#"(inspect-spin "marker")"#);
    let (first, took) = inspect(&session.dir);
    assert_eq!(first["status"], "ok", "{first}");
    assert!(spins(&first), "{first}");
    assert!(took < Duration::from_secs(1), "{took:?}");
    let (second, _) = inspect(&session.dir);
    assert!(second["generation"].as_u64() > first["generation"].as_u64(), "{second}");
    assert!(session.child.try_wait().unwrap().is_none(), "the session ended");
}

#[test]
fn sigusr2_saves_a_snapshot() {
    let mut session = session("usr2", r#"(inspect-spin "marker")"#);
    assert!(Command::new("kill").args(["-USR2", &session.child.id().to_string()]).status().unwrap().success());
    let start = Instant::now();
    let saved = loop {
        let found = std::fs::read_dir(session.dir.join("store")).ok()
            .and_then(|mut entries| entries.find_map(|entry| {
                let path = entry.ok()?.path();
                path.extension().is_some_and(|e| e == "json").then_some(path)
            }));
        if let Some(path) = found { break path; }
        assert!(start.elapsed() < Duration::from_secs(10), "no snapshot was saved");
        std::thread::sleep(Duration::from_millis(20));
    };
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
    assert!(spins(&saved), "{saved}");
    assert!(session.child.try_wait().unwrap().is_none(), "the session ended");
}

#[test]
fn a_session_with_no_safe_point_says_so_in_time() {
    let session = session("wait", r#"(inspect-ready) (sleep-for 60)"#);
    let (answer, took) = inspect_until(&session.dir, |answer| answer["status"] != "ok");
    assert_eq!(answer["status"], "busy", "{answer}");
    assert_eq!(answer["detail"], "no safe point");
    assert!(took < Duration::from_secs(4), "{took:?}");
}

#[test]
fn a_restarted_listener_owns_its_own_store() {
    let session = session("restart", r#"(romacs--inspect-listen (concat (getenv "MARK") "/")) (inspect-spin "marker")"#);
    let (answer, _) = inspect(&session.dir);
    assert_eq!(answer["status"], "ok", "{answer}");
    std::thread::sleep(Duration::from_millis(300));
    let saved = std::fs::read_dir(session.dir.join("store")).map_or(0, |entries| entries.count());
    assert_eq!(saved, 0, "a stopped writer took a later listener's snapshot");
}

#[test]
fn a_slow_client_does_not_hold_the_listener() {
    let session = session("slow", r#"(inspect-spin "marker")"#);
    let mut slow = UnixStream::connect(session.dir.join("inspect.sock")).unwrap();
    slow.write_all(b"in").unwrap();
    let keep = std::thread::spawn(move || {
        for _ in 0..3 { std::thread::sleep(Duration::from_millis(300)); if slow.write_all(b"s").is_err() { break; } }
    });
    std::thread::sleep(Duration::from_millis(100));
    let (answer, took) = inspect(&session.dir);
    assert_eq!(answer["status"], "ok", "{answer}");
    assert!(took < Duration::from_secs(2), "{took:?}");
    let _ = keep.join();
}

#[test]
fn a_long_motion_is_inspected_and_finishes() {
    let session = session("motion", r#"(switch-to-buffer (generate-new-buffer "big"))
(insert (make-string 5000000 ?\n))
(goto-char 1)
(inspect-ready)
(let ((moved (vertical-motion 5000000)))
  (with-temp-file (expand-file-name "moved" (getenv "MARK")) (insert (format "%d" moved))))
(inspect-spin "marker")"#);
    let start = Instant::now();
    let moved = session.dir.join("moved");
    let (answer, _) = inspect_until(&session.dir, |answer| calls(answer, "vertical-motion") || moved.exists());
    assert_eq!(answer["status"], "ok", "{answer}");
    assert!(calls(&answer, "vertical-motion"), "{answer}");
    while !session.dir.join("moved").exists() {
        assert!(start.elapsed() < Duration::from_secs(120), "the motion never ended");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(std::fs::read_to_string(session.dir.join("moved")).unwrap(), "5000000");
}

#[test]
fn a_long_search_is_inspected_and_goes_on() {
    let mut session = session("search", r#"(inspect-ready) (string-match "^\\(\\(a\\)\\2?\\)*$" (concat (make-string 48 ?a) "b"))"#);
    let (answer, took) = inspect_until(&session.dir, |answer| calls(answer, "string-match"));
    assert_eq!(answer["status"], "ok", "{answer}");
    assert_eq!(answer["snapshot"]["in"], "regexp", "{answer}");
    let names: Vec<_> = answer["snapshot"]["frames"].as_array().unwrap().iter().filter_map(|frame| frame["fn"].as_str()).collect();
    assert!(names.contains(&"string-match"), "{names:?}");
    assert!(took < Duration::from_secs(1), "{took:?}");
    assert!(session.child.try_wait().unwrap().is_none(), "the session ended");
}

#[test]
fn only_the_one_verb_is_answered() {
    let session = session("verb", r#"(inspect-spin "marker")"#);
    let mut stream = UnixStream::connect(session.dir.join("inspect.sock")).unwrap();
    stream.write_all(b"eval (kill-emacs)\n").unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    assert_eq!(answer, "");
    let (answer, _) = inspect(&session.dir);
    assert_eq!(answer["status"], "ok", "{answer}");
}
