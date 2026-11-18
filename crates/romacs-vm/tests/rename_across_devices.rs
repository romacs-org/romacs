//! `rename-file` across filesystems (fileio.c Frename_file on EXDEV): the
//! file is copied as what it is and then deleted, as pinned GNU does, and
//! `backup-buffer`'s rename backup crosses filesystems with it. Each case
//! runs in pinned GNU and here on the same fixtures, between a tmpfs
//! directory (`/run/user/UID`, else `/dev/shm`) and one under the test's
//! temporary directory (on /home), and compares the value or signal and
//! both directories afterwards: types, modes, contents, link targets, times.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// A tmpfs directory and a /home one, on different filesystems.
fn roots(name: &str) -> (PathBuf, PathBuf) {
    let home = std::env::temp_dir().join(format!("romacs-rename-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let device = |p: &Path| std::fs::metadata(p).map(|m| m.dev()).ok();
    let uid = std::fs::metadata(&home).unwrap().uid();
    let tmpfs = [PathBuf::from(format!("/run/user/{uid}")), PathBuf::from("/dev/shm")].into_iter()
        .find(|p| device(p).is_some_and(|d| Some(d) != device(&home)))
        .expect("a tmpfs directory on another filesystem than the temporary directory");
    let tmpfs = tmpfs.join(format!("romacs-rename-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmpfs);
    std::fs::create_dir_all(&tmpfs).unwrap();
    (tmpfs, home)
}

fn clear(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700));
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
}

/// The fixture: in A (tmpfs) a file, a directory with a file, a relative
/// symlink, an unreadable file, a FIFO, a directory holding a regular file
/// then a FIFO, and a file in a directory that denies unlinking; in B
/// (/home) a file, an empty directory and a symlink back to A's file.
/// Times are fixed so a kept time shows.
fn fixture(a: &Path, b: &Path) {
    clear(a);
    clear(b);
    let at = |p: &Path, s: u64| std::fs::File::options().write(true).open(p).unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(s)).unwrap();
    std::fs::write(a.join("f"), "file\n").unwrap();
    std::fs::set_permissions(a.join("f"), std::fs::Permissions::from_mode(0o640)).unwrap();
    at(&a.join("f"), 1_500_000_000);
    std::fs::create_dir(a.join("d")).unwrap();
    std::fs::write(a.join("d/inner"), "inner\n").unwrap();
    at(&a.join("d/inner"), 1_400_000_000);
    std::os::unix::fs::symlink("f", a.join("l")).unwrap();
    std::fs::write(a.join("locked"), "secret\n").unwrap();
    std::fs::set_permissions(a.join("locked"), std::fs::Permissions::from_mode(0)).unwrap();
    assert!(std::process::Command::new("mkfifo").arg(a.join("fifo")).status().unwrap().success());
    std::fs::create_dir(a.join("mixed")).unwrap();
    std::fs::write(a.join("mixed/a"), "a\n").unwrap();
    at(&a.join("mixed/a"), 1_300_000_000);
    assert!(std::process::Command::new("mkfifo").arg(a.join("mixed/p")).status().unwrap().success());
    std::fs::create_dir(a.join("ro")).unwrap();
    std::fs::write(a.join("ro/f"), "kept\n").unwrap();
    at(&a.join("ro/f"), 1_200_000_000);
    std::fs::set_permissions(a.join("ro"), std::fs::Permissions::from_mode(0o555)).unwrap();
    std::fs::write(b.join("exists"), "old\n").unwrap();
    std::fs::create_dir(b.join("dir")).unwrap();
    std::os::unix::fs::symlink(a.join("f"), b.join("back")).unwrap();
}

/// DIR's entries, sorted, each with its type, mode, content or link target
/// and, for a file, its modification time when it is a fixture's.
fn snapshot(base: &Path, at: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(at) else { return };
    let mut entries: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        let name = path.strip_prefix(base).unwrap().display().to_string();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o7777;
        if meta.file_type().is_symlink() {
            out.push(format!("{name} -> {}", std::fs::read_link(&path).unwrap().display()));
        } else if meta.is_dir() {
            out.push(format!("{name}/ {mode:o}"));
            snapshot(base, &path, out);
        } else if meta.is_file() {
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| format!("<{e}>"));
            // A file this run wrote has a time of its own: only a kept one compares.
            let time = if meta.mtime() < 1_600_000_000 { meta.mtime().to_string() } else { "new".into() };
            out.push(format!("{name} {mode:o} {text:?} mtime {time}"));
        } else {
            out.push(format!("{name} special {mode:o}"));
        }
    }
}

