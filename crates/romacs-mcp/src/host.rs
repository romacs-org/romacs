//! `romacs-agent-host --dir DIR --socket SOCKET -- PROGRAM ARG...`: the
//! supervisor of one agent session (lisp/agents/romacs-agent.el). It runs the
//! harness (Claude Code's stream-json CLI, or any program speaking lines),
//! owns its stdin and stdout, and outlives every Romacs server: a server
//! attaches over SOCKET, is sent the journal from a sequence number on and
//! then each new line, and writes lines to the harness. When the server
//! exits, crashes or is replaced, the harness and its turn go on; the next
//! server attaches and replays. A session whose DIR is removed is gone: its
//! harness is ended, and the host exits with it.
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
//! pose as attachments. `--input-images JSON` gives what the user attached,
//! and `--input-tag JSON` the request it answers, kept as `"romacsRequest"`
//! beside them: what finds the prompt in the records again.
//!
//! Once a line written to the harness has reached its stdin, whole, the host
//! journals `{"seq":M,"t":SECS,"written":SEQ}`, SEQ being that line's `"in"`
//! record: a reader knows the harness was given it, even after a restart.
//! A line whose write failed has none, and nor may one the harness took
//! just before it exited: its exit settles every request `written` would
//! have, as an answer no longer waiting on the harness.  Nothing is
//! journaled after the exit.
#![forbid(unsafe_code)]

use serde_json::{Value as Json, json};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

const USAGE: &str = "usage: romacs-agent-host --dir DIR --socket SOCKET [--input FILE] [--input-images JSON] [--input-tag JSON] [--note JSON] [--store FILE] (-- PROGRAM [ARG...] | --notes-only)";

struct Config {
    dir: PathBuf,
    socket: PathBuf,
    input: Option<PathBuf>,
    note: Option<Json>,
    input_images: Option<Json>,
    input_tag: Option<Json>,
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
    let (mut dir, mut socket, mut input, mut note, mut input_images, mut input_tag, mut store) = (None, None, None, None, None, None, None);
    let mut notes_only = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => dir = args.next().map(PathBuf::from),
            "--socket" => socket = args.next().map(PathBuf::from),
            "--input" => input = Some(args.next().map(PathBuf::from).ok_or(USAGE)?),
            "--input-images" => input_images = Some(args.next().and_then(|text| serde_json::from_str(&text).ok()).ok_or(USAGE)?),
            "--input-tag" => input_tag = Some(args.next().and_then(|text| serde_json::from_str(&text).ok()).ok_or(USAGE)?),
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
            Ok(Config { dir, socket, input, note, input_images, input_tag, store, notes_only, argv }),
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
    /// The harness's exit is journaled: nothing more is, so a late writer
    /// never numbers a record after it, where the session's next host may
    /// be numbering its own.
    closed: bool,
}

/// How long a write to the attached server may block before it is
/// detached: a stalled reader never holds up the journal or the exit.
const ATTACHED_WRITE: std::time::Duration = std::time::Duration::from_secs(2);

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// One journal record of LINE under KEY, kept as JSON when it is JSON.
/// The bytes of a harness's output read at a time, and the most of its
/// lines journaled and sent in one write.
const OUTPUT_READ: usize = 1 << 16;
const OUTPUT_BATCH: usize = 4096;

fn record(seq: u64, key: &str, line: &str) -> String {
    let value = serde_json::from_str::<Json>(line).unwrap_or_else(|_| Json::String(line.to_owned()));
    json!({"seq": seq, "t": now(), key: value}).to_string()
}

/// How long a write waits for another's lock before it is busy: longer
/// than any transaction in normal use holds it (each is one record, or a
/// few, and commits in milliseconds), so a writer never sees one locked.
const STORE_BUSY: std::time::Duration = std::time::Duration::from_secs(5);

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
    /// back to: nothing is appended before it is.  LEN is where the next
    /// line begins: the file's length, kept here, not asked of the file a
    /// line at a time (a flood's lines are many).
    Journal { file: File, path: PathBuf, cut: Option<u64>, len: u64 },
    Sqlite { db: rusqlite::Connection, session: String },
}

impl Store {
    /// The journal FILE at PATH, appended to from its end: its length
    /// known, or none (a failed write is cut back to where it began).
    fn journal(file: File, path: PathBuf) -> Result<Store, String> {
        let len = file.metadata().map_err(|e| format!("{}: {e}", path.display()))?.len();
        Ok(Store::Journal { file, path, cut: None, len })
    }

    fn open(dir: &Path, database: Option<&Path>) -> Result<Store, String> {
        match database {
            None => {
                let path = dir.join("journal.jsonl");
                let file = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                Store::journal(file, path)
            }
            Some(database) => {
                let session = dir.file_name().and_then(|n| n.to_str()).ok_or("--dir names no session")?.to_owned();
                // Its lock held by another (the writer making the store, a
                // migration's transaction) is waited out, as a record's
                // write is: a host that gave up here would leave its
                // session unrun.
                loop {
                    match Self::open_sqlite(database) {
                        Ok(db) => return Ok(Store::Sqlite { db, session }),
                        Err(rusqlite::Error::SqliteFailure(error, _))
                            if matches!(error.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) =>
                        {
                            std::thread::sleep(STORE_RETRY)
                        }
                        Err(error) => return Err(format!("{}: {error}", database.display())),
                    }
                }
            }
        }
    }

