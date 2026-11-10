mod dependency_coherence;
mod test;
mod window_icon;

use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

type DynError = Box<dyn Error>;
type Result<T> = std::result::Result<T, DynError>;

fn main() {
    if let Err(err) = run_xtask(repository_root(), env::args_os().skip(1)) {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run_xtask(repo_root: PathBuf, args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let mut args = args.into_iter();
    let command = args.next().unwrap_or_else(|| OsString::from("help"));
    match command.to_str() {
        Some("help" | "--help" | "-h") => {
            println!("{}", usage_text());
            Ok(())
        }
        Some("check-dependency-coherence") => {
            if let Some(unexpected) = args.next() {
                return Err(format!(
                    "check-dependency-coherence does not accept arguments; found `{}`",
                    unexpected.to_string_lossy()
                )
                .into());
            }
            dependency_coherence::run(&repo_root)
        }
        Some("render-window-icon") => window_icon::run(&repo_root, args),
        Some("pin-reference") => run_workspace_cli(&repo_root, "romacs-parity-reference", args),
        Some("test") => test::run(repo_root, args),
        Some("fresh-build" | "test-daemon-gui" | "check-bootstrap-autoloads" | "gc-stress" | "perf") => {
            Err(format!(
                "`{}` was retired with the legacy runtime and its build pipeline; use VM2 Cargo targets directly\n\n{}",
                command.to_string_lossy(),
                usage_text()
            )
            .into())
        }
        _ => Err(format!("unknown command: {}\n\n{}", command.to_string_lossy(), usage_text()).into()),
    }
}

fn workspace_cli_command(
    repo_root: &Path,
    package: &str,
    args: impl IntoIterator<Item = OsString>,
) -> Command {
    let mut command = Command::new(cargo_program());
    command
        .current_dir(repo_root)
        .args(["run", "--quiet", "-p", package, "--"])
        .args(args);
    command
}

fn run_workspace_cli(
    repo_root: &Path,
    package: &str,
    args: impl IntoIterator<Item = OsString>,
) -> Result<()> {
    let status = workspace_cli_command(repo_root, package, args).status()?;
    if status.success() {
        return Ok(());
    }
    std::process::exit(status.code().unwrap_or(1));
}

fn repository_root() -> PathBuf {
    env::var_os("NEXTEST_WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn cargo_program() -> PathBuf {
    env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"))
}

fn usage_text() -> &'static str {
    "Usage: cargo xtask COMMAND [ARGS]

Commands:
  test -- [cargo test arguments]       Run Cargo tests without legacy Lisp preparation
  check-dependency-coherence           Check Windows native dependency type families
  pin-reference [ARGS]                Manage the pinned GNU parity reference
  render-window-icon --out-dir DIR    Render the canonical macOS iconset

Build and run VM2 targets directly with Cargo. The legacy fresh-build,
daemon GUI, bootstrap-autoload, GC-stress and perf-binary commands are retired."
}

#[cfg(test)]
mod tests;
