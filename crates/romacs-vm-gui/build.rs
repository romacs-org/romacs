//! `ROMACS_BUILD_ID`: what this build of the executable is computed from, so
//! that what depends only on that (compiled Lisp, `compiled_lisp`) is shared
//! by every build of the same inputs, wherever its executable lies and
//! however its bytes came out. Two builds of one commit differ in their code
//! sections (romacs#277), so the executable cannot stand for its inputs.
//!
//! It hashes the workspace's crates as a build reads them (every file under
//! `crates/` but tests, harnesses, fixtures, benches, examples and notes,
//! and every file a source compiles in with `include_str!` or
//! `include_bytes!`, and every workspace file a build script names, such as
//! `etc/rgb.txt`),
//! `Cargo.lock` and the root `Cargo.toml`, and the configuration a build
//! takes from cargo: profile, target, features, cfg, `RUSTFLAGS`, the
//! compiler's version and the environment native builds read (`CC`,
//! `CFLAGS`, the bundled `SQLite`'s `SQLITE_*`). The libraries the process loads are not known here:
//! the embedder hashes them at run time, as the world store does.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::{env, fs};

/// Whether PATH (relative to the workspace) is no input of the executable.
fn skipped(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.components().any(|c| {
        matches!(c.as_os_str().to_str(), Some("tests" | "harness" | "fixtures" | "benches" | "examples" | "target"))
    }) || name.ends_with("_tests.rs")
        || name.ends_with("_test.rs")
        || name == "tests.rs"
        || path.extension().is_some_and(|e| e == "md")
}

fn files(workspace: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        let relative = path.strip_prefix(workspace).unwrap().to_path_buf();
        if skipped(&relative) {
            continue;
        }
        if path.is_dir() {
            // A crate's sources are found by listing them, so a file added
            // there (a new primitive module) changes the id: Cargo watches a
            // directory's whole tree, not only the files listed last time.
            if path.file_name().is_some_and(|n| n == "src") {
                println!("cargo:rerun-if-changed={}", path.display());
            }
            files(workspace, &path, out);
        } else {
            out.push(relative);
        }
    }
}

/// The files SOURCE (a Rust file under WORKSPACE) compiles in with
/// `include_str!` or `include_bytes!` and a literal path: inputs of the
/// executable wherever they lie (`lisp/` among them).
fn included(workspace: &Path, source: &Path, out: &mut Vec<PathBuf>) {
    let Ok(text) = fs::read_to_string(workspace.join(source)) else { return };
    for marker in ["include_str!(\"", "include_bytes!(\""] {
        for (at, _) in text.match_indices(marker) {
            let rest = &text[at + marker.len()..];
            let Some(end) = rest.find('"') else { continue };
            let target = workspace.join(source).parent().unwrap().join(&rest[..end]);
            if let Ok(target) = target.canonicalize()
                && let Ok(relative) = target.strip_prefix(workspace)
                && target.is_file()
            {
                out.push(relative.to_path_buf());
            }
        }
    }
}

/// The workspace files a build script SOURCE names by a literal path from
/// the workspace's root (`etc/rgb.txt`): what it generates code from.
fn named_by_build_script(workspace: &Path, source: &Path, out: &mut Vec<PathBuf>) {
    let Ok(text) = fs::read_to_string(workspace.join(source)) else { return };
    for literal in text.split('"').skip(1).step_by(2) {
        let candidate = workspace.join(literal);
        if !literal.is_empty() && !literal.contains("..") && candidate.is_file() {
            out.push(PathBuf::from(literal));
        }
    }
}