    fn open_sqlite(database: &Path) -> rusqlite::Result<rusqlite::Connection> {
        let db = rusqlite::Connection::open(database)?;
        db.busy_timeout(STORE_BUSY)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "NORMAL")?;
        // As lisp/agents/romacs-agent-events.el makes them.
        db.execute_batch(EVENTS_SCHEMA)?;
        Ok(db)
    }

    /// Keep RECORDS, each (SEQ, TEXT), in order, all or none: one write of
    /// the journal, one transaction of the database; on disk before this
    /// returns when SYNC.
    fn keep(&mut self, records: &[(u64, String)], sync: bool) -> Result<(), String> {
        match self {
            Store::Journal { file, cut, len, .. } => {
                // A write that fails part way (a full disk) is undone, so its
                // retry appends a whole line, never one after a fragment; a
                // cut that failed too is made before anything more is written.
                if let Some(start) = *cut {
                    file.set_len(start).map_err(|e| e.to_string())?;
                    *cut = None;
                    *len = start;
                }
                let start = *len;
                let lines: String = records.iter().map(|(_, text)| format!("{text}\n")).collect();
                if let Err(error) = file.write_all(lines.as_bytes()) {
                    if file.set_len(start).is_err() { *cut = Some(start); }
                    return Err(error.to_string());
                }
                *len = start + lines.len() as u64;
                if sync { let _ = file.sync_data(); }
                Ok(())
            }
            Store::Sqlite { db, session } => {
                // A turn's end, an exit, a note: as durable as the journal's sync.
                if sync { db.pragma_update(None, "synchronous", "FULL").map_err(|e| e.to_string())?; }
                let kept = (|| {
                    // Its write lock taken at once, the busy wait with it: a
                    // deferred one taken on its first insert could not wait.
                    let transaction = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                    {
                        let mut insert = transaction.prepare_cached(
                            "insert into events (session, seq, t, kind, record) values (?1, ?2, ?3, ?4, ?5)")?;
                        for (seq, text) in records {
                            let parsed = serde_json::from_str::<Json>(text).ok();
                            let kind = parsed.as_ref().and_then(|record| {
                                record.as_object()?.keys().find(|k| *k != "seq" && *k != "t").cloned()
                            });
                            let time = parsed.as_ref().and_then(|r| r.get("t")?.as_f64());
                            insert.execute(rusqlite::params![&*session, *seq as i64, time, kind, text])?;
                        }
                    }
                    transaction.commit()
                })();
                if sync { let _ = db.pragma_update(None, "synchronous", "NORMAL"); }
                kept.map_err(|e| e.to_string())
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
        self.append_all(&[(self.seq, text.to_owned())], sync);
    }

    /// Keep RECORDS, each (SEQ, TEXT), then send them to the attached server
    /// in one write: kept first, as `append' keeps one.
    fn append_all(&mut self, records: &[(u64, String)], sync: bool) {
        if records.is_empty() || self.closed { return; }
        {
            // Timed from the first try: each waits its busy timeout too.
            let since = std::time::Instant::now();
            let mut said = false;
            let mut told = false;
            // Kept before it is sent, whatever fails: a busy database, a full
            // disk or an I/O error holds up the harness's output (its pipe
            // fills) until the record is kept, never sends one it could not
            // replay.
            while let Err(error) = self.store.keep(records, sync) {
                if !said { said = true; eprintln!("romacs-agent-host: store: {error}"); }
                std::thread::sleep(STORE_RETRY);
                if since.elapsed() >= STORE_FAILING && !told {
                    told = true;
                    self.send(&json!({"storage": "failing"}).to_string());
                }
            }
        }
        self.send_lines(&records.iter().map(|(_, text)| format!("{text}\n")).collect::<String>());
    }

    /// Send TEXT, a line, to the attached server.
    fn send(&mut self, text: &str) {
        self.send_lines(&format!("{text}\n"));
    }

    /// Send LINE, whole lines, to the attached server.
    fn send_lines(&mut self, line: &str) {
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

    /// Journal and send the harness's output LINES, as they came: in one
    /// write, a flood's lines that were there together.
    fn output(&mut self, lines: &[String]) {
        let mut records = Vec::with_capacity(lines.len());
        let mut sync = false;
        for line in lines {
            // A session kept in SQLite drops its thinking-token counters, which
            // no reader folds: numbered, they would leave a seq kept nowhere,
            // which a resumed host would number again.
            if matches!(self.store, Store::Sqlite { .. }) && counter(line) { continue; }
            self.seq += 1;
            sync |= line.contains("\"type\":\"result\"") || line.contains("\"method\":\"turn/completed\"");
            records.push((self.seq, record(self.seq, "out", line)));
        }
        self.append_all(&records, sync);
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
/// Attach a server to the session on STREAM, after replaying what it has
/// not seen; a notes-only host (no HARNESS) says so in its acknowledgement,
/// so a server that wants a supervisor lets it go and looks again.
fn attach(shared: &Arc<Mutex<Shared>>, mut stream: UnixStream, request: &Json, harness: bool) -> Option<u64> {
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
    let acknowledgement = if harness { json!({"attached": id}) } else { json!({"attached": id, "notesOnly": true}) };
    if writeln!(stream, "{acknowledgement}").is_err() { return None; }
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

/// The record of a per-turn prompt TEXT, with the IMAGES attached to it
/// and the request TAG it answers, when given.
fn input_record(seq: u64, text: &str, images: Option<&Json>, tag: Option<&Json>) -> String {
    let mut input = json!({"prompt": text, "images": images.cloned().unwrap_or_else(|| json!([]))});
    if let Some(tag) = tag { input["romacsRequest"] = tag.clone(); }
    json!({"seq": seq, "t": now(), "in": input}).to_string()
}

impl Shared {
    /// Journal the harness's exit with CODE, the journal's last record.
    fn exit(&mut self, code: i32) {
        self.seq += 1;
        let entry = json!({"seq": self.seq, "t": now(), "exit": code}).to_string();
        self.append(&entry, true);
        self.closed = true;
    }

    /// Journal that the line of `"in"` record SEQ reached the harness.
    fn written(&mut self, seq: u64) {
        self.seq += 1;
        let entry = json!({"seq": self.seq, "t": now(), "written": seq}).to_string();
        self.append(&entry, false);
    }
}

/// Serve one connection: its attach line, then the lines it writes.
/// Serve one attachment: its notes journaled, and with a HARNESS its
/// other lines too, written to the harness; a notes-only host takes notes
/// alone.
fn serve(shared: Arc<Mutex<Shared>>, stream: UnixStream, harness: bool) {
    let Ok(reading) = stream.try_clone() else { return };
    let mut lines = BufReader::new(reading).lines();
    let Some(Ok(first)) = lines.next() else { return };
    let Some(id) = serde_json::from_str::<Json>(&first).ok().and_then(|r| attach(&shared, stream, &r, harness)) else { return };
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
        let seq = {
            let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
            // A server whose attachment was stolen writes nothing more, nor
            // one whose harness's exit is journaled: a line written now
            // would have no record.
            if shared.closed || shared.attached.as_ref().map(|(attached, _)| *attached) != Some(id) { return; }
            shared.seq += 1;
            let entry = record(shared.seq, "in", &journaled);
            shared.append(&entry, false);
            shared.seq
        };
        let written = stdin.as_mut().map(|stdin| writeln!(stdin, "{text}").and_then(|()| stdin.flush()));
        // Still holding the stdin: the next line's records come after these.
        if matches!(written, Some(Ok(()))) {
            shared.lock().unwrap_or_else(|e| e.into_inner()).written(seq);
        } else {
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
    // A notes-only host writes to a session that is there: one whose
    // directory was removed is gone, and is not made again for a note.
    if config.notes_only && !config.dir.is_dir() {
        return Err(format!("{}: the session is gone", config.dir.display()));
    }
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
    let mut shared = Shared { seq, store, attached: None, next_attachment: 0, stdin: Arc::new(Mutex::new(None)), closed: false };
    // The run's first record: its output is read only from here on.
    if let Some(note) = &config.note {
        shared.seq += 1;
        let entry = json!({"seq": shared.seq, "t": now(), "note": note}).to_string();
        shared.append(&entry, true);
    }
    let input = match &config.input {
        Some(file) => {
            let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            shared.seq += 1;
            let entry = input_record(shared.seq, &text, config.input_images.as_ref(), config.input_tag.as_ref());
            shared.append(&entry, true);
            let _ = std::fs::remove_file(file);
            Some((shared.seq, text, stdin.take()))
        }
        None => None,
    };
    shared.stdin = Arc::new(Mutex::new(stdin));
    let shared = Arc::new(Mutex::new(shared));
    if let Some((seq, text, mut writer)) = input {
        // Its own thread: a harness slow to read never holds up the host.
        // Journaled `written`, then closed: such a harness reads to its end.
        let writing = shared.clone();
        std::thread::spawn(move || {
            if writer.as_mut().is_some_and(|w| w.write_all(text.as_bytes()).and_then(|()| w.flush()).is_ok()) {
                writing.lock().unwrap_or_else(|e| e.into_inner()).written(seq);
            }
            drop(writer);
        });
    }
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
        let mut reader = BufReader::with_capacity(OUTPUT_READ, stdout);
        let mut line = Vec::new();
        let mut batch: Vec<String> = Vec::new();
        'reading: loop {
            // A line, then those already read in behind it, whole: kept and
            // sent together, a flood in few writes; a line alone at once.
            loop {
                line.clear();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break 'reading,
                    Ok(_) => batch.push(String::from_utf8_lossy(&line).trim_end_matches(['\n', '\r']).to_owned()),
                }
                if batch.len() >= OUTPUT_BATCH || !reader.buffer().contains(&b'\n') { break; }
            }
            reading.lock().unwrap_or_else(|e| e.into_inner()).output(&batch);
            batch.clear();
        }
        if !batch.is_empty() { reading.lock().unwrap_or_else(|e| e.into_inner()).output(&batch); }
        let _ = drained_tx.send(());
    });
    let status = wait_while_kept(&mut child, &config.dir)?;
    // What the harness wrote before it ended, unless a descendant holds
    // its stdout open.
    let _ = drained.recv_timeout(std::time::Duration::from_secs(1));
    let code = status.code().unwrap_or(128 + std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0));
    shared.lock().unwrap_or_else(|e| e.into_inner()).exit(code);
    let _ = std::fs::remove_file(&config.socket);
    Ok(0)
}

/// A host with no harness: it keeps the notes its server sends while that
/// server is attached, and ends with the attachment (or when none comes).
fn run_notes(config: &Config, listener: UnixListener, store: Store, seq: u64) -> Result<i32, String> {
    write_atomic(&config.dir.join("host.json"), &json!({"pid": std::process::id(), "harness": null,
        "socket": config.socket, "started": now(), "argv": [], "notesOnly": true}).to_string())?;
    let shared = Arc::new(Mutex::new(Shared { seq, store, attached: None, next_attachment: 0, stdin: Arc::new(Mutex::new(None)), closed: false }));
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

/// How often the host looks at its harness and its session's directory.
const WATCH: std::time::Duration = std::time::Duration::from_millis(250);

/// The harness's exit status. A session whose directory was removed is
/// gone (a test's tree deleted while its harness waits, a session the
/// editor removed): nothing reads what its harness does, so it is ended,
/// and the host with it, rather than kept forever by a harness waiting on
/// its stdin.
fn wait_while_kept(child: &mut std::process::Child, dir: &Path) -> Result<std::process::ExitStatus, String> {
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status);
        }
        // Gone only when it is confirmed gone: an error reading it (a
        // moment's I/O trouble) is no end of the session.
        if std::fs::metadata(dir).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) {
            let _ = child.kill();
        }
        std::thread::sleep(WATCH);
    }
}

