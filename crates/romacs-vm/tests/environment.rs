use romacs_vm::Vm;
#[path = "support/own_test.rs"]
mod own_test;

fn cases() -> Vec<&'static str> {
    vec![
        r#"(getenv-internal "ROMACS_ENV_UNSET" '("ROMACS_ENV_OTHER=other"))"#,
        r#"(getenv-internal "ROMACS_ENV_TEST" '("ROMACS_ENV_TEST=first" "ROMACS_ENV_TEST=second"))"#,
        r#"(getenv-internal "ROMACS_ENV_TEST" '("ROMACS_ENV_TEST" "ROMACS_ENV_TEST=second"))"#,
        r#"(getenv-internal "ROMACS_ENV_TEST" '("ROMACS_ENV_TEST=" "ROMACS_ENV_TEST=second"))"#,
        r#"(getenv-internal "ROMACS_ENV_TEST" '(4 nil wrong "ROMACS_ENV_TEST=value"))"#,
        r#"(getenv-internal "ROMACS_ENV_TEST" '("ROMACS_ENV_OTHER=value" . dotted))"#,
        r#"(getenv-internal "ROMACS_ENV_TEST" '("ROMACS_ENV_TEST_EXTRA=no" "ROMACS_ENV_TEST=yes"))"#,
        r#"(getenv-internal "romacs_env_test" '("ROMACS_ENV_TEST=value"))"#,
        r#"(getenv-internal "" '("=empty-name"))"#,
        r#"(getenv-internal "ROMACS=ENV" '("ROMACS=ENV=value"))"#,
        r#"(getenv-internal "ROMACS\0ENV" '("ROMACS\0ENV=value\0tail"))"#,
        r#"(getenv-internal (unibyte-string 255) (list (unibyte-string 255 61 254 0 253)))"#,
        r#"(getenv-internal "λ" '("λ=Ж"))"#,
        r#"(getenv-internal "X" (list (concat "X=" (unibyte-string 195 169))))"#,
        r#"(getenv-internal "X" (list (concat "X=" (unibyte-string 195 169 255))))"#,
        r#"(getenv-internal "X" (list (concat "X=" (unibyte-string 237 160 128))))"#,
        r#"(getenv-internal "X" (list (concat "X=" (unibyte-string 192 128))))"#,
        r#"(getenv-internal 17 '("ROMACS_ENV_TEST=value"))"#,
        r#"(let ((process-environment '("ROMACS_ENV_TEST=process"))) (getenv-internal "ROMACS_ENV_TEST" nil))"#,
        r#"(let ((process-environment '("ROMACS_ENV_TEST=process"))) (getenv-internal "ROMACS_ENV_TEST" t))"#,
        r#"(let ((process-environment '("ROMACS_ENV_TEST=process"))) (getenv-internal "ROMACS_ENV_TEST" 'wrong))"#,
        r#"(let ((process-environment '("ROMACS_ENV_TEST" "ROMACS_ENV_TEST=process"))) (getenv-internal "ROMACS_ENV_TEST"))"#,
        r#"(let ((process-environment '("ROMACS_ENV_TEST=process"))) (getenv-internal "ROMACS_ENV_TEST" '("ROMACS_ENV_TEST=list")))"#,
        r#"(let ((process-environment nil) (initial-environment '("DISPLAY=initial" "ROMACS_ENV_TEST=initial"))) (getenv-internal "DISPLAY"))"#,
        r#"(let ((process-environment nil) (initial-environment '("DISPLAY=initial"))) (getenv-internal "DISPLAY" t))"#,
        r#"(let ((process-environment '("DISPLAY")) (initial-environment '("DISPLAY=initial"))) (getenv-internal "DISPLAY"))"#,
        r#"(let ((process-environment nil) (initial-environment '("ROMACS_ENV_TEST=initial"))) (getenv-internal "ROMACS_ENV_TEST"))"#,
    ]
}

#[test]
fn getenv_internal_matches_pinned_gnu_bytes_and_precedence() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    for expression in cases() {
        let source = format!("(condition-case err (let ((value {expression})) (if (stringp value) (list (multibyte-string-p value) (vconcat value)) value)) (error (list 'signal (car err))))");
        let result = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(result.status.success(), "synthetic environment probe failed");
        let expected = String::from_utf8(result.stdout).unwrap();
        let mut vm = Vm::new();
        assert_eq!(vm.eval_source(&source).and_then(|v| v.display()).as_deref(), Ok(expected.as_str()), "{expression}");
    }
}

