//! `romacs-relay`: the agent session protocol (doc/agent-protocol.org) over a
//! tailnet. It holds no agent logic: it carries the protocol's lines between
//! a WebSocket and the agents service's Unix socket.
//!
//! `romacs-relay serve [--socket PATH] [--service PATH] [--page FILE]`
//! listens on a private Unix socket that `tailscale serve --https=443
//! unix:PATH` publishes on the tailnet. `GET /` serves the phone page;
//! `/agents` upgrades to a WebSocket. Before anything else, the request must
//! carry `Tailscale-User-Login` (Serve sets it for a user's device, and strips
//! what a caller sends) naming a login of the allowlist,
//! `$XDG_CONFIG_HOME/romacs/relay.json` (`{"allow": ["LOGIN"]}`), and an
//! `Origin`, when there is one, must be the page's own. The relay then
//! connects to the service's `service-relay.sock` and says first who called
//! (`agents/transport`); a caller's message naming that method is dropped.
//!
//! `romacs-relay connect URL --socket PATH` runs on another machine: each
//! connection to its local socket is carried over one WebSocket to URL
//! (`wss://HOST/agents`), so a Romacs there speaks to a Unix socket, as it
//! does at home. When either end goes, so does the other.
#![forbid(unsafe_code)]

use serde_json::{Value as Json, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;
use tungstenite::{Message, WebSocket};

const USAGE: &str = "usage: romacs-relay serve [--socket PATH] [--service PATH] [--page FILE]\n       romacs-relay connect URL --socket PATH\n       romacs-relay check [--socket PATH]";

/// The most bytes of one protocol message (Framing), a line or a WebSocket
/// message alike.
const MESSAGE_LIMIT: usize = 1024 * 1024;

/// The most bytes of a request's head before the WebSocket upgrade.
const HEAD_LIMIT: usize = 64 * 1024;

/// How long a WebSocket read waits before the relay looks at the other end.
const POLL: Duration = Duration::from_millis(25);

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

fn default_socket() -> PathBuf {
    runtime_dir().join("romacs/relay/http.sock")
}

fn default_service() -> PathBuf {
    runtime_dir().join("romacs/agents/service/service-relay.sock")
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

/// A request's head: its path and headers (names lowercased), and the bytes
/// read, which the WebSocket handshake reads again.
struct Head {
    path: String,
    headers: Vec<(String, String)>,
    bytes: Vec<u8>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }
}

/// Read a request's head from STREAM, none past `HEAD_LIMIT`.
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
            Ok(httparse::Status::Complete(_)) => {
                let path = request.path.unwrap_or("/").to_owned();
                let headers = request.headers.iter()
                    .map(|h| (h.name.to_ascii_lowercase(), String::from_utf8_lossy(h.value).into_owned()))
                    .collect();
                return Ok(Head { path, headers, bytes });
            }
            Ok(httparse::Status::Partial) if bytes.len() < HEAD_LIMIT => {}
            Ok(httparse::Status::Partial) => return Err("the request's head is too long".into()),
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// Why HEAD may not reach the agents, or none: its caller's login must be
/// in ALLOWED, and its Origin, when it has one, the page's own.
fn refusal(head: &Head, allowed: &[String]) -> Option<&'static str> {
    let Some(login) = head.header("tailscale-user-login") else { return Some("no tailnet login") };
    if !allowed.iter().any(|allowed| allowed == login) { return Some("this login may not") }
    if let Some(origin) = head.header("origin") {
        let host = head.header("host").unwrap_or("");
        if origin != format!("https://{host}") { return Some("another site's page") }
    }
    None
}