/// Whether one holds the session in DIR: its host.lock, which a host takes
/// first and holds its whole run, and a move of its records (`--stage',
/// `--hold') holds while it runs.  Asked by `--supervised DIR...'.
fn supervised(dir: &Path) -> bool {
    match OpenOptions::new().write(true).open(dir.join("host.lock")) {
        Ok(file) => match file.try_lock() {
            Err(std::fs::TryLockError::WouldBlock) => true,
            Ok(()) => { let _ = file.unlock(); false }
            Err(std::fs::TryLockError::Error(_)) => false,
        },
        Err(_) => false,
    }
}

/// The session in DIR's lock, taken now or not at all: a move of its
/// records holds it, and never waits for a host that runs.
fn take_lock(dir: &Path) -> Result<File, String> {
    let path = dir.join("host.lock");
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(format!("{}: a host holds it", path.display())),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
    }
}

/// The rows a stage commits a transaction: others' writes wait no longer.
const STAGE_CHUNK: usize = 20_000;

/// Write SESSION's JOURNAL into DATABASE as records not yet its own (no
/// `imported' row: no reader reads them): every record with a seq, as its
/// line, but thinking-token counters, up to the journal's size as this
/// began; its last line too when whole and unended.  What an earlier stage
/// wrote is removed first; one imported already is refused.  (SIZE,
/// COUNT): the size read and the records written.  On an error, what it
/// wrote is removed.  lisp/agents/romacs-agent-events.el moves a journal
/// so, then makes the records the session's own.
fn stage(journal: &Path, database: &Path, session: &str) -> Result<(u64, u64), String> {
    // No host writes the journal while it is read: none starts meanwhile.
    let _lock = take_lock(journal.parent().ok_or("a journal names no session")?)?;
    let size = std::fs::metadata(journal).map(|m| m.len()).unwrap_or(0);
    let mut db = rusqlite::Connection::open(database).map_err(|e| format!("{}: {e}", database.display()))?;
    db.busy_timeout(std::time::Duration::from_secs(60)).map_err(|e| e.to_string())?;
    db.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
    db.pragma_update(None, "synchronous", "NORMAL").map_err(|e| e.to_string())?;
    db.execute_batch(EVENTS_SCHEMA).map_err(|e| e.to_string())?;
    {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        let imported: bool = tx.query_row("select exists (select 1 from imported where session = ?1)", [session], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        if imported { return Err(format!("staging {session}: imported already")); }
        tx.execute("delete from events where session = ?1", [session]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    let written = stage_lines(&mut db, journal, size, session);
    if written.is_err() {
        let _ = db.execute("delete from events where session = ?1", [session]);
    }
    written.map(|count| (size, count))
}

/// A line as the supervisor writes it, `{"KIND":...,"seq":N,"t":F}': its
/// (SEQ, T, KIND) read from its ends, nothing else parsed; None for any
/// other, which is parsed whole.
fn line_fields(text: &str) -> Option<(i64, Option<f64>, Option<String>)> {
    let rest = text.strip_prefix("{\"")?;
    let kind = &rest[..rest.find('"')?];
    if kind.is_empty() || kind == "seq" || kind == "t" || !kind.bytes().all(|b| b.is_ascii_lowercase())
        || !rest[kind.len()..].starts_with("\":") {
        return None;
    }
    // From a character's start: a byte 80 from the end may be inside one.
    let start = (text.len().saturating_sub(80)..text.len()).find(|&i| text.is_char_boundary(i))?;
    let tail = &text[start..];
    let at = tail.rfind(",\"seq\":")?;
    let fields = tail[at + 7..].strip_suffix('}')?;
    let (seq, time) = fields.split_once(",\"t\":")?;
    Some((seq.parse().ok()?, Some(time.parse().ok()?), Some(kind.to_owned())))
}

fn stage_lines(db: &mut rusqlite::Connection, journal: &Path, size: u64, session: &str) -> Result<u64, String> {
    use std::io::Read;
    // None at all: nothing to write.
    if size == 0 { return Ok(0); }
    let file = File::open(journal).map_err(|e| format!("{}: {e}", journal.display()))?;
    let mut reader = BufReader::with_capacity(1 << 20, file.take(size));
    let mut line = Vec::new();
    let (mut count, mut pending) = (0u64, 0usize);
    let mut tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).map_err(|e| e.to_string())? == 0 { break; }
        let ended = line.last() == Some(&b'\n');
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\n', '\r']);
        if text.trim().is_empty() { continue; }
        // A harness's counter only: an `in` record may quote one.
        if text.starts_with("{\"out\":") && text.contains("thinking_tokens") && counter(text) { continue; }
        // As the supervisor writes a line (its kind first, its seq and time
        // last): read from its ends; another, parsed.
        let (seq, time, kind) = match (ended, line_fields(text)) {
            (true, Some(fields)) => fields,
            _ => {
                let record = serde_json::from_str::<Json>(text);
                // Its last line, unended: a whole record, or nothing written.
                if !ended && !matches!(record, Ok(Json::Object(_))) {
                    return Err(format!("staging {session}: its last line is cut short"));
                }
                let Ok(Json::Object(record)) = record else { continue };
                let Some(seq) = record.get("seq").and_then(Json::as_i64) else { continue };
                (seq, record.get("t").and_then(Json::as_f64),
                 record.keys().find(|key| *key != "seq" && *key != "t").cloned())
            }
        };
        tx.execute("insert into events (session, seq, t, kind, record) values (?1, ?2, ?3, ?4, ?5)",
                   rusqlite::params![session, seq, time, kind, text]).map_err(|e| e.to_string())?;
        count += 1;
        pending += 1;
        if pending >= STAGE_CHUNK {
            tx.commit().map_err(|e| e.to_string())?;
            tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
            pending = 0;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(count)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some((flag, dirs)) = args.split_first() && flag == "--supervised" {
        // Each held session's directory, a line each.
        for dir in dirs.iter().filter(|dir| supervised(Path::new(dir))) { println!("{dir}"); }
        std::process::exit(0);
    }
    if let [flag, dir] = args.as_slice() && flag == "--hold" {
        // Held until stdin closes: what the caller does meanwhile no host
        // starts under.
        match take_lock(Path::new(dir)) {
            Ok(_lock) => {
                println!("held");
                let _ = std::io::stdout().flush();
                let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
                std::process::exit(0)
            }
            Err(error) => { eprintln!("romacs-agent-host: {error}"); std::process::exit(1) }
        }
    }
    if let [flag, journal, store_flag, database, session_flag, session] = args.as_slice()
        && flag == "--stage" && store_flag == "--store" && session_flag == "--session" {
        match stage(Path::new(journal), Path::new(database), session) {
            Ok((size, count)) => { println!("{size} {count}"); std::process::exit(0) }
            Err(error) => { eprintln!("romacs-agent-host: {error}"); std::process::exit(1) }
        }
    }
    let code = parse(args).and_then(run).unwrap_or_else(|error| {
        eprintln!("romacs-agent-host: {error}");
        1
    });
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_read_from_its_ends_is_what_a_parse_reads() {
        // Its kind, seq and time, exactly: a float as `str::parse' reads it.
        for (line, want) in [
            (r#"{"in":{"x":1},"seq":3,"t":1794678357.2928457}"#, Some((3, Some(1794678357.2928457), Some("in".to_owned())))),
            (r#"{"out":{"t":2,"seq":9},"seq":12,"t":0.1}"#, Some((12, Some(0.1), Some("out".to_owned())))),
            (r#"{"seq":3,"t":1.5,"note":{}}"#, None),
            // A nested seq and time are not the line's.
            (r#"{"t":4.0,"seq":11,"note":{"seq":1,"t":2}}"#, None),
            (r#"{"in":{"seq":5,"t":1},"seq":7,"t":2.5}"#, Some((7, Some(2.5), Some("in".to_owned())))),
        ] {
            assert_eq!(line_fields(line), want);
            if let Some((seq, time, kind)) = want {
                let parsed: Json = serde_json::from_str(line).unwrap();
                assert_eq!(parsed["seq"].as_i64(), Some(seq));
                assert_eq!(parsed["t"].as_f64(), time);
                assert_eq!(parsed.as_object().unwrap().keys().find(|k| *k != "seq" && *k != "t").cloned(), kind);
            }
        }
    }

    #[test]
    fn every_line_the_supervisor_writes_reads_from_its_ends_as_a_parse() {
        // Lines as the supervisor writes them (json!, its keys sorted, so
        // the kind first and seq, t last), times of every size and
        // precision: the fields read from the line's ends are a parse's.
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        for i in 0..200_000u64 {
            x ^= x << 13; x ^= x >> 7; x ^= x << 17;
            let time = match i % 4 {
                0 => 1.7e9 + (x % 1_000_000_000) as f64 / 1e7,
                1 => f64::from_bits(0x3ff0_0000_0000_0000 | (x >> 12)) * 1e9,
                2 => (x % 100_000) as f64,
                _ => f64::from_bits(x >> 2).abs().min(1e15),
            };
            let kind = ["in", "out", "note", "exit"][(i % 4) as usize];
            let line = json!({"seq": x % 10_000_000, "t": time, kind: {"seq": 1, "t": 2.5, "text": "x"}}).to_string();
            let parsed: Json = serde_json::from_str(&line).unwrap();
            assert_eq!(line_fields(&line),
                       Some((parsed["seq"].as_i64().unwrap(), parsed["t"].as_f64(), Some(kind.to_owned()))),
                       "{line}");
        }
    }

    #[test]
    fn a_line_of_megabytes_is_read_from_its_ends() {
        let line = format!("{{\"out\":{{\"text\":\"{}\"}},\"seq\":10,\"t\":3.0}}", "a".repeat(4_000_000));
        assert_eq!(line_fields(&line), Some((10, Some(3.0), Some("out".to_owned()))));
    }

    #[test]
    fn a_character_across_the_tails_start_is_read_past() {
        // Each offset of a three-byte character around 80 bytes from the end.
        for pad in 0..4 {
            let line = format!("{{\"out\":{{\"text\":\"{}€{}\"}},\"seq\":10,\"t\":3.0}}", "a".repeat(40), "b".repeat(56 + pad));
            assert_eq!(line_fields(&line), Some((10, Some(3.0), Some("out".to_owned()))), "{line}");
        }
    }

    #[test]
    fn a_stage_writes_every_record_and_holds_its_session() {
        let dir = std::env::temp_dir().join(format!("romacs-host-stage-{}", std::process::id()));
        let session = dir.join("aaaaaaaa-0000-4000-8000-000000000005");
        std::fs::create_dir_all(&session).unwrap();
        let journal = session.join("journal.jsonl");
        std::fs::write(&journal, concat!(
            "{\"in\":{\"prompt\":\"p\",\"romacsRequest\":{\"subtype\":\"thinking_tokens\",\"type\":\"system\"}},\"seq\":1,\"t\":1.25}\n",
            "{\"out\":{\"subtype\":\"thinking_tokens\",\"type\":\"system\"},\"seq\":2,\"t\":2.0}\n",
            "{\"seq\":3,\"t\":3.5,\"note\":{\"type\":\"run\"}}\n",
            "not json\n",
            "{\"out\":{\"type\":\"assistant\",\"text\":\"thinking_tokens are words\"},\"seq\":5,\"t\":5.0}\n",
            "{\"out\":{\"x\":true},\"seq\":6,\"t\":6.0}")).unwrap();
        let database = dir.join("sessions.sqlite");
        // A host holding it: refused, nothing written.
        let held = lock_session(&session).unwrap();
        assert!(stage(&journal, &database, "s").is_err());
        drop(held);
        let (size, count) = stage(&journal, &database, "s").unwrap();
        assert_eq!(size, std::fs::metadata(&journal).unwrap().len());
        // The counter and the line no record left out; a prompt whose tag
        // looks like a counter, a line mentioning thinking tokens, the parsed
        // line and the unended last one in, each as it was written.
        assert_eq!(count, 4);
        let rows: Vec<(i64, f64, String)> = rusqlite::Connection::open(&database).unwrap()
            .prepare("select seq, t, kind from events where session = 's' order by seq").unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap().map_while(Result::ok).collect();
        assert_eq!(rows, [(1, 1.25, "in".to_owned()), (3, 3.5, "note".to_owned()), (5, 5.0, "out".to_owned()),
                          (6, 6.0, "out".to_owned())]);
        let kept: String = rusqlite::Connection::open(&database).unwrap()
            .query_row("select record from events where seq = 5", [], |row| row.get(0)).unwrap();
        assert_eq!(kept, "{\"out\":{\"type\":\"assistant\",\"text\":\"thinking_tokens are words\"},\"seq\":5,\"t\":5.0}");
        // Not held once it ended.
        assert!(!supervised(&session));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_session_is_supervised_while_its_lock_is_held() {
        let dir = std::env::temp_dir().join(format!("romacs-host-supervised-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // None ever ran: no lock file.
        assert!(!supervised(&dir));
        let held = lock_session(&dir).unwrap();
        assert!(supervised(&dir));
        drop(held);
        // Its host gone: the lock file left, held by none.
        assert!(!supervised(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_harness_whose_session_directory_was_removed_is_ended() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-gone-{}", std::process::id()));
        create_private(&dir).unwrap();
        // A harness that waits on its stdin, as an idle agent does.
        let mut child = Command::new("cat").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        let _stdin = child.stdin.take();
        let removed = dir.clone();
        std::thread::spawn(move || { std::thread::sleep(WATCH * 2); std::fs::remove_dir_all(&removed).unwrap(); });
        let started = std::time::Instant::now();
        let status = wait_while_kept(&mut child, &dir).unwrap();
        assert!(!status.success(), "ended by the host, not by itself: {status:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "ended within a few looks");
    }

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
        let shared = Arc::new(Mutex::new(Shared { seq: 1, store: Store::journal(journal, path.clone()).unwrap(), attached: None, next_attachment: 0,
                                                  stdin: Arc::new(Mutex::new(None)), closed: false }));
        let request: Json = serde_json::from_str(r#"{"attach":{"from":1,"steal":false}}"#).unwrap();
        let (server, ours) = UnixStream::pair().unwrap();
        assert_eq!(attach(&shared, server, &request, true), Some(1));
        let mut lines = BufReader::new(ours.try_clone().unwrap()).lines();
        let replayed: Json = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(replayed["seq"], 1);
        let kept: Json = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(kept["attached"], 1);
        let (other, theirs) = UnixStream::pair().unwrap();
        assert_eq!(attach(&shared, other, &request, true), None);
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
        let shared = Arc::new(Mutex::new(Shared { seq: 0, store: Store::journal(journal, path.clone()).unwrap(), attached: None, next_attachment: 0,
                                                  stdin: Arc::new(Mutex::new(None)), closed: false }));
        let (server, mut ours) = UnixStream::pair().unwrap();
        let serving = std::thread::spawn(move || serve(shared, server, false));
        writeln!(ours, r#"{{"attach":{{"from":1,"steal":false}}}}"#).unwrap();
        writeln!(ours, r#"{{"write":"{{\"type\":\"control_request\"}}"}}"#).unwrap();
        writeln!(ours, r#"{{"note":{{"type":"plan"}}}}"#).unwrap();
        ours.shutdown(std::net::Shutdown::Write).unwrap();
        serving.join().unwrap();
        // Its acknowledgement says what it is.
        let acknowledged: Json = BufReader::new(ours).lines().map_while(Result::ok)
            .filter_map(|l| serde_json::from_str::<Json>(&l).ok()).find(|r| r.get("attached").is_some()).unwrap();
        assert_eq!(acknowledged["notesOnly"], true);
        let journaled = std::fs::read_to_string(&path).unwrap();
        let records: Vec<Json> = journaled.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(records.len(), 1, "{journaled}");
        assert_eq!(records[0]["note"]["type"], "plan");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_notes_only_host_never_makes_a_removed_session_again() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-gone-session-{}", std::process::id()));
        let socket = std::env::temp_dir().join(format!("romacs-agent-host-gone-{}.sock", std::process::id()));
        let config = parse(["--dir", dir.to_str().unwrap(), "--socket", socket.to_str().unwrap(), "--notes-only"]
            .map(String::from).to_vec()).unwrap();
        assert!(run(config).unwrap_err().contains("the session is gone"));
        assert!(!dir.exists());
    }

    #[test]
    fn a_stolen_attachment_is_closed() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-steal-{}", std::process::id()));
        create_private(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, "").unwrap();
        let journal = OpenOptions::new().append(true).open(&path).unwrap();
        let shared = Arc::new(Mutex::new(Shared { seq: 0, store: Store::journal(journal, path.clone()).unwrap(), attached: None, next_attachment: 0,
                                                  stdin: Arc::new(Mutex::new(None)), closed: false }));
        let attach_request: Json = serde_json::from_str(r#"{"attach":{"from":1,"steal":false}}"#).unwrap();
        let steal_request: Json = serde_json::from_str(r#"{"attach":{"from":1,"steal":true}}"#).unwrap();
        let (first, ours) = UnixStream::pair().unwrap();
        assert_eq!(attach(&shared, first, &attach_request, true), Some(1));
        let (second, theirs) = UnixStream::pair().unwrap();
        assert_eq!(attach(&shared, second, &steal_request, true), Some(2));
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
        let shared = Arc::new(Mutex::new(Shared { seq: 0, store: Store::journal(journal, path.clone()).unwrap(), attached: None, next_attachment: 0,
                                                  stdin: Arc::new(Mutex::new(None)), closed: false }));
        let request: Json = serde_json::from_str(r#"{"attach":{"from":1,"steal":false}}"#).unwrap();
        let (server, ours) = UnixStream::pair().unwrap();
        drop(ours);
        assert_eq!(attach(&shared, server, &request, true), None);
        assert!(shared.lock().unwrap().attached.is_none());
        let (next, theirs) = UnixStream::pair().unwrap();
        assert!(attach(&shared, next, &request, true).is_some());
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
        let with: Json = serde_json::from_str(&input_record(3, "see", Some(&json!(["/a.png"])), None)).unwrap();
        assert_eq!(with["in"], json!({"prompt": "see", "images": ["/a.png"]}));
        let without: Json = serde_json::from_str(&input_record(3, "see", None, None)).unwrap();
        assert_eq!(without["in"], json!({"prompt": "see", "images": []}));
        // The request it answers, kept beside them: what finds it again.
        let tagged: Json = serde_json::from_str(&input_record(3, "see", None, Some(&json!("c-7")))).unwrap();
        assert_eq!(tagged["in"], json!({"prompt": "see", "images": [], "romacsRequest": "c-7"}));
    }

    #[test]
    fn a_prompt_shaped_like_a_record_stays_its_words() {
        let text = r#"{"text":"quoted","images":["/a.png"]}"#;
        let kept: Json = serde_json::from_str(&input_record(3, text, None, None)).unwrap();
        assert_eq!(kept["in"]["prompt"], json!(text));
        assert_eq!(kept["in"]["images"], json!([]));
    }

    #[test]
    fn a_line_that_reached_the_harness_is_journaled_written() {
        // Written whole: its `in` record, then `written` naming it.  Its
        // write failed (no stdin): its `in` record alone.
        for (harness, expect_written) in [(true, true), (false, false)] {
            let dir = std::env::temp_dir().join(format!("romacs-agent-host-written-{}-{harness}", std::process::id()));
            create_private(&dir).unwrap();
            let path = dir.join("journal.jsonl");
            std::fs::write(&path, "").unwrap();
            let journal = OpenOptions::new().append(true).open(&path).unwrap();
            let mut child = harness.then(|| Command::new("cat").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap());
            let stdin = child.as_mut().and_then(|child| child.stdin.take());
            let shared = Arc::new(Mutex::new(Shared { seq: 0, store: Store::journal(journal, path.clone()).unwrap(), attached: None,
                                                      next_attachment: 0, stdin: Arc::new(Mutex::new(stdin)), closed: false }));
            let (server, mut ours) = UnixStream::pair().unwrap();
            let serving = std::thread::spawn(move || serve(shared, server, true));
            // A record missing fails the test, never hangs it.
            ours.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            writeln!(ours, r#"{{"attach":{{"from":1,"steal":false}}}}"#).unwrap();
            let mut lines = BufReader::new(ours.try_clone().unwrap()).lines();
            assert_eq!(serde_json::from_str::<Json>(&lines.next().unwrap().unwrap()).unwrap()["attached"], 1);
            writeln!(ours, r#"{{"write":"{{\"type\":\"user\"}}"}}"#).unwrap();
            let input: Json = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            assert_eq!((input["seq"].as_u64(), &input["in"]), (Some(1), &json!({"type": "user"})));
            if expect_written {
                let written: Json = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                assert_eq!((written["seq"].as_u64(), written["written"].as_u64()), (Some(2), Some(1)));
            }
            ours.shutdown(std::net::Shutdown::Both).unwrap();
            serving.join().unwrap();
            assert_eq!(last_seq(&path), if expect_written { 2 } else { 1 });
            if let Some(mut child) = child { let _ = child.kill(); let _ = child.wait(); }
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_dir(&dir);
        }
    }

    #[test]
    fn nothing_is_journaled_after_the_exit() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-closed-{}", std::process::id()));
        create_private(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, "").unwrap();
        let journal = OpenOptions::new().append(true).open(&path).unwrap();
        let mut shared = Shared { seq: 1, store: Store::journal(journal, path.clone()).unwrap(), attached: None, next_attachment: 0,
                                  stdin: Arc::new(Mutex::new(None)), closed: false };
        shared.exit(0);
        // A late writer's acknowledgement: not journaled after the exit.
        shared.written(1);
        assert_eq!(last_seq(&path), 2);
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_note_is_journaled_for_the_attached_server_alone() {
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-note-{}", std::process::id()));
        create_private(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, "").unwrap();
        let journal = OpenOptions::new().append(true).open(&path).unwrap();
        let (server, ours) = UnixStream::pair().unwrap();
        let mut shared = Shared { seq: 4, store: Store::journal(journal, path.clone()).unwrap(), attached: Some((1, server)), next_attachment: 1,
                                  stdin: Arc::new(Mutex::new(None)), closed: false };
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
                                  next_attachment: 0, stdin: Arc::new(Mutex::new(None)), closed: false };
        shared.output(&[r#"{"type":"assistant","message":{"id":"m1"}}"#.to_owned()]);
        shared.output(&[r#"{"type":"system","subtype":"thinking_tokens","tokens":5}"#.to_owned()]);
        shared.output(&[r#"{"type":"result","result":"done"}"#.to_owned()]);
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
    fn a_store_another_holds_is_opened_once_it_is_let_go() {
        let dir = std::env::temp_dir().join(format!("romacs-host-held-{}", std::process::id()));
        let session = dir.join("aaaaaaaa-0000-4000-8000-000000000003");
        std::fs::create_dir_all(&session).unwrap();
        let database = dir.join("sessions.sqlite");
        // Another connection makes the store and holds its lock past a
        // connection's wait, as the writer making it does.
        let (held, holding) = std::sync::mpsc::channel();
        let holder = { let database = database.clone(); std::thread::spawn(move || {
            let db = rusqlite::Connection::open(&database).unwrap();
            db.pragma_update(None, "journal_mode", "WAL").unwrap();
            db.execute_batch("begin immediate; create table first (x);").unwrap();
            held.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(700));
            db.execute_batch("commit").unwrap();
        }) };
        holding.recv().unwrap();
        let started = std::time::Instant::now();
        let store = Store::open(&session, Some(&database));
        assert!(store.is_ok(), "{:?}", store.err());
        assert!(started.elapsed() >= std::time::Duration::from_millis(400));
        holder.join().unwrap();
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
                                  stdin: Arc::new(Mutex::new(None)), closed: false };
        let started = std::time::Instant::now();
        let fixer = { let database = database.clone(); std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            rusqlite::Connection::open(&database).unwrap().execute_batch(EVENTS_SCHEMA).unwrap();
        }) };
        shared.output(&[r#"{"type":"assistant"}"#.to_owned()]);
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
    fn lines_output_together_are_kept_and_sent_in_order_as_one() {
        // In a journal and in SQLite: each its own record, numbered in turn,
        // kept in order, then sent in order; a counter dropped from SQLite's.
        let dir = std::env::temp_dir().join(format!("romacs-host-batch-{}", std::process::id()));
        let session = dir.join("aaaaaaaa-0000-4000-8000-000000000003");
        std::fs::create_dir_all(&session).unwrap();
        let lines = [r#"{"type":"assistant","n":1}"#, r#"{"type":"system","subtype":"thinking_tokens","tokens":5}"#,
                     r#"{"type":"assistant","n":2}"#, r#"{"type":"result","result":"done"}"#].map(String::from);
        for (database, kept) in [(None, 4), (Some(dir.join("sessions.sqlite")), 3)] {
            let store = Store::open(&session, database.as_deref()).unwrap();
            let (server, client) = UnixStream::pair().unwrap();
            let mut shared = Shared { seq: 0, store, attached: Some((1, server)), next_attachment: 1,
                                      stdin: Arc::new(Mutex::new(None)), closed: false };
            shared.output(&lines);
            assert_eq!(shared.store.last_seq(), kept);
            let mut replayed = Vec::new();
            shared.store.replay(1, |line| { replayed.push(line.to_owned()); true });
            client.set_read_timeout(Some(std::time::Duration::from_secs(1))).unwrap();
            let sent: Vec<String> = BufReader::new(client).lines().take(kept as usize).map(Result::unwrap).collect();
            assert_eq!(sent, replayed);
            let seqs: Vec<u64> = sent.iter().map(|l| serde_json::from_str::<Json>(l).unwrap()["seq"].as_u64().unwrap()).collect();
            assert_eq!(seqs, (1..=kept).collect::<Vec<_>>());
            assert!(sent[0].contains(r#""n":1"#) && sent[kept as usize - 2].contains(r#""n":2"#));
        }
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
        let mut store = Store::Journal { file, path: path.clone(), cut: Some(10), len: 0 };
        store.keep(&[(2, r#"{"seq":2}"#.to_owned())], false).unwrap();
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
    fn writers_never_see_the_store_locked() {
        // Two sessions' hosts keep records at once while a third writer
        // (the editor's, a turn's rows) holds a transaction for a second:
        // each record waits its turn, none is refused as locked.
        let dir = std::env::temp_dir().join(format!("romacs-agent-host-locked-{}", std::process::id()));
        create_private(&dir).unwrap();
        let file = dir.join("store.sqlite");
        let sessions: Vec<PathBuf> = ["a", "b"].iter().map(|name| dir.join(name)).collect();
        for session in &sessions { create_private(session).unwrap(); }
        let stores: Vec<Store> = sessions.iter().map(|session| Store::open(session, Some(&file)).unwrap()).collect();
        // The lock held here before any writer starts, for a second.
        let mut held = rusqlite::Connection::open(&file).unwrap();
        let transaction = held.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        transaction.execute("insert into events (session, seq, t, kind, record) values ('held', 1, 0, 'note', '{}')", []).unwrap();
        // Released once both writers are on their way to it, plus a second:
        // their first records wait on the lock.
        let (ready, started) = std::sync::mpsc::channel();
        let writers: Vec<_> = stores.into_iter().map(|mut store| {
            let ready = ready.clone();
            std::thread::spawn(move || {
                ready.send(()).unwrap();
                (1..=50u64).map(|seq| store.keep(&[(seq, format!("{{\"seq\":{seq},\"note\":{{}}}}"))], false))
                    .filter(|kept| kept.is_err()).count()
            })
        }).collect();
        for _ in 0..2 { started.recv_timeout(std::time::Duration::from_secs(10)).unwrap(); }
        std::thread::sleep(std::time::Duration::from_secs(1));
        // Still held: no writer's record went in meanwhile.
        assert_eq!(transaction.query_row("select count(*) from events where session in ('a', 'b')", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
        transaction.commit().unwrap();
        let refused: usize = writers.into_iter().map(|writer| writer.join().unwrap()).sum();
        assert_eq!(refused, 0);
        let kept: i64 = rusqlite::Connection::open(&file).unwrap()
            .query_row("select count(*) from events where session in ('a', 'b')", [], |row| row.get(0)).unwrap();
        assert_eq!(kept, 100);
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
