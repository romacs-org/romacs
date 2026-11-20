//! `romacs-compiled-lisp-stats` (crates/romacs-vm/src/prims/compiled_lisp.rs) counts
//! what a session's loads took: the first session over an empty store
//! compiles the library it requires, the next takes that artifact and
//! compiles nothing, and one with compiled Lisp off reads source, with no
//! store and no identity. A compile process's record has a file of its own
//! (`compiled_lisp::write_record`), which nothing the compiled file prints
//! reaches, and which none of its compile-time code can read.
use std::path::Path;
use std::process::Command;

/// Its compile prints to the compile process's standard output what the
/// old protocol read there: a raw byte, record-like lines, and a last
/// line with no newline. The process's record has its own file, so none
/// of it counts. It keeps what its compile saw of the record's variable.
const LIBRARY: &str = ";;; -*- lexical-binding: t -*-
(eval-when-compile
  (princ \"romacs-compile-held\\nromacs-compiled-held: /elsewhere.rlc\\nromacs-compiled: /elsewhere.rlc\\n\" t)
  (princ (string-make-unibyte (string 255)) t))
(defconst t-stats-env (eval-when-compile (getenv \"ROMACS_COMPILE_RECORD\")))
(defun t-stats-f () 1)
(provide 't-stats-u)
";

/// What the session writes of its stats, after requiring the library, to
/// the file `stats' in its HOME (a source load of the library prints to
/// standard output): (TAKEN COMPILED SOURCE-ON-MISS SOURCE-OFF
/// IDENTITY-KNOWN RECORD-SEEN STORE).
const FORM: &str = "(progn (require 't-stats-u)
  (let ((s (romacs-compiled-lisp-stats)))
    (with-temp-file (expand-file-name \"stats\" (getenv \"HOME\"))
      (prin1 (list (plist-get s :taken) (plist-get s :compiled) (plist-get s :source-on-miss)
                   (plist-get s :source-off) (stringp (plist-get s :identity)) (and t-stats-env t)
                   (plist-get s :store))
             (current-buffer)))))";

/// A session's stats, as it wrote them.
#[derive(Debug)]
struct Stats {
    taken: u64,
    compiled: u64,
    on_miss: u64,
    off: u64,
    identity: bool,
    /// Whether the library's compile saw the record's variable.
    record_seen: bool,
    store: String,
}

/// How a session's temporary directory is named.
#[derive(Clone, Copy, PartialEq)]
enum Tmp {
    Absolute,
    /// `tmp', relative to the session's directory, its HOME.
    Relative,
}

/// A batch session in the fresh HOME ROOT/NAME over ROOT's store, compiled
/// Lisp ON or off, its TMPDIR named as TMP: its stats.
fn session(root: &Path, name: &str, on: bool, tmp: Tmp) -> Stats {
    let home = root.join(name);
    for dir in ["config", "cache", "tmp"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"));
    command.args(["-Q", "--batch", "-L"]).arg(root.join("lib")).args(["--eval", FORM])
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache")).env("LANG", "C.UTF-8")
        .env("ROMACS_COMPILED_LISP", if on { "on" } else { "off" }).env("ROMACS_COMPILED_LISP_STORE", root.join("store"))
        .envs(["ROMACS_RUNTIME_ROOT", "ROMACS_LISP_DIR", "LD_LIBRARY_PATH"].iter()
              .filter_map(|name| Some((name, std::env::var_os(name)?))));
    match tmp {
        Tmp::Absolute => command.env("TMPDIR", home.join("tmp")),
        Tmp::Relative => command.current_dir(&home).env("TMPDIR", "tmp"),
    };
    let output = command.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{name}: {stderr}");
    let written = std::fs::read_to_string(home.join("stats")).unwrap_or_else(|e| panic!("{name}: no stats ({e}): {stderr}"));
    let fields: Vec<&str> = written.trim().trim_start_matches('(').trim_end_matches(')').splitn(7, ' ').collect();
    let [taken, compiled, on_miss, off, identity, record_seen, store] = fields.as_slice() else { panic!("{name}: {written}\n{stderr}") };
    let count = |field: &str| field.parse::<u64>().unwrap_or_else(|_| panic!("{name}: {written}"));
    Stats { taken: count(taken), compiled: count(compiled), on_miss: count(on_miss), off: count(off),
            identity: *identity == "t", record_seen: *record_seen == "t", store: store.trim_matches('"').to_owned() }
}

/// A fresh ROOT with the library, beside an `.elc': GNU would load
/// compiled code, so Romacs does.
fn library_root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("romacs-compiled-stats-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::write(root.join("lib/t-stats-u.el"), LIBRARY).unwrap();
    std::fs::write(root.join("lib/t-stats-u.elc"), "").unwrap();
    root
}

/// The compile records left in ROOT's directories DIRS.
fn records_left(root: &Path, dirs: &[&str]) -> Vec<std::path::PathBuf> {
    dirs.iter().filter_map(|dir| std::fs::read_dir(root.join(dir)).ok()).flatten().flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("romacs-compile-record-"))
        .map(|entry| entry.path()).collect()
}

#[test]
fn the_stats_count_what_each_load_took() {
    let root = library_root("count");
    let store = root.join("store").display().to_string();
    let first = session(&root, "a", true, Tmp::Absolute);
    assert!(first.compiled >= 1 && first.off == 0 && first.identity && first.store == store, "the first session: {first:?}");
    assert!(!first.record_seen, "the library's compile saw the record's variable: {first:?}");
    let second = session(&root, "b", true, Tmp::Absolute);
    assert_eq!((second.compiled, second.on_miss, second.off, second.identity, second.store.as_str()),
               (0, 0, 0, true, store.as_str()), "the second session: {second:?}");
    assert!(second.taken >= 1, "the second session took no artifact: {second:?}");
    let off = session(&root, "c", false, Tmp::Absolute);
    assert_eq!((off.taken, off.compiled, off.on_miss, off.identity, off.store.as_str()), (0, 0, 0, false, "nil"),
               "compiled Lisp off: {off:?}");
    assert!(off.off >= 1, "compiled Lisp off counted no load: {off:?}");
    // A compile's record is made in the temporary directory of the process
    // that asked (a session's, or a compile's own, the store's tmp/), which
    // reads and removes it.
    let left = records_left(&root, &["a/tmp", "b/tmp", "c/tmp", "store/tmp"]);
    assert!(left.is_empty(), "records left behind: {left:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A session whose TMPDIR is relative to its directory names its compile's
/// record absolutely: the compile process works in another directory.
#[test]
fn a_relative_tmpdir_still_carries_the_record() {
    let root = library_root("relative");
    let stats = session(&root, "a", true, Tmp::Relative);
    assert!(stats.compiled >= 1 && stats.on_miss == 0, "the compile's record was lost: {stats:?}");
    let left = records_left(&root, &["a/tmp", "store/tmp"]);
    assert!(left.is_empty(), "records left behind: {left:?}");
    let _ = std::fs::remove_dir_all(&root);
}