#[test]
fn dynamic_environment_binding_and_gc_preserve_exact_values() {
    let mut vm = Vm::new();
    let source = r#"(progn
      (setq process-environment '("ROMACS_ENV_TEST=base"))
      (defun environment-test-read () (getenv-internal "ROMACS_ENV_TEST"))
      (list (special-variable-p 'process-environment)
            (special-variable-p 'initial-environment)
            (let ((process-environment '("ROMACS_ENV_TEST=dynamic")))
              (garbage-collect) (equal (environment-test-read) "dynamic"))
            (condition-case nil
                (let ((process-environment '("ROMACS_ENV_TEST=unwind"))) (error "stop"))
              (error (equal (environment-test-read) "base")))))"#;
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(t t t t)");
    let source = r#"(let* ((entry (copy-sequence "ROMACS_ENV_TEST=value"))
                          (value (getenv-internal "ROMACS_ENV_TEST" (list entry))))
                     (aset value 0 ?x) (equal entry "ROMACS_ENV_TEST=value"))"#;
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "t");
}

#[test]
fn environment_snapshot_is_local_to_each_vm() {
    if std::env::var_os("ROMACS_ENVIRONMENT_CHILD").is_some() {
        let mut first = Vm::new();
        let mut second = Vm::new();
        let source = r#"(list (equal (getenv-internal "ROMACS_ENV_TEST") "launch")
            (equal (getenv-internal "ROMACS_ENV_BYTES") (unibyte-string 255 254))
            (not (eq process-environment initial-environment))
            (let ((entry (car process-environment)))
              (eq entry (car initial-environment))))"#;
        assert_eq!(first.eval_source(source).unwrap().display().unwrap(), "(t t t t)");
        first.eval_source(r#"(setq process-environment '("ROMACS_ENV_TEST=local"))"#).unwrap();
        assert_eq!(second.eval_source(r#"(equal (getenv-internal "ROMACS_ENV_TEST") "launch")"#).unwrap().display().unwrap(), "t");
        assert_eq!(first.eval_source(r#"(equal (getenv-internal "ROMACS_ENV_TEST" initial-environment) "launch")"#).unwrap().display().unwrap(), "t");
        return;
    }
    use std::os::unix::ffi::OsStringExt;
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "environment_snapshot_is_local_to_each_vm")])
        .env_clear().env("ROMACS_ENVIRONMENT_CHILD", "1")
        .env("ROMACS_ENV_TEST", "launch")
        .env("ROMACS_ENV_BYTES", std::ffi::OsString::from_vec(vec![255, 254]))
        .output().unwrap();
    assert!(result.status.success(), "synthetic snapshot child failed: {}", String::from_utf8_lossy(&result.stdout));
}

// GNU appends its installation's libexec helper directory (exec-directory) to
// exec-path; Romacs installs no helpers, so the comparison drops GNU's tail.
const EXEC_PATH_PROBE: &str = r#"(prin1 (list (butlast exec-path) (special-variable-p 'exec-path)
  (let ((exec-path '("/let"))) (garbage-collect) exec-path)))"#;
const EXEC_PATH_ROMACS: &str = r#"(prin1-to-string (list exec-path (special-variable-p 'exec-path)
  (let ((exec-path '("/let"))) (garbage-collect) exec-path)))"#;

#[test]
fn exec_path_decodes_the_launch_path_like_gnu() {
    if std::env::var_os("ROMACS_EXEC_PATH_CHILD").is_some() {
        // First access is exec-path itself, before process-environment.
        let value: String = serde_json::from_str(&Vm::new().eval_source(EXEC_PATH_ROMACS).unwrap().display().unwrap()).unwrap();
        println!("ROMACS-EXEC-PATH:{value}");
        return;
    }
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    for path in [Some(":/bin::/usr/bin:"), Some(""), None, Some("/usr/local/bin:/opt/λ/bin"), Some(":")] {
        let configure = |command: &mut std::process::Command| {
            command.env_clear().env("LANG", "C.UTF-8");
            if let Some(path) = path { command.env("PATH", path); }
        };
        let mut gnu = std::process::Command::new(&oracle);
        gnu.args(["-Q", "--batch", "--eval", EXEC_PATH_PROBE]);
        configure(&mut gnu);
        let gnu = gnu.output().unwrap();
        assert!(gnu.status.success(), "oracle failed for {path:?}");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.args(["--exact", &own_test::name(module_path!(), "exec_path_decodes_the_launch_path_like_gnu"), "--nocapture"]).env("ROMACS_EXEC_PATH_CHILD", "1");
        configure(&mut child);
        child.env("ROMACS_EXEC_PATH_CHILD", "1");
        let child = child.output().unwrap();
        let stdout = String::from_utf8_lossy(&child.stdout);
        assert!(child.status.success(), "romacs child failed for {path:?}: {stdout}");
        let romacs = stdout.split("ROMACS-EXEC-PATH:").nth(1).unwrap_or("").lines().next().unwrap_or("");
        assert_eq!(romacs, String::from_utf8_lossy(&gnu.stdout), "PATH={path:?}");
    }
}
