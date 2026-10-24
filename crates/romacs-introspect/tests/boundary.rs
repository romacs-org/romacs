//! romacs-introspect must stay evaluator-free: it outlives romacs-core.

#[test]
fn no_romacs_crate_is_a_dependency() {
    let manifest = include_str!("../Cargo.toml");
    let mut section = "";
    let mut offending = Vec::new();
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            section = t;
            continue;
        }
        if t.starts_with('#') || !section.contains("dependencies") {
            continue;
        }
        if let Some((key, _)) = t.split_once('=')
            && key.trim().starts_with("romacs")
        {
            offending.push(format!("{section} {}", key.trim()));
        }
    }
    assert!(
        offending.is_empty(),
        "romacs-introspect must not depend on any runtime crate: {offending:?}"
    );
}
