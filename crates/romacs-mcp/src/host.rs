//! `romacs-agent-host --dir DIR --socket SOCKET -- PROGRAM ARG...`: the
//! supervisor of one agent session (lisp/agents/romacs-agent.el). It runs the
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
//! unless it steals the attachment, which closes the first's connection.
//!
//! The socket speaks lines: the server's first is `{"attach":{"from":N,
//! "steal":BOOL}}`, then `{"write":LINE}` for each line to the harness and
//! `{"note":OBJECT}` for a record of the server's own: journaled in its
//! place among the harness's lines, never written to it. With `"journal":LINE`
//! beside a write, that line is journaled in its place: a message carrying
//! image bytes is kept as a reference, as a replay reads every record.
//!
//! `--note JSON` journals `{"seq":N,"t":SECS,"note":JSON}` as the run's
//! first record, before anything the harness writes: never written to
//! the harness, it marks where this run starts (a server folding a
//! harness's startup state from the journal starts over there).
//!
//! `--input FILE` gives a harness that reads its whole stdin (one run per
//! turn, its prompt on stdin) FILE as that input: journaled as an `"in"`
//! record, written, and the stdin closed, with no server needed; FILE is
//! then removed. That record is always `{"prompt":TEXT,"images":JSON}`, made
//! here with TEXT a string: a prompt is never read as JSON, so its words cannot
//! pose as attachments. `--input-images JSON` gives what the user attached.
#![forbid(unsafe_code)]

use serde_json::{Value as Json, json};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

const USAGE: &str = "usage: romacs-agent-host --dir DIR --socket SOCKET [--input FILE] [--input-images JSON] [--note JSON] [--store FILE] (-- PROGRAM [ARG...] | --notes-only)";

struct Config {
    dir: PathBuf,
    socket: PathBuf,
    input: Option<PathBuf>,
    note: Option<Json>,
    input_images: Option<Json>,
    /// The SQLite database the session's records go to, instead of its
    /// journal (`--store FILE`): a session Romacs has imported.
    store: Option<PathBuf>,
    /// No harness (`--notes-only`): it keeps the notes the server that
    /// attaches sends, and ends with that attachment.  A session's records
    /// are written by its host alone, this one when no harness runs.
    notes_only: bool,
    argv: Vec<String>,
}

fn parse(args: Vec<String>) -> Result<Config, String> {
    let mut args = args.into_iter();
    let (mut dir, mut socket, mut input, mut note, mut input_images, mut store) = (None, None, None, None, None, None);
    let mut notes_only = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => dir = args.next().map(PathBuf::from),
            "--socket" => socket = args.next().map(PathBuf::from),
            "--input" => input = Some(args.next().map(PathBuf::from).ok_or(USAGE)?),
            "--input-images" => input_images = Some(args.next().and_then(|text| serde_json::from_str(&text).ok()).ok_or(USAGE)?),
            "--note" => note = Some(args.next().and_then(|text| serde_json::from_str::<Json>(&text).ok()).ok_or(USAGE)?),
            "--store" => store = Some(args.next().map(PathBuf::from).ok_or(USAGE)?),
            "--notes-only" => notes_only = true,
            "--" => break,
            _ => return Err(USAGE.into()),
        }
    }
    let argv: Vec<String> = args.collect();
    match (dir, socket) {
        (Some(dir), Some(socket)) if (argv.is_empty() == notes_only) && socket.is_absolute() =>
            Ok(Config { dir, socket, input, note, input_images, store, notes_only, argv }),
        _ => Err(USAGE.into()),
    }
}

/// The journal and the attached server, under one lock: a record is
/// journaled and sent in the same order for every reader.
struct Shared {
    seq: u64,
    store: Store,
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

/// How long a record waits before it is tried again while the database is
/// busy, and how long before the attached server is told storage fails.
const STORE_RETRY: std::time::Duration = std::time::Duration::from_millis(100);
const STORE_FAILING: std::time::Duration = std::time::Duration::from_secs(30);

/// Whether harness output LINE is a thinking-token counter: a count the next
/// one supersedes, which no reader folds, sent but not kept.
fn counter(line: &str) -> bool {
    line.contains("\"subtype\":\"thinking_tokens\"") && line.contains("\"type\":\"system\"")
}

/// The records' tables, as lisp/agents/romacs-agent-events.el makes them.
const EVENTS_SCHEMA: &str = "create table if not exists events (
       session text not null, seq integer not null, t real, kind text, record text not null);
     create index if not exists events_seq on events (session, seq);
     create index if not exists events_kind on events (session, kind, seq);
     create table if not exists imported (session text primary key, at real) without rowid;";

