use super::*;

fn candidate(name: &str) -> (&OsStr, PathBuf) {
    (OsStr::new(name), Path::new("/runtime").join(name))
}

#[test]
fn explicit_wayland_wins_even_without_an_ambient_socket() {
    let selection = select(Some(candidate("oracle")), None, Some(OsStr::new(":1"))).unwrap();
    assert_eq!(selection.name(), OsStr::new("oracle"));
    assert_eq!(selection.pixel_policy(), PixelPolicy::Logical);
}

#[test]
fn existing_wayland_socket_wins_over_x11() {
    let selection = select(None, Some(candidate("wayland-4")), Some(OsStr::new(":1"))).unwrap();
    assert_eq!(selection.name(), OsStr::new("wayland-4"));
    assert_eq!(selection.pixel_policy().effective_scale(1.5), 1.5);
}

#[test]
fn x11_is_used_without_a_wayland_socket() {
    let selection = select(None, None, Some(OsStr::new(":1"))).unwrap();
    assert_eq!(selection.name(), OsStr::new(":1"));
    assert_eq!(selection.pixel_policy(), PixelPolicy::Physical);
    assert_eq!(selection.pixel_policy().effective_scale(1.5), 1.0);
}

#[test]
fn absent_displays_name_both_environment_variables() {
    for display in [None, Some(OsStr::new(""))] {
        let error = select(None, None, display).unwrap_err();
        assert!(
            error.contains("WAYLAND_DISPLAY") && error.contains("or DISPLAY"),
            "{error}"
        );
    }
}
