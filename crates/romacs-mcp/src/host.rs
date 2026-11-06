//! `romacs-agent-host --dir DIR --socket SOCKET -- PROGRAM ARG...`: the
//! supervisor of one agent session (rlisp/romacs-agent.el). It runs the
//! harness (Claude Code's stream-json CLI, or any program speaking lines),
//! owns its stdin and stdout, and outlives every Romacs server: a server
//! attaches over SOCKET, is sent the journal from a sequence number on and
//! then each new line, and writes lines to the harness. When the server
//! exits, crashes or is replaced, the harness and its turn go on; the next
//! server attaches and replays.
//!
//! DIR/journal.jsonl holds every line, one record each: `{"seq":N,"t":SECS,
//! "out":LINE}` for the harness's output, `"in"` for what was written to it,
//! and a last `{"seq":N,"t":SECS,"exit":STATUS}`. A line that is JSON is
//! kept as JSON, any other as a string. A record that ends a turn (a
//! `result`) is synced to disk. DIR/host.json names the host's and the
//! harness's pids. One server is attached at a time: another is refused
//! unless it steals the attachment.
//!
//! The socket speaks lines: the server's first is `{"attach":{"from":N,
//! "steal":BOOL}}`, then `{"write":LINE}` for each line to the harness. With
//! `"journal":LINE` beside it, that line is journaled in its place: a message
//! carrying image bytes is kept as a reference, as a replay reads every record.
//!
//! `--note JSON` journals `{"seq":N,"t":SECS,"note":JSON}` as the run's
//! first record, before anything the harness writes: never written to
//! the harness, it marks where this run starts (a server folding a
//! harness's startup state from the journal starts over there).
//!
//! `--input FILE` gives a harness that reads its whole stdin (one run per
//! turn, its prompt on stdin) FILE as that input: journaled as an `"in"`
//! record, written, and the stdin closed, with no server needed; FILE is
//! then removed. `--input-images JSON` journals that record as `{"text":
//! TEXT,"images":JSON}`: what the user attached, which its text never says.
#![forbid(unsafe_code)]

use serde_json::{Value as Json, json};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

const USAGE: &str = "usage: romacs-agent-host --dir DIR --socket SOCKET [--input FILE] [--input-images JSON] [--note JSON] -- PROGRAM [ARG...]";

struct Config {
    dir: PathBuf,
    socket: PathBuf,
    input: Option<PathBuf>,
    note: Option<Json>,
    input_images: Option<Json>,
    argv: Vec<String>,
}

fn parse(args: Vec<String>) -> Result<Config, String> {
    let mut args = args.into_iter();
    let (mut dir, mut socket, mut input, mut note, mut input_images) = (None, None, None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => dir = args.next().map(PathBuf::from),
            "--socket" => socket = args.next().map(PathBuf::from),
            "--input" => input = Some(args.next().map(PathBuf::from).ok_or(USAGE)?),
            "--input-images" => input_images = Some(args.next().and_then(|text| serde_json::from_str(&text).ok()).ok_or(USAGE)?),
            "--note" => note = Some(args.next().and_then(|text| serde_json::from_str::<Json>(&text).ok()).ok_or(USAGE)?),
            "--" => break,
            _ => return Err(USAGE.into()),
        }
    }
    let argv: Vec<String> = args.collect();
    match (dir, socket) {
        (Some(dir), Some(socket)) if !argv.is_empty() && socket.is_absolute() => Ok(Config { dir, socket, input, note, input_images, argv }),
        _ => Err(USAGE.into()),
    }
}

/// The journal and the attached server, under one lock: a record is
/// journaled and sent in the same order for every reader.
struct Shared {
    seq: u64,
    journal: File,
    path: PathBuf,
    attached: Option<(u64, UnixStream)>,
    next_attachment: u64,
    /// The harness's stdin, its own lock: a slow harness never holds up
    /// journaling or the exit.
    stdin: Arc<Mutex<Option<ChildStdin>>>,
}

/// How long a write to the attached server may block before it is
/// detached: a stalled reader never holds up the journal or the exit.
const ATTACHED_WRITE: std::time::Duration = std::time::Duration::from_secs(2);

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// One journal record of LINE under KEY, kept as JSON when it is JSON.
fn record(seq: u64, key: &str, line: &str) -> String {
    let value = serde_json::from_str::<Json>(line).unwrap_or_else(|_| Json::String(line.to_owned()));
    json!({"seq": seq, "t": now(), key: value}).to_string()
}

