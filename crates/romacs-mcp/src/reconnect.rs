//! `romacs-mcp --socket PATH --reconnect --session ID [--reply-timeout S]`:
//! the relay an agent session's harness runs as its Romacs MCP server
//! (lisp/romacs/romacs-agent.el). Unlike the plain relay it reads the lines it
//! copies, so the harness's MCP session outlives the Romacs server it
//! reaches:
//!
//! - it says which session it serves, and its own incarnation, first
//!   (`romacs/hello`), and keeps the harness's `initialize' and
//!   `notifications/initialized';
//! - when the server goes away, each call it had sent and not seen answered
//!   is answered `outcome unknown' (it may have run: the harness asks
//!   `romacs_call_outcome'), and a call made while no server listens is
//!   answered `not executed' (safe to repeat);
//! - a call unanswered past the reply timeout (the editor busy) is
//!   withdrawn with `romacs/cancel': answered `not executed' when the
//!   server says it was still queued, else `outcome unknown'; a reply that
//!   comes after is dropped, as the call's outcome is the server's record;
//! - it reconnects, says hello and replays the handshake, and copies lines
//!   again; a reply is forwarded only to a call still waiting for it;
//! - a server that ends each connection at once (a Romacs its session is
//!   not attached to closes its relays) is asked again less and less often,
//!   up to `REFUSED_MAX` apart, and at once when the socket path is
//!   replaced (the route relinked to the process attaching the session):
//!   an orphaned harness costs the editor nothing while it waits;
//! - a writer thread owns the server's socket behind a bounded queue, so a
//!   busy server that stops reading never stops the relay reading the
//!   harness: past the queue a call is refused `not executed';
//! - it answers `romacs_debug_inspect' itself, from the session's inspect
//!   endpoint, and answers `initialize' and `tools/list' itself when the
//!   server is silent, so a harness that starts against a frozen session
//!   can still ask where it is stuck, and says the list changed once the
//!   server answers. The tool's answer is diagnostics: a timed-out call
//!   keeps its own outcome.
//!
//! The harness never sees its MCP server end.
use serde_json::{Value as Json, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Config {
    pub socket: PathBuf,
    pub session: String,
    pub reply_timeout: Duration,
}

/// How long a withdrawal waits for the server's answer.
const CANCEL_GRACE: Duration = Duration::from_secs(10);

/// A call sent upstream and not answered.
/// Lines waiting for the server; past it a call is refused.
const QUEUE: usize = 64;
/// How long `initialize' and `tools/list' wait for the server.
const DISCOVERY: Duration = Duration::from_secs(2);
/// The tool the relay answers itself.
const INSPECT_TOOL: &str = "romacs_debug_inspect";
/// Inspections at once; past them one is refused.
const INSPECTIONS: usize = 4;
/// How long an inspection may take, its connect included.
const INSPECT_LIMIT: Duration = Duration::from_secs(10);
/// The keeper's tick: how often it watches calls and the route.
const TICK: Duration = Duration::from_millis(250);
/// A connection the server ends sooner than this was refused.
const REFUSED_WITHIN: Duration = Duration::from_secs(1);
/// The longest wait between attempts at a server that keeps refusing.
const REFUSED_MAX: Duration = Duration::from_secs(30);

struct Call {
    id: Json,
    method: String,
    sent: Instant,
    /// The withdrawal sent for it, and when.
    cancel: Option<(String, Instant)>,
    /// The writer has taken it: the server may have it. Until then a
    /// withdrawn call is simply not sent.
    written: bool,
    /// What the writer knows this admission by, as an id can be reused.
    token: u64,
    /// `initialize''s requested protocol version.
    version: Option<Json>,
}

impl Call {
    /// A call the relay can answer itself when the server is silent.
    fn discovery(&self) -> bool { matches!(self.method.as_str(), "initialize" | "tools/list") }
}

/// A line for the server. With a call, the writer sends it only while that
/// admission is still waiting.
struct Line {
    text: String,
    call: Option<(String, u64)>,
}

/// The server connection: the queue to its writer, its number, and a handle
/// that shuts both halves down.
struct Upstream {
    number: u64,
    since: Instant,
    queue: SyncSender<Line>,
    control: UnixStream,
}

#[derive(Default)]
struct State {
    /// The server connection while there is one.
    upstream: Option<Upstream>,
    connections: u64,
    tokens: u64,
    /// By the call's id as JSON text.
    inflight: HashMap<String, Call>,
    /// A withdrawal's id to the call it withdraws.
    cancels: HashMap<String, String>,
    initialize: Option<Json>,
    initialized: Option<String>,
    /// The harness holds the relay's own tool list, not the server's.
    provisional: bool,
    /// The relay's `tools/list' probe, which ends that.
    probe: Option<String>,
    /// The harness's own `tools/list' params, which the probe carries: a
    /// stateless client's protocol metadata travels in each request, and
    /// the server refuses a list asked without it.
    list_params: Option<Json>,
    probes: u64,
    /// Connections in a row the server ended within `REFUSED_WITHIN`.
    refusals: u32,
}

/// How long to wait before the next connection attempt, after FAILURES
/// connects in a row that found no server and REFUSALS connections in a
/// row it ended at once: no server, 100 ms doubling to 2 s; a refusing
/// one, 500 ms doubling to `REFUSED_MAX`.
fn retry_after(failures: u32, refusals: u32) -> Duration {
    let doubled = |base: u64, n: u32, max: Duration| Duration::from_millis(base.saturating_mul(1 << n.min(16))).min(max);
    if refusals > 0 { doubled(250, refusals, REFUSED_MAX) } else { doubled(50, failures.max(1), Duration::from_secs(2)) }
}

/// What the socket path is now: a link's or socket's inode and change
/// time. Relinking a route (rename(2)) or a new listener makes another.
fn path_identity(path: &std::path::Path) -> Option<(u64, i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).ok().map(|m| (m.ino(), m.ctime(), m.ctime_nsec()))
}

