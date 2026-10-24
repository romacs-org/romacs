//! `romacs-introspect-live --write | --check | --print`
use romacs_introspect_live::{embedded_snapshot_path, snapshot_path};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mode = std::env::args().nth(1).unwrap_or_default();
    if !matches!(mode.as_str(), "--write" | "--check" | "--print") {
        eprintln!("usage: romacs-introspect-live --write | --check | --print");
        return ExitCode::from(2);
    }
    let live = match romacs_introspect_live::enumerate() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("romacs-introspect-live: enumeration failed: {e}");
            return ExitCode::from(2);
        }
    };
    let text = serde_json::to_string_pretty(&live.to_json()).unwrap() + "\n";
    if mode == "--print" {
        print!("{text}");
        return ExitCode::SUCCESS;
    }
    let mut stale = false;
    for path in [snapshot_path(), embedded_snapshot_path()] {
        if mode == "--write" {
            if let Err(e) = std::fs::write(&path, &text) {
                eprintln!("write {}: {e}", path.display());
                return ExitCode::from(2);
            }
            println!("wrote {}", path.display());
        } else {
            match std::fs::read(&path) {
                Ok(bytes) if bytes == text.as_bytes() => println!("matches {}", path.display()),
                Ok(_) => {
                    eprintln!("STALE: {}", path.display());
                    stale = true;
                }
                Err(e) => {
                    eprintln!("read {}: {e}", path.display());
                    return ExitCode::from(2);
                }
            }
        }
    }
    if stale {
        eprintln!("regenerate: cargo run -p romacs-introspect-live -- --write");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
