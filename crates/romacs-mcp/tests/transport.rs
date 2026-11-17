//! The relay's transport tests (tests/transport.py) against this build's
//! `romacs-mcp`: real pipes and sockets, a fixture peer, the plain relay
//! and the reconnecting one a session's harness runs. Here so that a change
//! to the relay runs them in every check that selects this package's tests.
use std::path::Path;
use std::process::Command;

#[test]
fn the_relay_s_transport_holds() {
    // Short: the fixture's sockets live under it, and a socket's path is
    // capped at 108 bytes.
    let short = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| Path::new(d).is_dir()).unwrap_or_else(|| "/tmp".into());
    let output = Command::new("python3")
        .arg("-B")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/transport.py"))
        .arg(env!("CARGO_BIN_EXE_romacs-mcp"))
        .env("TMPDIR", short)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success() && !stderr.contains("skipped"), "TRANSPORT:\n{stderr}");
}