/// Where a session's records are kept: its journal file, or the events
/// table of the agents' SQLite database once Romacs has imported it.
enum Store {
    /// CUT, when set, is where a failed write's line began, not yet cut
    /// back to: nothing is appended before it is.
    Journal { file: File, path: PathBuf, cut: Option<u64> },
    Sqlite { db: rusqlite::Connection, session: String },
}

impl Store {
    fn open(dir: &Path, database: Option<&Path>) -> Result<Store, String> {
        match database {
            None => {
                let path = dir.join("journal.jsonl");
                let file = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(Store::Journal { file, path, cut: None })
            }
            Some(database) => {
                let session = dir.file_name().and_then(|n| n.to_str()).ok_or("--dir names no session")?.to_owned();
                let db = rusqlite::Connection::open(database).map_err(|e| format!("{}: {e}", database.display()))?;
                db.busy_timeout(std::time::Duration::from_millis(200)).map_err(|e| e.to_string())?;
                db.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
                db.pragma_update(None, "synchronous", "NORMAL").map_err(|e| e.to_string())?;
                // As lisp/agents/romacs-agent-events.el makes them.
                db.execute_batch(EVENTS_SCHEMA).map_err(|e| e.to_string())?;
                Ok(Store::Sqlite { db, session })
            }
        }
    }

