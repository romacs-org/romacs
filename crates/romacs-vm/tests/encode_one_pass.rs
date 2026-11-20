//! `encode-coding-string` of multibyte text in a UTF-8, raw-text or undecided
//! coding system, which encodes in one pass over the internal form
//! (prim/leaf/coding.rs `encode_internal_plain`): the bytes, multibyteness
//! and `last-coding-system-used` are pinned GNU's for GNU's own coding
//! systems (its Lisp booted), every line end, a signature, raw bytes and
//! characters past Unicode, and a megabyte of text.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
#[path = "support/gnu_batch.rs"]
mod gnu_batch;

const CODINGS: &[&str] = &[
    "utf-8", "utf-8-unix", "utf-8-dos", "utf-8-mac", "utf-8-with-signature", "utf-8-with-signature-dos",
    "utf-8-auto", "utf-8-auto-mac", "utf-8-emacs", "utf-8-emacs-dos", "prefer-utf-8", "prefer-utf-8-dos",
    "raw-text", "raw-text-dos", "raw-text-mac", "no-conversion", "binary", "undecided", "undecided-dos",
    "undecided-mac",
];

/// Multibyte sources: empty, ASCII with LF and CRLF, UTF-8 (a BMP and an
/// astral character), raw bytes (eight-bit characters), a character past
/// Unicode, a unibyte string made multibyte (its high bytes raw), mixed.
const SOURCES: &[&str] = &[
    "(string-to-multibyte \"\")",
    "(string-to-multibyte \"abc\\ndef\\n\")",
    "(string-to-multibyte \"a\\r\\nb\\rc\")",
    "(string #x3bb #xe9 ?\\n #x1d518 ?\\n)",
    "(string #x3fffff #x3fff80 ?a ?\\n #x3fffc3)",
    "(string #x110000 ?\\n #x3fff7f)",
    "(string-to-multibyte (unibyte-string 97 195 169 255 10))",
    "(concat \"é\\r\\n\" (string #x3fffff) \"x\\ny\")",
];

fn expressions() -> Vec<String> {
    let mut cases = Vec::new();
    for coding in CODINGS {
        for source in SOURCES {
            cases.push(format!(
                "(let* ((source {source}) (result (encode-coding-string source '{coding})))
                   (list (multibyte-string-p result) (vconcat result) last-coding-system-used))"));
        }
        // NOCOPY: the same string back where GNU gives it back.
        cases.push(format!(
            "(let* ((source (string-to-multibyte \"plain\")) (result (encode-coding-string source '{coding} t)))
               (list (eq result source) (vconcat result) last-coding-system-used))"));
    }
    for coding in ["utf-8-dos", "utf-8-mac", "raw-text-dos"] {
        cases.push(format!(
            "(let* ((inhibit-eol-conversion t) (result (encode-coding-string (string ?a ?\\n #xe9 ?\\n) '{coding})))
               (list (vconcat result) last-coding-system-used))"));
    }
    // A megabyte: its digest and length.
    for coding in ["utf-8-unix", "utf-8-dos", "raw-text-unix", "utf-8-emacs-unix"] {
        cases.push(format!(
            "(let* ((source (apply #'concat (make-list 20000 (concat \"Ünïcödé λ ✓ 日本\\n\" (string #x3fffff #x1d518) \"\\r\\n\"))))
                    (result (encode-coding-string source '{coding})))
               (list (length result) (secure-hash 'sha1 result)))"));
    }
    cases
}

#[test]
fn encoding_matches_pinned_gnu() {
    let source = |expression: &str| format!("(condition-case e {expression} (error (list 'signal (car e))))");
    let expressions = expressions();
    let answers = gnu_batch::outputs(&expressions.iter().map(|e| format!("(prin1 {})", source(e))).collect::<Vec<_>>());
    let mut vm = gnu_lisp::gnu_lisp(&[]);
    let mut failures = Vec::new();
    for (expression, expected) in expressions.iter().zip(answers) {
        let actual = vm.eval_source(&source(expression)).and_then(romacs_vm::Local::display);
        if actual.as_deref() != Ok(expected.as_str()) { failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual:?}")); }
    }
    assert!(failures.is_empty(), "{} of {} differ:\n{}", failures.len(), expressions.len(), failures.join("\n"));
}
