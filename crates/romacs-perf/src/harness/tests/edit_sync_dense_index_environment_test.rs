//! Exact request-owned passthrough policy; no process environment mutation.
#[test]
fn dense_edit_index_control_is_forwarded_and_lookalike_is_rejected() {
    use std::ffi::OsString;
    let variables = [
        (
            OsString::from("ROMACS_EDIT_SYNC_DENSE_INDEX"),
            OsString::from("on"),
        ),
        (
            OsString::from("ROMACS_EDIT_SYNC_DENSE_INDEX_TRACE"),
            OsString::from("unrelated"),
        ),
    ];
    assert_eq!(
        crate::harness::passthrough_from(variables)
            .into_iter()
            .collect::<Vec<_>>(),
        vec![(
            "ROMACS_EDIT_SYNC_DENSE_INDEX".to_owned(),
            OsString::from("on")
        )]
    );
}
