//! A session image's container (rlisp/warm-restart.el): a printed Lisp form
//! and a table of text blobs, each the raw internal bytes of one buffer's
//! whole text. Lisp prints and reads the form; only the text, which is the
//! bulk, crosses here, as memory copies, never through a coding system.
//! Not in GNU (DIVERGENCES.md VM2-SESSION-IMAGE).
//!
//! The file is `ROMACS-SESSION 1`, one header line per part (`FORM` first,
//! then one `BLOB` per buffer: its byte length, 1 when multibyte, and the
//! SHA-1 of its bytes), an empty line, then the parts' bytes in that
//! order. It is written to a temporary name in its directory (mode 0600,
//! as it holds buffer text), synced, renamed over FILE and the directory
//! synced, so a reader sees a whole image or none; a reader refuses an
//! image any part of which differs from its digest, before any of it is
//! used.
use super::*;
use crate::host;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::os::unix::ffi::OsStrExt;

const MAGIC: &[u8] = b"ROMACS-SESSION 1\n";

/// One part's bytes and whether they are multibyte internal text.
#[derive(Debug)]
struct Part { bytes: Vec<u8>, multibyte: bool }

fn file_fault(cx: &mut Leaf<'_>, operation: &str, file: Value, message: String) -> Fault {
    let tail = cx.cons(file, Value::NIL);
    let message = cx.string(&message);
    let tail = cx.cons(message, tail);
    let operation = cx.string(operation);
    let data = cx.cons(operation, tail);
    Fault::Signal(cx.intern("file-error"), data)
}

fn io_text(e: std::io::Error) -> String {
    let message = e.to_string();
    message.rsplit_once(" (os error ").map_or(message.clone(), |(s, _)| s.to_owned())
}

fn path_of(cx: &Leaf<'_>, file: Value) -> std::result::Result<PathBuf, Fault> {
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(&cx.file_name_bytes(file)?)))
}

fn sha1(bytes: &[u8]) -> String {
    crate::digest::digest("sha1", bytes).expect("sha1 is an algorithm").iter().map(|b| format!("{b:02x}")).collect()
}

fn header(parts: &[(&str, usize, bool, String)]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    for (kind, len, multibyte, digest) in parts {
        out.extend_from_slice(format!("{kind} {len} {} {digest}\n", u8::from(*multibyte)).as_bytes());
    }
    out.push(b'\n');
    out
}

/// Write HEAD, the form's bytes and each buffer's text to a temporary file
/// beside PATH, sync it, and rename it over PATH.
fn publish(cx: &Leaf<'_>, path: &Path, head: &[u8], form: &[u8], buffers: &[Value]) -> std::io::Result<u64> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let (temporary, mut file) = host::temp_file(&parent.join(".session-XXXXXX"))?;
    let written = (|| {
        file.write_all(head)?;
        file.write_all(form)?;
        for &buffer in buffers {
            cx.with_buffer_text(buffer, |bytes, _| file.write_all(bytes))
                .map_err(|_| std::io::Error::other("buffer killed while writing"))??;
        }
        file.sync_all()?;
        host::rename(&temporary, path)?;
        host::sync_dir(parent)?;
        Ok(head.len() as u64)
    })();
    if written.is_err() { let _ = host::remove_file(&temporary); }
    written
}