fn respond(stream: &mut impl Write, status: &str, kind: &str, body: &[u8]) {
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\
                           Cache-Control: no-store\r\n\r\n", body.len());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// Whether LINE, a client's message, names the transport method: only the
/// relay says who called.
fn names_transport(line: &str) -> bool {
    serde_json::from_str::<Json>(line).ok()
        .is_some_and(|message| message.get("method").and_then(Json::as_str) == Some("agents/transport"))
}

/// Carry messages between WS and LOCAL, a Unix stream of lines, until either
/// ends: each text message a line, each line a text message; a line DROP
/// says is not carried on. WS's underlying stream reads with a timeout
/// (`POLL`), so its reads give way to the local end's lines.
fn pump<S: Read + Write>(ws: &mut WebSocket<S>, local: UnixStream, drop: fn(&str) -> bool) {
    let (lines_tx, lines) = mpsc::channel::<Option<String>>();
    let reading = match local.try_clone() { Ok(reading) => reading, Err(_) => return };
    let mut writing = local;
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reading);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(n) if n > MESSAGE_LIMIT + 1 => break,
                Ok(_) => { if lines_tx.send(Some(line.trim_end_matches(['\n', '\r']).to_owned())).is_err() { return } }
            }
        }
        let _ = lines_tx.send(None);
    });
    'carrying: loop {
        match ws.read() {
            Ok(Message::Text(text)) => {
                let text = text.as_str();
                if text.len() > MESSAGE_LIMIT { break }
                if !drop(text) && writeln!(writing, "{}", text.trim_end_matches(['\n', '\r'])).is_err() { break }
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
    let _ = ws.close(None);
    let _ = ws.flush();
    let _ = writing.shutdown(std::net::Shutdown::Both);
}

/// Serve one connection from tailscale's Serve on STREAM.
fn serve_one(mut stream: UnixStream, service: &Path, page: Option<&Path>, allowed: &[String]) {
    let head = match read_head(&mut stream) {
        Ok(head) => head,
        Err(_) => return respond(&mut stream, "400 Bad Request", "text/plain", b"bad request\n"),
    };
    if let Some(why) = refusal(&head, allowed) {
        return respond(&mut stream, "403 Forbidden", "text/plain", format!("{why}\n").as_bytes());
    }
    match head.path.split('?').next().unwrap_or("") {
        "/" => match page.and_then(|page| std::fs::read(page).ok()) {
            Some(body) => respond(&mut stream, "200 OK", "text/html; charset=utf-8", &body),
            None => respond(&mut stream, "404 Not Found", "text/plain", b"no page here yet\n"),
        },
        "/agents" => {
            let login = head.header("tailscale-user-login").unwrap_or("").to_owned();
            let peer = head.header("tailscale-user-name").unwrap_or("").to_owned();
            let Ok(mut agents) = UnixStream::connect(service) else {
                return respond(&mut stream, "503 Service Unavailable", "text/plain", b"the agents service is not running\n");
            };
            let transport = json!({"jsonrpc": "2.0", "method": "agents/transport",
                                   "params": {"via": "tailnet", "login": login, "peer": peer}});
            if writeln!(agents, "{transport}").is_err() { return }
            if stream.set_read_timeout(Some(POLL)).is_err() { return }
            let prefixed = Prefixed { prefix: head.bytes, at: 0, inner: stream };
            if let Ok(mut ws) = tungstenite::accept(prefixed) { pump(&mut ws, agents, names_transport) }
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"not found\n"),
    }
}

fn private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Listen on SOCKET, in a directory of its own, replacing a socket nothing
/// answers at.
fn listen(socket: &Path) -> Result<UnixListener, String> {
    if let Some(dir) = socket.parent() { private_dir(dir)?; }
    if socket.exists() {
        if UnixStream::connect(socket).is_ok() { return Err(format!("{}: something answers there", socket.display())) }
        std::fs::remove_file(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    }
    UnixListener::bind(socket).map_err(|e| format!("{}: {e}", socket.display()))
}

fn serve(socket: &Path, service: PathBuf, page: Option<PathBuf>) -> Result<(), String> {
    let listener = listen(socket)?;
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

/// Carry LOCAL, one connection to the local socket, over a WebSocket to URL.
fn connect_one(local: UnixStream, url: &str, tls: Arc<rustls::ClientConfig>) {
    let host_port = url.split("://").nth(1).and_then(|rest| rest.split('/').next()).unwrap_or("");
    let address = if host_port.contains(':') { host_port.to_owned() } else { format!("{host_port}:443") };
    let Ok(tcp) = std::net::TcpStream::connect(&address) else { return };
    let Ok((mut ws, _)) = tungstenite::client_tls_with_config(url, tcp, None, Some(tungstenite::Connector::Rustls(tls)))
    else { return };
    // Polled once connected: the handshakes above wait as long as they take.
    let socket = match ws.get_ref() {
        tungstenite::stream::MaybeTlsStream::Rustls(stream) => &stream.sock,
        tungstenite::stream::MaybeTlsStream::Plain(stream) => stream,
        _ => return,
    };
    if socket.set_read_timeout(Some(POLL)).is_err() { return }
    pump(&mut ws, local, |_| false);
}

fn connect(url: String, socket: &Path) -> Result<(), String> {
    if !url.starts_with("wss://") { return Err(format!("{url}: a wss:// URL")) }
    let listener = listen(socket)?;
    let tls = tls();
    for local in listener.incoming().map_while(Result::ok) {
        let (url, tls) = (url.clone(), tls.clone());
        std::thread::spawn(move || connect_one(local, &url, tls));
    }
    Ok(())
}

/// Whether tailscale serves SOCKET: what `tailscale serve status` says.
fn check(socket: &Path) -> Result<(), String> {
    let status = std::process::Command::new("tailscale").args(["serve", "status"]).output()
        .map_err(|e| format!("tailscale: {e}"))?;
    let text = String::from_utf8_lossy(&status.stdout);
    let allowed = allowlist(&allowlist_file());
    println!("socket: {}", socket.display());
    println!("allowlist: {} ({} login{})", allowlist_file().display(), allowed.len(), if allowed.len() == 1 { "" } else { "s" });
    if text.contains(&*socket.to_string_lossy()) {
        println!("tailscale serves it:\n{text}");
        Ok(())
    } else {
        Err(format!("tailscale does not serve it: run\n  tailscale serve --bg --https=443 unix:{}", socket.display()))
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
            let service = option("--service").map(PathBuf::from).unwrap_or_else(default_service);
            let page = option("--page").map(PathBuf::from);
            serve(&socket.unwrap_or_else(default_socket), service, page)
        }
        "connect" => match (rest.first().cloned(), socket) {
            (Some(url), Some(socket)) => connect(url, &socket),
            _ => Err(USAGE.into()),
        },
        "check" => check(&socket.unwrap_or_else(default_socket)),
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
        assert_eq!(refusal(&head(&["Host: box.ts.net", "Tailscale-User-Login: me@example",
                                   "Origin: https://box.ts.net"]), &allowed), None);
        // A native client sends no Origin.
        assert_eq!(refusal(&head(&["Host: box.ts.net", "Tailscale-User-Login: me@example"]), &allowed), None);
    }

    #[test]
    fn only_the_relay_says_who_called() {
        assert!(names_transport(r#"{"jsonrpc":"2.0","method":"agents/transport","params":{"login":"x"}}"#));
        assert!(!names_transport(r#"{"jsonrpc":"2.0","id":1,"method":"agents/hello","params":{}}"#));
        assert!(!names_transport("not json"));
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

    #[test]
    fn a_websocket_reaches_the_service_after_its_caller_is_named() {
        let dir = std::env::temp_dir().join(format!("romacs-relay-ws-{}", std::process::id()));
        private_dir(&dir).unwrap();
        let (service, said) = fake_service(&dir);
        let (relay_end, client_end) = UnixStream::pair().unwrap();
        let allowed = vec!["me@example".to_owned()];
        std::thread::spawn(move || serve_one(relay_end, &service, None, &allowed));
        let request = tungstenite::http::Request::builder()
            .uri("ws://box.ts.net/agents").header("Host", "box.ts.net")
            .header("Tailscale-User-Login", "me@example").header("Tailscale-User-Name", "Me")
            .header("Connection", "Upgrade").header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
            .body(()).unwrap();
        let (mut ws, _) = tungstenite::client(request, client_end).unwrap();
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
    fn the_page_is_served_at_the_root() {
        let dir = std::env::temp_dir().join(format!("romacs-relay-page-{}", std::process::id()));
        private_dir(&dir).unwrap();
        let page = dir.join("index.html");
        std::fs::write(&page, "<p>agents</p>").unwrap();
        let (relay_end, mut client_end) = UnixStream::pair().unwrap();
        let allowed = vec!["me@example".to_owned()];
        std::thread::spawn(move || serve_one(relay_end, Path::new("/nonexistent"), Some(&page), &allowed));
        write!(client_end, "GET / HTTP/1.1\r\nHost: box.ts.net\r\nTailscale-User-Login: me@example\r\n\r\n").unwrap();
        let mut reply = String::new();
        client_end.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200") && reply.ends_with("<p>agents</p>"), "{reply}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