fn state(a: &Path, b: &Path) -> Vec<String> {
    let mut out = vec!["A:".into()];
    snapshot(a, a, &mut out);
    out.push("B:".into());
    snapshot(b, b, &mut out);
    out
}

/// EXPRESSION's value or signal, printed as character codes so the two
/// printers' string escapes cannot differ.
fn wrapped(expression: &str) -> String {
    format!("(string-to-list (prin1-to-string (condition-case err (list 'value {expression}) (error (list 'signal err)))))")
}

fn text(codes: &str) -> String {
    codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
        .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect()
}

/// Run each of CASES (with A and B in them) in pinned GNU and here, each on
/// a fresh fixture, and say how they differ.
fn compare(name: &str, cases: &[&str]) -> Vec<String> {
    let (a, b) = roots(name);
    let fill = |case: &str| case.replace("A/", &format!("{}/", a.display())).replace("B/", &format!("{}/", b.display()));
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = gnu_lisp::gnu_lisp(&[]);
    let mut failures = Vec::new();
    for case in cases {
        let expression = fill(case);
        fixture(&a, &b);
        let output = std::process::Command::new(&oracle).args(["-Q", "--batch", "--eval", &format!("(prin1 {})", wrapped(&expression))])
            .stdin(std::process::Stdio::null()).output().unwrap();
        let gnu = (text(&String::from_utf8_lossy(&output.stdout)), state(&a, &b));
        fixture(&a, &b);
        let romacs = (vm.eval_source(&wrapped(&expression)).and_then(romacs_vm::Local::display).map(|v| text(&v)).unwrap_or_else(|e| e), state(&a, &b));
        if gnu != romacs { failures.push(format!("{case}\n  GNU    {:?}\n  Romacs {:?}", gnu, romacs)); }
    }
    clear(&a);
    clear(&b);
    let _ = std::fs::remove_dir(&a);
    let _ = std::fs::remove_dir_all(&b);
    failures
}

