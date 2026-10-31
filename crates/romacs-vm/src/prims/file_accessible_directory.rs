//! GNU fileio.c's POSIX searchable-directory predicate.
use super::*;

defsubr! { name = "file-accessible-directory-p", gnu = "fileio.c:3255", effect = Global,
fn file_accessible_directory_p(cx, filename: Value) {
    if cx.string_bytes(filename)?.contains(&0) {
        return Err(Fault::Wrong("filenamep", filename));
    }
    let (path, expanded) = super::fileio::expanded_path(cx, filename)?;
    super::fileio::refuse_handler(cx, expanded, "file-accessible-directory-p")?;
    // GNU appends /./ before checking existence. The kernel must traverse the
    // directory, enforcing search permission without requiring read access.
    // Keep this bytes: Path::join/normalization must not discard the dot.
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let mut searchable = path.as_os_str().as_bytes().to_vec();
    if searchable.last() != Some(&b'/') { searchable.push(b'/'); }
    searchable.extend_from_slice(b"./");
    let searchable = std::ffi::OsString::from_vec(searchable);
    Ok(if crate::host::probe(crate::host::Probe::Exists, &searchable) { Value::TRUE } else { Value::NIL })
}}
