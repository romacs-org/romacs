//! The bundled Iosevka (assets/fonts) resolves to itself as pinned GNU's
//! does, with only those four files in Fontconfig: this binary's one test
//! owns the process's Fontconfig and points FONTCONFIG_FILE, before
//! Fontconfig starts, at a configuration with no font directory, then
//! registers the files as the GUI does. No host font takes part.

use romacs_display_protocol::font::FontSlantKind::Italic;
use romacs_font::{FontRequest, FontResolver};

#[test]
fn the_bundled_iosevka_resolves_to_itself_as_gnu_does() {
    // Pinned GNU pgtk, -Q, with only the four files in its Fontconfig
    // (tests/gnu/pinned-fonts.txt, run 1): each style draws every character
    // of "Καλημέρα Жж A" from its own file, "Καλημέρα" is 64px in each at
    // 16px, and an expanded or condensed request takes the normal face, the
    // only width Romacs ships.
    let root = std::env::temp_dir().join(format!("romacs-bundled-font-{}", std::process::id()));
    let config = romacs_test_fonts::isolated_fontconfig(&root, &[]).unwrap();
    // SAFETY: the binary's only test, before anything reads the environment
    // on another thread.
    unsafe { std::env::set_var("FONTCONFIG_FILE", &config) };
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
    let _ = std::fs::remove_dir_all(&root);
}
