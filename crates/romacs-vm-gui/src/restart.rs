//! `romacs --restart-supervise PLAN`: a restart's supervisor, run outside
//! the session it restarts (lisp/romacs/warm-restart.el `romacs-restart`).
//! The old session wrote its image and the PLAN, a JSON file, and is
//! ending. The supervisor waits for it to end, starts the target build
//! as the same server, through that build's own client, restoring the
//! image, then asks the new session, over its socket, until it is the
//! target (its executable), shows a window, has run its init and has
//! restored the image, or a deadline passes. A target that fails is
//! stopped and the old build started again with the image kept for it,
//! judged the same way. Every phase is written to the plan's record, a
//! JSON file the session that serves reads back (`romacs_restart_status`),
//! and the serving session is told the verdict (`romacs-restart-verdict`).
use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The form a session answers about itself: what the checks read. Plain
/// Lisp every build has, so an older build being rolled back to answers it.
const HEALTH: &str = r#"(json-serialize
  (list :pid (emacs-pid)
        :exe (expand-file-name invocation-name invocation-directory)
        :init (if after-init-time t :false)
        :window (if (seq-some (lambda (f) (and (frame-visible-p f) (display-graphic-p f))) (frame-list)) t :false)
        :restore (let ((step (and (boundp 'romacs-progress-state) (alist-get 'restore romacs-progress-state))))
                   (cond (step (symbol-name (plist-get step :status)))
                         ((bound-and-true-p warm-restart-last-report) "done")
                         (t "pending")))
        :restoreError (or (plist-get (bound-and-true-p warm-restart-last-report) :error) :null)
        :agents (let ((step (and (boundp 'romacs-progress-state) (alist-get 'agents romacs-progress-state))))
                  (if step (symbol-name (plist-get step :status)) :null))))"#;

/// One side of a restart: the build that starts, and how.
#[derive(Clone, Debug, PartialEq)]
struct Side {
    exe: PathBuf,
    flags: Vec<String>,
    set: Vec<(String, String)>,
    unset: Vec<String>,
}

#[derive(Debug, PartialEq)]
struct Plan {
    server: String,
    image: PathBuf,
    keep: PathBuf,
    record: PathBuf,
    old_pid: u32,
    target: Side,
    old: Side,
    exit: Duration,
    start: Duration,
    ready: Duration,
}

fn side(json: &Json) -> Option<Side> {
    let strings = |key: &str| -> Vec<String> {
        json[key].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
    };
    Some(Side {
        exe: PathBuf::from(json["exe"].as_str()?),
        flags: strings("flags"),
        set: json["set"].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect())
            .unwrap_or_default(),
        unset: strings("unset"),
    })
}

fn plan(json: &Json) -> Result<Plan, String> {
    let path = |key: &str| json[key].as_str().map(PathBuf::from).ok_or(format!("the plan has no {key}"));
    let seconds = |key: &str, default: u64| Duration::from_secs(json["deadlines"][key].as_u64().unwrap_or(default));
    Ok(Plan {
        server: json["server"].as_str().ok_or("the plan names no server")?.to_owned(),
        image: path("image")?,
        keep: path("keep")?,
        record: path("record")?,
        old_pid: json["old"]["pid"].as_u64().and_then(|p| u32::try_from(p).ok()).ok_or("the plan has no old pid")?,
        target: side(&json["target"]).ok_or("the plan has no target")?,
        old: side(&json["old"]).ok_or("the plan has no old build")?,
        exit: seconds("exit", 60),
        start: seconds("start", 180),
        ready: seconds("ready", 300),
    })
}

