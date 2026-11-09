//! `romacs --server NAME [OPTION...] [[+LINE[:COLUMN]] FILE...]`: a client
//! of the named server session. It connects to the server's native socket
//! (`$XDG_RUNTIME_DIR/romacs/NAME/server.sock`, rlisp/romacs-server.el),
//! starting the server first when none listens: as a transient systemd user
//! unit, `romacs-server-NAME`, else detached with `setsid`. Then it asks
//! for a window showing FILES (`romacs/frame`), evaluates a form
//! (`romacs/eval`) or stops the server (`romacs/stop`), prints the reply
//! and exits. With files and without `--no-wait` it returns when every
//! file is done (`C-x #`), as emacsclient does. Options it does not know
//! are the server's launch options, used only when it starts one.
use serde_json::{Value as Json, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: romacs --server NAME [-n|--no-wait] [-e|--eval FORM] [--stop[=ask|all|none]] \
[--no-start] [LAUNCH-OPTION...] [[+LINE[:COLUMN]] FILE...]";

/// Launch options that take a value, forwarded with it to a server this
/// client starts.
const VALUED: [&str; 4] = ["--geometry", "--world-store", "--gc-floor", "--restore"];

/// Environment a started server does not inherit: the client's terminal
/// and its own unit's bookkeeping.
const LOCAL_ENV: [&str; 12] = ["PWD", "OLDPWD", "SHLVL", "_", "TERM", "COLUMNS", "LINES", "INVOCATION_ID",
                               "JOURNAL_STREAM", "SYSTEMD_EXEC_PID", "MANAGERPID", "NOTIFY_SOCKET"];

/// Whether ARGV makes this process a client rather than a session.
pub fn selected(argv: &[String]) -> bool {
    argv.iter().any(|arg| arg == "--server" || arg.starts_with("--server="))
}

/// NAME as a server's name: a socket directory's name, short enough for
/// the 108-byte socket path limit.
pub fn server_name(name: &str) -> Result<String, String> {
    let valid = (1..=32).contains(&name.len()) && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    if valid { Ok(name.to_owned()) } else { Err(format!("invalid server name {name:?}")) }
}

#[derive(Debug, Default, PartialEq)]
struct Request {
    name: String,
    /// Absolute paths, each with its line and column.
    files: Vec<(PathBuf, Option<(u64, u64)>)>,
    eval: Option<String>,
    wait: bool,
    stop: Option<String>,
    start: bool,
    /// The server's launch options, when this client starts it.
    launch: Vec<String>,
}

fn position(arg: &str) -> Option<(u64, u64)> {
    let spec = arg.strip_prefix('+')?;
    let (line, column) = spec.split_once(':').unwrap_or((spec, "0"));
    Some((line.parse().ok()?, column.parse().ok()?))
}

fn parse(argv: &[String], cwd: &Path) -> Result<Request, String> {
    let mut request = Request { name: "romacs".into(), wait: true, start: true, ..Default::default() };
    let mut args = argv.iter();
    let mut at = None;
    let mut files_only = false;
    while let Some(arg) = args.next() {
        let mut value = || args.next().cloned().ok_or_else(|| format!("{arg} needs a value\n{USAGE}"));
        match arg.as_str() {
            _ if files_only => request.files.push((cwd.join(arg), at.take())),
            "--" => files_only = true,
            "--server" => request.name = server_name(&value()?)?,
            "-n" | "--no-wait" => request.wait = false,
            "-e" | "--eval" => request.eval = Some(value()?),
            "--stop" => request.stop = Some("ask".into()),
            "--no-start" => request.start = false,
            "--help" => return Err(USAGE.into()),
            option if option.starts_with("--server=") => request.name = server_name(&option["--server=".len()..])?,
            option if option.starts_with("--stop=") => {
                let save = &option["--stop=".len()..];
                if !["ask", "all", "none"].contains(&save) { return Err(format!("--stop={save}: ask, all or none")); }
                request.stop = Some(save.into());
            }
            option if VALUED.contains(&option) => {
                let given = value()?;
                request.launch.extend([option.to_owned(), given]);
            }
            option if option.starts_with('+') => at = Some(position(option).ok_or_else(|| format!("bad position {option}"))?),
            option if option.starts_with('-') => request.launch.push(option.to_owned()),
            file => request.files.push((cwd.join(file), at.take())),
        }
    }
    Ok(request)
}

/// `$XDG_RUNTIME_DIR', else a private directory GNU's server.el would use.
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").expect("/proc/self"));
        PathBuf::from(format!("/tmp/romacs{uid}"))
    })
}

