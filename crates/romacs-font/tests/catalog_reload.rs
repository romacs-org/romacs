//! A catalog Fontconfig reloads compares unequal to the one before it, and
//! lists the font that made it reload: what a kept listing checks before it
//! answers. This binary's one test owns the process's Fontconfig and points
//! `FONTCONFIG_FILE`, before Fontconfig starts, at a configuration whose only
//! font directory starts empty and which checks for changes every second.

use romacs_font::entity::{self, Catalog};
use std::time::Duration;

#[test]
fn a_font_installed_in_a_font_directory_reloads_the_catalog() {
    let root = std::env::temp_dir().join(format!("romacs-catalog-reload-{}", std::process::id()));
    let fonts = root.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    let config = root.join("fonts.conf");
    std::fs::write(&config, format!("<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n\
        <fontconfig><dir>{}</dir><cachedir>{}</cachedir><config><rescan><int>1</int></rescan></config></fontconfig>\n",
        fonts.display(), root.join("cache").display())).unwrap();
    // SAFETY: the binary's only test, before anything reads the environment
    // on another thread.
    unsafe { std::env::set_var("FONTCONFIG_FILE", &config) };
    let before = Catalog::current().expect("Fontconfig");
    assert!(entity::list(Some("Iosevka")).is_empty(), "no font installed yet");
    assert!(Catalog::current().as_ref() == Some(&before), "no change, no reload");
    // Fontconfig compares whole seconds: the font directory changes a second
    // after the catalog was read, and the next check comes a second later.
    std::thread::sleep(Duration::from_millis(1200));
    std::fs::copy(&romacs_test_fonts::bundled_iosevka()[0], fonts.join("Iosevka-Regular.ttf")).unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    let after = Catalog::current().expect("Fontconfig");
    assert!(after != before, "the changed font directory reloads the catalog");
    assert!(!entity::list(Some("Iosevka")).is_empty(), "the reloaded catalog lists the new font");
    assert!(Catalog::current().as_ref() == Some(&after), "one reload per change");
    let _ = std::fs::remove_dir_all(&root);
}
