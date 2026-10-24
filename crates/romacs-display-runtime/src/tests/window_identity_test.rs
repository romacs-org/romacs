use super::*;

#[test]
fn packaged_desktop_entry_matches_typed_runtime_identity() {
    let desktop_entry = include_str!("../../assets/romacs.desktop");

    assert_eq!(ROMACS_APPLICATION.app_id().as_str(), "romacs");
    assert_eq!(
        ROMACS_APPLICATION.desktop_file_id().as_str(),
        "romacs.desktop"
    );
    assert_eq!(ROMACS_APPLICATION.icon_name().as_str(), "romacs");
    assert!(desktop_entry.contains("\nIcon=romacs\n"));
    assert!(desktop_entry.contains("\nStartupWMClass=romacs\n"));
}

#[test]
fn linux_window_attributes_use_packaged_desktop_id() {
    for wayland in [true, false] {
        let attrs = linux_window_identity(WindowAttributes::default(), wayland);
        assert!(format!("{attrs:?}").contains(ROMACS_APPLICATION.app_id().as_str()));
    }
}
