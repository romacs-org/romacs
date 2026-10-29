//! GNU fileio.c's POSIX searchable-directory predicate.
use super::*;

defsubr! { name = "file-accessible-directory-p", gnu = "fileio.c:3255", effect = Global,
fn file_accessible_directory_p(cx, filename: Value) {
    if cx.string_text(filename)?.contains('\0') {
        return Err(Fault::Wrong("filenamep", filename));
    }
    let path = super::fileio::expand_file_name(cx, filename)?;
    let expanded = cx.string(&path);
    super::fileio::refuse_handler(cx, expanded, "file-accessible-directory-p")?;
    // GNU appends /./ before checking existence. The kernel must traverse the
    // directory, enforcing search permission without requiring read access.
    // Keep this a string: Path::join/normalization must not discard the dot.
    let searchable = if path.ends_with('/') { format!("{path}./") } else { format!("{path}/./") };
    Ok(if crate::host::probe(crate::host::Probe::Exists, &searchable) { Value::TRUE } else { Value::NIL })
}}
