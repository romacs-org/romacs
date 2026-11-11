//! load-with-code-conversion for a source file without a coding cookie
//! that is not UTF-8: the coding priority's detection decides, as for
//! `insert-file-contents' (Latin-1, raw-text as for leim/quail/Punct.el,
//! Shift-JIS), and strings read from it are what pinned GNU reads.
use romacs_vm::Vm;
use std::path::{Path, PathBuf};
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;

const PROBE: &str = "(list (multibyte-string-p sd-s) (append sd-s nil) (symbol-name sd-sym))";

#[test]
fn cookieless_non_utf8_sources_decode_as_gnu_detects() {
    let dir: PathBuf = std::env::temp_dir().join(format!("romacs-source-detection-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let files: [(&str, &[u8]); 3] = [
        ("latin.el", b"(setq sd-s \"caf\xe9\" sd-sym 'latin)\n"),
        ("raw.el", b"(setq sd-s \"\xe6\xb1\x89 \xa3\xa0\" sd-sym 'raw)\n;; \xa3\xa0\n"),
        ("sjis.el", b"(setq sd-s \"\x82\xa0x\x85y\" sd-sym 'sjis)\n"),
    ];
    for (name, bytes) in files { std::fs::write(dir.join(name), bytes).unwrap(); }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "lisp/romacs/romacs-loadup-bootstrap.el");
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for (name, _) in files {
        let file = serde_json::to_string(dir.join(name).to_str().unwrap()).unwrap();
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(progn (load {file} nil t t) (prin1 (prin1-to-string {PROBE})))")])
            .env("LANG", "C.UTF-8").output().unwrap();
        let gnu = String::from_utf8(output.stdout).unwrap();
        let loaded = vm.load_file(&dir.join(name)).map(|_| ());
        let ours = match loaded.and_then(|()| vm.eval_source(&format!("(prin1-to-string {PROBE})"))).and_then(|v| v.display()) {
            Ok(text) => text,
            Err(error) => format!("VM error: {error}"),
        };
        if gnu != ours { failures.push(format!("{name}\n  gnu:    {gnu}\n  romacs: {ours}")); }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
