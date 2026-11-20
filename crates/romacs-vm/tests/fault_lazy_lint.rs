//! The lazy Fault lint. A `Fault` has drop glue (`Error(String)`), so an
//! argument that builds one eagerly, `.ok_or(Fault::Wrong(..))`, builds it and
//! calls its destructor on every success as well. The lazy form,
//! `.ok_or_else(|| Fault::Wrong(..))`, builds it on failure only.
//!
//! This test refuses the eager form anywhere in romacs-vm's sources: a call of
//! `.ok_or(`, `.unwrap_or(Err(`, `.map_or(Err(` or `.or(Err(` whose argument
//! starts with a path ending in `Fault::` (`Fault::`, `prim::Fault::`,
//! `crate::prim::Fault::`, any other). It reads code only: comments and the
//! contents of string and char literals are removed, and whitespace does not
//! separate a spelling. There is no exemption. Clippy's
//! `unnecessary_lazy_evaluations` asks for the opposite form; lib.rs declares
//! why it is expected.
use std::path::{Path, PathBuf};

/// The calls whose argument is evaluated even when the call returns its receiver.
const EAGER: &[&str] = &[".ok_or(", ".unwrap_or(Err(", ".map_or(Err(", ".or(Err("];

fn ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The number of `#`s of the raw string literal (`r"`, `r#"`, `br"`) starting at
/// S[I], if one does.
fn raw_string(s: &[u8], i: usize) -> Option<usize> {
    let starts_token = |j: usize| j == 0 || !ident(s[j - 1]);
    let prefixed = s[i] == b'r' && (starts_token(i) || (s[i - 1] == b'b' && starts_token(i - 1)));
    let hashes = s[i + 1..].iter().take_while(|&&b| b == b'#').count();
    (prefixed && s.get(i + 1 + hashes) == Some(&b'"')).then_some(hashes)
}

/// SRC's code without comments, literal contents or whitespace: each byte kept,
/// with its offset in SRC. A literal leaves its quotes, so `"..."` cannot join
/// the code on either side of it.
fn code(src: &str) -> Vec<(u8, usize)> {
    let s = src.as_bytes();
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let (mut out, mut i) = (Vec::new(), 0);
    while i < s.len() {
        let c = s[i];
        if c == b'/' && at(i + 1) == b'/' {
            while i < s.len() && s[i] != b'\n' { i += 1; }
        } else if c == b'/' && at(i + 1) == b'*' {
            let (mut depth, mut j) = (1, i + 2);
            while depth > 0 && j < s.len() {
                if s[j] == b'/' && at(j + 1) == b'*' { depth += 1; j += 2; }
                else if s[j] == b'*' && at(j + 1) == b'/' { depth -= 1; j += 2; }
                else { j += 1; }
            }
            i = j;
        } else if let Some(hashes) = raw_string(s, i) {
            let close: Vec<u8> = std::iter::once(b'"').chain(std::iter::repeat_n(b'#', hashes)).collect();
            let body = i + 2 + hashes;
            let end = s[body..].windows(close.len()).position(|w| w == close.as_slice()).map_or(s.len(), |p| body + p + close.len());
            out.extend([(b'"', i), (b'"', i)]);
            i = end;
        } else if c == b'"' {
            let mut j = i + 1;
            while j < s.len() && s[j] != b'"' { j += if s[j] == b'\\' { 2 } else { 1 }; }
            out.extend([(b'"', i), (b'"', i)]);
            i = j + 1;
        } else if c == b'\'' {
            // A char literal ('x', '\n', '\u{..}', 'é'), else a lifetime or label.
            let end = if at(i + 1) == b'\\' {
                s[i + 3..].iter().position(|&b| b == b'\'').map(|p| i + 3 + p + 1)
            } else {
                let width = match at(i + 1) { 0xf0.. => 4, 0xe0.. => 3, 0xc0.. => 2, _ => 1 };
                (at(i + 1 + width) == b'\'').then_some(i + width + 2)
            };
            if let Some(end) = end {
                out.extend([(b'\'', i), (b'\'', i)]);
                i = end;
            } else {
                out.push((c, i));
                i += 1;
            }
        } else {
            if !c.is_ascii_whitespace() { out.push((c, i)); }
            i += 1;
        }
    }
    out
}