impl Shared {
    /// Journal TEXT (a whole record) and send it to the attached server.
    fn append(&mut self, text: &str, sync: bool) {
        let line = format!("{text}\n");
        if let Err(error) = self.journal.write_all(line.as_bytes()) {
            eprintln!("romacs-agent-host: journal: {error}");
        }
        if sync { let _ = self.journal.sync_data(); }
        if let Some((_, stream)) = &mut self.attached
            && stream.write_all(line.as_bytes()).is_err() {
            // Closed, so the server sees its attachment end and attaches again.
            let _ = stream.shutdown(std::net::Shutdown::Both);
            self.attached = None;
        }
    }

    fn output(&mut self, line: &str) {
        self.seq += 1;
        let sync = line.contains("\"type\":\"result\"");
        self.append(&record(self.seq, "out", line), sync);
    }
}

/// The sequence number of the journal's last record, 0 for none: a host
/// resuming a session goes on numbering where the last one stopped.
fn last_seq(path: &Path) -> u64 {
    let Ok(file) = File::open(path) else { return 0 };
    BufReader::new(file).lines().map_while(Result::ok).filter_map(|line| {
        serde_json::from_str::<Json>(&line).ok()?.get("seq")?.as_u64()
    }).last().unwrap_or(0)
}

/// Attach STREAM: replay the journal from FROM, then make it the attached
/// server. Done under the lock, so no record is lost or sent twice.
fn attach(shared: &Arc<Mutex<Shared>>, mut stream: UnixStream, request: &Json) -> Option<u64> {
    let from = request.pointer("/attach/from").and_then(Json::as_u64).unwrap_or(0);
    let steal = request.pointer("/attach/steal").and_then(Json::as_bool).unwrap_or(false);
    let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
    if shared.attached.is_some() && !steal {
        let _ = writeln!(stream, "{}", json!({"refused": "attached", "pid": std::process::id()}));
        return None;
    }
    let _ = shared.journal.flush();
    if stream.set_write_timeout(Some(ATTACHED_WRITE)).is_err() { return None; }
    if let Ok(file) = File::open(&shared.path) {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let seq = serde_json::from_str::<Json>(&line).ok().and_then(|r| r.get("seq")?.as_u64()).unwrap_or(0);
            if seq >= from && writeln!(stream, "{line}").is_err() { return None; }
        }
    }
    let id = shared.next_attachment + 1;
    // Kept: the server may now act as this session's (its relays' route).
    // Told first, then published: a server gone before it hears this is
    // never the attached one.
    if writeln!(stream, "{}", json!({"attached": id})).is_err() { return None; }
    let clone = stream.try_clone().ok()?;
    shared.next_attachment = id;
    shared.attached = Some((id, clone));
    Some(id)
}

/// A server's request: the line for the harness and the line for the journal.
fn lines_of(request: &Json) -> Option<(String, String)> {
    let write = request.get("write")?.as_str()?.to_owned();
    let journal = request.get("journal").and_then(Json::as_str).map_or_else(|| write.clone(), str::to_owned);
    Some((write, journal))
}

/// The record of a per-turn prompt TEXT, with the IMAGES attached to it.
fn input_record(seq: u64, text: &str, images: Option<&Json>) -> String {
    match images {
        Some(images) => json!({"seq": seq, "t": now(), "in": {"text": text, "images": images}}).to_string(),
        None => record(seq, "in", text),
    }
}

/// Serve one connection: its attach line, then the lines it writes.
fn serve(shared: Arc<Mutex<Shared>>, stream: UnixStream) {
    let Ok(reading) = stream.try_clone() else { return };
    let mut lines = BufReader::new(reading).lines();
    let Some(Ok(first)) = lines.next() else { return };
    let Some(id) = serde_json::from_str::<Json>(&first).ok().and_then(|r| attach(&shared, stream, &r)) else { return };
    for line in lines.map_while(Result::ok) {
        let Some((text, journaled)) = serde_json::from_str::<Json>(&line).ok().as_ref().and_then(lines_of) else { continue };
        // The harness's stdin first, then the journal briefly: lines are
        // journaled in the order they are written, and a writer blocked on
        // a full stdin never holds the journal (the output, the exit).
        let stdin = shared.lock().unwrap_or_else(|e| e.into_inner()).stdin.clone();
        let mut stdin = stdin.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
            // A server whose attachment was stolen writes nothing more.
            if shared.attached.as_ref().map(|(attached, _)| *attached) != Some(id) { return; }
            shared.seq += 1;
            let entry = record(shared.seq, "in", &journaled);
            shared.append(&entry, false);
        }
        let written = stdin.as_mut().map(|stdin| writeln!(stdin, "{text}").and_then(|()| stdin.flush()));
        if !matches!(written, Some(Ok(()))) {
            eprintln!("romacs-agent-host: harness stdin closed");
        }
    }
    let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
    if shared.attached.as_ref().map(|(attached, _)| *attached) == Some(id) { shared.attached = None; }
}

