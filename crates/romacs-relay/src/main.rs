//! `romacs-relay`: the agent session protocol (doc/agent-protocol.org) over a
//! tailnet. It holds no agent logic: it carries the protocol's messages, one
//! JSON line each, between a WebSocket and the agents service's Unix socket.
//!
//! `romacs-relay serve [--socket PATH] [--service PATH] [--page FILE]`
//! listens on a private Unix socket that `tailscale serve --https=443
//! unix:PATH` publishes on the tailnet. `GET /` serves the phone page;
//! `GET /agents` upgrades to a WebSocket. Before anything else, the request
//! must carry `Tailscale-User-Login` (Serve sets it for a user's device, and
//! strips what a caller sends) naming a login of the allowlist,
//! `$XDG_CONFIG_HOME/romacs/relay.json` (`{"allow": ["LOGIN"]}`), and an
//! `Origin`, when there is one, must be the page's own: Serve passes the
//! public authority as `X-Forwarded-Host`. The relay then connects to the
//! service's `service-relay.sock` and says first who called
//! (`agents/transport`); a caller's message naming that method is dropped.
//!
//! `romacs-relay connect URL --socket PATH` runs on another machine: each
//! connection to its local socket is carried over one WebSocket to URL
//! (`wss://HOST/agents`), so a Romacs there speaks to a Unix socket, as it
//! does at home. When either end goes, so does the other.
//!
//! Its sockets are in directories of the user's alone, under
//! `$XDG_RUNTIME_DIR`: only a stale socket of the user's is ever replaced.
//! Every read is bounded (a request head, a line, a WebSocket message), and
//! every write is given a deadline, so a stalled peer ends its connection
//! rather than pin the relay.
#![forbid(unsafe_code)]

use serde_json::{Value as Json, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

const USAGE: &str = "usage: romacs-relay serve [--socket PATH] [--service PATH] [--page FILE]\n       romacs-relay connect URL --socket PATH\n       romacs-relay check [--socket PATH]";

/// The most bytes of one protocol message (Framing), a line or a WebSocket
/// message alike.
const MESSAGE_LIMIT: usize = 1024 * 1024;

/// The most bytes of a request's head before the WebSocket upgrade.
const HEAD_LIMIT: usize = 64 * 1024;

/// How long a WebSocket read waits before the relay looks at the other end.
const POLL: Duration = Duration::from_millis(25);

/// How long a connection, a handshake or a write may take before its
/// connection is given up.
const DEADLINE: Duration = Duration::from_secs(10);

/// The lines read from the Unix end and not yet sent on: past them, its
/// reader waits, and so does the end it reads from.
const QUEUE: usize = 64;

/// The most bytes a WebSocket's stream gives one turn of the pump: past
/// them its reads would block, so a message whose fragments never end
/// still lets the pump carry the local lines and see their end.
const BUDGET: usize = 64 * 1024;

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(MESSAGE_LIMIT)).max_frame_size(Some(MESSAGE_LIMIT))
}

/// The runtime directory, as its components spell it: a trailing `/` would
/// have its symlink followed where it is checked.
fn runtime_dir() -> Result<PathBuf, String> {
    runtime_dir_of(std::env::var_os("XDG_RUNTIME_DIR"))
}

fn runtime_dir_of(value: Option<std::ffi::OsString>) -> Result<PathBuf, String> {
    let dir = value.map(PathBuf::from)
        .ok_or_else(|| "XDG_RUNTIME_DIR is not set: the relay's sockets live there".to_owned())?;
    if !dir.is_absolute() { return Err(format!("{}: XDG_RUNTIME_DIR is not absolute", dir.display())) }
    Ok(dir.components().collect())
}

fn default_socket() -> Result<PathBuf, String> {
    Ok(runtime_dir()?.join("romacs/relay/http.sock"))
}

fn default_service() -> Result<PathBuf, String> {
    Ok(runtime_dir()?.join("romacs/agents/service/service-relay.sock"))
}

fn allowlist_file() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_default()
        .join("romacs/relay.json")
}

