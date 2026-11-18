//! A catalog Fontconfig reloads compares unequal to the one before it, lists
//! the font that made it reload, and still lists the fonts the process
//! registered, as the GUI registers its bundled ones (romacs#318). This
//! binary's one test owns the process's Fontconfig and points
//! `FONTCONFIG_FILE`, before Fontconfig starts, at a configuration whose only
//! font directory starts empty and which checks for changes every second.

use romacs_font::entity::{self, Catalog};
use std::time::Duration;

#[test]
fn a_font_installed_in_a_font_directory_reloads_the_catalog_keeping_registered_fonts() {
    let root = std::env::temp_dir().join(format!("romacs-catalog-reload-{}", std::process::id()));
    let fonts = root.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    let config = root.join("fonts.conf");
    std::fs::write(&config, format!("<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n\
        <fontconfig><dir>{}</dir><cachedir>{}</cachedir><config><rescan><int>1</int></rescan></config></fontconfig>\n",
        fonts.display(), root.join("cache").display())).unwrap();
    // UNSAFE(Fontconfig takes a configuration file before it starts only from FONTCONFIG_FILE, and std's set_var is unsafe)
    // SAFETY: the binary's only test, before anything reads the environment
    // on another thread.
    unsafe { std::env::set_var("FONTCONFIG_FILE", &config) };
    let bundled = romacs_test_fonts::bundled_iosevka();
    assert!(romacs_font::register_font_file(&bundled[1]), "Fontconfig refused {:?}", bundled[1]);
    let weights = || {
        let mut weights: Vec<i64> = entity::list(Some("Iosevka")).into_iter().map(|font| font.weight).collect();
        weights.sort_unstable();
        weights
    };
    let before = Catalog::current().expect("Fontconfig");
    assert_eq!(weights(), [200], "only the registered bold before the install");
    assert!(Catalog::current().as_ref() == Some(&before), "no change, no reload");
    // Fontconfig compares whole seconds: the font directory changes a second
    // after the catalog was read, and the next check comes a second later.
    std::thread::sleep(Duration::from_millis(1200));
    std::fs::copy(&bundled[0], fonts.join("Iosevka-Regular.ttf")).unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    let after = Catalog::current().expect("Fontconfig");
    assert!(after != before, "the changed font directory reloads the catalog");
    assert_eq!(weights(), [80, 200], "the reloaded catalog lists the installed regular and the registered bold");
    assert!(Catalog::current().as_ref() == Some(&after), "one reload per change");
    let _ = std::fs::remove_dir_all(&root);
}
