#![forbid(unsafe_code)]
#[cfg(unix)]
mod relay;
#[cfg(unix)]
mod reconnect;

fn main() {
    #[cfg(unix)]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.iter().any(|arg| arg == "--inspect") {
            std::process::exit(inspect(&args));
        }
        if args.iter().any(|arg| arg == "--reconnect") {
            match reconnecting(&args) {
                Some(config) => reconnect::main(config),
                None => {
                    eprintln!("usage: romacs-mcp (--socket ABSOLUTE-PATH | --server NAME) --reconnect --session ID [--reply-timeout SECONDS]");
                    std::process::exit(1);
                }
            }
        }
        relay::main();
    }
    #[cfg(not(unix))]
    {
        eprintln!("romacs-mcp: Unix sockets are unsupported on this platform");
        std::process::exit(1);
    }
}

/// `--inspect (--socket PATH | --server NAME)`: ask a session for a snapshot
/// of its call stack and print the answer. The endpoint beside the session's
/// socket is served off its evaluator, so this answers while MCP cannot.
#[cfg(unix)]
fn inspect(args: &[String]) -> i32 {
    use std::io::{Read, Write};
    let mut socket = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = args.next().map(std::path::PathBuf::from),
            "--server" => socket = args.next().and_then(|name| server_socket(name)),
            _ => {}
        }
    }
    let Some(socket) = socket else {
        eprintln!("usage: romacs-mcp --inspect (--socket ABSOLUTE-PATH | --server NAME)");
        return 2;
    };
    let answer = (|| -> std::io::Result<Vec<u8>> {
        let mut stream = std::os::unix::net::UnixStream::connect(socket.with_file_name("inspect.sock"))?;
        // Past the session's own bound, for a client served after another.
        stream.set_read_timeout(Some(std::time::Duration::from_secs(8)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(8)))?;
        stream.write_all(b"inspect\n")?;
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer)?;
        Ok(answer)
    })();
    match answer {
        Ok(answer) if !answer.is_empty() => { let _ = std::io::stdout().write_all(&answer); 0 }
        Ok(_) => { eprintln!("romacs-mcp: the session gave no answer"); 1 }
        Err(error) => { eprintln!("romacs-mcp: {error}"); 1 }
    }
}

/// The native socket of the Romacs server NAME (rlisp/romacs-server.el):
/// `$XDG_RUNTIME_DIR/romacs/NAME/server.sock'.
#[cfg(unix)]
pub(crate) fn server_socket(name: &str) -> Option<std::path::PathBuf> {
    let valid = (1..=32).contains(&name.len()) && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(format!("/tmp/romacs{}", uid())));
    valid.then(|| runtime.join("romacs").join(name).join("server.sock"))
}

#[cfg(unix)]
fn uid() -> u32 {
    std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").expect("/proc/self"))
}

/// `--socket PATH | --server NAME', `--reconnect --session ID [--reply-timeout S]', in any order.
#[cfg(unix)]
fn reconnecting(args: &[String]) -> Option<reconnect::Config> {
    let (mut socket, mut session) = (None, None);
    let mut reply_timeout = std::time::Duration::from_secs(120);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = args.next().map(std::path::PathBuf::from),
            "--server" => socket = server_socket(args.next()?),
            "--session" => session = args.next().cloned(),
            "--reply-timeout" => reply_timeout = std::time::Duration::from_secs_f64(
                args.next()?.parse::<f64>().ok().filter(|s| *s > 0.0)?),
            "--reconnect" => {}
            _ => return None,
        }
    }
    let socket = socket.filter(|s| s.is_absolute())?;
    Some(reconnect::Config { socket, session: session.filter(|s| !s.is_empty())?, reply_timeout })
}
