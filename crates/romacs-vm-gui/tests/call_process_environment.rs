//! callproc.c make_environment_block and emacs_spawn against pinned GNU, in
//! a `--batch' session: a child sees each name's first `process-environment'
//! entry, a lone name hides the later definitions of its name and is left
//! out, and the child runs in a session of its own with no signal ignored
//! or blocked.
#[path = "support/installed.rs"]
mod installed;

const ENVIRONMENT: &str = r#"(let ((process-environment '("RA=1" "RB" "RA=2" "RB=3" "RC=4" "RC=5" "RD=" "PATH=/usr/bin:/bin")))
  (princ (with-temp-buffer (call-process "/usr/bin/env" nil t) (buffer-string))))"#;
const CHILD: &str = r#"(princ (with-temp-buffer
  (call-process "/bin/sh" nil t nil "-c" "grep -E '^Sig(Ign|Blk)' /proc/self/status; test \"$(ps -o sid= -p $$ | tr -d ' ')\" = $$ && echo own-session")
  (buffer-string)))"#;

fn run(program: &str, args: &[&str], source: &str) -> String {
    let home = std::env::temp_dir().join(format!("romacs-call-process-env-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let output = std::process::Command::new(program).args(args).args(["--batch", "--eval", source])
        .env("HOME", &home).output().unwrap();
    assert!(output.status.success(), "{program}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn gnu(source: &str) -> String {
    run(&std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle"), &["-Q"], source)
}

fn romacs(source: &str) -> String {
    run(installed::romacs(), &[], source)
}

/// The lines of TEXT that name a test variable, sorted.
fn variables(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text.lines().filter(|l| l.starts_with('R') && l.as_bytes().get(2) == Some(&b'=')).map(str::to_owned).collect();
    lines.sort();
    lines
}

#[test]
fn a_child_sees_the_environment_and_session_gnu_gives_it() {
    let ours = variables(&romacs(ENVIRONMENT));
    assert_eq!(ours, variables(&gnu(ENVIRONMENT)));
    assert_eq!(ours, ["RA=1", "RC=4", "RD="]);
    let child = romacs(CHILD);
    assert_eq!(child, gnu(CHILD));
    assert!(child.contains("own-session"), "{child}");
}
