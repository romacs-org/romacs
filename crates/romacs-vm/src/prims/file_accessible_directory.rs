//! GNU fileio.c's POSIX searchable-directory predicate.
use super::*;

defsubr! { name = "file-accessible-directory-p", gnu = "fileio.c:3255", effect = Global,
fn file_accessible_directory_p(cx, filename: Value) -> Step {
    if cx.string_text(filename)?.contains('\0') {
        return Err(Fault::Wrong("filenamep", filename));
    }
    super::fileio::expanding(cx, filename, accessible_expanded, vec![])
}}

fn accessible_expanded(cx: &mut Leaf<'_>, expanded: Value, _: &[Value]) -> Next {
    if let Some(call) = super::fileio::handler(cx, expanded, "file-accessible-directory-p", vec![expanded])? { return call; }
    let path = cx.string_text(expanded)?;
    // GNU appends /./ before checking existence. The kernel must traverse the
    // directory, enforcing search permission without requiring read access.
    // Keep this a string: Path::join/normalization must not discard the dot.
    let searchable = if path.ends_with('/') { format!("{path}./") } else { format!("{path}/./") };
    Ok(Step::Done(if crate::host::probe(crate::host::Probe::Exists, &searchable) { Value::TRUE } else { Value::NIL }))
}
