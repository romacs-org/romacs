//! A font Romacs registers against a system copy of the same face: the
//! bundled Iosevka (assets/fonts) and an Iosevka the system has as well.
//! The test runs again in a child whose `FONTCONFIG_FILE` names a
//! configuration whose only font directory holds a copy of the bundled
//! Regular, so the "system" face is the same family, style and version, and
//! only the order Fontconfig lists them in could decide.

#![forbid(unsafe_code)]

#[path = "../../romacs-vm/tests/support/own_test.rs"]
mod own_test;
#[path = "support/fontconfig_child.rs"]
mod fontconfig_child;

use romacs_font::{FontRequest, FontResolver};

#[test]
fn a_registered_font_wins_a_tie_with_the_systems_copy() {
    let bundled = &romacs_test_fonts::bundled_iosevka()[0];
    let Some(root) = fontconfig_child::root() else {
        let root = std::env::temp_dir().join(format!("romacs-bundled-font-tie-{}", std::process::id()));
        let system = root.join("fonts");
        std::fs::create_dir_all(&system).unwrap();
        std::fs::copy(bundled, system.join("Iosevka-Regular.ttf")).unwrap();
        let config = romacs_test_fonts::isolated_fontconfig(&root, &[&system]).unwrap();
        let test = own_test::name(module_path!(), "a_registered_font_wins_a_tie_with_the_systems_copy");
        fontconfig_child::run(&test, &root, &config);
        let _ = std::fs::remove_dir_all(&root);
        return;
    };
    let system = root.join("fonts");
    let file = |resolver: &mut FontResolver| {
        let font = resolver.resolve_primary(&FontRequest::new("Iosevka", 16.0)).expect("Iosevka");
        std::path::PathBuf::from(font.identity.file_path.expect("a font file")).canonicalize().unwrap()
    };
    // Unregistered, the system's copy is the only Iosevka.
    assert_eq!(file(&mut FontResolver::new().unwrap()), system.join("Iosevka-Regular.ttf").canonicalize().unwrap());
    assert!(romacs_font::register_font_file(bundled));
    assert_eq!(file(&mut FontResolver::new().unwrap()), bundled.canonicalize().unwrap(),
               "the registered file wins the tie");
}
