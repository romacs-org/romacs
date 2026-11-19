//! files.el `file-truename' natively on its common path: a local name, no
//! file name handler, no COUNTER or PREV-DIRS. Each component is joined to
//! the true name so far and asked for its link target (readlink), as GNU's
//! Lisp asks `file-symlink-p' of each directory level; a link's target
//! replaces the component and is resolved in turn. A walk that follows many
//! links or runs deep is declined, and GNU's own Lisp decides it, error
//! included: its link budget counts differently (LISP-NATIVE-TRUENAME).
use super::*;
use std::os::unix::ffi::OsStrExt;

/// The most links a walk follows, and the deepest path it walks, before it
/// leaves the name to the Lisp algorithm: both well inside GNU's budget of
/// 100 steps, however GNU spends it.
const MOST_LINKS: usize = 3;
const DEEPEST: usize = 20;

defsubr! { name = "romacs--file-truename-local", gnu = "files.el:1393", effect = Global,
fn file_truename_local(cx, filename: Value) -> Step {
    // The Lisp caller made FILENAME absolute; anything else is its to walk.
    let path = super::fileio::file_name_bytes(cx, filename)?;
    if path.first() != Some(&b'/') {
        return Ok(Step::Done(Value::NIL));
    }
    Ok(Step::Done(match truename(&path, |p| read_link(p)) {
        Some(true_name) => super::fileio::file_name_value(cx, &true_name),
        None => Value::NIL,
    }))
}}

/// The link target of the host path P, or None when P names no link.
fn read_link(p: &[u8]) -> Option<Vec<u8>> {
    crate::host::read_link(std::ffi::OsStr::from_bytes(p)).ok().map(|t| t.as_os_str().as_bytes().to_vec())
}

/// The true name of the absolute PATH, LINK giving each name's link
/// target; None when the walk is left to the Lisp algorithm.
fn truename(path: &[u8], link: impl Fn(&[u8]) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let trailing = path.len() > 1 && path.last() == Some(&b'/');
    // The components still to walk, the next last.
    let mut pending: Vec<Vec<u8>> = components(path).rev().collect();
    let mut resolved: Vec<u8> = b"/".to_vec();
    let mut links = 0;
    while let Some(part) = pending.pop() {
        if pending.len() > DEEPEST {
            return None;
        }
        match part.as_slice() {
            b"." => {}
            b".." => parent(&mut resolved),
            name => {
                let mut candidate = resolved.clone();
                if candidate.last() != Some(&b'/') {
                    candidate.push(b'/');
                }
                candidate.extend_from_slice(name);
                match link(&candidate) {
                    Some(target) => {
                        links += 1;
                        if links > MOST_LINKS {
                            return None;
                        }
                        if target.first() == Some(&b'/') {
                            resolved = b"/".to_vec();
                        }
                        pending.extend(components(&target).rev());
                    }
                    None => resolved = candidate,
                }
            }
        }
    }
    if trailing && resolved != b"/" {
        resolved.push(b'/');
    }
    Some(resolved)
}

/// PATH's non-empty components, in order.
fn components(path: &[u8]) -> impl DoubleEndedIterator<Item = Vec<u8>> + '_ {
    path.split(|&b| b == b'/').filter(|part| !part.is_empty()).map(<[u8]>::to_vec)
}

/// RESOLVED, a true name, made its parent; the root is its own parent.
fn parent(resolved: &mut Vec<u8>) {
    match resolved.iter().rposition(|&b| b == b'/') {
        Some(0) | None => resolved.truncate(1),
        Some(i) => resolved.truncate(i),
    }
}

#[cfg(test)]
mod tests {
    use super::truename;

    fn links(table: &'static [(&'static str, &'static str)]) -> impl Fn(&[u8]) -> Option<Vec<u8>> {
        move |p| table.iter().find(|(from, _)| from.as_bytes() == p).map(|(_, to)| to.as_bytes().to_vec())
    }

    fn walk(path: &str, table: &'static [(&'static str, &'static str)]) -> Option<String> {
        truename(path.as_bytes(), links(table)).map(|t| String::from_utf8(t).unwrap())
    }

    #[test]
    fn a_walk_resolves_links_as_gnu_does() {
        let table: &[(&str, &str)] = &[("/a/l", "/a/real"), ("/a/real/rel", "../other"), ("/a/up", "sub/x")];
        assert_eq!(walk("/a/l/f", table).as_deref(), Some("/a/real/f"));
        assert_eq!(walk("/a/real/rel/g", table).as_deref(), Some("/a/other/g"));
        // `..' after a link climbs from the link's target.
        assert_eq!(walk("/a/l/../h", table).as_deref(), Some("/a/h"));
        assert_eq!(walk("/a/up", table).as_deref(), Some("/a/sub/x"));
        assert_eq!(walk("/a/l/", table).as_deref(), Some("/a/real/"));
        assert_eq!(walk("/a//b/./c", table).as_deref(), Some("/a/b/c"));
        assert_eq!(walk("/..", table).as_deref(), Some("/"));
        assert_eq!(walk("/", table).as_deref(), Some("/"));
    }

    #[test]
    fn a_loop_or_a_long_chain_is_left_to_lisp() {
        let table: &[(&str, &str)] = &[("/x", "/y"), ("/y", "/x")];
        assert_eq!(walk("/x/f", table), None);
        let deep = format!("/{}", vec!["d"; 30].join("/"));
        assert_eq!(walk(&deep, &[]), None);
    }
}