    /// Keep record TEXT, its seq SEQ; on disk before this returns when SYNC.
    fn keep(&mut self, seq: u64, text: &str, sync: bool) -> Result<(), String> {
        match self {
            Store::Journal { file, cut, .. } => {
                // A write that fails part way (a full disk) is undone, so its
                // retry appends a whole line, never one after a fragment; a
                // cut that failed too is made before anything more is written.
                if let Some(start) = *cut {
                    file.set_len(start).map_err(|e| e.to_string())?;
                    *cut = None;
                }
                let start = file.metadata().map_err(|e| e.to_string())?.len();
                let written = file.write_all(format!("{text}\n").as_bytes());
                if let Err(error) = written {
                    if file.set_len(start).is_err() { *cut = Some(start); }
                    return Err(error.to_string());
                }
                if sync { let _ = file.sync_data(); }
                Ok(())
            }
            Store::Sqlite { db, session } => {
                let kind = serde_json::from_str::<Json>(text).ok().and_then(|record| {
                    record.as_object()?.keys().find(|k| *k != "seq" && *k != "t").cloned()
                });
                let time = serde_json::from_str::<Json>(text).ok().and_then(|r| r.get("t")?.as_f64());
                // A turn's end, an exit, a note: as durable as the journal's sync.
                if sync { db.pragma_update(None, "synchronous", "FULL").map_err(|e| e.to_string())?; }
                let kept = db.execute("insert into events (session, seq, t, kind, record) values (?1, ?2, ?3, ?4, ?5)",
                                      rusqlite::params![&*session, seq as i64, time, kind, text]);
                if sync { let _ = db.pragma_update(None, "synchronous", "NORMAL"); }
                kept.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }

    /// The seq after which this run numbers: its last record's, or a turn
    /// row's first seq when one is later (a fork's borrowed turns, numbered
    /// when it was made).
    fn last_numbered(&self) -> u64 {
        let turns = match self {
            Store::Journal { .. } => 0,
            Store::Sqlite { db, session } => db
                .query_row("select coalesce(max(first_seq), 0) from turns where session = ?1", [session], |row| row.get::<_, i64>(0))
                .map_or(0, |seq| seq as u64),
        };
        self.last_seq().max(turns)
    }

    /// The seq of the last record kept, 0 for none.
    fn last_seq(&self) -> u64 {
        match self {
            Store::Journal { path, .. } => last_seq(path),
            Store::Sqlite { db, session } => db
                .query_row("select coalesce(max(seq), 0) from events where session = ?1", [session], |row| row.get::<_, i64>(0))
                .map_or(0, |seq| seq as u64),
        }
    }

    /// Call EACH with every kept record from seq FROM on, in order; false
    /// when EACH does.
    fn replay(&mut self, from: u64, mut each: impl FnMut(&str) -> bool) -> bool {
        match self {
            Store::Journal { file, path, .. } => {
                let _ = file.flush();
                let Ok(read) = File::open(&*path) else { return true };
                for line in BufReader::new(read).lines().map_while(Result::ok) {
                    let seq = serde_json::from_str::<Json>(&line).ok().and_then(|r| r.get("seq")?.as_u64()).unwrap_or(0);
                    if seq >= from && !each(&line) { return false; }
                }
                true
            }
            Store::Sqlite { db, session } => {
                let Ok(mut statement) = db.prepare("select record from events where session = ?1 and seq >= ?2 order by seq, rowid") else { return true };
                let Ok(rows) = statement.query_map(rusqlite::params![&*session, from as i64], |row| row.get::<_, String>(0)) else { return true };
                for line in rows.map_while(Result::ok) {
                    if !each(&line) { return false; }
                }
                true
            }
        }
    }
}

impl Shared {
    /// Keep TEXT (a whole record, its seq the current one) and send it to
    /// the attached server: kept first (on disk when SYNC), so whatever is
    /// sent can be replayed.
    fn append(&mut self, text: &str, sync: bool) {
        {
            let seq = self.seq;
            let mut waited = std::time::Duration::ZERO;
            let mut told = false;
            // Kept before it is sent, whatever fails: a busy database, a full
            // disk or an I/O error holds up the harness's output (its pipe
            // fills) until the record is kept, never sends one it could not
            // replay.
            while let Err(error) = self.store.keep(seq, text, sync) {
                if waited.is_zero() { eprintln!("romacs-agent-host: store: {error}"); }
                std::thread::sleep(STORE_RETRY);
                waited += STORE_RETRY;
                if waited >= STORE_FAILING && !told {
                    told = true;
                    self.send(&json!({"storage": "failing"}).to_string());
                }
            }
        }
        self.send(text);
    }

    /// Send TEXT, a line, to the attached server.
    fn send(&mut self, text: &str) {
        let line = format!("{text}\n");
        if let Some((_, stream)) = &mut self.attached
            && stream.write_all(line.as_bytes()).is_err() {
            // Closed, so the server sees its attachment end and attaches again.
            let _ = stream.shutdown(std::net::Shutdown::Both);
            self.attached = None;
        }
    }

    /// Journal the attached server ID's NOTE; a server whose attachment
    /// was stolen journals nothing more.
    fn note(&mut self, id: u64, note: &Json) -> bool {
        if self.attached.as_ref().map(|(attached, _)| *attached) != Some(id) { return false; }
        self.seq += 1;
        let entry = json!({"seq": self.seq, "t": now(), "note": note}).to_string();
        // Romacs's own record: as durable as a turn's end.
        self.append(&entry, true);
        true
    }

    fn output(&mut self, line: &str) {
        // A session kept in SQLite drops its thinking-token counters, which
        // no reader folds: numbered, they would leave a seq kept nowhere,
        // which a resumed host would number again.
        if matches!(self.store, Store::Sqlite { .. }) && counter(line) { return; }
        self.seq += 1;
        let sync = line.contains("\"type\":\"result\"") || line.contains("\"method\":\"turn/completed\"");
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
    if stream.set_write_timeout(Some(ATTACHED_WRITE)).is_err() { return None; }
    if !shared.store.replay(from, |line| writeln!(stream, "{line}").is_ok()) { return None; }
    let id = shared.next_attachment + 1;
    // Kept: the server may now act as this session's (its relays' route).
    // Told first, then published: a server gone before it hears this is
    // never the attached one.
    if writeln!(stream, "{}", json!({"attached": id})).is_err() { return None; }
    let clone = stream.try_clone().ok()?;
    shared.next_attachment = id;
    // A stolen attachment is closed: its server hears it ended, lets its
    // relays go to the route that now leads to the thief, and does not go on
    // taking the session's calls.
    if let Some((_, stolen)) = shared.attached.replace((id, clone)) {
        let _ = stolen.shutdown(std::net::Shutdown::Both);
    }
    Some(id)
}

/// A server's request: the line for the harness and the line for the journal.
/// A `journal` that is no string refuses the request: falling back to `write`
/// would journal the very bytes it was to keep out.
fn lines_of(request: &Json) -> Option<(String, String)> {
    let write = request.get("write")?.as_str()?.to_owned();
    let journal = match request.get("journal") {
        None => write.clone(),
        Some(journal) => journal.as_str()?.to_owned(),
    };
    Some((write, journal))
}

/// The record of a per-turn prompt TEXT, with the IMAGES attached to it.
fn input_record(seq: u64, text: &str, images: Option<&Json>) -> String {
    json!({"seq": seq, "t": now(), "in": {"prompt": text, "images": images.cloned().unwrap_or_else(|| json!([]))}}).to_string()
}

/// Serve one connection: its attach line, then the lines it writes.
/// Serve one attachment: its notes journaled, and with a HARNESS its
/// other lines too, written to the harness; a notes-only host takes notes
/// alone.
fn serve(shared: Arc<Mutex<Shared>>, stream: UnixStream, harness: bool) {
    let Ok(reading) = stream.try_clone() else { return };
    let mut lines = BufReader::new(reading).lines();
    let Some(Ok(first)) = lines.next() else { return };
    let Some(id) = serde_json::from_str::<Json>(&first).ok().and_then(|r| attach(&shared, stream, &r)) else { return };
    for line in lines.map_while(Result::ok) {
        let Ok(request) = serde_json::from_str::<Json>(&line) else { continue };
        if let Some(note) = request.get("note") {
            if !shared.lock().unwrap_or_else(|e| e.into_inner()).note(id, note) { return; }
            continue;
        }
        if !harness { continue; }
        let Some((text, journaled)) = lines_of(&request) else { continue };
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

/// How long a host waits for its session's lock: another host of it (a
/// notes-only one, a run ending) holds it for no longer.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The session's lock, held for the host's life: one host writes a
/// session's records at a time.  Its file is opened close-on-exec, so no
/// child holds it.
fn lock_session(dir: &Path) -> Result<File, String> {
    let path = dir.join("host.lock");
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let end = std::time::Instant::now() + LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < end =>
                std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(std::fs::TryLockError::WouldBlock) => return Err(format!("{}: another host holds it", path.display())),
            Err(std::fs::TryLockError::Error(e)) => return Err(format!("{}: {e}", path.display())),
        }
    }
}

/// How long a notes-only host waits for its server to attach.
const NOTES_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

fn run(config: Config) -> Result<i32, String> {
    create_private(&config.dir)?;
    // Before the socket and the store: a second host of the session waits
    // here, and never answers for it or numbers its records meanwhile.
    let _lock = lock_session(&config.dir)?;
    let listener = listen(&config.socket)?;
    let store = Store::open(&config.dir, config.store.as_deref())?;
    let seq = store.last_numbered();
    if config.notes_only { return run_notes(&config, listener, store, seq); }
    let stderr = OpenOptions::new().create(true).append(true).open(config.dir.join("harness.stderr")).map_err(|e| e.to_string())?;
    let mut child = Command::new(&config.argv[0]).args(&config.argv[1..])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(stderr)
        .spawn().map_err(|e| format!("{}: {e}", config.argv[0]))?;
    write_atomic(&config.dir.join("host.json"), &json!({"pid": std::process::id(), "harness": child.id(),
        "socket": config.socket, "started": now(), "argv": config.argv}).to_string())?;
    let stdout = child.stdout.take().ok_or("harness stdout")?;
    let mut stdin = child.stdin.take();
    let mut shared = Shared { seq, store, attached: None, next_attachment: 0, stdin: Arc::new(Mutex::new(None)) };
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
            std::thread::spawn(move || serve(shared, stream, true));
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

/// A host with no harness: it keeps the notes its server sends while that
/// server is attached, and ends with the attachment (or when none comes).
fn run_notes(config: &Config, listener: UnixListener, store: Store, seq: u64) -> Result<i32, String> {
    write_atomic(&config.dir.join("host.json"), &json!({"pid": std::process::id(), "harness": null,
        "socket": config.socket, "started": now(), "argv": [], "notesOnly": true}).to_string())?;
    let shared = Arc::new(Mutex::new(Shared { seq, store, attached: None, next_attachment: 0, stdin: Arc::new(Mutex::new(None)) }));
    let accepting = shared.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let shared = accepting.clone();
            std::thread::spawn(move || serve(shared, stream, false));
        }
    });
    let start = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let shared = shared.lock().unwrap_or_else(|e| e.into_inner());
        let had = shared.next_attachment > 0;
        if shared.attached.is_none() && (had || start.elapsed() > NOTES_WAIT) { break; }
    }
    // Gone first, so no server attaches to a host about to end.
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
        let shared = Arc::new(Mutex::new(Shared { seq: 1, store: Store::Journal { file: journal, path: path.clone(), cut: None }, attached: None, next_attachment: 0,
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
    fn a_notes_only_host_journals_notes_alone() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-notes-{}", std::process::id()));
        create_private(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, "").unwrap();
        let journal = OpenOptions::new().append(true).open(&path).unwrap();
        let shared = Arc::new(Mutex::new(Shared { seq: 0, store: Store::Journal { file: journal, path: path.clone(), cut: None }, attached: None, next_attachment: 0,
                                                  stdin: Arc::new(Mutex::new(None)) }));
        let (server, mut ours) = UnixStream::pair().unwrap();
        let serving = std::thread::spawn(move || serve(shared, server, false));
        writeln!(ours, r#"{{"attach":{{"from":1,"steal":false}}}}"#).unwrap();
        writeln!(ours, r#"{{"write":"{{\"type\":\"control_request\"}}"}}"#).unwrap();
        writeln!(ours, r#"{{"note":{{"type":"plan"}}}}"#).unwrap();
        ours.shutdown(std::net::Shutdown::Write).unwrap();
        serving.join().unwrap();
        let journaled = std::fs::read_to_string(&path).unwrap();
        let records: Vec<Json> = journaled.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(records.len(), 1, "{journaled}");
        assert_eq!(records[0]["note"]["type"], "plan");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_stolen_attachment_is_closed() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-steal-{}", std::process::id()));
        create_private(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, "").unwrap();
        let journal = OpenOptions::new().append(true).open(&path).unwrap();
        let shared = Arc::new(Mutex::new(Shared { seq: 0, store: Store::Journal { file: journal, path: path.clone(), cut: None }, attached: None, next_attachment: 0,
                                                  stdin: Arc::new(Mutex::new(None)) }));
        let attach_request: Json = serde_json::from_str(r#"{"attach":{"from":1,"steal":false}}"#).unwrap();
        let steal_request: Json = serde_json::from_str(r#"{"attach":{"from":1,"steal":true}}"#).unwrap();
        let (first, ours) = UnixStream::pair().unwrap();
        assert_eq!(attach(&shared, first, &attach_request), Some(1));
        let (second, theirs) = UnixStream::pair().unwrap();
        assert_eq!(attach(&shared, second, &steal_request), Some(2));
        // The first server reads its acknowledgement, then the end (not a
        // connection left open: the read would time out).
        ours.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut lines = BufReader::new(ours).lines();
        let kept: Json = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(kept["attached"], 1);
        assert!(lines.next().is_none());
        let kept: Json = serde_json::from_str(&BufReader::new(theirs).lines().next().unwrap().unwrap()).unwrap();
        assert_eq!(kept["attached"], 2);
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
        let shared = Arc::new(Mutex::new(Shared { seq: 0, store: Store::Journal { file: journal, path: path.clone(), cut: None }, attached: None, next_attachment: 0,
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
        // Not a string: refused, never journaled as what was written.
        assert_eq!(lines_of(&json!({"write": "bytes", "journal": 7})), None);
        assert_eq!(lines_of(&json!({"write": "bytes", "journal": null})), None);
    }

    #[test]
    fn a_turn_s_images_are_journaled_beside_its_text() {
        let with: Json = serde_json::from_str(&input_record(3, "see", Some(&json!(["/a.png"])))).unwrap();
        assert_eq!(with["in"], json!({"prompt": "see", "images": ["/a.png"]}));
        let without: Json = serde_json::from_str(&input_record(3, "see", None)).unwrap();
        assert_eq!(without["in"], json!({"prompt": "see", "images": []}));
    }

    #[test]
    fn a_prompt_shaped_like_a_record_stays_its_words() {
        let text = r#"{"text":"quoted","images":["/a.png"]}"#;
        let kept: Json = serde_json::from_str(&input_record(3, text, None)).unwrap();
        assert_eq!(kept["in"]["prompt"], json!(text));
        assert_eq!(kept["in"]["images"], json!([]));
    }

    #[test]
    fn a_note_is_journaled_for_the_attached_server_alone() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-note-{}", std::process::id()));
        create_private(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, "").unwrap();
        let journal = OpenOptions::new().append(true).open(&path).unwrap();
        let (server, ours) = UnixStream::pair().unwrap();
        let mut shared = Shared { seq: 4, store: Store::Journal { file: journal, path: path.clone(), cut: None }, attached: Some((1, server)), next_attachment: 1,
                                  stdin: Arc::new(Mutex::new(None)) };
        let note = json!({"type": "selection"});
        assert!(shared.note(1, &note));
        assert!(!shared.note(2, &note), "a stolen attachment notes nothing");
        let seen: Json = serde_json::from_str(&BufReader::new(ours).lines().next().unwrap().unwrap()).unwrap();
        assert_eq!((seen["seq"].as_u64(), &seen["note"]), (Some(5), &note));
        assert_eq!(last_seq(&path), 5);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn records_kept_in_sqlite_number_on_replay_and_skip_counters() {
        let dir = std::env::temp_dir().join(format!("romacs-host-sqlite-{}", std::process::id()));
        let session = dir.join("aaaaaaaa-0000-4000-8000-000000000001");
        std::fs::create_dir_all(&session).unwrap();
        let database = dir.join("sessions.sqlite");
        let mut shared = Shared { seq: 0, store: Store::open(&session, Some(&database)).unwrap(), attached: None,
                                  next_attachment: 0, stdin: Arc::new(Mutex::new(None)) };
        shared.output(r#"{"type":"assistant","message":{"id":"m1"}}"#);
        shared.output(r#"{"type":"system","subtype":"thinking_tokens","tokens":5}"#);
        shared.output(r#"{"type":"result","result":"done"}"#);
        // The counter is dropped: no seq is left kept nowhere.
        assert_eq!(shared.seq, 2);
        assert_eq!(shared.store.last_seq(), 2);
        let mut replayed = Vec::new();
        assert!(shared.store.replay(2, |line| { replayed.push(line.to_owned()); true }));
        assert_eq!(replayed.len(), 1);
        assert!(replayed[0].contains("\"result\":\"done\""));
        // A new host for the same session numbers on.
        let again = Store::open(&session, Some(&database)).unwrap();
        assert_eq!(again.last_seq(), 2);
        let kinds: Vec<String> = rusqlite::Connection::open(&database).unwrap()
            .prepare("select kind from events order by seq").unwrap()
            .query_map([], |row| row.get(0)).unwrap().map_while(Result::ok).collect();
        assert_eq!(kinds, ["out", "out"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_record_that_cannot_be_kept_is_not_sent_until_it_is() {
        let dir = std::env::temp_dir().join(format!("romacs-host-unkept-{}", std::process::id()));
        let session = dir.join("aaaaaaaa-0000-4000-8000-000000000002");
        std::fs::create_dir_all(&session).unwrap();
        let database = dir.join("sessions.sqlite");
        let store = Store::open(&session, Some(&database)).unwrap();
        // The table gone: every insert fails until it is made again.
        rusqlite::Connection::open(&database).unwrap().execute_batch("drop table events").unwrap();
        let (server, mut client) = UnixStream::pair().unwrap();
        let mut shared = Shared { seq: 0, store, attached: Some((1, server)), next_attachment: 1,
                                  stdin: Arc::new(Mutex::new(None)) };
        let started = std::time::Instant::now();
        let fixer = { let database = database.clone(); std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            rusqlite::Connection::open(&database).unwrap().execute_batch(EVENTS_SCHEMA).unwrap();
        }) };
        shared.output(r#"{"type":"assistant"}"#);
        fixer.join().unwrap();
        // Sent only once kept.
        assert!(started.elapsed() >= std::time::Duration::from_millis(400));
        assert_eq!(shared.store.last_seq(), 1);
        client.set_read_timeout(Some(std::time::Duration::from_secs(1))).unwrap();
        let mut line = String::new();
        BufReader::new(&mut client).read_line(&mut line).unwrap();
        assert!(line.contains("\"seq\":1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_journal_line_cut_short_is_undone_before_the_next() {
        let path = std::env::temp_dir().join(format!("romacs-host-partial-{}.jsonl", std::process::id()));
        std::fs::write(&path, "{\"seq\":1}\n").unwrap();
        let file = OpenOptions::new().append(true).open(&path).unwrap();
        // A write that took part of its line, its cut failing too: the cut is
        // pending, and the next keep makes it before it appends.
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{\"seq\":2,\"out").unwrap();
        let mut store = Store::Journal { file, path: path.clone(), cut: Some(10) };
        store.keep(2, r#"{"seq":2}"#, false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"seq\":1}\n{\"seq\":2}\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_argv_follows_the_separator() {
        let config = parse(["--dir", "d", "--socket", "/s", "--", "claude", "-p"].map(String::from).to_vec()).unwrap();
        assert_eq!(config.argv, ["claude", "-p"]);
        assert!(parse(["--dir", "d", "--socket", "relative", "--", "x"].map(String::from).to_vec()).is_err());
        let noted = parse(["--dir", "d", "--socket", "/s", "--note", r#"{"type":"run"}"#, "--", "x"].map(String::from).to_vec()).unwrap();
        assert_eq!(noted.note, Some(json!({"type": "run"})));
        assert!(parse(["--dir", "d", "--socket", "/s", "--note", "not json", "--", "x"].map(String::from).to_vec()).is_err());
        let notes = parse(["--dir", "d", "--socket", "/s", "--notes-only"].map(String::from).to_vec()).unwrap();
        assert!(notes.notes_only && notes.argv.is_empty());
        assert!(parse(["--dir", "d", "--socket", "/s", "--notes-only", "--", "x"].map(String::from).to_vec()).is_err());
        assert!(parse(["--dir", "d", "--socket", "/s"].map(String::from).to_vec()).is_err());
    }

    #[test]
    fn one_host_of_a_session_at_a_time() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-lock-{}", std::process::id()));
        create_private(&dir).unwrap();
        let held = lock_session(&dir).unwrap();
        let again = OpenOptions::new().write(true).open(dir.join("host.lock")).unwrap();
        assert!(matches!(again.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
        drop(held);
        assert!(again.try_lock().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_numbers_after_a_forks_borrowed_turns() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-turns-{}", std::process::id()));
        create_private(&dir).unwrap();
        let file = dir.join("store.sqlite");
        let session = dir.file_name().unwrap().to_str().unwrap().to_owned();
        let store = Store::open(&dir, Some(&file)).unwrap();
        if let Store::Sqlite { db, .. } = &store {
            db.execute_batch("create table if not exists turns (session text, first_seq integer)").unwrap();
            db.execute("insert into turns values (?1, 7)", [&session]).unwrap();
            db.execute("insert into events (session, seq, t, kind, record) values (?1, 3, 0, 'note', '{}')", [&session]).unwrap();
        }
        assert_eq!(store.last_numbered(), 7);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
