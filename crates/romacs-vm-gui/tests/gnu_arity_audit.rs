//! Every function Romacs defines under the name of a GNU DEFUN, native or
//! Lisp, takes GNU's arguments: its `func-arity' in a batch session equals
//! the DEFUN's MIN and MAX at the Lisp pin (tests/fixtures/gnu_defun_arity.json
//! in romacs-vm, from tools/gnu_arity.py).
use std::collections::BTreeMap;
use std::process::Command;

const SCRIPT: &str = r#";;; -*- lexical-binding: t -*-
(with-temp-buffer
  (insert-file-contents "names")
  (dolist (name (split-string (buffer-string) "\n" t))
    (let ((f (indirect-function (intern name))))
      (when (and f (not (autoloadp f)))
        (let ((arity (func-arity f)))
          (princ (format "%s %S %S\n" name (car arity) (cdr arity))))))))
"#;

fn gnu() -> BTreeMap<String, String> {
    let text = include_str!("../../romacs-vm/tests/fixtures/gnu_defun_arity.json");
    let fixture: serde_json::Value = serde_json::from_str(text).unwrap();
    let field = |v: &serde_json::Value| v.as_str().map_or_else(|| v.to_string(), str::to_owned);
    fixture["arity"].as_object().unwrap().iter()
        .map(|(name, a)| (name.clone(), format!("{} {}", field(&a[0]), field(&a[1]))))
        .collect()
}

#[test]
fn every_gnu_defun_romacs_defines_has_gnus_arity() {
    let gnu = gnu();
    let dir = std::env::temp_dir().join(format!("romacs-arity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe.el"), SCRIPT).unwrap();
    std::fs::write(dir.join("names"), gnu.keys().cloned().collect::<Vec<_>>().join("\n")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui")).args(["-Q", "--batch", "-l", "probe.el"])
        .current_dir(&dir).env("HOME", &dir).env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_CACHE_HOME", dir.join("cache")).env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_STATE_HOME", dir.join("state")).env("ROMACS_WORLD", "off")
        .output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let defined: Vec<(&str, &str)> = stdout.lines().filter_map(|l| l.split_once(' ')).collect();
    let wrong: Vec<String> = defined.iter()
        .filter(|(name, arity)| gnu.get(*name).map(String::as_str) != Some(*arity))
        .map(|(name, arity)| format!("{name}: Romacs {arity}, GNU {}", gnu[*name]))
        .collect();
    // The batch runtime defines most DEFUNs; an empty probe proves nothing.
    assert!(defined.len() > 1000, "only {} DEFUN names defined", defined.len());
    assert!(wrong.is_empty(), "arity differs from GNU:\n{}", wrong.join("\n"));
}