fn key(id: &Json) -> String { id.to_string() }

/// The answer to a call that cannot be relayed: a tool call gets a tool
/// error the model reads, anything else a JSON-RPC error.
fn refusal(id: &Json, method: &str, text: &str) -> String {
    if method == "tools/call" {
        json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": text}], "isError": true}})
    } else {
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": text}})
    }.to_string()
}

fn tool_answer(id: &Json, text: &str, failed: bool) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": text}], "isError": failed}}).to_string()
}

/// The tool's description, as `tools/list' gives it.
fn inspect_tool() -> Json {
    json!({"name": INSPECT_TOOL,
           "description": "Read the session's Lisp call stack, even while its evaluator is too busy to answer \
                           any other tool. Diagnostics only: it says nothing about a call that timed out.",
           "inputSchema": {"type": "object", "properties": {}},
           "annotations": {"readOnlyHint": true}})
}

/// The server's `tools/list' reply LINE, with the relay's own tool.
fn with_inspect_tool(line: &str) -> String {
    let Ok(mut reply) = serde_json::from_str::<Json>(line) else { return line.to_owned() };
    let Some(tools) = reply.pointer_mut("/result/tools").and_then(Json::as_array_mut) else { return line.to_owned() };
    tools.push(inspect_tool());
    reply.to_string()
}

/// `tools/list' answered with the server silent: the relay's tool alone.
fn local_list(id: &Json) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [inspect_tool()]}}).to_string()
}

/// `initialize' answered with the server silent.
fn local_initialize(id: &Json, version: Option<&Json>) -> String {
    let version = version.cloned().unwrap_or(json!("2025-06-18"));
    json!({"jsonrpc": "2.0", "id": id,
           "result": {"protocolVersion": version, "capabilities": {"tools": {"listChanged": true}},
                      "serverInfo": {"name": "Romacs", "version": "1"}}}).to_string()
}

