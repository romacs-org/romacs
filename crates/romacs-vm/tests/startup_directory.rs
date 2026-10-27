//! GNU's emacs_wd keeps a symlinked PWD's spelling when PWD names the same
//! directory as "." (glibc get_current_dir_name), and expand-file-name
//! joins a relative or empty HOME to it (fileio.c get_homedir). Both
//! engines start in the real directory with PWD naming it through a
//! symlink; the expansions must agree with pinned GNU.
use std::process::Command;

const EXPRESSION: &str = r#"(list (let ((process-environment (cons "HOME=" process-environment))) (list (expand-file-name "~/x") (expand-file-name "~"))) (let ((process-environment (cons "HOME=rel/h" process-environment))) (expand-file-name "~/y")))"#;

#[test]
fn relative_home_joins_the_startup_directory_spelled_as_pwd() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let base = std::env::temp_dir().join(format!("rt-startup-directory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let real = base.join("real");
    std::fs::create_dir_all(&real).unwrap();
    let alias = base.join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let run = |command: &mut Command| {
        let output = command.current_dir(&real).env("PWD", &alias).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    };
    let expected = run(Command::new(&gnu).args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {EXPRESSION}))")]));
    let request = format!("{}\n", serde_json::json!({ "source": format!("(prin1-to-string {EXPRESSION})") }));
    let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm"));
    child.arg("--eval-stream").current_dir(&real).env("PWD", &alias)
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = child.spawn().unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), request.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let reply: serde_json::Value = serde_json::from_slice(output.stdout.split(|&b| b == b'\n').next().unwrap()).unwrap();
    let actual = reply["result"]["value"].as_str().unwrap_or_else(|| panic!("{reply}")).to_owned();
    let _ = std::fs::remove_dir_all(&base);
    assert!(expected.contains("/alias/"), "GNU keeps the PWD spelling: {expected}");
    assert_eq!(actual, expected);
}
