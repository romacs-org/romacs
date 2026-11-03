//! `romacs-mcp --socket PATH --reconnect --session ID [--reply-timeout S]`:
//! the relay an agent session's harness runs as its Romacs MCP server
//! (rlisp/romacs-agent.el). Unlike the plain relay it reads the lines it
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
//! - it reconnects, says hello and replays the handshake, dropping the
//!   replies the harness already had, and copies lines again.
//!
//! The harness never sees its MCP server end.
use serde_json::{Value as Json, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
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
struct Call {
    id: Json,
    method: String,
    sent: Instant,
    /// The withdrawal sent for it, and when.
    cancel: Option<(String, Instant)>,
}

#[derive(Default)]
struct State {
    /// The server connection while there is one, and its number.
    upstream: Option<(u64, UnixStream)>,
    connections: u64,
    /// By the call's id as JSON text.
    inflight: HashMap<String, Call>,
    /// A withdrawal's id to the call it withdraws.
    cancels: HashMap<String, String>,
    /// Ids whose replies the harness already had or never asked for.
    swallow: HashSet<String>,
    initialize: Option<Json>,
    initialized: Option<String>,
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
}

impl Relay {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> { self.state.lock().unwrap_or_else(|e| e.into_inner()) }

    fn to_harness(&self, line: &str) {
        let mut out = self.stdout.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    /// One line from the harness.
    fn from_harness(&self, line: &str) {
        let message: Option<Json> = serde_json::from_str(line).ok();
        let id = message.as_ref().and_then(|m| m.get("id")).filter(|id| !id.is_null()).cloned();
        let method = message.as_ref().and_then(|m| m.get("method")).and_then(Json::as_str).unwrap_or("").to_owned();
        let mut state = self.lock();
        if method == "initialize" { state.initialize = message.clone(); }
        if method == "notifications/initialized" { state.initialized = Some(line.to_owned()); }
        let sent = match state.upstream.as_mut() {
            Some((_, stream)) => stream.write_all(format!("{line}\n").as_bytes()).is_ok(),
            None => false,
        };
        match id {
            Some(id) if sent => { state.inflight.insert(key(&id), Call { id, method, sent: Instant::now(), cancel: None }); }
            Some(id) => { drop(state); self.to_harness(&refusal(&id, &method, NOT_EXECUTED)); }
            None => {}
        }
    }

    /// One line from the server.
    fn from_server(&self, line: &str) {
        let message: Option<Json> = serde_json::from_str(line).ok();
        let Some(id) = message.as_ref().and_then(|m| m.get("id")).filter(|id| !id.is_null()).cloned() else {
            return self.to_harness(line);
        };
        let mut state = self.lock();
        if let Some(call_key) = state.cancels.remove(&key(&id)) {
            // A withdrawal's answer: the call it names is answered now.
            let withdrawn = message.as_ref().and_then(|m| m.pointer("/result/cancelled")) == Some(&Json::Bool(true));
            if let Some(call) = state.inflight.remove(&call_key) {
                state.swallow.insert(call_key);
                drop(state);
                let text = if withdrawn { WITHDRAWN.to_owned() } else { unknown(&call.id, "it ran past the reply timeout") };
                self.to_harness(&refusal(&call.id, &call.method, &text));
            }
            return;
        }
        if state.swallow.remove(&key(&id)) { return; }
        state.inflight.remove(&key(&id));
        drop(state);
        self.to_harness(line);
    }

    /// Withdraw calls past the reply timeout; answer those whose withdrawal
    /// went unanswered.
    fn watch(&self) {
        let now = Instant::now();
        let mut answers = Vec::new();
        let mut state = self.lock();
        let late: Vec<String> = state.inflight.iter()
            .filter(|(_, call)| call.cancel.is_none() && now.duration_since(call.sent) >= self.config.reply_timeout)
            .map(|(k, _)| k.clone()).collect();
        for call_key in late {
            let cancel_id = format!("romacs-relay-cancel-{call_key}");
            let request = json!({"jsonrpc": "2.0", "id": cancel_id, "method": "romacs/cancel",
                                 "params": {"id": state.inflight[&call_key].id}}).to_string();
            let sent = state.upstream.as_mut().is_some_and(|(_, s)| s.write_all(format!("{request}\n").as_bytes()).is_ok());
            if sent {
                state.cancels.insert(key(&json!(cancel_id)), call_key.clone());
                if let Some(call) = state.inflight.get_mut(&call_key) { call.cancel = Some((cancel_id, now)); }
            }
        }
        let unanswered: Vec<String> = state.inflight.iter()
            .filter(|(_, call)| call.cancel.as_ref().is_some_and(|(_, at)| now.duration_since(*at) >= CANCEL_GRACE))
            .map(|(k, _)| k.clone()).collect();
        for call_key in unanswered {
            if let Some(call) = state.inflight.remove(&call_key) {
                if let Some((cancel_id, _)) = &call.cancel { state.cancels.remove(&key(&json!(cancel_id))); }
                state.swallow.insert(call_key);
                answers.push(refusal(&call.id, &call.method, &unknown(&call.id, "Romacs stayed busy past the reply timeout")));
            }
        }
        drop(state);
        for answer in answers { self.to_harness(&answer); }
    }

    /// Connection NUMBER ended: answer what it left unanswered.
    fn lost(&self, number: u64) {
        let left: Vec<Call> = {
            let mut state = self.lock();
            if state.upstream.as_ref().map(|(n, _)| *n) != Some(number) { return; }
            state.upstream = None;
            state.cancels.clear();
            state.inflight.drain().map(|(_, call)| call).collect()
        };
        for call in left { self.to_harness(&refusal(&call.id, &call.method, &unknown(&call.id, "Romacs stopped while this call ran"))); }
    }

    /// Connect, say hello and replay the handshake; the connection's
    /// reader runs until it ends.
    fn connect(self: &Arc<Self>) -> bool {
        let Ok(stream) = UnixStream::connect(&self.config.socket) else { return false };
        let Ok(reading) = stream.try_clone() else { return false };
        let mut state = self.lock();
        state.connections += 1;
        let number = state.connections;
        let hello_id = json!(format!("romacs-relay-hello-{number}"));
        let mut lines = vec![json!({"jsonrpc": "2.0", "id": hello_id, "method": "romacs/hello",
                                    "params": {"session": self.config.session, "relay": self.incarnation}}).to_string()];
        state.swallow.insert(key(&hello_id));
        // After the first, the handshake is replayed under an id of its own.
        if let Some(mut initialize) = state.initialize.clone() {
            let id = json!(format!("romacs-relay-initialize-{number}"));
            initialize["id"] = id.clone();
            state.swallow.insert(key(&id));
            lines.push(initialize.to_string());
            lines.extend(state.initialized.clone());
        }
        let mut writer = stream;
        for line in lines {
            if writer.write_all(format!("{line}\n").as_bytes()).is_err() { return false; }
        }
        state.upstream = Some((number, writer));
        drop(state);
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
    let relay = Arc::new(Relay { state: Mutex::default(), stdout: Mutex::new(std::io::stdout()), config, incarnation });
    let keeper = relay.clone();
    // Reconnect for as long as the harness runs, backing off to 2 s, and
    // watch the calls' reply timeout.
    std::thread::spawn(move || {
        let mut wait = Duration::from_millis(50);
        loop {
            if keeper.connected() || keeper.connect() {
                wait = Duration::from_millis(250);
                keeper.watch();
            } else {
                wait = (wait * 2).min(Duration::from_secs(2));
            }
            std::thread::sleep(wait);
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
    fn a_lost_tool_call_is_a_tool_error_and_other_calls_are_rpc_errors() {
        let tool: Json = serde_json::from_str(&refusal(&json!(7), "tools/call", &unknown(&json!(7), "gone"))).unwrap();
        assert_eq!(tool["result"]["isError"], true);
        assert!(tool["result"]["content"][0]["text"].as_str().unwrap().contains("romacs_call_outcome"));
        let list: Json = serde_json::from_str(&refusal(&json!("a"), "tools/list", NOT_EXECUTED)).unwrap();
        assert_eq!(list["error"]["code"], -32000);
        assert_eq!(list["id"], "a");
    }
}
