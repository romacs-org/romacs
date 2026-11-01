//! `romacs-vm-gui [-Q] --batch ARGS...` against pinned GNU's `emacs [-Q]
//! --batch ARGS...`: the command line packages spawn through
//! `invocation-name` (startup.el `command-line-1`'s `-L`, `-l`, `-f` and
//! `--eval`, `command-line-args-left`, `kill-emacs` and its hook, and the
//! batch report of an uncaught error). Each case runs both in the same
//! private HOME and directory and compares the exit status and stdout; an
//! error case compares the status and the message, stderr's last line,
//! since GNU's batch backtrace (on stdout) is VM2-ERRTEXT1; a case of both
//! compares the status, stdout and stderr.
//! The HOME holds an init file, which a batch session never loads. A
//! Romacs session boots from its batch world in that HOME's cache
//! (src/batch.rs), which `--world-build` prepares first.
use std::path::Path;
use std::process::Command;

const LIBRARY: &str = r#"(defun t-batch ()
  (princ (format "args=%S\n" command-line-args-left))
  (setq command-line-args-left nil)
  (kill-emacs 7))
(provide 't-batch)
"#;

const ORDER: &str = "(princ \"b\")\n";

const BAD: &str = "(error \"in load %d\" 2)\n";

const INIT: &str = "(provide 't-init)\n(princ \"init loaded\\n\")\n";

#[derive(Clone, Copy, PartialEq)]
enum Compare { Stdout, LastStderr, Both }

struct Case { args: Vec<String>, compare: Compare }

fn cases(lib: &Path) -> Vec<Case> {
    use Compare::*;
    let (order, bad) = (lib.join("t-order.el").display().to_string(), lib.join("t-bad.el").display().to_string());
    let lib = lib.display().to_string();
    let case = |args: &[&str], stderr| Case { args: args.iter().map(|a| a.to_string()).collect(), compare: if stderr { LastStderr } else { Stdout } };
    let both = |args: &[&str]| Case { args: args.iter().map(|a| a.to_string()).collect(), compare: Both };
    vec![
        case(&["-Q", "--batch", "--eval",
               "(progn (add-hook 'kill-emacs-hook (lambda () (princ \"hook\\n\"))) (princ (format \"%S %S\\n\" noninteractive command-line-args-left)))",
               "-f", "ignore", "--eval", "(kill-emacs 3)"], false),
        case(&["-Q", "--batch", "-L", &lib, "-l", "t-batch", "-f", "t-batch", "one", "two"], false),
        case(&["-Q", "--batch", "-L", "lib", "--eval", "(progn (require 't-batch) (princ (featurep 't-batch)))"], false),
        case(&["--batch", "-Q", "--eval", "(error \"boom %d\" 1)"], true),
        case(&["-Q", "--batch"], false),
        // fileio.c's stream DEFUNs on the process's own streams: stdin
        // closed, stdout flushed and still written.
        case(&["-Q", "--batch", "--eval", "(progn (princ \"a\") (princ (list (set-binary-mode 'stdout t) (file--close-stream 'stdin) (unix-sync))))"], false),
        // files.el's save-buffer on a visit: the first save backs the
        // file up to bk~ and sets `buffer-backed-up'; the second keeps that
        // backup. The directory is under `temporary-file-directory', which
        // the default predicate exempts.
        case(&["-Q", "--batch", "--eval",
               "(let ((backup-enable-predicate #'always)) (with-temp-file \"bk\" (insert \"one\\n\")) (with-current-buffer (find-file-noselect \"bk\") (insert \"two\\n\") (save-buffer) (insert \"three\\n\") (save-buffer) (princ (list buffer-backed-up (local-variable-p 'buffer-backed-up) (with-temp-buffer (insert-file-contents \"bk~\") (buffer-string))))))"], false),
        // command-line's timestamps, which kill-emacs-hook functions read.
        case(&["-Q", "--batch", "--eval", "(princ (list (consp before-init-time) (time-less-p before-init-time after-init-time)))"], false),
        // `message' to stderr in order, stdout empty.
        both(&["-Q", "--batch", "--eval", "(progn (message \"m1\") (message \"m%d\" 2))"]),
        // -l and --eval in command-line order.
        case(&["-Q", "--batch", "--eval", "(princ \"a\")", "-l", &order, "--eval", "(princ \"c\n\")"], false),
        // A batch session has no window system, and one initial frame.
        case(&["-Q", "--batch", "--eval", "(princ (list noninteractive window-system initial-window-system (length (frame-list)) (display-graphic-p)))"], false),
        // Without -Q: still batch, still no init file.
        case(&["--batch", "--eval", "(princ (list noninteractive init-file-user user-init-file (featurep 't-init)))"], false),
        case(&["-q", "--batch", "--eval", "(princ (list noninteractive init-file-user (featurep 't-init)))"], false),
        // kill-emacs's status, taken modulo 256 by the host, and no argument.
        case(&["-Q", "--batch", "--eval", "(kill-emacs 300)"], false),
        case(&["-Q", "--batch", "--eval", "(kill-emacs)"], false),
        // An error in a loaded file reports and exits 255.
        case(&["-Q", "--batch", "-l", &bad], true),
    ]
}

/// BINARY with ARGS in DIR, its HOME; a Romacs session from its world in
/// DIR's cache, or from source without WORLD.
fn run(binary: &str, dir: &Path, args: &[String], world: bool) -> (Option<i32>, String, String) {
    let mut command = Command::new(binary);
    command.args(args).current_dir(dir)
        .env("HOME", dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data")).env("XDG_STATE_HOME", dir.join("state"));
    if !world { command.env("ROMACS_WORLD", "off"); }
    let output = command.output().unwrap();
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
    (output.status.code(), text(output.stdout), text(output.stderr))
}

#[test]
fn batch_command_line_matches_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = env!("CARGO_BIN_EXE_romacs-vm-gui");
    let dir = std::env::temp_dir().join(format!("romacs-batch-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/t-batch.el"), LIBRARY).unwrap();
    std::fs::write(dir.join("lib/t-order.el"), ORDER).unwrap();
    std::fs::write(dir.join("lib/t-bad.el"), BAD).unwrap();
    std::fs::create_dir_all(dir.join(".emacs.d")).unwrap();
    std::fs::write(dir.join(".emacs.d/init.el"), INIT).unwrap();
    world_build(&dir);
    let runs: Vec<_> = cases(&dir.join("lib")).into_iter().map(|case| {
        let (gnu, dir) = (gnu.clone(), dir.clone());
        std::thread::spawn(move || {
            let expected = run(&gnu, &dir, &case.args, true);
            let actual = run(romacs, &dir, &case.args, true);
            (case, expected, actual)
        })
    }).collect();
    let mut failures = Vec::new();
    for handle in runs {
        let (case, expected, actual) = handle.join().unwrap();
        let last = |stderr: &str| stderr.lines().last().unwrap_or("").to_owned();
        let shown = |(status, stdout, stderr): &(Option<i32>, String, String)|
            match case.compare {
                Compare::Stdout => (*status, stdout.clone(), String::new()),
                Compare::LastStderr => (*status, String::new(), last(stderr)),
                Compare::Both => (*status, stdout.clone(), stderr.clone()),
            };
        if shown(&expected) != shown(&actual) {
            failures.push(format!("{:?}\n  GNU    {:?}\n  Romacs {:?}\n  Romacs stderr {}", case.args, shown(&expected), shown(&actual), actual.2));
        }
    }
    // The GUI's world and the batch world, and the batch world's use file,
    // which only a restore makes.
    let worlds = (stored(&dir, "rwi").len(), stored(&dir, "use").len());
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(worlds, (2, 1), "the sessions restored the batch world --world-build prepared");
}

/// `--world-build` in DIR, the HOME `run` gives: the GUI's world and the
/// batch world of `run`'s environment.
fn world_build(dir: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui")).args(["--loadup", "--world-build"]).current_dir(dir)
        .env("HOME", dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data")).env("XDG_STATE_HOME", dir.join("state"))
        .env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY").env_remove("ROMACS_WORLD").output().unwrap();
    assert!(output.status.success(), "--world-build: {}", String::from_utf8_lossy(&output.stderr));
}

/// The files of DIR's default store with SUFFIX, sorted.
fn stored(dir: &Path, suffix: &str) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir.join("cache/romacs/worlds")).map(|entries| entries.flatten().map(|e| e.path())
        .filter(|path| path.extension().is_some_and(|e| e == suffix)).collect()).unwrap_or_default();
    files.sort();
    files
}