fn list_changed() -> String {
    json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}).to_string()
}

fn unknown(id: &Json, why: &str) -> String {
    format!("outcome unknown: {why} (request id {id}). Call romacs_identity, then romacs_call_outcome with this id, \
             and read the state again before repeating it.")
}

const NOT_EXECUTED: &str = "not executed: Romacs is not running or is restarting; nothing was done. Retry shortly.";
const WITHDRAWN: &str = "not executed: Romacs was busy past the reply timeout and the call was withdrawn before it \
                         ran; nothing was done. Retry when Romacs is idle.";

struct Relay {
    state: Mutex<State>,
    stdout: Mutex<std::io::Stdout>,
    config: Config,
    incarnation: String,
    inspections: AtomicUsize,
}

impl Relay {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> { self.state.lock().unwrap_or_else(|e| e.into_inner()) }

    fn to_harness(&self, line: &str) {
        let mut out = self.stdout.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    /// Queue a line for the server, without waiting.
    fn send(state: &State, text: String, call: Option<(String, u64)>) -> bool {
        state.upstream.as_ref().is_some_and(|upstream| upstream.queue.try_send(Line { text, call }).is_ok())
    }

    /// The harness has the relay's list: ask the server for its own, to
    /// say when the list changed.
    fn probe(state: &mut State) {
        if !state.provisional || state.probe.is_some() { return; }
        state.probes += 1;
        let id = format!("romacs-relay-probe-{}", state.probes);
        let mut request = json!({"jsonrpc": "2.0", "id": id, "method": "tools/list"});
        if let Some(params) = &state.list_params { request["params"] = params.clone(); }
        let request = request.to_string();
        if Self::send(state, request, None) { state.probe = Some(key(&json!(id))); }
    }

    /// The server is silent or gone: the relay's answer to a discovery call.
    fn settled(call: &Call) -> String {
        if call.method == "initialize" { local_initialize(&call.id, call.version.as_ref()) } else { local_list(&call.id) }
    }

    /// The harness has been given the relay's list. Only now, so that the
    /// change a probe reports always comes after the list it replaces.
    fn listed_locally(&self) {
        let mut state = self.lock();
        state.provisional = true;
        Self::probe(&mut state);
    }

    /// Answer the inspect tool, within its bounds, off the harness's thread:
    /// it waits on the session's endpoint, never on the server's connection.
    fn inspect(self: &Arc<Self>, id: Json) {
        if self.inspections.fetch_add(1, Ordering::SeqCst) >= INSPECTIONS {
            self.inspections.fetch_sub(1, Ordering::SeqCst);
            return self.to_harness(&tool_answer(&id, "too many inspections at once; try again", true));
        }
        let (done, answer) = std::sync::mpsc::channel();
        let relay = self.clone();
        std::thread::spawn(move || {
            let result = crate::inspect_answer(&relay.config.socket);
            relay.inspections.fetch_sub(1, Ordering::SeqCst);
            let _ = done.send(result);
        });
        let relay = self.clone();
        std::thread::spawn(move || {
            let (text, failed) = match answer.recv_timeout(INSPECT_LIMIT) {
                Ok(Ok(answer)) => (String::from_utf8_lossy(&answer).into_owned(), false),
                Ok(Err(error)) => (format!("no inspect endpoint: {error}"), true),
                Err(_) => ("the inspect endpoint did not answer in time".to_owned(), true),
            };
            relay.to_harness(&tool_answer(&id, &text, failed));
        });
    }

    /// One line from the harness.
    fn from_harness(self: &Arc<Self>, line: &str) {
        let message: Option<Json> = serde_json::from_str(line).ok();
        let id = message.as_ref().and_then(|m| m.get("id")).filter(|id| !id.is_null()).cloned();
        let method = message.as_ref().and_then(|m| m.get("method")).and_then(Json::as_str).unwrap_or("").to_owned();
        if method == "tools/call" && message.as_ref().and_then(|m| m.pointer("/params/name")).and_then(Json::as_str) == Some(INSPECT_TOOL) {
            if let Some(id) = id { self.inspect(id); }
            return;
        }
        let mut state = self.lock();
        if method == "initialize" { state.initialize = message.clone(); }
        if method == "notifications/initialized" { state.initialized = Some(line.to_owned()); }
        if method == "tools/list" { state.list_params = message.as_ref().and_then(|m| m.get("params")).cloned(); }
        let Some(id) = id else {
            Self::send(&state, line.to_owned(), None);
            return;
        };
        state.tokens += 1;
        let token = state.tokens;
        let version = message.as_ref().and_then(|m| m.pointer("/params/protocolVersion")).cloned();
        let call = Call { id, method, sent: Instant::now(), cancel: None, written: false, token, version };
        // `initialize' reaches the server even when answered here first:
        // the calls after it need the handshake.
        let tracked = (call.method != "initialize").then(|| (key(&call.id), token));
        if Self::send(&state, line.to_owned(), tracked) {
            // Under the lock the writer needs, so it finds the call.
            state.inflight.insert(key(&call.id), call);
            return;
        }
        drop(state);
        if call.discovery() {
            self.to_harness(&Self::settled(&call));
            if call.method == "tools/list" { self.listed_locally(); }
        } else {
            self.to_harness(&refusal(&call.id, &call.method, NOT_EXECUTED));
        }
    }

    /// One line from the server.
    fn from_server(&self, line: &str) {
        let message: Option<Json> = serde_json::from_str(line).ok();
        let id = message.as_ref().and_then(|m| m.get("id")).filter(|id| !id.is_null()).cloned();
        // Notifications, and the server's own requests, are the harness's.
        let (Some(id), None) = (id, message.as_ref().and_then(|m| m.get("method"))) else {
            return self.to_harness(line);
        };
        let mut state = self.lock();
        if state.probe.as_deref() == Some(key(&id).as_str()) {
            state.probe = None;
            if message.as_ref().is_some_and(|m| m.get("result").is_some()) && std::mem::take(&mut state.provisional) {
                drop(state);
                self.to_harness(&list_changed());
            }
            return;
        }
        if let Some(call_key) = state.cancels.remove(&key(&id)) {
            // A withdrawal's answer: the call it names is answered now.
            let withdrawn = message.as_ref().and_then(|m| m.pointer("/result/cancelled")) == Some(&Json::Bool(true));
            if let Some(call) = state.inflight.remove(&call_key) {
                drop(state);
                let text = if withdrawn { WITHDRAWN.to_owned() } else { unknown(&call.id, "it ran past the reply timeout") };
                self.to_harness(&refusal(&call.id, &call.method, &text));
            }
            return;
        }
        // No call waits for it: the relay's own, or one already answered.
        let Some(call) = state.inflight.remove(&key(&id)) else { return };
        drop(state);
        if call.method == "tools/list" {
            self.to_harness(&with_inspect_tool(line));
        } else {
            self.to_harness(line);
        }
    }

    /// Withdraw calls past the reply timeout; answer those whose withdrawal
    /// went unanswered.
    fn watch(&self) {
        let now = Instant::now();
        let mut answers = Vec::new();
        let mut state = self.lock();
        // A silent server: discovery is answered here. The call goes, so its
        // reply finds no one, and a queued `tools/list' is not sent.
        let silent: Vec<String> = state.inflight.iter()
            .filter(|(_, call)| call.discovery() && now.duration_since(call.sent) >= DISCOVERY)
            .map(|(k, _)| k.clone()).collect();
        let mut listed = false;
        for call_key in silent {
            if let Some(call) = state.inflight.remove(&call_key) {
                listed |= call.method == "tools/list";
                answers.push(Self::settled(&call));
            }
        }
        // A full queue refused the last probe: try again while it matters.
        Self::probe(&mut state);
        let late: Vec<String> = state.inflight.iter()
            .filter(|(_, call)| !call.discovery() && call.cancel.is_none() && now.duration_since(call.sent) >= self.config.reply_timeout)
            .map(|(k, _)| k.clone()).collect();
        for call_key in late {
            // Still queued: the writer finds it gone and never sends it.
            if !state.inflight[&call_key].written {
                if let Some(call) = state.inflight.remove(&call_key) { answers.push(refusal(&call.id, &call.method, WITHDRAWN)); }
                continue;
            }
            let cancel_id = format!("romacs-relay-cancel-{call_key}");
            let request = json!({"jsonrpc": "2.0", "id": cancel_id, "method": "romacs/cancel",
                                 "params": {"id": state.inflight[&call_key].id}}).to_string();
            // A full queue is a server not reading: the grace ends it too.
            Self::send(&state, request, None);
            state.cancels.insert(key(&json!(cancel_id)), call_key.clone());
            if let Some(call) = state.inflight.get_mut(&call_key) { call.cancel = Some((cancel_id, now)); }
        }
        let unanswered: Vec<String> = state.inflight.iter()
            .filter(|(_, call)| call.cancel.as_ref().is_some_and(|(_, at)| now.duration_since(*at) >= CANCEL_GRACE))
            .map(|(k, _)| k.clone()).collect();
        for call_key in unanswered {
            if let Some(call) = state.inflight.remove(&call_key) {
                if let Some((cancel_id, _)) = &call.cancel { state.cancels.remove(&key(&json!(cancel_id))); }
                answers.push(refusal(&call.id, &call.method, &unknown(&call.id, "Romacs stayed busy past the reply timeout")));
            }
        }
        drop(state);
        for answer in answers { self.to_harness(&answer); }
        if listed { self.listed_locally(); }
    }

    /// Connection NUMBER ended: answer what it left unanswered, and release
    /// its writer, which may be stuck on a peer that stopped reading.
    fn lost(&self, number: u64) {
        let mut answers = Vec::new();
        let mut listed = false;
        {
            let mut state = self.lock();
            if state.upstream.as_ref().map(|upstream| upstream.number) != Some(number) { return; }
            if let Some(upstream) = state.upstream.take() {
                let _ = upstream.control.shutdown(std::net::Shutdown::Both);
                state.refusals = if upstream.since.elapsed() < REFUSED_WITHIN { state.refusals.saturating_add(1) } else { 0 };
            }
            state.cancels.clear();
            state.probe = None;
            let left: Vec<Call> = state.inflight.drain().map(|(_, call)| call).collect();
            for call in left {
                listed |= call.method == "tools/list";
                answers.push(if call.discovery() {
                    Self::settled(&call)
                } else if call.written {
                    refusal(&call.id, &call.method, &unknown(&call.id, "Romacs stopped while this call ran"))
                } else {
                    refusal(&call.id, &call.method, NOT_EXECUTED)
                });
            }
        }
        for answer in answers { self.to_harness(&answer); }
        if listed { self.listed_locally(); }
    }

    /// The server's socket: lines in order, the blocking writes here, with
    /// no lock held. A call no longer waiting is skipped.
    fn write_lines(&self, lines: Receiver<Line>, mut stream: UnixStream) {
        for line in lines {
            if let Some((call, token)) = &line.call {
                match self.lock().inflight.get_mut(call).filter(|call| call.token == *token) {
                    Some(call) => call.written = true,
                    None => continue,
                }
            }
            if stream.write_all(format!("{}\n", line.text).as_bytes()).is_err() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                return;
            }
        }
    }

