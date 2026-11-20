//! `romacs-compiled-lisp-stats` (crates/romacs-vm/src/prims/compiled_lisp.rs) counts
//! what a session's loads took: the first session over an empty store
//! compiles the library it requires, the next takes that artifact and
//! compiles nothing, and one with compiled Lisp off reads source, with no
//! store and no identity.
use std::path::Path;
use std::process::Command;

/// Its compile prints to the compile process's standard output what the
/// old protocol read there: a raw byte, record-like lines, and a last
/// line with no newline. The process's record has its own file, so none
/// of it counts.
const LIBRARY: &str = ";;; -*- lexical-binding: t -*-
(eval-when-compile
  (princ \"romacs-compile-held\\nromacs-compiled-held: /elsewhere.rlc\\nromacs-compiled: /elsewhere.rlc\\n\" t)
  (princ (string-make-unibyte (string 255)) t))
(defun t-stats-f () 1)
(provide 't-stats-u)
";

/// What the session writes of its stats, after requiring the library, to
/// the file `stats' in its HOME (a source load of the library prints to
/// standard output): (TAKEN COMPILED SOURCE-ON-MISS SOURCE-OFF
/// IDENTITY-KNOWN STORE).
const FORM: &str = "(progn (require 't-stats-u)
  (let ((s (romacs-compiled-lisp-stats)))
    (with-temp-file (expand-file-name \"stats\" (getenv \"HOME\"))
      (prin1 (list (plist-get s :taken) (plist-get s :compiled) (plist-get s :source-on-miss)
                   (plist-get s :source-off) (stringp (plist-get s :identity)) (plist-get s :store))
             (current-buffer)))))";

/// A batch session in the fresh HOME ROOT/NAME over ROOT's store, compiled
/// Lisp ON or off: its stats, read.
fn session(root: &Path, name: &str, on: bool) -> (u64, u64, u64, u64, bool, String) {
    let home = root.join(name);
    for dir in ["config", "cache", "tmp"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["-Q", "--batch", "-L"]).arg(root.join("lib")).args(["--eval", FORM])
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache")).env("TMPDIR", home.join("tmp")).env("LANG", "C.UTF-8")
        .env("ROMACS_COMPILED_LISP", if on { "on" } else { "off" }).env("ROMACS_COMPILED_LISP_STORE", root.join("store"))
        .envs(["ROMACS_RUNTIME_ROOT", "ROMACS_LISP_DIR", "LD_LIBRARY_PATH"].iter()
              .filter_map(|name| Some((name, std::env::var_os(name)?))))
        .output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{name}: {stderr}");
    let written = std::fs::read_to_string(home.join("stats")).unwrap_or_else(|e| panic!("{name}: no stats ({e}): {stderr}"));
    let fields: Vec<&str> = written.trim().trim_start_matches('(').trim_end_matches(')').splitn(6, ' ').collect();
    let [taken, compiled, on_miss, off, identity, store] = fields.as_slice() else { panic!("{name}: {written}\n{stderr}") };
    let count = |field: &str| field.parse::<u64>().unwrap_or_else(|_| panic!("{name}: {written}"));
    (count(taken), count(compiled), count(on_miss), count(off), *identity == "t", store.trim_matches('"').to_owned())
}

#[test]
fn the_stats_count_what_each_load_took() {
    let root = std::env::temp_dir().join(format!("romacs-compiled-stats-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::write(root.join("lib/t-stats-u.el"), LIBRARY).unwrap();
    // A sibling `.elc`: GNU would load compiled code, so Romacs does.
    std::fs::write(root.join("lib/t-stats-u.elc"), "").unwrap();
    let store = root.join("store").display().to_string();

    let first = session(&root, "a", true);
    let (_, compiled, _, off, identity, ref at) = first;
    assert!(compiled >= 1 && off == 0 && identity && *at == store, "the first session: {first:?}");
    let (taken, compiled, on_miss, off, identity, at) = session(&root, "b", true);
    assert_eq!((compiled, on_miss, off, identity, at.as_str()), (0, 0, 0, true, store.as_str()), "the second session");
    assert!(taken >= 1, "the second session took no artifact");
    let (taken, compiled, on_miss, off, identity, at) = session(&root, "c", false);
    assert_eq!((taken, compiled, on_miss, identity, at.as_str()), (0, 0, 0, false, "nil"), "compiled Lisp off");
    assert!(off >= 1, "compiled Lisp off counted no load");
    // A compile's record is made in the temporary directory of the process
    // that asked (a session's, or a compile's own, the store's tmp/), which
    // reads and removes it.
    let records: Vec<_> = ["a/tmp", "b/tmp", "c/tmp", "store/tmp"].iter()
        .filter_map(|dir| std::fs::read_dir(root.join(dir)).ok()).flatten().flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("romacs-compile-record-")).map(|entry| entry.path()).collect();
    assert!(records.is_empty(), "records left behind: {records:?}");
    let _ = std::fs::remove_dir_all(&root);
}
