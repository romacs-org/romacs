use super::*;

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let base = env::temp_dir().join(format!("vm2-attribution-{}-{name}", std::process::id()));
        fs::create_dir(&base).unwrap();
        Self(base)
    }

    fn generate(&self, checkout: &str) -> (String, String, serde_json::Value) {
        let root = self.0.join(checkout);
        let out = root.join("out");
        fs::create_dir_all(root.join("src/prims/tests")).unwrap();
        fs::create_dir(&out).unwrap();
        fs::write(root.join("src/lib.rs"), "static PRIMITIVES: &[()] = &[];").unwrap();
        for (file, name, ident) in [
            ("domain.rs", "z-production", "production"),
            ("early.rs", "y-production", "early"),
            ("call_test.rs", "b-fixture", "fixture"),
            ("tests/nested.rs", "a-nested-fixture", "nested_fixture"),
        ] {
            fs::write(root.join("src/prims").join(file), format!(
                "defsubr! {{ name = \"{name}\", gnu = \"fns.c:1\", effect = Pure, fn {ident}(cx) {{ Ok(Value::NIL) }} }}"
            )).unwrap();
        }
        generate_primitives(&root, &out);
        (
            fs::read_to_string(out.join("prims_mods.rs")).unwrap(),
            fs::read_to_string(out.join("generated_prims.rs")).unwrap(),
            serde_json::from_str(&fs::read_to_string(out.join("primitives.json")).unwrap())
                .unwrap(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn checkout_tests_component_does_not_change_module_attribution() {
    let fixture = Fixture::new("checkout");
    for checkout in ["ordinary/romacs", "x/tests/romacs"] {
        let (modules, table, manifest) = fixture.generate(checkout);
        for (module, test_only) in [
            ("domain", false),
            ("early", false),
            ("call_test", true),
            ("tests__nested", true),
        ] {
            let line = modules
                .lines()
                .find(|line| line.ends_with(&format!("mod {module};")))
                .unwrap();
            assert_eq!(
                line.starts_with("#[cfg(test)]"),
                test_only,
                "{checkout}: {module}: {line}"
            );
        }
        for (descriptor, test_only) in [
            ("domain::production", false),
            ("early::early", false),
            ("call_test::fixture", true),
            ("tests__nested::nested_fixture", true),
        ] {
            let line = table
                .lines()
                .find(|line| line.contains(&format!("{descriptor}::DESC")))
                .unwrap();
            assert_eq!(
                line.contains("#[cfg(test)]"),
                test_only,
                "{checkout}: {line}"
            );
        }
        let rows = manifest.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "y-production");
        assert_eq!(rows[1]["name"], "z-production");
        assert_eq!(rows[1]["rust"], "src/prims/domain.rs:1");
    }
}

#[test]
fn test_descriptors_follow_all_production_descriptors() {
    let fixture = Fixture::new("ordinals");
    let (_, table, manifest) = fixture.generate("ordinary/romacs");
    let descriptors: Vec<_> = table
        .lines()
        .filter(|line| line.contains("::DESC"))
        .collect();
    let production: Vec<_> = descriptors
        .iter()
        .filter(|line| !line.contains("#[cfg(test)]"))
        .copied()
        .collect();
    assert_eq!(
        &descriptors[..production.len()],
        production.as_slice(),
        "production ordinals must not move in test images"
    );
    assert_eq!(production.len(), manifest.as_array().unwrap().len());
    assert_eq!(descriptors.len(), 4);
    assert!(descriptors[0].contains("early::early::DESC"));
    assert!(descriptors[1].contains("domain::production::DESC"));
    assert!(descriptors[2].contains("tests__nested::nested_fixture::DESC"));
    assert!(descriptors[3].contains("call_test::fixture::DESC"));
}

#[test]
fn actual_test_image_preserves_production_manifest_ordinals() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/primitives.json"))).unwrap();
    let rows = manifest.as_array().unwrap();
    assert!(!rows.is_empty());
    assert!(crate::prim::GENERATED.len() > rows.len());
    for (ordinal, row) in rows.iter().enumerate() {
        assert_eq!(
            crate::prim::GENERATED[ordinal].name,
            row["name"].as_str().unwrap(),
            "production ordinal {ordinal}"
        );
    }
}
