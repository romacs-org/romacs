use super::*;

#[test]
fn cache_environment_is_resolved_in_subprocesses() {
    const CHILD: &str = "ROMACS_FIXTURE_CACHE_PROBE";
    if let Some(expected) = std::env::var_os(CHILD) {
        let expected = PathBuf::from(expected);
        assert_eq!(cache_root(), expected);
        std::fs::create_dir_all(&expected).unwrap();
        let name = "environment-probe.bin";
        let path = expected.join(name);
        std::fs::write(&path, b"verified cache probe").unwrap();
        let sha = crate::inventory::sha256_hex(b"verified cache probe");
        let resolved = pinned_file(name, "https://127.0.0.1:1/never", &sha).unwrap();
        assert_eq!(resolved, path);
        std::fs::write(&path, b"corrupted").unwrap();
        assert!(pinned_file(name, "https://127.0.0.1:1/never", &sha).is_err());
        return;
    }
    let root = std::env::temp_dir().join(format!("romacs-cache-probe-{}", std::process::id()));
    for mode in ["override", "home", "workspace"] {
        let home = root.join(mode).join("home");
        let workspace = root.join(mode).join("workspace");
        let expected = match mode {
            "override" => root.join(mode).join("explicit"),
            "home" => home.join(".cache/romacs-test-fonts"),
            _ => workspace.join("tmp/font-fixtures"),
        };
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "pinned::tests::cache_environment_is_resolved_in_subprocesses",
                "--nocapture",
            ])
            .env(CHILD, &expected)
            .env("NEXTEST_WORKSPACE_ROOT", &workspace)
            .env_remove("ROMACS_TEST_FONTS_DIR")
            .env_remove("HOME");
        if mode == "override" {
            child
                .env("ROMACS_TEST_FONTS_DIR", &expected)
                .env("HOME", &home);
        } else if mode == "home" {
            child.env("HOME", &home);
        }
        let output = child.output().unwrap();
        assert!(output.status.success(), "{mode}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{output:?}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// The bytes the cache holds are what the hash describes: a cached fixture
/// with the pinned hash is accepted, and one with any other hash is not.
#[test]
fn verification_accepts_only_the_pinned_bytes() {
    let root = cache_root();
    std::fs::create_dir_all(&root).expect("cache root");
    let path = root.join(format!("verify-probe-{}.bin", std::process::id()));
    std::fs::write(&path, b"pinned bytes").expect("write probe");

    let good = crate::inventory::sha256_hex(b"pinned bytes");
    assert!(matches!(verify(&path, &good), Ok(Ok(()))));
    match verify(&path, &"0".repeat(64)) {
        Ok(Err(message)) => assert!(message.contains("SHA-256"), "{message}"),
        other => panic!("a wrong hash must be reported: {other:?}"),
    }
    std::fs::remove_file(&path).ok();
}

/// A cached fixture is trusted without touching the network: the URL below
/// cannot resolve, and the call still succeeds because the bytes on disk
/// verify. This is also what keeps a suite that already fetched its fixtures
/// runnable offline.
#[test]
fn a_verified_cache_entry_needs_no_network() {
    let root = cache_root();
    std::fs::create_dir_all(&root).expect("cache root");
    let name = format!("cache-probe-{}.bin", std::process::id());
    let path = root.join(&name);
    std::fs::write(&path, b"cached fixture").expect("write probe");
    let sha256 = crate::inventory::sha256_hex(b"cached fixture");

    let resolved = pinned_file(&name, "https://127.0.0.1:1/never", &sha256)
        .expect("a verified cache entry resolves offline");
    assert_eq!(resolved, path);
    std::fs::remove_file(&path).ok();
}