/// What a session's HEALTH answer says is missing for it to count as up,
/// as SIDE's: None when nothing is.
fn missing(health: &Json, side: &Side) -> Option<String> {
    let exe = health["exe"].as_str().map(PathBuf::from);
    let same = |a: &Path, b: &Path| a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(a, b)| a == b);
    if !exe.as_deref().is_some_and(|exe| same(exe, &side.exe)) {
        return Some(format!("identity: it runs {}, not {}", exe.map_or("?".into(), |e| e.display().to_string()), side.exe.display()));
    }
    if health["window"] != json!(true) { return Some("window: no window shown".into()); }
    if health["init"] != json!(true) { return Some("init: not done".into()); }
    match health["restore"].as_str() {
        Some("done") => None,
        Some("failed") => Some(format!("restore: failed: {}", health["restoreError"].as_str().unwrap_or("?"))),
        other => Some(format!("restore: {}", other.unwrap_or("?"))),
    }
}

/// The record: the plan's phases, checks and verdict, rewritten whole at
/// each change so a reader never sees half of it.
struct Record { path: PathBuf, json: Json, begun: Instant }

impl Record {
    fn note(&mut self, phase: &str, detail: Json) {
        let at = (self.begun.elapsed().as_secs_f64() * 100.0).round() / 100.0;
        if let Some(phases) = self.json["phases"].as_array_mut() { phases.push(json!({"phase": phase, "at": at, "detail": detail})); }
        self.json["state"] = json!(phase);
        let temporary = self.path.with_extension("json.tmp");
        if std::fs::write(&temporary, format!("{}\n", self.json)).is_ok() { let _ = std::fs::rename(&temporary, &self.path); }
    }
}

fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().next()) != Some("Z"))
}

fn wait_until(deadline: Instant, mut done: impl FnMut() -> bool) -> bool {
    while Instant::now() < deadline {
        if done() { return true; }
        std::thread::sleep(Duration::from_millis(200));
    }
    done()
}

fn unit(server: &str) -> String { format!("romacs-server-{server}") }

