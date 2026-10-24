//! Derive the introspection index from the tree being built.
//!
//! Why `build.rs` and not only a committed generator: source spans move with
//! every edit to `romacs-core` or VM2, so a committed span file would be stale
//! the moment anyone touched those crates and would conflict in every merge.
//! Deriving them here makes the spans exactly as new as the binary that embeds
//! them -- a stale span is impossible, not merely detected.
//!
//! The one input this cannot derive is the live registry, because booting the
//! runtimes needs `romacs-core`, which this crate must never link. That comes
//! from `data/registry.json`, written by `romacs-introspect-live`, whose gate
//! test fails when the committed snapshot differs from what the runtimes
//! actually register. See README.md.

#[path = "build/derive.rs"]
mod derive;
#[path = "src/fingerprint.rs"]
mod fingerprint;

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // Inputs. Directories are scanned recursively by Cargo for modification.
    for p in ["crates", "test", "lisp", "DIVERGENCES.md"] {
        println!("cargo:rerun-if-changed={}", root.join(p).display());
    }
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("data/registry.json").display()
    );
    println!("cargo:rerun-if-changed=build/derive.rs");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    let registry_text = std::fs::read_to_string(manifest.join("data/registry.json"))
        .expect("romacs-introspect: data/registry.json is missing; run `cargo run -p romacs-introspect-live -- --write`");
    let registry: serde_json::Value = serde_json::from_str(&registry_text)
        .expect("romacs-introspect: data/registry.json is not valid JSON");

    let mut index = derive::derive(&root, &registry);
    index["registry"] = registry;
    index["build"] = build_provenance(&root);
    index["build"]["input_hash"] =
        serde_json::json!(fingerprint::input_hash(&root).expect("registry inputs"));
    index["build"]["generated_on"] = serde_json::json!(format!(
        "{}-{}",
        std::env::var("CARGO_CFG_TARGET_OS").expect("target OS"),
        std::env::var("CARGO_CFG_TARGET_ARCH").expect("target arch")
    ));
    for path in fingerprint::input_roots(&root).expect("registry input roots") {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        ".cargo",
        "rust-toolchain.toml",
        "rust-toolchain",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(name).display());
    }

    std::fs::write(out.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Commit, whether the derivation's inputs were clean, and when. Cleanliness
/// is scoped to the inputs because that is what it is evidence about: an
/// unrelated dirty file does not make a span wrong, a dirty `romacs-core` file
/// does.
fn build_provenance(root: &Path) -> serde_json::Value {
    let derived_at = match std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        Some(s) => s,
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };
    let Some(commit) = git(root, &["rev-parse", "HEAD"]) else {
        return serde_json::json!({
            "commit": null,
            "inputs_clean": null,
            "dirty_inputs": [],
            "derived_at_unix": derived_at,
            "git": "unavailable: no git repository at the build root, so the commit and cleanliness are unknown",
        });
    };
    // Rerun when HEAD or the index moves, so the embedded commit cannot lag.
    if let Some(dir) = git(root, &["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/index");
    }
    if let Some(common) = git(root, &["rev-parse", "--git-common-dir"]) {
        let common = if Path::new(&common).is_absolute() {
            PathBuf::from(common)
        } else {
            root.join(common)
        };
        if let Some(head_ref) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={}", common.join(head_ref).display());
        }
        println!(
            "cargo:rerun-if-changed={}",
            common.join("packed-refs").display()
        );
    }
    let status = git(
        root,
        &[
            "status",
            "--porcelain",
            "--untracked-files=normal",
            "--",
            "crates/romacs-core/src",
            "crates/romacs-vm/src",
            "crates/romacs-introspect",
            "DIVERGENCES.md",
            "crates/*/GNU-DIVERGENCES.md",
            "test",
            "lisp",
        ],
    );
    let dirty: Vec<String> = status
        .as_deref()
        .unwrap_or("")
        .lines()
        .map(|l| l.get(3..).unwrap_or(l).to_string())
        .collect();
    serde_json::json!({
        "commit": commit,
        "inputs_clean": status.is_some().then_some(dirty.is_empty()),
        "dirty_inputs": dirty.iter().take(20).collect::<Vec<_>>(),
        "dirty_input_count": dirty.len(),
        "derived_at_unix": derived_at,
        "git": "available",
    })
}