    /// Connect, say hello and replay the handshake; the connection's
    /// reader runs until it ends.
    fn connect(self: &Arc<Self>) -> bool {
        let Ok(stream) = UnixStream::connect(&self.config.socket) else { return false };
        let Ok(reading) = stream.try_clone() else { return false };
        let Ok(control) = stream.try_clone() else { return false };
        let mut state = self.lock();
        state.connections += 1;
        let number = state.connections;
        let hello_id = json!(format!("romacs-relay-hello-{number}"));
        let mut lines = vec![json!({"jsonrpc": "2.0", "id": hello_id, "method": "romacs/hello",
                                    "params": {"session": self.config.session, "relay": self.incarnation}}).to_string()];
        // After the first, the handshake is replayed under an id of its own.
        if let Some(mut initialize) = state.initialize.clone() {
            initialize["id"] = json!(format!("romacs-relay-initialize-{number}"));
            lines.push(initialize.to_string());
            lines.extend(state.initialized.clone());
        }
        let (queue, waiting) = sync_channel(QUEUE);
        for text in lines {
            if queue.try_send(Line { text, call: None }).is_err() { return false; }
        }
        state.upstream = Some(Upstream { number, since: Instant::now(), queue, control });
        Self::probe(&mut state);
        drop(state);
        let relay = self.clone();
        std::thread::spawn(move || relay.write_lines(waiting, stream));
        let relay = self.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(reading).lines().map_while(Result::ok) { relay.from_server(&line); }
            relay.lost(number);
        });
        true
    }

    fn connected(&self) -> bool { self.lock().upstream.is_some() }
}

