//! files.el `file-truename' natively on its common path: a local name, no
//! file name handler, no COUNTER or PREV-DIRS. Each component is joined to
//! the true name so far and asked for its link target (readlink), as GNU's
//! Lisp asks `file-symlink-p' of each directory level; a link's target
//! replaces the component and is resolved in turn. Whatever GNU's own Lisp
//! might answer differently is declined, and that Lisp decides it, errors
//! included (LISP-NATIVE-TRUENAME): many links, a walk that might spend
//! GNU's budget of steps, a leading `//' and a target beginning with `//'
//! or ending in a slash (it keeps them), a NUL, and a name or target that
//! is not UTF-8 text.
use super::{Step, Value};
use std::os::unix::ffi::OsStrExt;

/// The most links a walk follows before it leaves the name to the Lisp
/// algorithm.
const MOST_LINKS: usize = 3;
/// GNU's budget of loop steps (COUNTER), shared by its levels. A level
/// costs a step per pass, and each link starts its level again, walking the
/// spliced name's directories afresh: GNU spends at most (LINKS + 1) x
/// (COMPONENTS + 1) steps, the name's components and every target's seen
/// together. A walk that might spend more is GNU's to answer, its error
/// included.
const GNU_STEPS: usize = 100;

defsubr! { name = "romacs--file-truename-local", gnu = "files.el:1393", effect = Global,
fn file_truename_local(cx, filename: Value) -> Step {
    // The Lisp caller made FILENAME absolute; anything else is its to walk.
    // Text only: a unibyte name with non-ASCII bytes, or a multibyte one
    // holding raw bytes or characters past Unicode, is GNU's to walk.
    let text = {
        let view = cx.string_view_from(filename, 0)?;
        if view.multibyte { std::str::from_utf8(view.bytes).is_ok() } else { view.bytes.is_ascii() }
    };
    let path = super::fileio::file_name_bytes(cx, filename)?;
    if !text || path.first() != Some(&b'/') {
        return Ok(Step::Done(Value::NIL));
    }
    Ok(Step::Done(match truename(&path, read_link) {
        None => Value::NIL,
        Some(Walk { true_name, through }) if through.is_empty() => super::fileio::file_name_value(cx, &true_name),
        // With the names each followed link spliced in, for the caller to
        // ask its handlers of: (TRUE-NAME SPLICED...).
        Some(Walk { true_name, through }) => {
            let mut list = Value::NIL;
            for name in through.iter().rev() {
                let name = super::fileio::file_name_value(cx, name);
                list = cx.cons(name, list);
            }
            let true_name = super::fileio::file_name_value(cx, &true_name);
            cx.cons(true_name, list)
        }
    }))
}}

/// The link target of the host path P, or None when P names no link.
fn read_link(p: &[u8]) -> Option<Vec<u8>> {
    crate::host::read_link(std::ffi::OsStr::from_bytes(p)).ok().map(|t| t.as_os_str().as_bytes().to_vec())
}

/// A walk's answer: the true name, and the name each followed link made,
/// its target spliced in where the link stood.
#[derive(Debug, PartialEq)]
struct Walk {
    true_name: Vec<u8>,
    through: Vec<Vec<u8>>,
}

