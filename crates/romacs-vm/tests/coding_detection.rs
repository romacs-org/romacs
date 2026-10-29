//! coding.c detect_coding for `insert-file-contents' without a coding
//! system: the ISO-2022, UTF-8, UTF-16, charset, Shift-JIS, Big5 and
//! emacs-mule detectors and the priority walk, compared with pinned GNU
//! live. A full case compares the decision, the decoded characters and the
//! `charset' property; a decision case, for a coding type VM2 has no
//! decoder for (ISO-2022, emacs-mule), compares only the base coding system
//! the walk chose, which VM2's refusal names (GNU settles its EOL variant
//! while decoding).
use romacs_vm::Vm;
use std::path::{Path, PathBuf};
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;

const FULL: &str = r##"(with-temp-buffer (PRIORITY (insert-file-contents FILE)) (list last-coding-system-used (append (buffer-string) nil) (get-text-property 1 'charset)))"##;
const DECISION: &str = r##"(with-temp-buffer (condition-case e (PRIORITY (insert-file-contents FILE) (coding-system-base last-coding-system-used)) (error (if (string-match "decoding with \\([^ ]+\\) is unsupported" (cadr e)) (coding-system-base (intern (match-string 1 (cadr e)))) e))))"##;

fn oracle(expression: &str) -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let source = format!("(prin1 (prin1-to-string {expression}))");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &source])
        .env("LANG", "C.UTF-8").output().unwrap();
    assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn romacs(vm: &mut Vm, expression: &str) -> String {
    match vm.eval_source(&format!("(prin1-to-string {expression})")).and_then(|v| v.display()) {
        Ok(text) => text,
        Err(error) => format!("VM error: {error}"),
    }
}

#[test]
fn insert_file_contents_detects_coding_systems_as_gnu() {
    let dir: PathBuf = std::env::temp_dir().join(format!("romacs-coding-detection-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let files: [(&str, &[u8]); 10] = [
        ("latin", b"caf\xe9\n"),
        ("sjis", b"\x82\xa0\x83A\xb1\r\n"),
        ("big5", b"\xa4@\xa4A\n"),
        ("u16le", b"\xff\xfeA\x00"),
        ("u16be", b"\xfe\xff\x00A"),
        ("nul", b"A\x00B\x00"),
        ("euc", b"\xb0\xa1\xb0\xa2\n"),
        ("iso7", b"\x1b$B$\"\x1b(B"),
        ("mule", b"\x92\xa4\xa2"),
        ("ss2", b"a\x8e\xb1b\n"),
    ];
    for (name, bytes) in files { std::fs::write(dir.join(name), bytes).unwrap(); }
    let cases = [
        (FULL, "latin", "progn"), (FULL, "sjis", "progn"), (FULL, "big5", "with-coding-priority '(chinese-big5)"),
        (FULL, "sjis", "with-coding-priority '(japanese-shift-jis)"), (FULL, "u16le", "progn"), (FULL, "u16be", "progn"),
        (FULL, "nul", "progn"), (FULL, "euc", "progn"),
        (DECISION, "iso7", "progn"), (DECISION, "mule", "with-coding-priority '(emacs-mule)"),
        (DECISION, "euc", "with-coding-priority '(chinese-iso-8bit)"), (DECISION, "ss2", "with-coding-priority '(iso-2022-8bit-ss2)"),
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let failures: Vec<String> = cases.iter()
        .map(|(form, file, priority)| form.replace("PRIORITY", priority)
            .replace("FILE", &serde_json::to_string(dir.join(file).to_str().unwrap()).unwrap()))
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(&expression), romacs(&mut vm, &expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