pub fn main(config: Config) -> ! {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let incarnation = format!("{}-{now}", std::process::id());
    let relay = Arc::new(Relay { state: Mutex::default(), stdout: Mutex::new(std::io::stdout()), config, incarnation, inspections: AtomicUsize::new(0) });
    let keeper = relay.clone();
    // Reconnect for as long as the harness runs (`retry_after`), at once
    // when the socket path is replaced, and watch the calls' reply timeout.
    std::thread::spawn(move || {
        let (mut failures, mut attempted, mut path) = (0, None::<Instant>, path_identity(&keeper.config.socket));
        loop {
            if !keeper.connected() {
                let now = path_identity(&keeper.config.socket);
                let wait = retry_after(failures, keeper.lock().refusals);
                if attempted.is_none_or(|at| at.elapsed() >= wait) || now != path {
                    (attempted, path) = (Some(Instant::now()), now);
                    failures = if keeper.connect() { 0 } else { failures + 1 };
                }
            }
            let connected = keeper.connected();
            if connected { keeper.watch(); }
            std::thread::sleep(if connected { TICK } else { TICK.min(retry_after(failures, keeper.lock().refusals)) });
        }
    });
    // The first connection, before any line: a harness starts with Romacs up.
    for _ in 0..40 {
        if relay.connected() { break; }
        std::thread::sleep(Duration::from_millis(50));
    }
    for line in std::io::stdin().lock().lines().map_while(Result::ok) { relay.from_harness(&line); }
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusing_server_is_asked_less_often_than_an_absent_one() {
        let ms = |n: u64| Duration::from_millis(n);
        assert_eq!([0, 1, 2, 5, 6].map(|failures| retry_after(failures, 0)), [ms(100), ms(100), ms(200), ms(1600), ms(2000)]);
        assert_eq!([1, 2, 3, 7, 40].map(|refusals| retry_after(9, refusals)), [ms(500), ms(1000), ms(2000), REFUSED_MAX, REFUSED_MAX]);
    }

    #[test]
    fn a_lost_tool_call_is_a_tool_error_and_other_calls_are_rpc_errors() {
        let tool: Json = serde_json::from_str(&refusal(&json!(7), "tools/call", &unknown(&json!(7), "gone"))).unwrap();
        assert_eq!(tool["result"]["isError"], true);
        assert!(tool["result"]["content"][0]["text"].as_str().unwrap().contains("romacs_call_outcome"));
        let list: Json = serde_json::from_str(&refusal(&json!("a"), "tools/list", NOT_EXECUTED)).unwrap();
        assert_eq!(list["error"]["code"], -32000);
        assert_eq!(list["id"], "a");
    }
}