/// The lines of SRC that build a Fault eagerly.
fn eager_lines(src: &str) -> Vec<usize> {
    let code = code(src);
    let text: Vec<u8> = code.iter().map(|&(b, _)| b).collect();
    let mut lines = Vec::new();
    for call in EAGER {
        for k in (0..text.len()).filter(|&k| text[k..].starts_with(call.as_bytes())) {
            let (mut j, mut last) = (k + call.len(), None);
            loop {
                let start = j;
                while j < text.len() && ident(text[j]) { j += 1; }
                if j == start || !text[j..].starts_with(b"::") { break; }
                last = Some(&text[start..j]);
                j += 2;
            }
            if last == Some(b"Fault".as_slice()) { lines.push(src[..code[k].1].matches('\n').count() + 1); }
        }
    }
    lines.sort_unstable();
    lines.dedup();
    lines
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
        if path.is_dir() { out.extend(rust_files(&path)); }
        else if path.extension().is_some_and(|x| x == "rs") { out.push(path); }
    }
    out
}

#[test]
fn no_fault_is_built_on_success() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sites = Vec::new();
    for path in rust_files(&crate_dir.join("src")) {
        let rel = path.strip_prefix(crate_dir).unwrap().display().to_string();
        sites.extend(eager_lines(&std::fs::read_to_string(&path).unwrap()).into_iter().map(|n| format!("{rel}:{n}")));
    }
    sites.sort();
    assert!(sites.is_empty(), "these build a Fault even on success; build it lazily, \
                              `.ok_or_else(|| Fault::..)` or `.unwrap_or_else(|| Err(Fault::..))`:\n{}", sites.join("\n"));
}

/// Each line marked `// eager` is flagged, and no other.
#[test]
fn the_lint_flags_each_eager_spelling_and_nothing_else() {
    const PLANTED: &str = r##"
fn planted<'a>(x: Option<u8>, r: Option<Result<u8, Fault>>, v: Value, s: &'a str) {
    x.ok_or(Fault::Wrong("stringp", v));  // eager
    x.ok_or( prim::Fault::NotUnicode(v));  // eager
    x.ok_or(crate::prim::Fault::Range(v, v));  // eager
    x.ok_or(  // eager
        Fault::EndOfFile);
    r.unwrap_or(Err(Fault::Wrong("p", v)));  // eager
    x.map_or(Err(Fault::EndOfFile), Ok);  // eager
    x.or(Err(Fault::EndOfFile));  // eager
    let c = '"'; x.ok_or(Fault::Circular(v));  // eager
    let e = 'é'; let q = '\''; x.ok_or(Fault::Circular(v));  // eager
    x.ok_or_else(|| Fault::Wrong("stringp", v));
    r.unwrap_or_else(|| Err(Fault::Wrong("p", v)));
    // x.ok_or(Fault::Wrong("p", v))
    /* x.ok_or(Fault::EndOfFile) /* nested */ x.ok_or(Fault::EndOfFile) */
    let s = ".ok_or(Fault::Wrong";
    let t = r#"x.ok_or(Fault::EndOfFile)"#;
    x.ok_or(SlotFault::OutOfRange);
    x.ok_or(Faults::Wrong);
    x.ok_or("invalid index");
    x.ok_or(fault);
}
"##;
    let expected: Vec<usize> = PLANTED.lines().enumerate().filter(|(_, l)| l.ends_with("// eager")).map(|(n, _)| n + 1).collect();
    assert_eq!(expected.len(), 9);
    assert_eq!(eager_lines(PLANTED), expected);
}
