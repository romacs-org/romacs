use super::*;

pub(super) fn run(repo_root: PathBuf, args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let status = test_command(&repo_root, test_args(args)?).status()?;
    if !status.success() {
        return Err(format!("cargo test failed: {status}").into());
    }
    Ok(())
}

fn test_command(repo_root: &Path, args: Vec<OsString>) -> Command {
    let mut command = Command::new(cargo_program());
    command.current_dir(repo_root).arg("test").args(args);
    command
}

fn test_args(args: impl IntoIterator<Item = OsString>) -> Result<Vec<OsString>> {
    let mut args = args.into_iter();
    match args.next() {
        None => Ok(Vec::new()),
        Some(separator) if separator == "--" => Ok(args.collect()),
        Some(_) => Err(
            "usage: cargo xtask test -- [cargo test arguments, including its own -- for libtest]"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_cargo_and_libtest_arguments_without_reparsing() {
        let input = ["--", "-p", "romacs-vm", "--lib", "load", "--", "--exact"];
        let actual = test_args(input.map(OsString::from)).unwrap();
        assert_eq!(
            actual,
            input[1..].iter().map(OsString::from).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_command_does_not_prepare_or_override_the_runtime() {
        let command = test_command(Path::new("/repo"), vec![OsString::from("--lib")]);
        assert_eq!(command.get_current_dir(), Some(Path::new("/repo")));
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["test", "--lib"]);
        assert_eq!(command.get_envs().count(), 0);
    }

    #[test]
    fn refuses_ambiguous_options_before_separator() {
        assert!(test_args([OsString::from("--no-byte-compile")]).is_err());
        assert!(test_args([]).unwrap().is_empty());
    }
}