/// Bind SOCKET, replacing only a socket no one listens on.
fn listen(socket: &Path) -> Result<UnixListener, String> {
    if socket.exists() {
        if UnixStream::connect(socket).is_ok() { return Err(format!("{} is live", socket.display())); }
        std::fs::remove_file(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    }
    if let Some(parent) = socket.parent() { create_private(parent)?; }
    UnixListener::bind(socket).map_err(|e| format!("{}: {e}", socket.display()))
}

fn create_private(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", dir.display()))
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, path)).map_err(|e| format!("{}: {e}", path.display()))
}

fn run(config: Config) -> Result<i32, String> {
    create_private(&config.dir)?;
    let listener = listen(&config.socket)?;
    let path = config.dir.join("journal.jsonl");
    let seq = last_seq(&path);
    let journal = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let stderr = OpenOptions::new().create(true).append(true).open(config.dir.join("harness.stderr")).map_err(|e| e.to_string())?;
    let mut child = Command::new(&config.argv[0]).args(&config.argv[1..])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(stderr)
        .spawn().map_err(|e| format!("{}: {e}", config.argv[0]))?;
    write_atomic(&config.dir.join("host.json"), &json!({"pid": std::process::id(), "harness": child.id(),
        "socket": config.socket, "started": now(), "argv": config.argv}).to_string())?;
    let stdout = child.stdout.take().ok_or("harness stdout")?;
    let mut stdin = child.stdin.take();
    let mut shared = Shared { seq, journal, path, attached: None, next_attachment: 0, stdin: Arc::new(Mutex::new(None)) };
    // The run's first record: its output is read only from here on.
    if let Some(note) = &config.note {
        shared.seq += 1;
        let entry = json!({"seq": shared.seq, "t": now(), "note": note}).to_string();
        shared.append(&entry, true);
    }
    if let Some(file) = &config.input {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        shared.seq += 1;
        let entry = input_record(shared.seq, &text, config.input_images.as_ref());
        shared.append(&entry, true);
        // Its own thread: a harness slow to read never holds up the host.
        let mut writer = stdin.take();
        std::thread::spawn(move || { if let Some(w) = writer.as_mut() { let _ = w.write_all(text.as_bytes()); } });
        let _ = std::fs::remove_file(file);
    }
    shared.stdin = Arc::new(Mutex::new(stdin));
    let shared = Arc::new(Mutex::new(shared));
    let accepting = shared.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let shared = accepting.clone();
            std::thread::spawn(move || serve(shared, stream));
        }
    });
    // Output is read on its own thread; the harness's exit is watched
    // here, since a descendant that inherited its stdout may keep that
    // open long after the harness itself ended.
    let (drained_tx, drained) = std::sync::mpsc::channel();
    let reading = shared.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&line);
                    reading.lock().unwrap_or_else(|e| e.into_inner()).output(text.trim_end_matches(['\n', '\r']));
                }
            }
        }
        let _ = drained_tx.send(());
    });
    let status = child.wait().map_err(|e| e.to_string())?;
    // What the harness wrote before it ended, unless a descendant holds
    // its stdout open.
    let _ = drained.recv_timeout(std::time::Duration::from_secs(1));
    let code = status.code().unwrap_or(128 + std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0));
    let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
    shared.seq += 1;
    let entry = json!({"seq": shared.seq, "t": now(), "exit": code}).to_string();
    shared.append(&entry, true);
    let _ = std::fs::remove_file(&config.socket);
    Ok(0)
}