#[test]
fn rename_file_across_filesystems_matches_gnu() {
    let failures = compare("cases", &[
        // A file: copied keeping its mode and time, then deleted.
        "(rename-file \"A/f\" \"B/g\")",
        // An existing target: refused, replaced when allowed, also when the
        // permission is a number (GNU asks only when the rename reports
        // the target, which a cross-device rename does not).
        "(rename-file \"A/f\" \"B/exists\")",
        "(rename-file \"A/f\" \"B/exists\" t)",
        "(rename-file \"A/f\" \"B/exists\" 1)",
        // Into a directory named as one.
        "(rename-file \"A/f\" \"B/dir/\")",
        // A directory, with and without its slash.
        "(rename-file \"A/d\" \"B/d2\")",
        "(rename-file \"A/d/\" \"B/d3\")",
        // A symlink: a new link to the same target.
        "(rename-file \"A/l\" \"B/l2\")",
        // A source it cannot read: the copy's error, the source kept.
        "(rename-file \"A/locked\" \"B/x\")",
        // A FIFO: the rename's own error.
        "(rename-file \"A/fifo\" \"B/p\")",
        // A missing source.
        "(rename-file \"A/missing\" \"B/x\")",
        // The other way, /home to tmpfs.
        "(rename-file \"B/exists\" \"A/h\")",
        // A failure after the destination is made: a directory whose
        // regular child is copied before its FIFO fails.
        "(rename-file \"A/mixed\" \"B/mixed\")",
        // A failure in the delete: the copy is made, the source's directory
        // denies unlinking it.
        "(rename-file \"A/ro/f\" \"B/moved\")",
    ]);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The backup setup: rename backups, never copies, into B/backups/.
const RENAME_BACKUPS: &str = "(backup-directory-alist (list (cons \".\" \"B/backups/\"))) (make-backup-files t)
    (backup-by-copying nil) (backup-by-copying-when-linked nil) (backup-by-copying-when-mismatch nil)
    (backup-by-copying-when-privileged-mismatch nil) (file-precious-flag nil) (version-control nil)";

/// A save of a file on tmpfs with backups under /home makes its backup by
/// renaming the old file across filesystems, as GNU's does, and saves.
#[test]
fn a_backup_crosses_filesystems() {
    let session = format!("(let* ({RENAME_BACKUPS} (b (find-file-noselect \"A/f\")))
           (with-current-buffer b
             (goto-char (point-max)) (insert \"one\\n\") (save-buffer)
             (set-buffer-modified-p nil))
           (kill-buffer b)
           (list (current-message) (with-temp-buffer (insert-file-contents \"A/f\") (buffer-string))
                 (mapcar (lambda (f) (with-temp-buffer (insert-file-contents f) (buffer-string)))
                         (directory-files \"B/backups/\" t \"~\\\\'\"))))");
    let failures = compare("backup", &[&session]);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // And the configured backup holds the original bytes, here as in GNU.
    let (a, b) = roots("backup-bytes");
    fixture(&a, &b);
    let mut vm = gnu_lisp::gnu_lisp(&[]);
    let fill = |case: &str| case.replace("A/", &format!("{}/", a.display())).replace("B/", &format!("{}/", b.display()));
    let value = vm.eval_source(&fill(&session)).and_then(romacs_vm::Local::display).unwrap();
    assert!(value.ends_with("(\"file\\n\"))"), "the backup holds the original bytes: {value}");
    assert_eq!(std::fs::read_to_string(a.join("f")).unwrap(), "file\none\n");
    clear(&a);
    clear(&b);
    let _ = std::fs::remove_dir(&a);
    let _ = std::fs::remove_dir_all(&b);
}

/// A destination that is the source through a link is refused before
/// anything is truncated (S1): through `rename-file` across filesystems
/// (B/back is a symlink on /home to A/f on tmpfs) and through plain
/// `copy-file` with OK-IF-ALREADY-EXISTS. GNU truncates the source first
/// and loses its bytes (ledger/divergences/VM2-COPY-FILE-SAME-FILE.md).
#[test]
fn a_destination_aliasing_the_source_keeps_its_bytes() {
    let (a, b) = roots("alias");
    let mut vm = gnu_lisp::gnu_lisp(&[]);
    for case in ["(rename-file \"A/f\" \"B/back\" t)", "(copy-file \"A/f\" \"B/back\" t)",
                 "(copy-file \"B/back\" \"A/f\" t)"] {
        fixture(&a, &b);
        let expression = case.replace("A/", &format!("{}/", a.display())).replace("B/", &format!("{}/", b.display()));
        let value = vm.eval_source(&wrapped(&expression)).and_then(romacs_vm::Local::display).map(|v| text(&v)).unwrap();
        assert!(value.contains("Input and output files are the same"), "{case}: {value}");
        assert_eq!(std::fs::read_to_string(a.join("f")).unwrap(), "file\n", "{case}: the source's bytes");
        assert!(std::fs::symlink_metadata(b.join("back")).unwrap().file_type().is_symlink(), "{case}: the link stays");
    }
    clear(&a);
    clear(&b);
    let _ = std::fs::remove_dir(&a);
    let _ = std::fs::remove_dir_all(&b);
}

/// Advice on `copy-file` and `make-symbolic-link` that returns without
/// copying cannot make a cross-device rename delete its source with no
/// destination (S2): the rename calls the primitives, as GNU's C does.
#[test]
fn advice_cannot_skip_the_copy_before_the_delete() {
    let (a, b) = roots("advice");
    let mut vm = gnu_lisp::gnu_lisp(&[]);
    vm.eval_source("(progn (defun t-skip (&rest _) nil)
                           (advice-add 'copy-file :override #'t-skip)
                           (advice-add 'make-symbolic-link :override #'t-skip))").unwrap();
    for (case, moved) in [("(rename-file \"A/f\" \"B/g\")", "g"), ("(rename-file \"A/l\" \"B/l2\")", "l2")] {
        fixture(&a, &b);
        let expression = case.replace("A/", &format!("{}/", a.display())).replace("B/", &format!("{}/", b.display()));
        vm.eval_source(&expression).unwrap();
        assert!(std::fs::symlink_metadata(b.join(moved)).is_ok(), "{case}: the destination exists");
    }
    vm.eval_source("(progn (advice-remove 'copy-file #'t-skip) (advice-remove 'make-symbolic-link #'t-skip))").unwrap();
    clear(&a);
    clear(&b);
    let _ = std::fs::remove_dir(&a);
    let _ = std::fs::remove_dir_all(&b);
}