/// The logins the allowlist file names; none when it is missing or malformed.
fn allowlist(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file).ok()
        .and_then(|text| serde_json::from_str::<Json>(&text).ok())
        .and_then(|config| config.get("allow").and_then(Json::as_array).cloned())
        .map(|logins| logins.iter().filter_map(|login| login.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

/// A stream whose first bytes are PREFIX, read already from INNER.
struct Prefixed<S> {
    prefix: Vec<u8>,
    at: usize,
    inner: S,
}

impl<S: Read> Read for Prefixed<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.at < self.prefix.len() {
            let n = buf.len().min(self.prefix.len() - self.at);
            buf[..n].copy_from_slice(&self.prefix[self.at..self.at + n]);
            self.at += n;
            return Ok(n);
        }
        self.inner.read(buf)
    }
}

impl<S: Write> Write for Prefixed<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> { self.inner.write(buf) }
    fn flush(&mut self) -> std::io::Result<()> { self.inner.flush() }
}

/// A stream whose reads give LEFT bytes within a turn of `POLL`, then would
/// block until it is refilled (`refill`); unbounded until first refilled,
/// for a handshake.
struct Budgeted<S> {
    inner: S,
    left: Arc<AtomicUsize>,
    /// The turn's end, in microseconds since START; `u64::MAX` for none.
    until: Arc<AtomicU64>,
    start: Instant,
}

impl<S> Budgeted<S> {
    fn new(inner: S) -> Self {
        Budgeted { inner, left: Arc::new(AtomicUsize::new(usize::MAX)), until: Arc::new(AtomicU64::new(u64::MAX)),
                   start: Instant::now() }
    }

    /// What gives the stream its next turn: its bytes and its time.
    fn refill(&self) -> impl Fn() + use<S> {
        let (left, until, start) = (self.left.clone(), self.until.clone(), self.start);
        move || {
            left.store(BUDGET, Ordering::Relaxed);
            let end = start.elapsed() + POLL;
            until.store(u64::try_from(end.as_micros()).unwrap_or(u64::MAX), Ordering::Relaxed);
        }
    }

    fn turn_over(&self) -> bool {
        let until = self.until.load(Ordering::Relaxed);
        until != u64::MAX && u64::try_from(self.start.elapsed().as_micros()).unwrap_or(u64::MAX) > until
    }
}

impl<S: Read> Read for Budgeted<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.left.load(Ordering::Relaxed);
        if left == 0 || self.turn_over() { return Err(std::io::ErrorKind::WouldBlock.into()) }
        let most = buf.len().min(left);
        let n = self.inner.read(&mut buf[..most])?;
        self.left.store(left.saturating_sub(n), Ordering::Relaxed);
        Ok(n)
    }
}

impl<S: Write> Write for Budgeted<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> { self.inner.write(buf) }
    fn flush(&mut self) -> std::io::Result<()> { self.inner.flush() }
}

/// A request's head: its method, path and headers (names lowercased), and
/// the bytes read, which the WebSocket handshake reads again.
struct Head {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    bytes: Vec<u8>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }
}

/// Read a request's head from STREAM, none longer than `HEAD_LIMIT`.
fn read_head(stream: &mut impl Read) -> Result<Head, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 { return Err("the request ended before its head did".into()); }
        bytes.extend_from_slice(&chunk[..n]);
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(&bytes) {
            Ok(httparse::Status::Complete(length)) if length <= HEAD_LIMIT => {
                let method = request.method.unwrap_or("").to_owned();
                let path = request.path.unwrap_or("/").to_owned();
                let headers = request.headers.iter()
                    .map(|h| (h.name.to_ascii_lowercase(), String::from_utf8_lossy(h.value).into_owned()))
                    .collect();
                return Ok(Head { method, path, headers, bytes });
            }
            Ok(httparse::Status::Partial) if bytes.len() < HEAD_LIMIT => {}
            Ok(_) => return Err("the request's head is too long".into()),
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// Why HEAD may not reach the agents, or none: its caller's login must be
/// in ALLOWED, and its Origin, when it has one, the page's own (the public
/// authority, which Serve passes as X-Forwarded-Host).
fn refusal(head: &Head, allowed: &[String]) -> Option<&'static str> {
    let Some(login) = head.header("tailscale-user-login") else { return Some("no tailnet login") };
    if !allowed.iter().any(|allowed| allowed == login) { return Some("this login may not") }
    if let Some(origin) = head.header("origin") {
        let authority = head.header("x-forwarded-host").or(head.header("host")).unwrap_or("");
        if origin != format!("https://{authority}") { return Some("another site's page") }
    }
    None
}