fn main() {
    let code = parse(std::env::args().skip(1).collect()).and_then(run).unwrap_or_else(|error| {
        eprintln!("romacs-agent-host: {error}");
        1
    });
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_lines_are_kept_as_json_and_others_as_strings() {
        let kept: Json = serde_json::from_str(&record(3, "out", r#"{"type":"result"}"#)).unwrap();
        assert_eq!(kept["out"]["type"], "result");
        let text: Json = serde_json::from_str(&record(4, "out", "plain text")).unwrap();
        assert_eq!((text["seq"].as_u64(), text["out"].as_str()), (Some(4), Some("plain text")));
    }

    #[test]
    fn a_resumed_journal_numbers_on() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-test-{}", std::process::id()));
        create_private(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, format!("{}\n{}\n", record(1, "out", "a"), record(2, "in", "b"))).unwrap();
        assert_eq!(last_seq(&path), 2);
        assert_eq!(last_seq(&dir.join("none")), 0);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_kept_attachment_is_acknowledged_after_the_replay_and_a_second_is_refused() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-attach-{}", std::process::id()));
        create_private(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, format!("{}\n", record(1, "out", "a"))).unwrap();
        let journal = OpenOptions::new().append(true).open(&path).unwrap();
        let shared = Arc::new(Mutex::new(Shared { seq: 1, journal, path: path.clone(), attached: None, next_attachment: 0,
                                                  stdin: Arc::new(Mutex::new(None)) }));
        let request: Json = serde_json::from_str(r#"{"attach":{"from":1,"steal":false}}"#).unwrap();
        let (server, ours) = UnixStream::pair().unwrap();
        assert_eq!(attach(&shared, server, &request), Some(1));
        let mut lines = BufReader::new(ours.try_clone().unwrap()).lines();
        let replayed: Json = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(replayed["seq"], 1);
        let kept: Json = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(kept["attached"], 1);
        let (other, theirs) = UnixStream::pair().unwrap();
        assert_eq!(attach(&shared, other, &request), None);
        let refused: Json = serde_json::from_str(&BufReader::new(theirs).lines().next().unwrap().unwrap()).unwrap();
        assert_eq!(refused["refused"], "attached");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_server_gone_before_its_acknowledgement_is_never_attached() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-gone-{}", std::process::id()));
        create_private(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, "").unwrap();
        let journal = OpenOptions::new().append(true).open(&path).unwrap();
        let shared = Arc::new(Mutex::new(Shared { seq: 0, journal, path: path.clone(), attached: None, next_attachment: 0,
                                                  stdin: Arc::new(Mutex::new(None)) }));
        let request: Json = serde_json::from_str(r#"{"attach":{"from":1,"steal":false}}"#).unwrap();
        let (server, ours) = UnixStream::pair().unwrap();
        drop(ours);
        assert_eq!(attach(&shared, server, &request), None);
        assert!(shared.lock().unwrap().attached.is_none());
        let (next, theirs) = UnixStream::pair().unwrap();
        assert!(attach(&shared, next, &request).is_some());
        let kept: Json = serde_json::from_str(&BufReader::new(theirs).lines().next().unwrap().unwrap()).unwrap();
        assert!(kept["attached"].is_u64());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_request_may_journal_another_line_than_it_writes() {
        let both = json!({"write": "bytes", "journal": "reference"});
        assert_eq!(lines_of(&both), Some(("bytes".into(), "reference".into())));
        assert_eq!(lines_of(&json!({"write": "line"})), Some(("line".into(), "line".into())));
        assert_eq!(lines_of(&json!({"journal": "no write"})), None);
    }

    #[test]
    fn a_turn_s_images_are_journaled_beside_its_text() {
        let with: Json = serde_json::from_str(&input_record(3, "see", Some(&json!(["/a.png"])))).unwrap();
        assert_eq!(with["in"], json!({"text": "see", "images": ["/a.png"]}));
        let without: Json = serde_json::from_str(&input_record(3, "see", None)).unwrap();
        assert_eq!(without["in"], json!("see"));
    }

    #[test]
    fn the_argv_follows_the_separator() {
        let config = parse(["--dir", "d", "--socket", "/s", "--", "claude", "-p"].map(String::from).to_vec()).unwrap();
        assert_eq!(config.argv, ["claude", "-p"]);
        assert!(parse(["--dir", "d", "--socket", "relative", "--", "x"].map(String::from).to_vec()).is_err());
        let noted = parse(["--dir", "d", "--socket", "/s", "--note", r#"{"type":"run"}"#, "--", "x"].map(String::from).to_vec()).unwrap();
        assert_eq!(noted.note, Some(json!({"type": "run"})));
        assert!(parse(["--dir", "d", "--socket", "/s", "--note", "not json", "--", "x"].map(String::from).to_vec()).is_err());
    }
}