/// Startup from a HOME and directory whose parent holds more entries than a
/// listing bound: GNU asks no directory whether names fold case, so loadup
/// and startup's `abbreviate-file-name` answer as beside any other.
#[test]
fn batch_startup_beside_a_huge_directory_matches_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = env!("CARGO_BIN_EXE_romacs-vm-gui");
    let dir = std::env::temp_dir().join(format!("romacs-batch-huge-{}", std::process::id()));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    for index in 0..4100 {
        std::fs::write(dir.join(format!("entry-{index}")), b"").unwrap();
    }
    let args: Vec<String> = ["-Q", "--batch", "--eval",
        "(princ (format \"%S\\n\" (list (file-name-case-insensitive-p default-directory) (abbreviate-file-name default-directory) (file-name-case-insensitive-p \"../entry-7\"))))"]
        .iter().map(|a| a.to_string()).collect();
    let expected = run(&gnu, &home, &args, false);
    let actual = run(romacs, &home, &args, false);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!((expected.0, expected.1.as_str()), (Some(0), "(nil \"~/\" nil)\n"), "GNU {expected:?}");
    assert_eq!((actual.0, actual.1.as_str()), (expected.0, expected.1.as_str()), "Romacs stderr {}", actual.2);
}

/// With ROMACS_WORLD=off, and in a fresh HOME, a batch session boots the
/// chain from source, as one does whenever its world cannot be had, and
/// makes no store: nothing is built for a HOME no Romacs ran in.
#[test]
fn batch_without_worlds_boots_from_source_as_pinned_gnu_does() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = env!("CARGO_BIN_EXE_romacs-vm-gui");
    let args: Vec<String> = ["-Q", "--batch", "--eval", "(progn (princ (list noninteractive window-system (length command-line-args))) (kill-emacs 4))"]
        .iter().map(|a| a.to_string()).collect();
    for world in [false, true] {
        let dir = std::env::temp_dir().join(format!("romacs-batch-source-{}-{world}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let expected = run(&gnu, &dir, &args, world);
        let actual = run(romacs, &dir, &args, world);
        let store = dir.join("cache/romacs/worlds").exists();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!((actual.0, actual.1), (expected.0, expected.1), "worlds {world}: Romacs stderr {}", actual.2);
        assert!(!store, "a session (worlds {world}) made a store");
    }
}

/// In a store that exists, a session without its world boots from source
/// while a builder prepares it, and the next restores it: one world per
/// locale, so sessions alternating LANG=C and C.UTF-8 restore both once
/// each was built, rebuilding neither, and answer as pinned GNU does.
#[test]
fn batch_prepares_its_world_per_locale_in_an_existing_store() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = env!("CARGO_BIN_EXE_romacs-vm-gui");
    let dir = std::env::temp_dir().join(format!("romacs-batch-locales-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("cache/romacs")).unwrap();
    std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700).create(dir.join("cache/romacs/worlds")).unwrap();
    let form = "(princ (list noninteractive current-language-environment locale-coding-system (terminal-coding-system)))";
    let session = |binary: &str, lang: &str| {
        let output = Command::new(binary).args(["-Q", "--batch", "--eval", form]).current_dir(&dir)
            .env("HOME", &dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
            .env("LANG", lang).env_remove("LC_ALL").env_remove("LC_CTYPE").env_remove("LC_MESSAGES").env_remove("ROMACS_WORLD")
            .output().unwrap();
        (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
    };
    // A builder publishes the image, then its manifest.
    let built = |count: usize| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
        while stored(&dir, "rwi").iter().filter(|image| image.with_extension("json").is_file()).count() < count {
            assert!(std::time::Instant::now() < deadline, "no world {count} after 600 s");
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    };
    let mut failures = Vec::new();
    let mut check = |lang: &str| {
        let (expected, actual) = (session(&gnu, lang), session(romacs, lang));
        if (expected.0, &expected.1) != (actual.0, &actual.1) {
            failures.push(format!("LANG={lang}: GNU {expected:?}, Romacs {actual:?}"));
        }
    };
    check("C.UTF-8");
    built(1);
    check("C");
    built(2);
    assert!(stored(&dir, "use").is_empty(), "a session restored a world it had to build");
    let images: Vec<_> = stored(&dir, "rwi").iter().map(|image| std::fs::metadata(image).unwrap().modified().unwrap()).collect();
    for lang in ["C.UTF-8", "C", "C.UTF-8", "C"] { check(lang); }
    let again: Vec<_> = stored(&dir, "rwi").iter().map(|image| std::fs::metadata(image).unwrap().modified().unwrap()).collect();
    let used = stored(&dir, "use").len();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!((used, again), (2, images), "both locales' worlds restored, neither rebuilt");
}
