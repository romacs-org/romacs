use super::*;

#[test]
fn the_cache_is_the_override_else_homes() {
    let explicit = PathBuf::from("/fixtures");
    let home = PathBuf::from("/home/a");
    assert_eq!(cache_root_from(Some(explicit.clone()), Some(home.clone())), Some(explicit));
    assert_eq!(
        cache_root_from(None, Some(home)),
        Some(PathBuf::from("/home/a/.cache/romacs-test-fonts"))
    );
    assert_eq!(cache_root_from(None, None), None);
}

/// Every file a function names is one the fetch script provides.
#[test]
fn the_manifest_pins_every_fixture() {
    let manifest = include_str!("../../fixtures.json");
    let spleen = SpleenFixtures::FILES.map(|file| format!("spleen-2.2.0/{file}"));
    for file in FILES.iter().copied().chain(spleen.iter().map(String::as_str)) {
        assert!(manifest.contains(&format!("\"file\": \"{file}\"")), "{file} is not in fixtures.json");
    }
}
