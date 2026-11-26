use sha2::{Digest, Sha256};
use std::{fs, path::Path};

#[path = "src/host/runtime_epoch.rs"]
mod runtime_epoch;

pub fn build_hash(workspace: &Path) -> Sha256 {
    let directory = workspace.join(runtime_epoch::DIRECTORY);
    println!("cargo:rerun-if-changed={}", directory.display());
    let digest = fs::read_dir(&directory)
        .and_then(|entries| runtime_epoch::digest(entries.map(|entry| entry.map(|entry| entry.file_name()))))
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()));
    println!("cargo:rustc-env=ROMACS_RUNTIME_EPOCH={digest}");
    Sha256::new_with_prefix(digest.as_bytes())
}
