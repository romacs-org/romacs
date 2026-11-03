//! Compiled Lisp across HOMEs (src/batch.rs `compile_environment`): a
//! compile runs in the store's own environment, so sessions under
//! different HOMEs share every artifact, and only a real input change (a
//! macro dependency's source, a variable compiles inherit) compiles again.
//! A compile-time read of HOME gives the store's, as GNU's installed `.elc`
//! gives its builder's.
use std::path::Path;
use std::process::Command;

const MACROS: &str = ";;; -*- lexical-binding: t -*-\n(defmacro t-store-val () 1)\n(provide 't-store-m)\n";

const USER: &str = ";;; -*- lexical-binding: t -*-
(eval-when-compile (require 't-store-m))
(defun t-store-f () (t-store-val))
(defconst t-store-home (eval-when-compile (getenv \"HOME\")))
(defconst t-store-path (eval-when-compile (getenv \"PATH\")))
(defconst t-store-org (eval-when-compile (getenv \"ORGANIZATION\")))
(provide 't-store-u)
";

/// A batch session in the fresh HOME ROOT/NAME with ENV over the store:
/// what it prints.
fn session(root: &Path, name: &str, env: &[(&str, &str)]) -> String {
    let home = root.join(name);
    for dir in ["config", "cache/romacs/worlds", "tmp"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"));
    command.args(["-Q", "--batch", "-L"]).arg(root.join("lib"))
        .args(["--eval", "(progn (require 't-store-u) (princ (list (t-store-f) t-store-home t-store-path t-store-org)))"])
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache")).env("TMPDIR", home.join("tmp")).env("LANG", "C.UTF-8")
        .env("ROMACS_COMPILED_LISP", "on").env("ROMACS_COMPILED_LISP_STORE", root.join("store"))
        .envs(["ROMACS_RUNTIME_ROOT", "ROMACS_LISP_DIR", "LD_LIBRARY_PATH"].iter()
              .filter_map(|name| Some((name, std::env::var_os(name)?))))
        .envs(env.iter().copied());
    let output = command.output().unwrap();
    assert!(output.status.success(), "{name}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The artifacts in ROOT's store, with when each was published: a compile
/// that publishes the same artifact again changes its time.
fn artifacts(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, std::time::SystemTime> {
    let generations = std::fs::read_dir(root.join("store")).unwrap().flatten().filter(|e| e.file_name().len() == 16);
    generations.flat_map(|g| std::fs::read_dir(g.path()).unwrap().flatten())
        .filter(|key| key.path().is_dir())
        .flat_map(|key| std::fs::read_dir(key.path()).unwrap().flatten())
        .filter(|f| f.path().extension().is_some_and(|e| e == "rlc"))
        .map(|f| (f.path(), f.metadata().unwrap().modified().unwrap()))
        .collect()
}

#[test]
fn homes_share_compiled_lisp_and_real_inputs_invalidate_it() {
    let root = std::env::temp_dir().join(format!("romacs-compiled-store-{}", std::process::id()));
    std::fs::create_dir_all(root.join("lib")).unwrap();
    let lib = |name: &str, text: &str| {
        std::fs::write(root.join("lib").join(format!("{name}.el")), text).unwrap();
        // A sibling `.elc`: GNU would load compiled code, so Romacs does.
        std::fs::write(root.join("lib").join(format!("{name}.elc")), "").unwrap();
    };
    lib("t-store-m", MACROS);
    lib("t-store-u", USER);
    let home = root.join("store/home").display().to_string();
    let expected = |value: u8, path: &str| format!("({value} {home} {path} nil)");

    // Only the artifacts that are new, or published again, since BEFORE.
    let compiled = |before: &std::collections::BTreeMap<_, _>| -> usize {
        artifacts(&root).iter().filter(|(path, time)| before.get(*path) != Some(*time)).count()
    };
    let first = session(&root, "a", &[]);
    assert_eq!(first, expected(1, "/usr/bin:/bin"));
    let warm = artifacts(&root);
    // Another HOME, and a variable compiles do not inherit: nothing compiles.
    assert_eq!(session(&root, "b", &[("ORGANIZATION", "elsewhere")]), first);
    assert_eq!(compiled(&warm), 0, "a second HOME shares every artifact");

    // A macro dependency's source changed: it and its user compile again.
    std::fs::write(root.join("lib/t-store-m.el"), MACROS.replace("() 1)", "() 2)")).unwrap();
    assert_eq!(session(&root, "c", &[]), expected(2, "/usr/bin:/bin"));
    assert_eq!(compiled(&warm), 2);
    let warm = artifacts(&root);
    assert_eq!(session(&root, "d", &[]), expected(2, "/usr/bin:/bin"));
    assert_eq!(compiled(&warm), 0);

    // An inherited variable the compile read changed: its reader compiles again.
    let path = "/usr/local/bin:/usr/bin:/bin";
    assert_eq!(session(&root, "e", &[("PATH", path)]), expected(2, path));
    assert_eq!(compiled(&warm), 1);
    let warm = artifacts(&root);
    assert_eq!(session(&root, "f", &[("PATH", path)]), expected(2, path));
    assert_eq!(compiled(&warm), 0, "the PATH a compile read holds again");
    let _ = std::fs::remove_dir_all(&root);
}
