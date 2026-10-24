#[test]
fn retained_face_gather_environment_is_forwarded_by_exact_name_only() {
    use std::ffi::OsString;
    let variables = [
        (
            OsString::from("ROMACS_RETAINED_FACE_GATHER"),
            OsString::from("on"),
        ),
        (
            OsString::from("ROMACS_RETAINED_FACE_GATHER_TRACE"),
            OsString::from("unrelated"),
        ),
    ];
    assert_eq!(
        crate::harness::passthrough_from(variables)
            .into_iter()
            .collect::<Vec<_>>(),
        vec![(
            "ROMACS_RETAINED_FACE_GATHER".to_owned(),
            OsString::from("on")
        )]
    );
}
