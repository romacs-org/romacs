//! A catalog Fontconfig reloads moves on, lists the font that made it
//! reload, and still lists the fonts the process registered, as the GUI
//! registers its bundled ones (romacs#318); a registered file that cannot be
//! added to the reloaded configuration is added once it can. This binary's
//! one test owns the process's Fontconfig and points `FONTCONFIG_FILE`,
//! before Fontconfig starts, at a configuration whose only font directory
//! starts empty and which checks for changes every second.

use romacs_font::entity::{self, Catalog};
use std::path::Path;
use std::time::Duration;

/// The weights Fontconfig lists for Iosevka, sorted.
fn weights() -> Vec<i64> {
    let mut weights: Vec<i64> = entity::list(Some("Iosevka")).into_iter().map(|font| font.weight).collect();
    weights.sort_unstable();
    weights
}

/// Install FONT in DIR as Fontconfig's next check will see it: Fontconfig
/// compares whole seconds, so the directory changes a second after the
/// catalog was read, and the check comes a second later.
fn install_and_wait(font: &Path, dir: &Path) {
    std::thread::sleep(Duration::from_millis(1200));
    std::fs::copy(font, dir.join(font.file_name().unwrap())).unwrap();
    std::thread::sleep(Duration::from_millis(1200));
}

#[test]
fn a_reloaded_catalog_lists_the_installed_font_and_the_registered_ones() {
    let root = std::env::temp_dir().join(format!("romacs-catalog-reload-{}", std::process::id()));
    let (fonts, registered) = (root.join("fonts"), root.join("registered"));
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::create_dir_all(&registered).unwrap();
    let config = root.join("fonts.conf");
    std::fs::write(&config, format!("<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n\
        <fontconfig><dir>{}</dir><cachedir>{}</cachedir><config><rescan><int>1</int></rescan></config></fontconfig>\n",
        fonts.display(), root.join("cache").display())).unwrap();
    // UNSAFE(Fontconfig takes a configuration file before it starts only from FONTCONFIG_FILE, and std's set_var is unsafe)
    // SAFETY: the binary's only test, before anything reads the environment
    // on another thread.
    unsafe { std::env::set_var("FONTCONFIG_FILE", &config) };
    let [regular, bold, italic, _] = romacs_test_fonts::bundled_iosevka();
    let bold_copy = registered.join("Iosevka-Bold.ttf");
    std::fs::copy(&bold, &bold_copy).unwrap();
    assert!(romacs_font::register_font_file(&bold_copy), "Fontconfig refused {bold_copy:?}");

    let first = Catalog::current().expect("Fontconfig");
    assert_eq!(weights(), [200], "only the registered bold before an install");
    assert_eq!(Catalog::current(), Some(first), "no change, no reload");

    install_and_wait(&regular, &fonts);
    let second = Catalog::current().expect("Fontconfig");
    assert_ne!(second, first, "the changed font directory reloads the catalog");
    assert_eq!(weights(), [80, 200], "the installed regular and the registered bold");
    assert_eq!(Catalog::current(), Some(second), "one reload per change");

    std::fs::remove_file(&bold_copy).unwrap();
    install_and_wait(&italic, &fonts);
    let third = Catalog::current().expect("Fontconfig");
    assert_ne!(third, second, "the second install reloads the catalog");
    assert_eq!(weights(), [80, 80], "the registered bold is gone from its directory");

    std::fs::copy(&bold, &bold_copy).unwrap();
    let fourth = Catalog::current().expect("Fontconfig");
    assert_ne!(fourth, third, "adding the registered bold back moves the catalog");
    assert_eq!(weights(), [80, 80, 200], "the registered bold is listed again");
    let _ = std::fs::remove_dir_all(&root);
}