fn respond(stream: &mut impl Write, status: &str, kind: &str, body: &[u8]) {
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\
                           Cache-Control: no-store\r\n\r\n", body.len());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// Whether TEXT, a caller's message, may go on to the service: one line,
/// not naming the transport method (only the relay says who called).
fn carried(text: &str) -> bool {
    !text.contains(['\n', '\r'])
        && serde_json::from_str::<Json>(text).ok()
            .is_none_or(|message| message.get("method").and_then(Json::as_str) != Some("agents/transport"))
}

/// One line from READER, its newline left out: none at its end, past
/// `MESSAGE_LIMIT`, or unended (a line cut short is no message).
fn read_line(reader: &mut BufReader<UnixStream>) -> Option<String> {
    let mut line = Vec::new();
    let n = reader.by_ref().take(MESSAGE_LIMIT as u64 + 2).read_until(b'\n', &mut line).ok()?;
    if n == 0 || line.last() != Some(&b'\n') { return None }
    line.pop();
    if line.last() == Some(&b'\r') { line.pop(); }
    if line.len() > MESSAGE_LIMIT { return None }
    String::from_utf8(line).ok()
}

/// Carry messages between WS and LOCAL, a Unix stream of lines, until either
/// ends: each text message a line (those CARRY refuses not, one with a
/// newline ending it all), each line a text message.  WS's stream reads with
/// a timeout (`POLL`) and within a budget each turn (REFILL, `Budgeted`), so
/// its reads always give way to LOCAL's lines, which are carried in order to
/// their end; both write with a deadline.  At the end LOCAL is shut, which
/// ends its reader.
fn pump<S: Read + Write>(ws: &mut WebSocket<S>, local: UnixStream, carry: fn(&str) -> bool, refill: impl Fn()) {
    let (lines_tx, lines) = mpsc::sync_channel::<Option<String>>(QUEUE);
    let (Ok(reading), Ok(())) = (local.try_clone(), local.set_write_timeout(Some(DEADLINE))) else { return };
    let mut writing = local;
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reading);
        while let Some(line) = read_line(&mut reader) {
            if lines_tx.send(Some(line)).is_err() { return }
        }
        let _ = lines_tx.send(None);
    });
    'carrying: loop {
        refill();
        match ws.read() {
            Ok(Message::Text(text)) => {
                let text = text.as_str();
                if text.contains(['\n', '\r']) { break }
                if carry(text) && writeln!(writing, "{text}").is_err() { break }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
        loop {
            match lines.try_recv() {
                Ok(Some(line)) => { if ws.send(Message::text(line)).is_err() { break 'carrying } }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => break 'carrying,
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
    }
    // What the local end sent before it ended goes still, the WebSocket's
    // end or a write that failed whatever: a queue's worth at most, each
    // waited for a poll, so a service that goes on sending ends here too.
    refill();
    for _ in 0..QUEUE {
        match lines.recv_timeout(POLL) {
            Ok(Some(line)) => { if ws.send(Message::text(line)).is_err() { break } }
            _ => break,
        }
    }
    let _ = writing.shutdown(std::net::Shutdown::Both);
    drop(lines);
    let _ = ws.close(None);
    let _ = ws.flush();
}

/// Serve one connection from tailscale's Serve on STREAM.
fn serve_one(mut stream: UnixStream, service: &Path, page: Option<&Path>, allowed: &[String]) {
    if stream.set_read_timeout(Some(DEADLINE)).is_err() || stream.set_write_timeout(Some(DEADLINE)).is_err() { return }
    let head = match read_head(&mut stream) {
        Ok(head) => head,
        Err(_) => return respond(&mut stream, "400 Bad Request", "text/plain", b"bad request\n"),
    };
    if let Some(why) = refusal(&head, allowed) {
        return respond(&mut stream, "403 Forbidden", "text/plain", format!("{why}\n").as_bytes());
    }
    if head.method != "GET" {
        return respond(&mut stream, "405 Method Not Allowed", "text/plain", b"GET only\n");
    }
    match head.path.split('?').next().unwrap_or("") {
        "/" => match page.and_then(|page| std::fs::read(page).ok()) {
            Some(body) => respond(&mut stream, "200 OK", "text/html; charset=utf-8", &body),
            None => respond(&mut stream, "404 Not Found", "text/plain", b"no page here yet\n"),
        },
        "/agents" => {
            let login = head.header("tailscale-user-login").unwrap_or("").to_owned();
            let peer = head.header("tailscale-user-name").unwrap_or("").to_owned();
            let budgeted = Budgeted::new(stream);
            let refill = budgeted.refill();
            let prefixed = Prefixed { prefix: head.bytes, at: 0, inner: budgeted };
            let Ok(mut ws) = tungstenite::accept_with_config(prefixed, Some(ws_config())) else { return };
            let Some(mut agents) = within(DEADLINE, { let service = service.to_owned(); move || UnixStream::connect(service).ok() })
            else {
                let _ = ws.close(Some(tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Again,
                    reason: "the agents service is not running".into(),
                }));
                let _ = ws.flush();
                return;
            };
            let transport = json!({"jsonrpc": "2.0", "method": "agents/transport",
                                   "params": {"via": "tailnet", "login": login, "peer": peer}});
            if agents.set_write_timeout(Some(DEADLINE)).is_err() || writeln!(agents, "{transport}").is_err() { return }
            if ws.get_ref().inner.inner.set_read_timeout(Some(POLL)).is_err() { return }
            pump(&mut ws, agents, carried, refill);
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"not found\n"),
    }
}

/// This process's user: the owner of its /proc entry.
fn own_uid() -> Result<u32, String> {
    std::fs::metadata("/proc/self").map(|meta| meta.uid()).map_err(|e| format!("/proc/self: {e}"))
}

/// Whether META is a directory of UID's alone.
fn private(meta: &std::fs::Metadata, uid: u32) -> bool {
    meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0
}

/// Make SOCKET's directory one of the user's alone, under the runtime
/// directory, itself the user's alone: each directory below it made so when
/// missing, and one there already refused unless it is so, never changed.
fn private_dir(socket: &Path) -> Result<PathBuf, String> {
    let uid = own_uid()?;
    let runtime = runtime_dir()?;
    let meta = std::fs::symlink_metadata(&runtime).map_err(|e| format!("{}: {e}", runtime.display()))?;
    if !private(&meta, uid) { return Err(format!("{}: the runtime directory is not the user's alone", runtime.display())) }
    // No one else may replace it: each directory above it the user's or
    // root's, and writable by others only when sticky.
    for above in runtime.ancestors().skip(1) {
        let meta = std::fs::symlink_metadata(above).map_err(|e| format!("{}: {e}", above.display()))?;
        if !meta.is_dir() || (meta.uid() != uid && meta.uid() != 0) || (meta.mode() & 0o022 != 0 && meta.mode() & 0o1000 == 0) {
            return Err(format!("{}: others could replace the runtime directory there", above.display()));
        }
    }
    let below = socket.parent().and_then(|dir| dir.strip_prefix(&runtime).ok())
        .ok_or_else(|| format!("{}: a socket lives under {}", socket.display(), runtime.display()))?;
    let mut dir = runtime;
    for part in below.components() {
        dir.push(part);
        if std::fs::symlink_metadata(&dir).is_err() {
            std::fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let meta = std::fs::symlink_metadata(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if !private(&meta, uid) { return Err(format!("{}: not a directory of the user's alone", dir.display())) }
    }
    Ok(dir)
}

/// Listen on SOCKET, in a directory of the user's alone, holding its lock
/// for the life of the process: one relay a socket, and a stale socket of
/// the user's that refuses connections replaced only under it.
fn listen(socket: &Path) -> Result<(UnixListener, std::fs::File), String> {
    private_dir(socket)?;
    let lock_path = { let mut path = socket.as_os_str().to_owned(); path.push(".lock"); PathBuf::from(path) };
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    lock.try_lock().map_err(|_| format!("{}: another relay holds it", socket.display()))?;
    if let Ok(meta) = std::fs::symlink_metadata(socket) {
        if !meta.file_type().is_socket() || meta.uid() != own_uid()? {
            return Err(format!("{}: there, and no socket of the user's", socket.display()));
        }
        match UnixStream::connect(socket) {
            Ok(_) => return Err(format!("{}: something answers there", socket.display())),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                std::fs::remove_file(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
            }
            Err(error) => return Err(format!("{}: {error}", socket.display())),
        }
    }
    let listener = UnixListener::bind(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    Ok((listener, lock))
}

fn serve(socket: &Path, service: PathBuf, page: Option<PathBuf>) -> Result<(), String> {
    let (listener, _lock) = listen(socket)?;
    let file = allowlist_file();
    for stream in listener.incoming().map_while(Result::ok) {
        // Read anew for each caller: an edited allowlist counts at once.
        let allowed = allowlist(&file);
        let (service, page) = (service.clone(), page.clone());
        std::thread::spawn(move || serve_one(stream, &service, page.as_deref(), &allowed));
    }
    Ok(())
}

/// A TLS client configuration trusting the web's roots: tailscale's Serve
/// certificates are Let's Encrypt's.
fn tls() -> Arc<rustls::ClientConfig> {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring offers the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}

/// URL's host and port, a wss:// URL's.
fn endpoint(url: &str) -> Result<(String, u16), String> {
    let uri: tungstenite::http::Uri = url.parse().map_err(|e| format!("{url}: {e}"))?;
    if uri.scheme_str() != Some("wss") { return Err(format!("{url}: a wss:// URL")) }
    let host = uri.host().ok_or_else(|| format!("{url}: no host"))?.to_owned();
    Ok((host, uri.port_u16().unwrap_or(443)))
}

/// What F gives, run on a thread of its own, if it is done within LIMIT:
/// a step that cannot be interrupted (a name looked up, a connection made)
/// is waited for no longer; its thread ends when it does.
fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || { let _ = tx.send(f()); });
    rx.recv_timeout(limit).ok().flatten()
}

type ClientWs = WebSocket<tungstenite::stream::MaybeTlsStream<Budgeted<std::net::TcpStream>>>;

/// A WebSocket to URL at HOST:PORT, looked up, connected and shaken hands
/// within the deadline, or none; its stream polled once connected, and what
/// refills its budget.
fn open(url: String, (host, port): (String, u16), tls: Arc<rustls::ClientConfig>)
        -> Option<(ClientWs, impl Fn() + Send + use<>)> {
    use std::net::ToSocketAddrs;
    within(DEADLINE, move || {
        let address = (host.as_str(), port).to_socket_addrs().ok()?.next()?;
        let tcp = std::net::TcpStream::connect_timeout(&address, DEADLINE).ok()?;
        tcp.set_read_timeout(Some(DEADLINE)).ok()?;
        tcp.set_write_timeout(Some(DEADLINE)).ok()?;
        let polled = tcp.try_clone().ok()?;
        let budgeted = Budgeted::new(tcp);
        let refill = budgeted.refill();
        let (ws, _) = tungstenite::client_tls_with_config(url.as_str(), budgeted, Some(ws_config()),
                                                          Some(tungstenite::Connector::Rustls(tls))).ok()?;
        polled.set_read_timeout(Some(POLL)).ok()?;
        Some((ws, refill))
    })
}

/// Carry LOCAL, one connection to the local socket, over a WebSocket to URL.
fn connect_one(local: UnixStream, url: &str, endpoint: (String, u16), tls: Arc<rustls::ClientConfig>) {
    let Some((mut ws, refill)) = open(url.to_owned(), endpoint, tls) else { return };
    pump(&mut ws, local, |_| true, refill);
}

fn connect(url: String, socket: &Path) -> Result<(), String> {
    let endpoint = endpoint(&url)?;
    let (listener, _lock) = listen(socket)?;
    let tls = tls();
    for local in listener.incoming().map_while(Result::ok) {
        let (url, endpoint, tls) = (url.clone(), endpoint.clone(), tls.clone());
        std::thread::spawn(move || connect_one(local, &url, endpoint, tls));
    }
    Ok(())
}

/// Whether tailscale serves SOCKET: what `tailscale serve status` says.
fn check(socket: &Path) -> Result<(), String> {
    let status = std::process::Command::new("tailscale").args(["serve", "status"]).output()
        .map_err(|e| format!("tailscale: {e}"))?;
    let text = String::from_utf8_lossy(&status.stdout);
    let target = format!("unix:{}", socket.display());
    let allowed = allowlist(&allowlist_file());
    println!("socket: {}", socket.display());
    println!("allowlist: {} ({} login{})", allowlist_file().display(), allowed.len(), if allowed.len() == 1 { "" } else { "s" });
    if text.split_whitespace().any(|word| word == target) {
        println!("tailscale serves it:\n{text}");
        Ok(())
    } else {
        Err(format!("tailscale does not serve it: run\n  tailscale serve --bg --https=443 {target}"))
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let mut rest: Vec<String> = args.collect();
    let mut option = |name: &str| -> Option<String> {
        let at = rest.iter().position(|arg| arg == name)?;
        let value = rest.get(at + 1).cloned();
        rest.drain(at..(at + 2).min(rest.len()));
        value
    };
    let socket = option("--socket").map(PathBuf::from);
    let result = match command.as_str() {
        "serve" => {
            let service = option("--service").map(PathBuf::from);
            let page = option("--page").map(PathBuf::from);
            (|| serve(&socket.map_or_else(default_socket, Ok)?, service.map_or_else(default_service, Ok)?, page))()
        }
        "connect" => match (rest.first().cloned(), socket) {
            (Some(url), Some(socket)) => connect(url, &socket),
            _ => Err(USAGE.into()),
        },
        "check" => socket.map_or_else(default_socket, Ok).and_then(|socket| check(&socket)),
        _ => Err(USAGE.into()),
    };
    if let Err(error) = result {
        eprintln!("romacs-relay: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budgeted_stream_gives_its_budget_a_turn() {
        let mut stream = Budgeted::new(std::io::repeat(7));
        let mut buf = vec![0; 3 * BUDGET];
        // Unbounded until first refilled (a handshake).
        assert_eq!(stream.read(&mut buf).unwrap(), 3 * BUDGET);
        let refill = stream.refill();
        refill();
        assert_eq!(stream.read(&mut buf).unwrap(), BUDGET);
        assert_eq!(stream.read(&mut buf).unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        refill();
        assert_eq!(stream.read(&mut buf[..10]).unwrap(), 10);
        // A turn's time spent, its bytes left whatever: it gives way.
        std::thread::sleep(POLL + Duration::from_millis(5));
        assert_eq!(stream.read(&mut buf[..10]).unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    }

    #[test]
    fn the_runtime_directory_is_read_as_its_components_spell_it() {
        assert_eq!(runtime_dir_of(Some("/run/user/1000/".into())).unwrap().as_os_str(), "/run/user/1000");
        assert!(runtime_dir_of(Some("relative".into())).is_err());
        assert!(runtime_dir_of(None).is_err());
    }

    fn head(lines: &[&str]) -> Head {
        let text = format!("GET /agents HTTP/1.1\r\n{}\r\n\r\n", lines.join("\r\n"));
        read_head(&mut text.as_bytes()).unwrap()
    }

    #[test]
    fn a_caller_needs_an_allowed_login_and_its_own_origin() {
        let allowed = vec!["me@example".to_owned()];
        assert_eq!(refusal(&head(&["Host: box.ts.net"]), &allowed), Some("no tailnet login"));
        assert_eq!(refusal(&head(&["Host: box.ts.net", "Tailscale-User-Login: other@example"]), &allowed),
                   Some("this login may not"));
        assert_eq!(refusal(&head(&["Host: box.ts.net", "Tailscale-User-Login: me@example",
                                   "Origin: https://evil.example"]), &allowed),
                   Some("another site's page"));
        // As Serve passes it: Host rewritten, the public authority forwarded.
        assert_eq!(refusal(&head(&["Host: localhost", "X-Forwarded-Host: box.ts.net",
                                   "Tailscale-User-Login: me@example", "Origin: https://box.ts.net"]), &allowed), None);
        assert_eq!(refusal(&head(&["Host: localhost", "X-Forwarded-Host: box.ts.net",
                                   "Tailscale-User-Login: me@example", "Origin: https://localhost"]), &allowed),
                   Some("another site's page"));
        // A native client sends no Origin.
        assert_eq!(refusal(&head(&["Host: box.ts.net", "Tailscale-User-Login: me@example"]), &allowed), None);
    }

    #[test]
    fn a_head_past_its_limit_is_refused() {
        let text = format!("GET / HTTP/1.1\r\nX-Pad: {}\r\n\r\n", "x".repeat(HEAD_LIMIT));
        assert!(read_head(&mut text.as_bytes()).is_err());
    }

    #[test]
    fn only_the_relay_says_who_called_and_a_message_is_one_line() {
        assert!(!carried(r#"{"jsonrpc":"2.0","method":"agents/transport","params":{"login":"x"}}"#));
        assert!(carried(r#"{"jsonrpc":"2.0","id":1,"method":"agents/hello","params":{}}"#));
        assert!(!carried("{\"jsonrpc\":\"2.0\"}\n{\"method\":\"agents/transport\"}"));
    }

    #[test]
    fn only_a_wss_url_is_connected_to() {
        assert_eq!(endpoint("wss://box.ts.net/agents"), Ok(("box.ts.net".to_owned(), 443)));
        assert_eq!(endpoint("wss://box.ts.net:8443/agents"), Ok(("box.ts.net".to_owned(), 8443)));
        assert!(endpoint("ws://box.ts.net/agents").is_err());
    }

    /// A fake agents service: its socket, and the lines it was sent, each
    /// answered with an echo.
    fn fake_service(dir: &Path) -> (PathBuf, mpsc::Receiver<String>) {
        let socket = dir.join("service-relay.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writing = stream.try_clone().unwrap();
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let _ = writeln!(writing, r#"{{"echo":{line}}}"#);
                if tx.send(line).is_err() { break }
            }
        });
        (socket, rx)
    }

    fn upgrade(client_end: UnixStream) -> WebSocket<UnixStream> {
        let request = tungstenite::http::Request::builder()
            .uri("ws://box.ts.net/agents").header("Host", "localhost").header("X-Forwarded-Host", "box.ts.net")
            .header("Origin", "https://box.ts.net")
            .header("Tailscale-User-Login", "me@example").header("Tailscale-User-Name", "Me")
            .header("Connection", "Upgrade").header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
            .body(()).unwrap();
        tungstenite::client(request, client_end).unwrap().0
    }

    #[test]
    fn a_websocket_reaches_the_service_after_its_caller_is_named() {
        let dir = std::env::temp_dir().join(format!("romacs-relay-ws-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (service, said) = fake_service(&dir);
        let (relay_end, client_end) = UnixStream::pair().unwrap();
        let allowed = vec!["me@example".to_owned()];
        std::thread::spawn(move || serve_one(relay_end, &service, None, &allowed));
        let mut ws = upgrade(client_end);
        // Who called comes first, from the relay; a caller's own is dropped.
        let first: Json = serde_json::from_str(&said.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        assert_eq!(first["method"], "agents/transport");
        assert_eq!(first["params"], json!({"via": "tailnet", "login": "me@example", "peer": "Me"}));
        ws.send(Message::text(r#"{"jsonrpc":"2.0","method":"agents/transport","params":{"login":"forged"}}"#)).unwrap();
        ws.send(Message::text(r#"{"jsonrpc":"2.0","id":1,"method":"agents/hello"}"#)).unwrap();
        let next: Json = serde_json::from_str(&said.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        assert_eq!(next["method"], "agents/hello");
        // The service's lines come back as messages: the echo of the transport
        // line, then the hello's.
        let mut echoes = Vec::new();
        while echoes.len() < 2 {
            if let Message::Text(text) = ws.read().unwrap() { echoes.push(text.as_str().to_owned()) }
        }
        assert!(echoes[1].contains("agents/hello"));
        // A message of two lines ends the connection: neither reaches the service.
        ws.send(Message::text("{\"id\":2,\"method\":\"agents/hello\"}\n{\"id\":3}")).unwrap();
        assert!(said.recv_timeout(Duration::from_millis(500)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_caller_not_allowed_is_refused_before_anything() {
        let (relay_end, mut client_end) = UnixStream::pair().unwrap();
        std::thread::spawn(move || serve_one(relay_end, Path::new("/nonexistent"), None, &[]));
        write!(client_end, "GET /agents HTTP/1.1\r\nHost: box.ts.net\r\nTailscale-User-Login: me@example\r\n\r\n").unwrap();
        let mut reply = String::new();
        client_end.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    }

    #[test]
    fn the_page_is_served_at_the_root_to_a_get() {
        let dir = std::env::temp_dir().join(format!("romacs-relay-page-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let page = dir.join("index.html");
        std::fs::write(&page, "<p>agents</p>").unwrap();
        for (method, status) in [("GET", "200"), ("POST", "405")] {
            let (relay_end, mut client_end) = UnixStream::pair().unwrap();
            let (allowed, page) = (vec!["me@example".to_owned()], page.clone());
            std::thread::spawn(move || serve_one(relay_end, Path::new("/nonexistent"), Some(&page), &allowed));
            write!(client_end, "{method} / HTTP/1.1\r\nHost: box.ts.net\r\nTailscale-User-Login: me@example\r\n\r\n").unwrap();
            let mut reply = String::new();
            client_end.read_to_string(&mut reply).unwrap();
            assert!(reply.starts_with(&format!("HTTP/1.1 {status}")), "{reply}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
