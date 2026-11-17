//! Lines to stderr, each in one write. Unbuffered stderr takes a formatted
//! line (`eprintln!`) a piece at a time, a syscall apiece, and another
//! writer can come between the pieces: another thread, or Lisp's
//! `write-region` to /dev/stderr (the harnesses' marks). A line written
//! whole is never split by another, which the harnesses that read stderr
//! line by line rely on.
use std::io::Write;

/// Write TEXT and a newline to stderr in one write.
pub fn line(text: &str) {
    let _ = write_line(&mut std::io::stderr().lock(), text);
}

/// Write TEXT and a newline to OUT in one write.
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