pub fn socket_path(name: &str) -> PathBuf {
    runtime_dir().join("romacs").join(name).join("server.sock")
}

fn unit(name: &str) -> String { format!("romacs-server-{name}") }

/// Run the server for REQUEST as a transient user unit, or detached when
/// no user manager answers. A unit that already exists is a server some
/// other client is starting.
fn start(request: &Request) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("own executable: {e}"))?;
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/".into());
    let mut command = std::process::Command::new("systemd-run");
    command.args(["--user", "--quiet", "--collect", "-p", "OOMPolicy=continue"])
        // An abnormal end shows a desktop notification (`--server-exited').
        // The editor ends a SIGTERM (a stop, a logout) with status 15, as
        // GNU's does: a success here.
        .args(["-p", "SuccessExitStatus=15"])
        .arg("-p").arg(format!("ExecStopPost=\"{}\" --server-exited {}", exe.display(), request.name))
        .arg(format!("--unit={}", unit(&request.name)))
        .arg(format!("--working-directory={}", Path::new(&home).display()));
    // A slice for the server's unit, as a test or a desktop's policy sets.
    if let Some(slice) = std::env::var_os("ROMACS_SERVER_SLICE") {
        let mut option = std::ffi::OsString::from("--slice=");
        option.push(slice);
        command.arg(option);
    }
    // By name: systemd-run takes each value from its own environment, so
    // none (an API key, say) shows on a command line other users can read.
    for (key, _) in std::env::vars_os() {
        let key_text = key.to_string_lossy();
        if LOCAL_ENV.contains(&key_text.as_ref()) || key_text.starts_with("LISTEN_") || key_text.contains('=') { continue; }
        let mut setting = std::ffi::OsString::from("--setenv=");
        setting.push(&key);
        command.arg(setting);
    }
    let mut launch = server_launch(&request.launch);
    launch.extend(window_options(request).iter().map(|option| option.to_string()));
    command.arg("--").arg(&exe).args(["--serve", &request.name]).args(&launch);
    let output = command.stdin(std::process::Stdio::null()).output();
    match output {
        Ok(output) if output.status.success() => return Ok(()),
        Ok(output) if String::from_utf8_lossy(&output.stderr).contains("already") => return Ok(()),
        Ok(output) => eprintln!("romacs: systemd-run: {}", String::from_utf8_lossy(&output.stderr).trim()),
        Err(error) => eprintln!("romacs: systemd-run: {error}"),
    }
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&home).join(".local/state")).join("romacs");
    std::fs::create_dir_all(&state).map_err(|e| format!("{}: {e}", state.display()))?;
    let log = std::fs::File::create(state.join(format!("server-{}.log", request.name))).map_err(|e| e.to_string())?;
    std::process::Command::new("setsid").arg("--fork").arg(&exe).args(["--serve", &request.name]).args(&launch)
        .current_dir(&home).stdin(std::process::Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(log)
        .status().map_err(|e| format!("setsid: {e}"))
        .and_then(|status| if status.success() { Ok(()) } else { Err(format!("setsid: {status}")) })
}

