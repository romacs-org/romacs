//! startup.el normal-top-level's locale step in `romacs-vm-gui -Q --batch`
//! against pinned GNU's `emacs -Q --batch`: Unix's end-of-line mnemonics,
//! then `set-locale-environment' from the process's LANG, which sets the
//! language environment and the keyboard, terminal, locale and process
//! coding systems that `char-displayable-p' (hence
//! `truncate-string-ellipsis') and process decoding read.
use std::process::Command;

const FORM: &str = "(princ (list (terminal-coding-system) (keyboard-coding-system) locale-coding-system
  current-language-environment default-process-coding-system (default-value 'buffer-file-coding-system)
  (car (coding-system-priority-list)) eol-mnemonic-dos eol-mnemonic-mac (current-input-mode)
  (and (char-displayable-p #x2026) t) (and (char-displayable-p #x3bb) t)
  (locale-info 'codeset) (locale-info 'months) (locale-info 'paper)))";

#[test]
fn locale_startup_matches_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = env!("CARGO_BIN_EXE_romacs-vm-gui");
    let dir = std::env::temp_dir().join(format!("romacs-locale-startup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // None: no LANG at all, where `set-locale-environment' asks
    // `locale-info' for the C library's codeset.
    for lang in [Some("C.UTF-8"), Some("C"), Some("en_US.UTF-8"), None] {
        let run = |binary: &str| {
            let mut command = Command::new(binary);
            command.args(["-Q", "--batch", "--eval", FORM]).current_dir(&dir)
                .env("HOME", &dir).env_remove("LC_ALL").env_remove("LC_CTYPE").env_remove("LANG")
                // A batch world is per HOME and locale (src/batch.rs); this boots from source.
                .env("ROMACS_WORLD", "off");
            if let Some(lang) = lang { command.env("LANG", lang); }
            let output = command.output().unwrap();
            (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned(),
             String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default().to_owned())
        };
        let expected = run(&gnu);
        assert_eq!(expected.0, Some(0), "pinned GNU, LANG={lang:?}: {}", expected.2);
        let actual = run(romacs);
        assert_eq!((actual.0, &actual.1), (expected.0, &expected.1), "LANG={lang:?}: {}", actual.2);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A VM restored from a world takes the locale step again in its own
/// process (`romacs-world-restart'), from the state the chain had before
/// its first locale step: one built under en_US.UTF-8 or ja_JP.UTF-8 and
/// restored with LANG unset, C or C.UTF-8 answers as pinned GNU started
/// under that LANG, in every variable the step sets, and a later
/// `set-terminal-coding-system' with no coding system takes GNU's default.
#[test]
fn a_replayed_locale_step_matches_pinned_gnu_started_there() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = env!("CARGO_BIN_EXE_romacs-vm-gui");
    let dir = std::env::temp_dir().join(format!("romacs-locale-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = "(princ (list (terminal-coding-system) (keyboard-coding-system) locale-coding-system
      current-language-environment current-locale-environment current-iso639-language
      default-process-coding-system (default-value 'buffer-file-coding-system)
      default-terminal-coding-system default-keyboard-coding-system keyboard-coding-system
      default-sendmail-coding-system default-file-name-coding-system default-input-method
      system-messages-locale system-time-locale (and (boundp 'woman-locale) woman-locale)
      (coding-system-priority-list) coding-category-list (mapcar #'symbol-value coding-category-list)
      (char-width #x2500) (char-width #x3bb)
      (and (char-displayable-p #x2026) t) (and (char-displayable-p #x3bb) t)
      (progn (set-terminal-coding-system nil nil t) (terminal-coding-system))))";
    let run = |binary: &str, lang: Option<&str>, form: &str| {
        let mut command = Command::new(binary);
        command.args(["-Q", "--batch", "--eval", form]).current_dir(&dir)
            .env("HOME", &dir).env_remove("LC_ALL").env_remove("LC_CTYPE").env_remove("LANG")
            // A source boot: the test replays the restore itself.
            .env("ROMACS_WORLD", "off");
        if let Some(lang) = lang { command.env("LANG", lang); }
        let output = command.output().unwrap();
        (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned(),
         String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default().to_owned())
    };
    for lang in [None, Some("C"), Some("C.UTF-8")] {
        let expected = run(&gnu, lang, state);
        assert_eq!(expected.0, Some(0), "pinned GNU, LANG={lang:?}: {}", expected.2);
        let setenv = lang.map_or("(setenv \"LANG\")".to_owned(), |lang| format!("(setenv \"LANG\" {lang:?})"));
        for built in ["en_US.UTF-8", "ja_JP.UTF-8"] {
            let actual = run(romacs, Some(built), &format!("(progn {setenv} (romacs-world-restart) {state})"));
            assert_eq!((actual.0, &actual.1), (expected.0, &expected.1), "built under {built}, replayed with LANG={lang:?}: {}", actual.2);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
