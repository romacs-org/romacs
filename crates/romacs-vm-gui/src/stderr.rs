//! Lines to stderr, each handed over whole. Unbuffered stderr takes a
//! formatted line (`eprintln!`) a piece at a time, a write apiece. The lock
//! Rust's writers share keeps them apart, but a writer outside it, such as
//! Lisp's `write-region` to /dev/stderr (the harnesses' marks, on their own
//! file description) or C code, can come between the pieces. Handed over as
//! one buffer, a line goes out in one write unless the kernel takes it in
//! part: a pipe takes up to PIPE_BUF bytes whole, an append-only file each
//! write whole, and `write_all` writes what a short write left.
use std::io::Write;

/// Write TEXT and a newline to stderr, handed over as one buffer.
pub fn line(text: &str) {
    let _ = write_line(&mut std::io::stderr().lock(), text);
}

/// Write TEXT and a newline to OUT, handed over as one buffer.
fn write_line(out: &mut impl Write, text: &str) -> std::io::Result<()> {
    let mut whole = Vec::with_capacity(text.len() + 1);
    whole.extend_from_slice(text.as_bytes());
    whole.push(b'\n');
    out.write_all(&whole)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    /// A sink that keeps each write it is handed apart.
    struct Writes(Vec<Vec<u8>>);
    impl Write for Writes {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.push(bytes.to_vec());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }

    #[test]
    fn a_line_and_its_newline_are_one_write() {
        let mut out = Writes(vec![]);
        super::write_line(&mut out, r#"romacs-stall: {"ms":1}"#).unwrap();
        assert_eq!(out.0, vec![b"romacs-stall: {\"ms\":1}\n".to_vec()]);
    }
}
