#[path = "../build/derive.rs"]
mod derive;
use derive::{SourceFile, vm2_sites};

#[test]
fn defsubr_span_follows_the_human_declaration() {
    let declaration = "defsubr! { name = \"probe\", gnu = \"fns.c:1\", effect = Pure,\nfn probe(cx, value: Value) { Ok(value) }}\n";
    for (file, padding) in [
        ("crates/romacs-vm/src/prims/a.rs", 0),
        ("crates/romacs-vm/src/prims/moved.rs", 7),
    ] {
        let source = SourceFile::new(
            file.into(),
            format!("{}{declaration}", "\n".repeat(padding)),
        );
        let sites = vm2_sites(&[source]);
        assert_eq!(sites.len(), 1, "one human declaration, not generated table");
        assert_eq!(sites[0].name, "probe");
        assert_eq!(sites[0].file, file);
        assert_eq!(sites[0].line, padding + 1);
        assert_eq!(sites[0].kind, "defsubr");
        assert_eq!(sites[0].variant, "probe");
    }
}

#[test]
fn adapter_macro_examples_are_not_generated_registrations() {
    let source = SourceFile::new("crates/romacs-vm/src/prim.rs".into(),
        "#[cfg(test)] mod tests { defsubr! { name = \"fixture\", gnu = \"fns.c:1\", effect = Pure, fn fixture(cx) { todo!() }} }".into());
    assert!(vm2_sites(&[source]).is_empty());
}

#[test]
fn test_fixture_is_not_a_production_registration_site() {
    let source = SourceFile::new("crates/romacs-vm/src/prims/callback_test.rs".into(),
        "defsubr! { name = \"romacs--t2-assoc\", gnu = \"fns.c:1\", effect = Global, fn fixture(cx) -> Step { todo!() }}".into());
    assert!(vm2_sites(&[source]).is_empty());
}

#[test]
fn every_generated_primitive_has_an_exact_human_declaration_span() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join("crates/romacs-vm/src/prims");
    let files: Vec<_> = derive::rs_files(&dir)
        .into_iter()
        .map(|path| {
            SourceFile::new(
                path.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                std::fs::read_to_string(path).unwrap(),
            )
        })
        .collect();
    let sites = vm2_sites(&files);
    let expected: usize = files
        .iter()
        .filter(|f| !derive::is_test_path(&f.rel))
        .map(|f| {
            f.text
                .lines()
                .filter(|line| line.trim_start().starts_with("defsubr!"))
                .count()
        })
        .sum();
    assert!(expected > 0, "production declarations exist");
    assert_eq!(sites.len(), expected, "every production declaration found");
    for site in sites {
        let source = std::fs::read_to_string(root.join(&site.file)).unwrap();
        let line = source.lines().nth(site.line - 1).unwrap();
        assert!(
            line.contains("defsubr!"),
            "{}:{}: {line}",
            site.file,
            site.line
        );
        assert!(!site.name.starts_with("romacs--t2-"));
        let romacs_introspect::Lookup::Found(description) =
            romacs_introspect::Index::embedded().describe(&site.name)
        else {
            panic!("{} not registered", site.name)
        };
        assert!(
            description.vm2.iter().any(|v| v.site.file == site.file
                && v.site.line == site.line
                && v.kind == "defsubr"),
            "{}: {:?}",
            site.name,
            description.vm2
        );
    }
}

#[test]
fn production_registry_excludes_t2_fixture() {
    assert!(
        !romacs_introspect::Index::embedded()
            .registry()
            .vm2
            .iter()
            .any(|n| n == "romacs--t2-assoc")
    );
}
