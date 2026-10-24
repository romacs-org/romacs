use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const HARNESS_INPUTS: &[&str] = &[
    "crates",
    ".cargo",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
];

fn git_stdout(workspace_root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn absolute_git_path(workspace_root: &Path, name: &str) -> Option<PathBuf> {
    git_stdout(
        workspace_root,
        &["rev-parse", "--path-format=absolute", "--git-path", name],
    )
    .filter(|value| !value.is_empty())
    .map(PathBuf::from)
}

/// Git files whose contents select the checked-out commit.
///
/// `--path-format=absolute` is important here: Cargo resolves a relative
/// `rerun-if-changed` path from this crate, while Git resolves it from the
/// worktree.  It also lets Git select the per-worktree HEAD directory rather
/// than making this build script reconstruct Git's worktree layout.
pub(crate) fn git_metadata_watch_paths(workspace_root: &Path) -> Option<Vec<PathBuf>> {
    let mut paths = vec![absolute_git_path(workspace_root, "HEAD")?];
    if let Some(reference) = git_stdout(workspace_root, &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = absolute_git_path(workspace_root, &reference)
    {
        paths.push(path);
    }
    for metadata in ["index", "packed-refs"] {
        if let Some(path) = absolute_git_path(workspace_root, metadata)
            && path.is_file()
        {
            paths.push(path);
        }
    }
    paths.sort_unstable();
    paths.dedup();
    Some(paths)
}

fn emit_source_watch_paths(workspace_root: &Path) {
    // The acceptance harness consists of xtask and its local crate graph. A
    // recursive watch here makes Cargo rerun this script when dirty compiled
    // sources are restored without changing HEAD, so the build-time dirty
    // marker cannot remain stale.
    for relative in HARNESS_INPUTS {
        let path = workspace_root.join(relative);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

pub(crate) fn tracked_harness_inputs_dirty(workspace_root: &Path) -> Option<bool> {
    let mut arguments = vec!["status", "--porcelain", "--untracked-files=no", "--"];
    arguments.extend_from_slice(HARNESS_INPUTS);
    git_stdout(workspace_root, &arguments).map(|status| !status.is_empty())
}

fn main() {
    let workspace_root = PathBuf::from(
        env::var_os("CARGO_WORKSPACE_DIR")
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).into_os_string()),
    );
    let workspace_root = workspace_root.canonicalize().unwrap_or(workspace_root);
    let revision = git_stdout(&workspace_root, &["rev-parse", "HEAD"])
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=ROMACS_PERF_GIT_SHA={revision}");
    let inputs_dirty = tracked_harness_inputs_dirty(&workspace_root).unwrap_or(true);
    println!("cargo:rustc-env=ROMACS_PERF_INPUTS_DIRTY={inputs_dirty}");

    if let Some(paths) = git_metadata_watch_paths(&workspace_root) {
        for path in paths {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    emit_source_watch_paths(&workspace_root);
}