/// `--server-exited NAME': the server unit's ExecStopPost. systemd says how
/// the server ended ($SERVICE_RESULT, $EXIT_CODE, $EXIT_STATUS); an end
/// other than success (a crash, a kill, a failing status) shows the desktop
/// notification a launcher shows for a standalone session, naming the
/// journal that holds the server's output. A stop or a logout's TERM is a
/// success to systemd and shows none.
pub fn exited(argv: &[String]) -> i32 {
    let Some(name) = argv.get(1).filter(|name| server_name(name).is_ok()) else {
        eprintln!("usage: romacs --server-exited NAME");
        return 2;
    };
    let var = |key: &str| std::env::var(key).unwrap_or_default();
    let (result, code, status) = (var("SERVICE_RESULT"), var("EXIT_CODE"), var("EXIT_STATUS"));
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&std::env::var_os("HOME").unwrap_or_else(|| "/".into())).join(".local/state"));
    let session = record_end(&state, name, &var("INVOCATION_ID"), &result, &code, &status, now());
    if let Some((title, mut body)) = crash_notice(name, &result, &code, &status) {
        if let Some(directory) = &session { body.push_str(&format!("\nSession: {}", directory.display())); }
        eprintln!("romacs: {title}; {body}");
        let _ = std::process::Command::new("notify-send").env_remove("LD_LIBRARY_PATH")
            .args(["-u", "critical", "-a", "Romacs", &title, &body]).status();
    }
    0
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Server NAME's session record (rlisp/romacs-usage-log.el) at its unit's
/// end, named by STATE/romacs/server-NAME.session for the systemd
/// INVOCATION that ran it: how systemd says it ended (its status, signal
/// and core dump: the server's own end cannot know its exit code) goes in
/// its session.json, and an end the server could not write itself (a
/// crash, a kill) gets its time.  A pointer another invocation
/// wrote (an earlier server's, this one failing before its own) is left
/// alone.  Return the directory, when this run has a record.
fn record_end(state: &Path, name: &str, invocation: &str, result: &str, code: &str, status: &str, at: f64) -> Option<PathBuf> {
    let pointer: Json = serde_json::from_str(&std::fs::read_to_string(state.join(format!("romacs/server-{name}.session"))).ok()?).ok()?;
    if pointer.get("invocation").and_then(Json::as_str) != Some(invocation) || invocation.is_empty() { return None; }
    let directory = PathBuf::from(pointer.get("directory")?.as_str()?);
    let file = directory.join("session.json");
    let mut record: Json = serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok()?;
    let object = record.as_object_mut()?;
    object.insert("service".into(), json!({"result": result, "code": code, "status": status}));
    if !object.contains_key("end") {
        let start = object.get("start").and_then(Json::as_f64).unwrap_or(at);
        object.insert("end".into(), json!(at));
        object.insert("seconds".into(), json!(((at - start) * 1000.0).round() / 1000.0));
    }
    let signal = matches!(code, "killed" | "dumped").then(|| format!("SIG{status}"));
    object.insert("status".into(), if code == "exited" { json!(status.parse::<i64>().ok()) } else { Json::Null });
    object.insert("signal".into(), json!(signal));
    object.insert("coredump".into(), json!(code == "dumped"));
    std::fs::write(&file, format!("{}\n", serde_json::to_string_pretty(&record).ok()?)).ok()?;
    Some(directory)
}

/// The notification for server NAME's end, or None for a clean one.
fn crash_notice(name: &str, result: &str, code: &str, status: &str) -> Option<(String, String)> {
    if result.is_empty() || result == "success" { return None; }
    let how = match code {
        "exited" => format!("status {status}"),
        "killed" => format!("signal {status}"),
        "dumped" => format!("signal {status}, core dumped"),
        _ => result.to_owned(),
    };
    Some((format!("Romacs exited ({how})"), format!("Log: journalctl --user -u {}", unit(name))))
}

/// LAUNCH as a server's: GNU's Lisp (`--loadup') unless it names a mode,
/// since a server runs server.el.
fn server_launch(launch: &[String]) -> Vec<String> {
    let mode = launch.iter().any(|a| ["--loadup", "--gnu-lisp", "--no-gnu-lisp"].contains(&a.as_str()));
    (!mode).then(|| "--loadup".to_owned()).into_iter().chain(launch.iter().cloned()).collect()
}

