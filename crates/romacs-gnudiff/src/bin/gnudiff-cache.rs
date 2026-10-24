//! Explicit GNU cache refresh; normal crate tests never request refresh.
use romacs_gnudiff::{Oracle, Runner};
use std::{io, path::PathBuf};

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("refresh")) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: cargo run -p romacs-gnudiff --bin gnudiff-cache -- refresh FILE...",
        ));
    }
    let files: Vec<_> = args.map(PathBuf::from).collect();
    let home = std::env::var_os("HOME")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME unavailable"))?;
    let oracle = Oracle::pin(std::path::Path::new("/usr/bin/emacs"))?;
    let sha = oracle.sha256.clone();
    Runner::new(oracle, PathBuf::from(home).join(".cache/romacs-gnudiff"))?.refresh(&files)?;
    println!("GNU sha256={sha}; refreshed {} files", files.len());
    Ok(())
}
