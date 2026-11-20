//! `romacs-compiled-lisp-stats` (crates/romacs-vm/src/prims/compiled_lisp.rs) counts
//! what a session's loads took: the first session over an empty store
//! compiles the library it requires, the next takes that artifact and
//! compiles nothing, and one with compiled Lisp off reads source, with no
//! store and no identity.
use std::path::Path;
use std::process::Command;

const LIBRARY: &str = ";;; -*- lexical-binding: t -*-\n(defun t-stats-f () 1)\n(provide 't-stats-u)\n";

/// What the session prints of its stats after requiring the library:
/// (TAKEN COMPILED SOURCE-ON-MISS SOURCE-OFF IDENTITY-KNOWN STORE).
const FORM: &str = "(progn (require 't-stats-u)
  (let ((s (romacs-compiled-lisp-stats)))
    (prin1 (list (plist-get s :taken) (plist-get s :compiled) (plist-get s :source-on-miss)
                 (plist-get s :source-off) (stringp (plist-get s :identity)) (plist-get s :store)))))";

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
    let (stdout, stderr) = (String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{name}: {stderr}");
    let fields: Vec<&str> = stdout.trim().trim_start_matches('(').trim_end_matches(')').splitn(6, ' ').collect();
    let [taken, compiled, on_miss, off, identity, store] = fields.as_slice() else { panic!("{name}: {stdout}\n{stderr}") };
    let count = |field: &str| field.parse::<u64>().unwrap_or_else(|_| panic!("{name}: {stdout}"));
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
    let _ = std::fs::remove_dir_all(&root);
}