// Write the session image FILE: FORM, a string (the printed plist), and
// the whole text of each buffer of BUFFERS, a list, narrowing aside.
// Return the file's size in bytes.
defsubr! { name = "romacs--session-image-write", gnu = "romacs:session-image", effect = Global,
fn session_image_write(cx, file: Value, form: Value, buffers: Value) {
    let path = path_of(cx, file)?;
    let form_view = cx.string_view(form)?;
    let mut list = Vec::new();
    let mut tail = buffers;
    while let Some((buffer, rest)) = cx.pair(tail) {
        list.push(buffer);
        tail = rest;
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", buffers)); }
    let mut parts = vec![("FORM", form_view.bytes().len(), form_view.multibyte, sha1(form_view.bytes()))];
    for &buffer in &list {
        if !cx.lifecycle_live_p(buffer) { return Err(Fault::Wrong("buffer-live-p", buffer)); }
        let (len, multibyte, digest) = cx.with_buffer_text(buffer, |bytes, multibyte| (bytes.len(), multibyte, sha1(bytes)))?;
        parts.push(("BLOB", len, multibyte, digest));
    }
    let head = header(&parts);
    let body: u64 = parts.iter().map(|(_, len, _, _)| *len as u64).sum();
    match publish(cx, &path, &head, form_view.bytes(), &list) {
        Ok(head) => Value::integer((head + body) as i64).map_err(|e| Fault::Error(e.into())),
        Err(e) => Err(file_fault(cx, "Writing session image", file, io_text(e))),
    }
}}

/// The parts of image BYTES, or why it is not one.
fn parts(bytes: &[u8]) -> std::result::Result<Vec<Part>, String> {
    let rest = bytes.strip_prefix(MAGIC).ok_or("not a session image")?;
    let end = rest.windows(2).position(|w| w == b"\n\n").ok_or("truncated header")?;
    let mut lengths = Vec::new();
    for line in rest[..end].split(|&b| b == b'\n') {
        let line = std::str::from_utf8(line).map_err(|_| "bad header line")?;
        let mut words = line.split(' ');
        let (kind, len, multibyte, digest) = (words.next(), words.next(), words.next(), words.next());
        let len: usize = len.and_then(|n| n.parse().ok()).ok_or("bad part length")?;
        let multibyte = match multibyte { Some("1") => true, Some("0") => false, _ => return Err("bad part flag".into()) };
        let digest = digest.ok_or("part without a digest")?;
        let expected = if lengths.is_empty() { "FORM" } else { "BLOB" };
        if kind != Some(expected) || words.next().is_some() { return Err(format!("bad header line {line:?}")); }
        lengths.push((len, multibyte, digest));
    }
    let mut data = &rest[end + 2..];
    let mut out = Vec::with_capacity(lengths.len());
    for (len, multibyte, digest) in lengths {
        if data.len() < len { return Err("truncated image".into()); }
        let (part, tail) = data.split_at(len);
        if sha1(part) != digest { return Err(format!("part {} differs from its digest", out.len())); }
        if multibyte && !internal(part) { return Err(format!("part {} is not internal text", out.len())); }
        out.push(Part { bytes: part.to_vec(), multibyte });
        data = tail;
    }
    if !data.is_empty() { return Err("trailing bytes after the last part".into()); }
    if out.is_empty() { return Err("no form".into()); }
    Ok(out)
}

/// Whether BYTES is text in GNU's internal multibyte form, as a string
/// made of it needs.
fn internal(bytes: &[u8]) -> bool {
    if std::str::from_utf8(bytes).is_ok() { return true; }
    let mut rest = bytes;
    while !rest.is_empty() {
        match crate::lstring::decode_char(rest) {
            Some((_, width)) if width > 0 => rest = &rest[width..],
            _ => return false,
        }
    }
    true
}

// Read the session image FILE: (FORM . BLOBS), FORM the printed plist as a
// string and BLOBS a vector of the text strings, in the order written.
defsubr! { name = "romacs--session-image-read", gnu = "romacs:session-image", effect = Reads,
fn session_image_read(cx, file: Value) {
    let path = path_of(cx, file)?;
    let bytes = host::read_file(&path).map_err(|e| file_fault(cx, "Reading session image", file, io_text(e)))?;
    let mut parts = parts(&bytes).map_err(|reason| file_fault(cx, "Reading session image", file, reason))?.into_iter();
    drop(bytes);
    let form = parts.next().expect("parts has a form");
    let form = cx.make_string(form.bytes, form.multibyte);
    let blobs: Vec<Value> = parts.map(|p| cx.make_string(p.bytes, p.multibyte)).collect();
    let blobs = cx.vector_from(blobs);
    Ok(cx.cons(form, blobs))
}}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(parts: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = header(&parts.iter().map(|(k, b, m)| (*k, b.len(), *m, sha1(b))).collect::<Vec<_>>());
        for (_, bytes, _) in parts { out.extend_from_slice(bytes); }
        out
    }

    #[test]
    fn parts_round_trip_and_refuse_damage() {
        let raw = [b'a', 0xC1, 0xBF, b'z'];
        let whole = image(&[("FORM", b"(:schema 1)", true), ("BLOB", "λ\n".as_bytes(), true),
                            ("BLOB", &[0xff, 0x00], false), ("BLOB", &raw, true)]);
        let read = parts(&whole).unwrap();
        assert_eq!(read.len(), 4);
        assert_eq!(read[1].bytes, "λ\n".as_bytes());
        assert!(!read[2].multibyte && read[2].bytes == [0xff, 0x00]);
        assert_eq!(read[3].bytes, raw);
        assert!(parts(&whole[..whole.len() - 1]).unwrap_err().contains("truncated"));
        let mut longer = whole.clone();
        longer.push(b'x');
        assert!(parts(&longer).unwrap_err().contains("trailing"));
        let bad = image(&[("FORM", b"()", true), ("BLOB", &[0xff], true)]);
        assert!(parts(&bad).unwrap_err().contains("internal"));
        let mut flipped = whole.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        assert!(parts(&flipped).unwrap_err().contains("digest"), "a same-length corruption is refused");
        assert!(parts(b"ROMACS-SESSION 2\n").is_err());
    }
}
