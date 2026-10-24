use super::super::*;

#[test]
fn retired_commands_fail_before_launching_any_build() {
    for command in [
        "fresh-build",
        "test-daemon-gui",
        "check-bootstrap-autoloads",
        "gc-stress",
        "perf",
    ] {
        let error = run_xtask(PathBuf::from("/nonexistent"), [OsString::from(command)])
            .unwrap_err()
            .to_string();
        assert!(error.contains("retired"), "{command}: {error}");
    }
}

#[test]
fn unknown_commands_and_build_flags_are_not_fresh_builds() {
    for command in ["missing", "--release", "--profile"] {
        let error = run_xtask(PathBuf::from("/nonexistent"), [OsString::from(command)])
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown command"), "{error}");
    }
}

#[test]
fn retained_workspace_launchers_preserve_arguments_and_root() {
    for package in ["romacs-parity-reference", "romacs-infra"] {
        let command = workspace_cli_command(
            Path::new("/repo"),
            package,
            ["--help", "space separated"].map(OsString::from),
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/repo")));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "run",
                "--quiet",
                "-p",
                package,
                "--",
                "--help",
                "space separated"
            ]
        );
    }
}

#[test]
fn help_and_empty_invocation_do_not_build() {
    run_xtask(PathBuf::from("/nonexistent"), []).unwrap();
    run_xtask(PathBuf::from("/nonexistent"), [OsString::from("--help")]).unwrap();
}

#[test]
fn dependency_check_rejects_extra_arguments_before_metadata() {
    let error = run_xtask(
        PathBuf::from("/nonexistent"),
        ["check-dependency-coherence", "unexpected"].map(OsString::from),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("does not accept arguments"));
}
