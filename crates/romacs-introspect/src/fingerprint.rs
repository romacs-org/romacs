//! Shared registry input identity, independent of Git and output snapshots.
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Hash sorted workspace-relative UTF-8 paths and bytes, each length-framed.
/// The local normal/build dependency closure is discovered from manifests,
/// including target-specific dependencies and all local workspace patches.
/// Only manifests, src trees and build scripts/helpers are source inputs.
/// Registry dependencies are pinned by Cargo.lock; their downloaded source is
/// not a mutable workspace input. See README.md for the scope contract.
pub fn input_hash(root: &Path) -> Result<String, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut paths = BTreeSet::new();
    for path in input_roots(&root)? {
        files(&path, &mut paths)?;
    }
    let mut hash = Sha256::new();
    hash.update(b"romacs-registry-inputs-v2\0");
    for path in paths {
        let relative = path.strip_prefix(&root).map_err(|e| e.to_string())?;
        let name = relative
            .to_str()
            .ok_or("non-UTF-8 input path")?
            .replace('\\', "/");
        if matches!(
            name.as_str(),
            "crates/romacs-introspect/data/registry.json"
                | "crates/romacs-introspect-live/data/registry.json"
        ) {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        for part in [name.as_bytes(), bytes.as_slice()] {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part);
        }
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Files/directories Cargo must watch, including new files in dependency trees.
pub fn input_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let workspace = manifest(&root.join("Cargo.toml"))?;
    std::fs::metadata(root.join("Cargo.lock")).map_err(|e| format!("Cargo.lock: {e}"))?;
    let mut crates = BTreeSet::new();
    dependencies(
        &root,
        &root.join("crates/romacs-introspect-live"),
        &workspace,
        &mut crates,
    )?;
    if let Some(registries) = workspace.get("patch").and_then(toml::Value::as_table) {
        for patches in registries.values().filter_map(toml::Value::as_table) {
            for patch in patches.values() {
                if let Some(path) = patch.get("path").and_then(toml::Value::as_str) {
                    dependencies(&root, &root.join(path), &workspace, &mut crates)?;
                }
            }
        }
    }
    let mut paths = BTreeSet::new();
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "rust-toolchain",
        ".cargo",
        "rlisp/vm",
    ] {
        let p = root.join(name);
        if p.exists() {
            paths.insert(p);
        }
    }
    for dir in crates {
        paths.insert(dir.join("Cargo.toml"));
        for name in ["src", "build.rs", "build"] {
            let path = dir.join(name);
            if path.try_exists().map_err(|e| e.to_string())? {
                paths.insert(path);
            }
        }
    }
    Ok(paths.into_iter().collect())
}

fn manifest(path: &Path) -> Result<toml::Value, String> {
    toml::from_str(&std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn dependencies(
    root: &Path,
    dir: &Path,
    workspace: &toml::Value,
    seen: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    if !dir.starts_with(root) {
        return Err("local dependency outside workspace".into());
    }
    if !seen.insert(dir.clone()) {
        return Ok(());
    }
    let doc = manifest(&dir.join("Cargo.toml"))?;
    let mut sections = vec![&doc];
    if let Some(targets) = doc.get("target").and_then(toml::Value::as_table) {
        sections.extend(targets.values());
    }
    for section in sections {
        for kind in ["dependencies", "build-dependencies"] {
            if let Some(deps) = section.get(kind).and_then(toml::Value::as_table) {
                for (name, value) in deps {
                    let inherited =
                        value.get("workspace").and_then(toml::Value::as_bool) == Some(true);
                    let value = if inherited {
                        &workspace["workspace"]["dependencies"][name]
                    } else {
                        value
                    };
                    if let Some(path) = value.get("path").and_then(toml::Value::as_str) {
                        dependencies(
                            root,
                            &if inherited {
                                root.join(path)
                            } else {
                                dir.join(path)
                            },
                            workspace,
                            seen,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Deterministic cache/editor exclusions, also applied in Git-free exports.
pub fn excluded_name(name: &str) -> bool {
    matches!(name, "target" | ".git" | "__pycache__")
        || name.starts_with(".#")
        || (name.starts_with('#') && name.ends_with('#'))
        || name.ends_with('~')
        || name.ends_with(".swp")
        || name.ends_with(".swo")
}

fn files(path: &Path, out: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("non-UTF-8 input path")?;
    if excluded_name(name) {
        return Ok(());
    }
    if std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err(format!(
            "symlink input is not supported: {}",
            path.display()
        ));
    }
    if path.is_dir() {
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            files(&entry.map_err(|e| e.to_string())?.path(), out)?;
        }
    } else {
        out.insert(path.to_owned());
    }
    Ok(())
}
