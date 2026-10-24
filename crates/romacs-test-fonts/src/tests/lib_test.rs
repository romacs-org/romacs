use crate::*;

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
        let sha = sha256_bytes(b"verified cache probe");
        let resolved = prepare_pinned_file(
            name,
            "https://127.0.0.1:1/never",
            Box::leak(sha.clone().into_boxed_str()),
        )
        .unwrap();
        assert_eq!(resolved, path);
        std::fs::write(&path, b"corrupted").unwrap();
        assert!(
            prepare_pinned_file(
                name,
                "https://127.0.0.1:1/never",
                Box::leak(sha.clone().into_boxed_str())
            )
            .is_err()
        );
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
                "tests::lib_test::cache_environment_is_resolved_in_subprocesses",
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

#[test]
fn downloads_and_verifies_every_public_fixture() {
    let fixtures = spleen_2_2_0();
    for path in [
        fixtures.bdf(),
        fixtures.pcf(),
        fixtures.pcf_gz(),
        fixtures.otb(),
        fixtures.woff(),
        fixtures.woff2(),
    ] {
        assert!(path.is_file(), "{}", path.display());
    }
    assert!(gzip_expands_to(&fixtures.pcf_gz(), &fixtures.pcf()).unwrap());
}

#[test]
fn downloads_the_nonzero_face_woff2_collection_fixture() {
    let bytes = std::fs::read(crate::woff2_collection()).expect("downloaded WOFF2 collection");
    assert_eq!(&bytes[..8], b"wOF2ttcf");
}

#[test]
fn downloads_the_pinned_monospace_and_symbol_fixtures() {
    for path in [
        crate::symbols_nerd_font_mono_3_4_0(),
        crate::jetbrains_mono_nerd_font_regular_3_4_0(),
        crate::noto_sans_mono_variable(),
        crate::noto_color_emoji_2_051(),
        crate::noto_color_emoji_colrv1(),
        crate::nabla_color_colrv1(),
        crate::mplus_1_code_thin(),
        crate::lxgw_wenkai_nerd_regular_1_522(),
        crate::plemol_jp_console_nf_regular(),
    ] {
        let bytes = std::fs::read(path).expect("downloaded fixture");
        assert_eq!(&bytes[..4], b"\0\x01\0\0", "{}", path.display());
    }
}