/// Environment variables the native parts of a build read (cc, pkg-config,
/// the bundled `SQLite`'s compile-time settings): what changes the code they
/// make without a file changing.
/// Each is also a rerun trigger; the prefixes are hashed when present, and
/// the names among them the bundled `SQLite`'s build reads are listed here so
/// a change reruns this script.
const NATIVE_ENV: &[&str] = &["CC", "CXX", "AR", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "PKG_CONFIG_PATH",
    "PKG_CONFIG_LIBDIR", "LIBSQLITE3_FLAGS", "LIBSQLITE3_SYS_USE_PKG_CONFIG", "SQLITE_MAX_VARIABLE_NUMBER",
    "SQLITE_MAX_EXPR_DEPTH", "SQLITE_MAX_COLUMN", "CC_SHELL_ESCAPED_FLAGS"];
const NATIVE_ENV_PREFIXES: &[&str] = &["SQLITE_", "LIBSQLITE3", "TREE_SITTER"];

/// The `cc` crate's settings, each read in its target and build-kind forms
/// (`CFLAGS_x86_64-unknown-linux-gnu`, `CFLAGS_x86_64_unknown_linux_gnu`,
/// `TARGET_CFLAGS`, `HOST_CFLAGS`): every form is hashed and watched.
const CC_ENV: &[&str] = &["CC", "CXX", "AR", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "ARFLAGS", "CRATE_CC_NO_DEFAULTS"];

/// Every variable name `CC_ENV` reads for TARGET and HOST. One literal
/// selector of name templates, the shape `tools/gate` reads to pass the same
/// variables through to a gated build.
fn cc_variants(target: &str, host: &str) -> Vec<String> {
    let mut names = Vec::new();
    for &base in CC_ENV {
        for triple in [target, host] {
            let under = triple.replace('-', "_");
            for name in [base.to_owned(), format!("{base}_{triple}"), format!("{base}_{under}"), format!("HOST_{base}"), format!("TARGET_{base}")] {
                names.push(name);
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn main() {
    let workspace = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let workspace = workspace.canonicalize().unwrap();
    let mut inputs = Vec::new();
    files(&workspace, &workspace.join("crates"), &mut inputs);
    let mut extra = Vec::new();
    for source in inputs.iter().filter(|p| p.extension().is_some_and(|e| e == "rs")) {
        included(&workspace, source, &mut extra);
        if source.file_name().is_some_and(|n| n == "build.rs") {
            named_by_build_script(&workspace, source, &mut extra);
        }
    }
    inputs.extend(extra);
    inputs.sort();
    inputs.dedup();
    inputs.push(PathBuf::from("Cargo.lock"));
    inputs.push(PathBuf::from("Cargo.toml"));
    let mut hash = Sha256::new();
    for relative in &inputs {
        let path = workspace.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path).unwrap();
        hash.update(relative.to_string_lossy().as_bytes());
        hash.update([0]);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    let cc = cc_variants(&env::var("TARGET").unwrap(), &env::var("HOST").unwrap());
    let mut config: Vec<(String, String)> = env::vars()
        .filter(|(name, _)| {
            name.starts_with("CARGO_FEATURE_")
                || name.starts_with("CARGO_CFG_")
                || matches!(name.as_str(), "PROFILE" | "OPT_LEVEL" | "DEBUG" | "TARGET" | "CARGO_ENCODED_RUSTFLAGS")
                || NATIVE_ENV.contains(&name.as_str())
                || NATIVE_ENV_PREFIXES.iter().any(|p| name.starts_with(p))
                || cc.contains(name)
        })
        .collect();
    config.sort();
    for (name, value) in config {
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    let watched = ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC"].iter().chain(NATIVE_ENV).map(|n| (*n).to_owned()).chain(cc);
    for name in watched {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let rustc = std::process::Command::new(env::var_os("RUSTC").unwrap()).arg("-vV").output().unwrap();
    assert!(rustc.status.success());
    hash.update(rustc.stdout);
    let id = hash.finalize().iter().fold(String::new(), |mut id, b| {
        use std::fmt::Write;
        let _ = write!(id, "{b:02x}");
        id
    });
    println!("cargo:rustc-env=ROMACS_BUILD_ID={id}");
}
