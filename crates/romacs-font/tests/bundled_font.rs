//! The bundled Iosevka (assets/fonts) resolves to itself as pinned GNU's
//! does, with only those four files in Fontconfig: the test runs again in a
//! child whose FONTCONFIG_FILE names a configuration with no font
//! directory, and registers the files as the GUI does. No host font takes
//! part.

#![forbid(unsafe_code)]

#[path = "../../romacs-vm/tests/support/own_test.rs"]
mod own_test;
#[path = "support/fontconfig_child.rs"]
mod fontconfig_child;

use romacs_display_protocol::font::FontSlantKind::Italic;
use romacs_font::{FontRequest, FontResolver};

#[test]
fn the_bundled_iosevka_resolves_to_itself_as_gnu_does() {
    // Pinned GNU pgtk, -Q, with only the four files in its Fontconfig
    // (tests/gnu/pinned-fonts.txt, run 1): each style draws every character
    // of "Καλημέρα Жж A" from its own file, "Καλημέρα" is 64px in each at
    // 16px, and an expanded or condensed request takes the normal face, the
    // only width Romacs ships.
    if fontconfig_child::root().is_none() {
        let root = std::env::temp_dir().join(format!("romacs-bundled-font-{}", std::process::id()));
        let config = romacs_test_fonts::isolated_fontconfig(&root, &[]).unwrap();
        let test = own_test::name(module_path!(), "the_bundled_iosevka_resolves_to_itself_as_gnu_does");
        fontconfig_child::run(&test, &root, &config);
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    let bundled = romacs_test_fonts::bundled_iosevka();
    for path in &bundled {
        assert!(romacs_font::register_font_file(path), "Fontconfig refused {path:?}");
    }
    let iosevka = || FontRequest::new("Iosevka", 16.0);
    let mut resolver = FontResolver::new().unwrap();
    for (request, file, (weight, slant)) in [
        (iosevka(), &bundled[0], (400, false)),
        (iosevka().with_weight(700), &bundled[1], (700, false)),
        (iosevka().with_slant(Italic), &bundled[2], (400, true)),
        (iosevka().with_weight(700).with_slant(Italic), &bundled[3], (700, true)),
        (iosevka().with_width(7), &bundled[0], (400, false)),
        (iosevka().with_width(3), &bundled[0], (400, false)),
    ] {
        let file = file.canonicalize().unwrap();
        let font = resolver.resolve_primary(&request).expect("Iosevka");
        assert_eq!((font.family.as_str(), font.weight, font.slant == Italic, font.width), ("Iosevka", weight, slant, 5), "{request:?}");
        let mut greek = 0.0;
        for ch in "Καλημέρα Жж A".chars() {
            let found = resolver.resolve_char(&request, ch).expect("resolved");
            let path = std::path::PathBuf::from(found.font.identity.file_path.expect("a font file"));
            assert_eq!((found.font.family.as_str(), path.canonicalize().unwrap()), ("Iosevka", file.clone()), "{request:?} {ch}");
            if "Καλημέρα".contains(ch) {
                greek += found.glyph.advance_px;
            }
        }
        assert_eq!(greek, 64.0, "{request:?}: Greek pixel width");
    }
}