/// The unit's last journal lines, for a server that did not start.
fn unit_log(name: &str) -> String {
    std::process::Command::new("journalctl")
        .args(["--user", "--no-pager", "-n", "20", "-o", "cat", "-u", &unit(name)])
        .output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn start_timeout() -> Duration {
    Duration::from_secs(std::env::var("ROMACS_SERVER_START_TIMEOUT").ok().and_then(|s| s.parse().ok()).unwrap_or(90))
}

/// A connection to REQUEST's server, started first when none listens, and
/// whether this client started it.
fn connect(request: &Request) -> Result<(UnixStream, bool), String> {
    let socket = socket_path(&request.name);
    match UnixStream::connect(&socket) {
        Ok(stream) => return Ok((stream, false)),
        Err(error) if !request.start || !matches!(error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) =>
            return Err(format!("{}: {error}", socket.display())),
        Err(_) => {}
    }
    start(request)?;
    let deadline = Instant::now() + start_timeout();
    while Instant::now() < deadline {
        if let Ok(stream) = UnixStream::connect(&socket) { return Ok((stream, true)); }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(format!("server {} did not start within {}s\n{}", request.name, start_timeout().as_secs(), unit_log(&request.name)))
}

/// Send one JSON-RPC request and read its reply's result or error.
fn call(stream: &mut UnixStream, reader: &mut impl BufRead, id: u64, method: &str, params: Json) -> Result<Json, String> {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    stream.write_all(format!("{line}\n").as_bytes()).map_err(|e| format!("send: {e}"))?;
    let mut reply = String::new();
    loop {
        reply.clear();
        if reader.read_line(&mut reply).map_err(|e| format!("read: {e}"))? == 0 {
            return Err("the server closed the connection".into());
        }
        let message: Json = serde_json::from_str(&reply).map_err(|e| format!("reply: {e}"))?;
        if message.get("id") != Some(&json!(id)) { continue; }
        if let Some(error) = message.get("error") {
            return Err(error.get("message").and_then(Json::as_str).unwrap_or("error").to_owned());
        }
        return Ok(message.get("result").cloned().unwrap_or(Json::Null));
    }
}

/// Whether REQUEST asks for the window: files, or nothing else asked.
fn wants_frame(request: &Request) -> bool {
    request.stop.is_none() && (request.eval.is_none() || !request.files.is_empty())
}

/// What a server this client starts is told of its window.  It will ask for
/// one: the server makes it while it starts.  With nothing to visit it will
/// show the landing, which the server must choose before its first paint.
fn window_options(request: &Request) -> &'static [&'static str] {
    match (wants_frame(request), request.files.is_empty()) {
        (false, _) => &[],
        (true, true) => &["--serve-frame", "--serve-landing"],
        (true, false) => &["--serve-frame"],
    }
}

fn frame_params(request: &Request, cwd: &Path) -> Json {
    let files: Vec<Json> = request.files.iter().map(|(path, at)| {
        let mut file = json!({"path": path.to_string_lossy()});
        if let Some((line, column)) = at { file["line"] = json!(line); file["column"] = json!(column); }
        file
    }).collect();
    json!({"files": files, "cwd": cwd.to_string_lossy(), "wait": request.wait && !request.files.is_empty()})
}

fn run(argv: &[String]) -> Result<i32, String> {
    let begun = Instant::now();
    let cwd = std::env::current_dir().map_err(|e| format!("current directory: {e}"))?;
    let request = parse(argv, &cwd)?;
    let (mut stream, started) = connect(&request)?;
    let connected = begun.elapsed();
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut id = 0;
    let mut next = || { id += 1; id };
    if wants_frame(&request) {
        call(&mut stream, &mut reader, next(), "romacs/frame", frame_params(&request, &cwd))?;
    }
    if let Some(code) = &request.eval {
        let result = call(&mut stream, &mut reader, next(), "romacs/eval", json!({"code": code}))?;
        println!("{}", result.get("value").and_then(Json::as_str).unwrap_or(""));
    }
    if let Some(save) = &request.stop {
        call(&mut stream, &mut reader, next(), "romacs/stop", json!({"save": save}))?;
    }
    if std::env::var_os("ROMACS_CLIENT_TIMING").is_some() {
        eprintln!("romacs-client: {}", json!({"started": started, "connect_ms": connected.as_secs_f64() * 1e3,
                                              "reply_ms": begun.elapsed().as_secs_f64() * 1e3}));
    }
    Ok(0)
}

pub fn main(argv: &[String]) -> i32 {
    match run(argv) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("romacs: {error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_servers_end_fills_its_session_record() {
        let root = std::env::temp_dir().join(format!("romacs-record-end-{}", std::process::id()));
        let directory = root.join("romacs/sessions/20261104T100000-42");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(root.join("romacs/server-work.session"),
                       json!({"directory": directory, "pid": 42, "invocation": "run-1"}).to_string()).unwrap();
        // A crash: the server wrote only its start.
        std::fs::write(directory.join("session.json"), r#"{"pid":42,"start":100.0}"#).unwrap();
        // Another run's end (a later server failing before its own record)
        // touches nothing.
        assert_eq!(record_end(&root, "work", "run-2", "exit-code", "exited", "1", 111.0), None);
        assert_eq!(std::fs::read_to_string(directory.join("session.json")).unwrap(), r#"{"pid":42,"start":100.0}"#);
        assert_eq!(record_end(&root, "work", "run-1", "core-dump", "dumped", "SEGV", 112.5), Some(directory.clone()));
        let record: Json = serde_json::from_str(&std::fs::read_to_string(directory.join("session.json")).unwrap()).unwrap();
        assert_eq!((record["end"].as_f64(), record["seconds"].as_f64()), (Some(112.5), Some(12.5)));
        assert_eq!((record["signal"].as_str(), record["coredump"].as_bool()), (Some("SIGSEGV"), Some(true)));
        assert_eq!(record["service"]["result"], "core-dump");
        // An end the server wrote itself keeps its time; its status is
        // systemd's, (kill-emacs 3) exiting 3.
        std::fs::write(directory.join("session.json"), r#"{"pid":42,"start":100.0,"end":105.0}"#).unwrap();
        record_end(&root, "work", "run-1", "exit-code", "exited", "3", 106.0);
        let record: Json = serde_json::from_str(&std::fs::read_to_string(directory.join("session.json")).unwrap()).unwrap();
        assert_eq!((record["end"].as_f64(), record["status"].as_i64()), (Some(105.0), Some(3)));
        assert_eq!((record["signal"].as_str(), record["coredump"].as_bool()), (None, Some(false)));
        assert!(record_end(&root, "other", "run-1", "success", "exited", "0", 1.0).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_an_abnormal_end_is_noticed() {
        assert_eq!(crash_notice("romacs", "success", "exited", "0"), None);
        assert_eq!(crash_notice("romacs", "", "", ""), None);
        assert_eq!(crash_notice("romacs", "signal", "killed", "KILL"),
                   Some(("Romacs exited (signal KILL)".into(), "Log: journalctl --user -u romacs-server-romacs".into())));
        assert_eq!(crash_notice("work", "core-dump", "dumped", "SEGV").unwrap().0, "Romacs exited (signal SEGV, core dumped)");
        assert_eq!(crash_notice("work", "exit-code", "exited", "1").unwrap().0, "Romacs exited (status 1)");
        assert_eq!(crash_notice("work", "oom-kill", "killed", "KILL").unwrap().0, "Romacs exited (signal KILL)");
    }

    fn args(list: &[&str]) -> Vec<String> { list.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn files_take_the_position_before_them_and_unknown_options_launch_the_server() {
        let request = parse(&args(&["--server", "work", "--loadup", "--geometry", "800x600", "+3:4", "a.txt", "b", "-n"]),
                            Path::new("/home/u")).unwrap();
        assert_eq!(request.name, "work");
        assert_eq!(request.files, vec![(PathBuf::from("/home/u/a.txt"), Some((3, 4))), (PathBuf::from("/home/u/b"), None)]);
        assert_eq!(request.launch, args(&["--loadup", "--geometry", "800x600"]));
        assert!(!request.wait);
    }

    /// Only a client with nothing to visit has the server land before its
    /// first paint: the landing must not flash before a file.
    #[test]
    fn a_server_lands_for_a_window_with_nothing_to_visit() {
        let options = |argv: &[&str]| window_options(&parse(&args(argv), Path::new("/")).unwrap()).to_vec();
        assert_eq!(options(&["-n"]), ["--serve-frame", "--serve-landing"]);
        assert_eq!(options(&["a.txt"]), ["--serve-frame"]);
        assert!(options(&["-e", "(+ 1 2)"]).is_empty());
        assert!(options(&["--stop=none"]).is_empty());
    }

    #[test]
    fn a_name_is_one_short_path_component() {
        assert!(server_name("romacs").is_ok());
        for bad in ["", "../x", ".hidden", "a/b", &"x".repeat(33)] { assert!(server_name(bad).is_err(), "{bad}"); }
    }

    #[test]
    fn stop_and_eval_are_parsed() {
        let request = parse(&args(&["--server=s", "--stop=all", "-e", "(+ 1 2)"]), Path::new("/")).unwrap();
        assert_eq!((request.stop.as_deref(), request.eval.as_deref()), (Some("all"), Some("(+ 1 2)")));
        assert!(parse(&args(&["--server=s", "--stop=maybe"]), Path::new("/")).is_err());
    }

    #[test]
    fn a_server_runs_gnu_lisp_unless_told_a_mode() {
        assert_eq!(server_launch(&args(&["-Q"])), args(&["--loadup", "-Q"]));
        assert_eq!(server_launch(&args(&["--gnu-lisp"])), args(&["--gnu-lisp"]));
    }

    #[test]
    fn a_line_alone_is_column_zero() {
        assert_eq!(position("+12"), Some((12, 0)));
        assert_eq!(position("+x"), None);
    }
}