/// The true name of the absolute PATH, LINK giving each name's link
/// target; None when the walk is left to the Lisp algorithm.
fn truename(path: &[u8], link: impl Fn(&[u8]) -> Option<Vec<u8>>) -> Option<Walk> {
    if path.starts_with(b"//") || path.contains(&0) || std::str::from_utf8(path).is_err() {
        return None;
    }
    let trailing = path.len() > 1 && path.last() == Some(&b'/');
    // The components still to walk, the next last.
    let mut pending: Vec<Vec<u8>> = components(path).rev().collect();
    let mut seen = pending.len();
    let mut resolved: Vec<u8> = b"/".to_vec();
    let mut through = Vec::new();
    while let Some(part) = pending.pop() {
        if (through.len() + 1) * (seen + 1) > GNU_STEPS {
            return None;
        }
        match part.as_slice() {
            b"." => {}
            b".." => parent(&mut resolved),
            name => {
                let candidate = joined(&resolved, name);
                match link(&candidate) {
                    Some(target) => {
                        if through.len() == MOST_LINKS
                            || target.last() == Some(&b'/')
                            || target.starts_with(b"//")
                            || target.contains(&0)
                            || std::str::from_utf8(&target).is_err()
                        {
                            return None;
                        }
                        through.push(if target.first() == Some(&b'/') { target.clone() } else { joined(&resolved, &target) });
                        if target.first() == Some(&b'/') {
                            resolved = b"/".to_vec();
                        }
                        let before = pending.len();
                        pending.extend(components(&target).rev());
                        seen += pending.len() - before;
                    }
                    None => resolved = candidate,
                }
            }
        }
    }
    if trailing && resolved != b"/" {
        resolved.push(b'/');
    }
    Some(Walk { true_name: resolved, through })
}

/// NAME joined under the directory RESOLVED.
fn joined(resolved: &[u8], name: &[u8]) -> Vec<u8> {
    let mut out = resolved.to_vec();
    if out.last() != Some(&b'/') {
        out.push(b'/');
    }
    out.extend_from_slice(name);
    out
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
        truename(path.as_bytes(), links(table)).map(|w| String::from_utf8(w.true_name).unwrap())
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
        assert_eq!(walk("/a/b/./c", table).as_deref(), Some("/a/b/c"));
        assert_eq!(walk("/a//b", table).as_deref(), Some("/a/b"));
        assert_eq!(walk("/..", table).as_deref(), Some("/"));
        assert_eq!(walk("/", table).as_deref(), Some("/"));
        assert_eq!(walk("/a/λ", table).as_deref(), Some("/a/λ"));
    }

    #[test]
    fn a_walk_names_what_each_followed_link_spliced_in() {
        let table: &[(&str, &str)] = &[("/a/l", "/fake:x/../plain"), ("/a/r", "sub/../y")];
        let w = truename(b"/a/l", links(table)).unwrap();
        assert_eq!(w.through, vec![b"/fake:x/../plain".to_vec()]);
        let w = truename(b"/a/r/f", links(table)).unwrap();
        assert_eq!(w.through, vec![b"/a/sub/../y".to_vec()]);
        assert!(truename(b"/a/b", links(table)).unwrap().through.is_empty());
    }

    #[test]
    fn what_gnu_might_answer_otherwise_is_left_to_lisp() {
        let table: &[(&str, &str)] = &[("/x", "/y"), ("/y", "/x"), ("/d", "real/"), ("/bad", "\u{0}")];
        // A loop, or a long chain of links.
        assert_eq!(walk("/x/f", table), None);
        // A walk that might spend GNU's 100 steps: many components, or
        // fewer through a link, which walks them again.
        let deep = |n: usize| format!("/{}", vec!["d"; n].join("/"));
        assert!(walk(&deep(98), &[]).is_some());
        assert_eq!(walk(&deep(100), &[]), None);
        let long: &[(&str, &str)] = &[("/a", "b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q")];
        assert!(walk("/a/1/2/3/4/5", long).is_some());
        assert_eq!(walk(&format!("/a{}", deep(40)), long), None);
        // A target ending in a slash, a leading `//', a NUL.
        assert_eq!(walk("/d/f", table), None);
        let doubled: &[(&str, &str)] = &[("/l", "//tmp/missing")];
        assert_eq!(walk("/l", doubled), None);
        assert_eq!(walk("//a/b", table), None);
        assert_eq!(truename(b"/a\0b", links(table)), None);
        assert_eq!(walk("/bad", table), None);
        // A name that is not UTF-8 text.
        assert_eq!(truename(b"/a/\xff", links(table)), None);
    }
}
