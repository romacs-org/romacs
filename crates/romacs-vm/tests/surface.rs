use romacs_vm::{Form, ImageRegion, Vm};
use sha2::{Digest, Sha256};
use std::path::Path;

fn hash_sources(root: &Path, dir: &Path, hash: &mut Sha256) {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            hash_sources(root, &path, hash);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let bytes = std::fs::read(&path).unwrap();
            hash.update(
                path.strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .as_bytes(),
            );
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
}

fn replay(primitive: &str) {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/s87-surface.json")).unwrap();
    // compiler_id stays in the evidence as provenance but is deliberately NOT
    // asserted. VM2_COMPILER_ID hashes the workspace Cargo.lock, which is
    // correct for its real job -- invalidating cached IR and mapped images
    // when any dependency changes codegen -- and wrong as an oracle pin.
    // Adding an unrelated crate to the workspace changed the lock and failed
    // all three of these tests with crates/romacs-vm byte-identical and every
    // recorded answer still correct.
    //
    // source_sha256 likewise stays as recorded provenance, not an asserted
    // pin: it hashes all VM2 Rust sources, which we intentionally rewrite even
    // when primitive behaviour is unchanged. Like compiler_id, that digest is
    // correct for its real job and wrong as an oracle pin.
    //
    // Nothing behavioural is lost. generator_sha256 pins the harness, and the
    // 69 cases replay actual primitive results against recorded GNU answers --
    // so a source or dependency change that genuinely changes behaviour fails
    // on the answers, which is a stronger check than a digest comparison.
    assert_eq!(evidence["protocol"], "surface-compiled-v1");
    assert_eq!(evidence["regime"], "GNU byte-compiled / VM2 compiled");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hash = Sha256::new();
    hash_sources(root, &root.join("src"), &mut hash);
    let source_hash: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let _source_hash = source_hash; // Diagnostic provenance, not an oracle pin.
    let generator = std::fs::read(root.join("tools/surface_oracle.py")).unwrap();
    assert_eq!(
        evidence["generator_sha256"],
        Sha256::digest(generator)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let rows: Vec<_> = evidence["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["primitive"] == primitive)
        .collect();
    assert_eq!(rows.len(), 23);
    let store = std::env::temp_dir().join(format!("vm2-s87-{primitive}-{}", std::process::id()));
    std::fs::create_dir_all(&store).unwrap();
    for row in rows {
        assert_eq!(row["version"], "32.0.50");
        let source = row["source"].as_str().unwrap();
        let expected = row["expected"].as_str().unwrap().to_owned();
        let form = Form::from_json(row["transport"].clone()).unwrap();
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.read_source("(garbage [1 2 3])").unwrap();
        let entry = vm.compile(&form).unwrap();
        vm.collect_garbage();
        assert!(vm.heap_stats().reclaimed > 0);
        let key = vm.publish_code_image(&store, entry, [0; 32]);
        // The second execution uses the now-cached ordinary call target.
        for _ in 0..2 {
            let actual = vm.execute(entry).map(|value| value.root()).and_then(|value| {
                vm.collect_garbage();
                value.get(&vm)?.display()
            });
            assert_eq!(actual, Ok(expected.clone()), "heap: {source}");
        }
        if let Ok(key) = key {
            // SAFETY: the content-addressed store is private and never overwritten.
            let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
            let (mut mapped, entry) = Vm::from_code_image(image, [0; 32]).unwrap();
            mapped.set_collection_floor(1);
            mapped.collect_garbage();
            for _ in 0..2 {
                let actual = mapped.execute(entry).map(|value| value.root()).and_then(|value| {
                    mapped.collect_garbage();
                    value.get(&mapped)?.display()
                });
                assert_eq!(actual, Ok(expected.clone()), "mapped: {source}");
            }
        } else {
            // Existing executable-image format cannot encode boxed float literals.
            assert_eq!(row["transport"][1], 3.5, "unmapped case: {source}");
            assert_eq!(
                key.unwrap_err(),
                "code image supports only immutable string/cons literals"
            );
        }
        // Source requests collect at their boundary. Never retain a raw Value
        // across the next request; only inspect that request's rewritten result.
        let mut source_vm = Vm::new();
        source_vm.set_collection_floor(1);
        for _ in 0..2 {
            source_vm
                .read_source(&format!("({})", "1 ".repeat(100)))
                .unwrap();
            let before = source_vm.heap_stats().collections;
            let actual = source_vm
                .eval_source(source)
                .and_then(|value| value.display());
            assert_eq!(actual, Ok(expected.clone()), "source request: {source}");
            assert!(source_vm.heap_stats().collections > before);
        }
    }
    std::fs::remove_dir_all(store).unwrap();
}

#[test]
fn recovered_consp_matches_compiled_gnu() {
    replay("consp");
}

#[test]
fn recovered_car_safe_matches_compiled_gnu() {
    replay("car-safe");
}

#[test]
fn recovered_symbolp_matches_compiled_gnu() {
    replay("symbolp");
}