/// The server unit's state, or None without a user manager.
fn unit_state(server: &str) -> Option<String> {
    let out = std::process::Command::new("systemctl").args(["--user", "show", "-p", "ActiveState", "--value", &unit(server)])
        .stdin(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn signal(pid: u32, signal: rustix::process::Signal) {
    // A stale number is ESRCH: nothing to end.
    if let Some(pid) = i32::try_from(pid).ok().and_then(rustix::process::Pid::from_raw) {
        let _ = rustix::process::kill_process(pid, signal);
    }
}

/// SIDE's command for server NAME, with ARGS after its launch flags.
fn command(side: &Side, server: &str, args: &[&str]) -> std::process::Command {
    let mut command = std::process::Command::new(&side.exe);
    command.args(["--server", server]).args(args);
    for name in &side.unset { command.env_remove(name); }
    for (name, value) in &side.set { command.env(name, value); }
    command.stdin(std::process::Stdio::null());
    command
}

/// Run COMMAND for at most LIMIT: its status and its output, or why not.
fn run(mut command: std::process::Command, limit: Duration) -> Result<(i32, String), String> {
    let mut child = command.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
        .spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let out = child.wait_with_output().map_err(|e| e.to_string())?;
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            return Ok((status.code().unwrap_or(-1), text));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("no answer within {}s", limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A Lisp string as the client prints it, read back.
fn lisp_string(printed: &str) -> Option<String> {
    let inner = printed.trim().strip_prefix('"')?.strip_suffix('"')?;
    let mut text = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' { text.push(match chars.next()? { 'n' => '\n', 't' => '\t', other => other }); } else { text.push(c); }
    }
    Some(text)
}

/// Start SIDE as the server, restoring IMAGE, and judge it: None when it
/// is up, else what failed. The pid of the session it judged, when any.
fn attempt(plan: &Plan, side: &Side, image: &Path, label: &str, record: &mut Record) -> (Option<String>, Option<u32>) {
    let image = image.display().to_string();
    let mut args: Vec<&str> = vec!["-n"];
    args.extend(side.flags.iter().map(String::as_str));
    args.extend(["--restore", image.as_str()]);
    let mut start = command(side, &plan.server, &args);
    start.env("ROMACS_SERVER_START_TIMEOUT", plan.start.as_secs().to_string());
    record.note(&format!("{label}-starting"), json!({"exe": side.exe, "image": image}));
    match run(start, plan.start + Duration::from_secs(10)) {
        Ok((0, _)) => record.note(&format!("{label}-serving"), json!({})),
        Ok((code, out)) => return (Some(format!("start: the client ended with {code}: {}", tail(&out))), None),
        Err(why) => return (Some(format!("start: {why}")), None),
    }
    let deadline = Instant::now() + plan.ready;
    let mut last = String::from("no answer");
    let mut pid = None;
    while Instant::now() < deadline {
        match run(command(side, &plan.server, &["--no-start", "-e", HEALTH]), Duration::from_secs(30)) {
            Ok((0, out)) => match lisp_string(&out).and_then(|text| serde_json::from_str::<Json>(&text).ok()) {
                Some(health) => {
                    pid = health["pid"].as_u64().and_then(|p| u32::try_from(p).ok());
                    match missing(&health, side) {
                        None => { record.note(&format!("{label}-up"), health); return (None, pid); }
                        Some(why) => last = why,
                    }
                }
                None => last = format!("health: unreadable answer: {}", tail(&out)),
            },
            Ok((code, out)) => last = format!("health: the client ended with {code}: {}", tail(&out)),
            Err(why) => last = format!("health: {why}"),
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    (Some(last), pid)
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(5)..].join(" | ")
}

/// End the server a failed attempt left: its unit stopped (TERM: a stop,
/// not a crash), else its process.
fn stop(server: &str, pid: Option<u32>) {
    if unit_state(server).is_some_and(|state| state == "active" || state == "activating") {
        let _ = std::process::Command::new("systemctl").args(["--user", "stop", &unit(server)]).status();
    } else if let Some(pid) = pid {
        signal(pid, rustix::process::Signal::TERM);
        if !wait_until(Instant::now() + Duration::from_secs(15), || !alive(pid)) { signal(pid, rustix::process::Signal::KILL); }
    }
}

/// The unit of the server is gone or inactive: a new one can start.
fn settled(server: &str) -> bool {
    unit_state(server).is_none_or(|state| matches!(state.as_str(), "inactive" | "failed" | ""))
}

fn tell(side: &Side, server: &str, record: &Path) {
    let form = format!("(when (fboundp 'romacs-restart-verdict) (romacs-restart-verdict {}) t)",
                       romacs_vm_gui::startup::lisp_string(&record.display().to_string()));
    let _ = run(command(side, server, &["--no-start", "-e", &form]), Duration::from_secs(30));
}

fn notify(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send").env_remove("LD_LIBRARY_PATH")
        .args(["-u", "critical", "-a", "Romacs", title, body]).status();
}

pub fn supervise(argv: &[String]) -> i32 {
    let Some(path) = argv.get(1) else {
        eprintln!("usage: romacs --restart-supervise PLAN");
        return 2;
    };
    let json: Json = match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
        Ok(json) => json,
        Err(error) => { eprintln!("romacs: restart plan {path}: {error}"); return 2; }
    };
    let plan = match plan(&json) {
        Ok(plan) => plan,
        Err(error) => { eprintln!("romacs: restart plan {path}: {error}"); return 2; }
    };
    let mut record = Record { path: plan.record.clone(), json: json!({"plan": json, "phases": [], "state": "begun"}), begun: Instant::now() };
    // The old session ends: its hooks may take a while; past the deadline
    // its image is already written, and it is ended.
    if !wait_until(Instant::now() + plan.exit, || !alive(plan.old_pid)) {
        record.note("old-killed", json!({"after": plan.exit.as_secs()}));
        signal(plan.old_pid, rustix::process::Signal::TERM);
        if !wait_until(Instant::now() + Duration::from_secs(10), || !alive(plan.old_pid)) { signal(plan.old_pid, rustix::process::Signal::KILL); }
    }
    wait_until(Instant::now() + Duration::from_secs(30), || settled(&plan.server));
    record.note("old-gone", json!({}));
    let (failed, pid) = attempt(&plan, &plan.target, &plan.image, "target", &mut record);
    let Some(why) = failed else {
        record.json["verdict"] = json!("verified");
        record.note("verified", json!({"seconds": record.begun.elapsed().as_secs_f64()}));
        tell(&plan.target, &plan.server, &plan.record);
        return 0;
    };
    record.json["failed"] = json!(why);
    record.note("target-failed", json!({"why": why}));
    stop(&plan.server, pid);
    wait_until(Instant::now() + Duration::from_secs(30), || settled(&plan.server));
    let (failed, _) = attempt(&plan, &plan.old, &plan.keep, "rollback", &mut record);
    match failed {
        None => {
            record.json["verdict"] = json!("rolled-back");
            record.note("rolled-back", json!({"seconds": record.begun.elapsed().as_secs_f64()}));
            tell(&plan.old, &plan.server, &plan.record);
            notify("Romacs restart rolled back", &format!("{why}\nRecord: {}", plan.record.display()));
            0
        }
        Some(again) => {
            record.json["verdict"] = json!("rollback-failed");
            record.note("rollback-failed", json!({"why": again}));
            notify("Romacs restart failed", &format!("{why}; the rollback too: {again}\nImage: {}\nRecord: {}",
                                                     plan.keep.display(), plan.record.display()));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Json {
        json!({"server": "s", "image": "/i/a.image", "keep": "/r/a.image", "record": "/r/a.json",
               "old": {"pid": 7, "exe": "/old/romacs-vm-gui", "flags": ["--loadup"]},
               "target": {"exe": "/new/romacs-vm-gui", "flags": ["--loadup"], "set": {"ROMACS_RUNTIME_ROOT": "/new"},
                          "unset": ["ROMACS_LISP_DIR"]},
               "deadlines": {"ready": 5}})
    }

    #[test]
    fn a_plan_reads_its_sides_and_deadlines() {
        let plan = plan(&example()).unwrap();
        assert_eq!(plan.old_pid, 7);
        assert_eq!(plan.target.set, [("ROMACS_RUNTIME_ROOT".to_owned(), "/new".to_owned())]);
        assert_eq!(plan.target.unset, ["ROMACS_LISP_DIR"]);
        assert_eq!(plan.old.exe, Path::new("/old/romacs-vm-gui"));
        assert_eq!((plan.ready, plan.start), (Duration::from_secs(5), Duration::from_secs(180)));
        let mut bad = example();
        bad["old"]["pid"] = json!(null);
        assert!(super::plan(&bad).is_err());
    }

    #[test]
    fn a_session_is_up_when_it_is_the_build_with_a_window_its_init_and_its_restore() {
        let exe = std::env::current_exe().unwrap();
        let side = Side { exe: exe.clone(), flags: vec![], set: vec![], unset: vec![] };
        let up = json!({"exe": exe, "window": true, "init": true, "restore": "done"});
        assert_eq!(missing(&up, &side), None);
        let other = Side { exe: PathBuf::from("/nonexistent/romacs-vm-gui"), ..side.clone() };
        assert!(missing(&up, &other).unwrap().starts_with("identity"));
        let mut no_window = up.clone();
        no_window["window"] = json!(false);
        assert!(missing(&no_window, &side).unwrap().starts_with("window"));
        let mut pending = up.clone();
        pending["restore"] = json!("running");
        assert_eq!(missing(&pending, &side).unwrap(), "restore: running");
        let mut failed = up;
        failed["restore"] = json!("failed");
        failed["restoreError"] = json!("Not a usable session image");
        assert_eq!(missing(&failed, &side).unwrap(), "restore: failed: Not a usable session image");
    }

    #[test]
    fn a_printed_lisp_string_reads_back() {
        assert_eq!(lisp_string(r#""{\"pid\":1,\"a\\\\b\":true}""#).as_deref(), Some(r#"{"pid":1,"a\\b":true}"#));
        assert_eq!(lisp_string("nil"), None);
    }
}
