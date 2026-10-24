//! Enumerate VM2 function bindings without linking the retired evaluator.
//! Only owned strings leave the fresh VM; no moving-heap Value escapes.
use romacs_introspect::{Registry, RegistryProvenance};
use std::collections::BTreeSet;
use std::path::Path;

pub const VM2_METHOD: &str = "A fresh romacs_vm::Vm::new() evaluates fboundp for every candidate name without invoking it. Candidates are string literals in crates/romacs-vm/src, including direct slot assignments and built-in Lisp definitions. Dynamically constructed names without literals cannot be discovered. VM2 entries record names, not arities.";

/// Bound names in `universe` and failed probes (which invalidate enumeration).
/// No candidate is invoked: a probe cannot exit, hang or mutate the host by
/// calling a zero-argument primitive. Only fboundp runs in the fresh VM.
pub fn vm2_registry(universe: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
    use romacs_vm::{Form, Vm};
    let mut vm = Vm::new();
    let mut found = Vec::new();
    let mut failures = Vec::new();
    for name in universe {
        let form = Form::List(vec![
            Form::Sym("fboundp".into()),
            Form::List(vec![Form::Sym("quote".into()), Form::Sym(name.clone())]),
        ]);
        match vm.compile(&form).and_then(|f| vm.execute(f)) {
            Ok(v) if v.is_nil() => {}
            Ok(v) if v.is_t() => found.push(name.clone()),
            Ok(_) => failures.push(format!("{name}: fboundp did not return nil or t")),
            Err(e) => failures.push(format!("{name}: fboundp failed: {e}")),
        }
    }
    (found, failures)
}

/// String literals in VM2's source: the probe universe.
pub fn vm2_literals(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![root.join("crates/romacs-vm/src")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(romacs_introspect::fingerprint::excluded_name)
            {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs")
                && let Ok(text) = std::fs::read_to_string(&p)
            {
                literals(&text, &mut out);
            }
        }
    }
    out
}

fn literals(text: &str, out: &mut BTreeSet<String>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' && (i == 0 || b[i - 1] != b'\'') {
            let mut j = i + 1;
            let mut s = String::new();
            while j < b.len() && b[j] != b'"' {
                if b[j] == b'\\' && j + 1 < b.len() {
                    j += 1;
                }
                s.push(b[j] as char);
                j += 1;
            }
            if !s.is_empty()
                && s.len() <= 80
                && s.is_ascii()
                && !s
                    .chars()
                    .any(|c| c.is_whitespace() || "()\"';`,".contains(c))
            {
                out.insert(s);
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
}

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn snapshot_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data/registry.json")
}

/// The core-free consumer embeds the exact same deterministic artifact.
pub fn embedded_snapshot_path() -> std::path::PathBuf {
    workspace_root().join("crates/romacs-introspect/data/registry.json")
}

pub fn snapshot() -> Result<Registry, String> {
    let text = std::fs::read_to_string(snapshot_path()).map_err(|e| e.to_string())?;
    let json = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Registry::from_json(&json)
}

/// Registered VM2 function names with provenance. The legacy schema's core
/// half is explicitly empty: no retired runtime is linked or booted.
pub fn enumerate() -> Result<Registry, String> {
    let root = workspace_root();
    let compiled_hash = romacs_introspect::Index::embedded()
        .provenance()
        .build_input_hash
        .as_deref()
        .ok_or("compiled registry input identity missing; rebuild romacs-introspect-live")?;
    with_verified_inputs(&root, compiled_hash, || {
        enumerate_verified(&root, compiled_hash)
    })
}

fn with_verified_inputs<T>(
    root: &Path,
    compiled_hash: &str,
    enumerate: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let verify = || -> Result<(), String> {
        let current = romacs_introspect::fingerprint::input_hash(root)?;
        if current != compiled_hash {
            return Err("registry source differs from the compiled runtime; rebuild romacs-introspect-live before enumeration".into());
        }
        Ok(())
    };
    verify()?;
    let result = enumerate()?;
    verify()?;
    Ok(result)
}

#[cfg(test)]
mod identity_tests {
    use super::with_verified_inputs;
    use romacs_introspect::fingerprint::input_hash;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(dir.path().join("Cargo.lock"), "locked").unwrap();
        let live = dir.path().join("crates/romacs-introspect-live");
        std::fs::create_dir_all(live.join("src")).unwrap();
        std::fs::write(
            live.join("Cargo.toml"),
            "[package]\nname='live'\nversion='1.0.0'\n",
        )
        .unwrap();
        std::fs::write(live.join("src/lib.rs"), "source").unwrap();
        dir
    }

    #[test]
    fn stale_compiled_identity_refuses_before_enumeration() {
        let dir = fixture();
        let result = with_verified_inputs(dir.path(), &"0".repeat(64), || -> Result<(), String> {
            panic!("stale binary must never enumerate")
        });
        assert!(result.unwrap_err().contains("rebuild"));
    }

    #[test]
    fn source_change_during_enumeration_refuses_afterwards() {
        let dir = fixture();
        let compiled = input_hash(dir.path()).unwrap();
        let result = with_verified_inputs(dir.path(), &compiled, || {
            std::fs::write(
                dir.path().join("crates/romacs-introspect-live/src/lib.rs"),
                "changed",
            )
            .unwrap();
            Ok(())
        });
        assert!(result.unwrap_err().contains("rebuild"));
    }

    #[test]
    fn matching_compiled_identity_accepts_the_result() {
        let dir = fixture();
        let compiled = input_hash(dir.path()).unwrap();
        assert_eq!(
            with_verified_inputs(dir.path(), &compiled, || Ok(42)),
            Ok(42)
        );
    }
}

fn enumerate_verified(root: &Path, compiled_hash: &str) -> Result<Registry, String> {
    let universe = vm2_literals(root);
    if universe.is_empty() {
        return Err("no VM2 source literals found; cannot enumerate an empty universe".into());
    }
    let (vm2, failures) = vm2_registry(&universe);
    if !failures.is_empty() {
        return Err(format!(
            "VM2 enumeration incomplete: {}",
            failures.join("; ")
        ));
    }
    let (_, src_vm2) = romacs_introspect::Index::embedded().source_registered();
    let live_vm2: BTreeSet<&str> = vm2.iter().map(String::as_str).collect();
    let source_only = src_vm2
        .iter()
        .filter(|n| !live_vm2.contains(n.as_str()))
        .map(|n| format!("vm2:{n}"))
        .collect();
    Ok(Registry {
        core: Vec::new(),
        vm2,
        provenance: RegistryProvenance {
            generated_by: "romacs-introspect-live".into(),
            generated_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            generated_on: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            input_hash: Some(compiled_hash.to_owned()),
            core_method: "Retired: no core runtime is linked or enumerated.".into(),
            vm2_method: VM2_METHOD.into(),
            vm2_universe: universe.len(),
            vm2_probe_panics: Vec::new(),
            source_only_at_generation: source_only,
        },
    })
}
