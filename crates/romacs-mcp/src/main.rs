#![forbid(unsafe_code)]
#[cfg(unix)]
mod relay;
#[cfg(unix)]
mod reconnect;

fn main() {
    #[cfg(unix)]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
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
