// std::fs::read(p) in a line comment.
/* std::env::var("A") in a /* nested */ block comment */
fn near_misses(p: &Path, meta: &Metadata) {
    let _ = "std::fs::read(p) Instant::now() \" libc::getpid()";
    let _ = r#"libc::getpid() "quoted" std::fs"#;
    let _ = b"std::process::id()";
    let _ = '"';
    let _ = '{';
    fn lifetimes<'a>(x: &'a str) -> &'a str { x }
    host::read_dir(p);
    crate::host::canonicalize(p);
    host::metadata(p);
    meta.file_type().is_dir();
    meta.file_type().is_file();
    romacs_fs::read(p);
    canonicalize_char(c);
    TtyPalette::canonicalize("White");
    env!("CARGO_MANIFEST_DIR");
    libc::getpid_like();
    std::process::id_like();
    vm.symbol("process-environment-like");
    var(cx, "default-directory");
    let _: std::fs::File = std::fs::File::from_raw_fd(fd);
    let _ = std::fs::Permissions::from_mode(0o644);
    fn types(_: &std::fs::Metadata, _: std::fs::FileType) {}
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::{ffi::OsStrExt, fs::{OpenOptionsExt, PermissionsExt}};
    use std::io::Write as _;
    use std::time::{Duration, Instant};
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use crate::host::{self, Input};
    use super::*;
    meta.file_type().is_symlink();
    entry.file_type().ok().filter(|t| !std::fs::FileType::is_symlink(t));
    MyPath::is_file(p);
    crate::host::kill(pid, 0);
    host::command("true");
    let _: libc::time_t = 0;
    let _ = libc::UTIME_NOW;
    libc::waitpid(pid, s, 0);
    type Item = u8;
    type Out = std::result::Result<Value, Fault>;
}

#[cfg(test)]
mod tests {
    fn t() { std::fs::read(p); std::env::var("A"); }
}

#[cfg(test)]
fn helper() { Instant::now(); }

#[cfg(all(test, target_os = "linux"))]
mod linux {
    fn t() { libc::getuid(); }
}
