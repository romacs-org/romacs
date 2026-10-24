use super::*;
use romacs_layout_input::{
    FaceHandle, FontAlternativeState, FontSelectionState, PlacedWindowSnapshot,
};

#[test]
fn alternative_snapshot_skipped_round_trip_reaches_retained_resolver() {
    let mut engine = LayoutEngine::new().unwrap();
    let mut snapshot = FrameSnapshot {
        font_selection: FontSelectionState::default(),
        font_alternatives: FontAlternativeState::default(),
        width: 400,
        height: 300,
        windows: vec![PlacedWindowSnapshot {
            x: 0,
            y: 0,
            snapshot: WindowSnapshot {
                id: 1,
                width: 400,
                height: 300,
                start_byte: 0,
                point_byte: 0,
                buffer_version: 0,
                text: TextSlice {
                    utf8: "A".into(),
                    face_runs: vec![],
                },
                mode_line: None,
                mode_line_face: FaceHandle(0),
                vscroll_px: 0,
                faces: vec![Face {
                    foreground: [255; 3],
                    background: [0; 3],
                    family: "monospace".into(),
                    pixel_size: 16.0,
                }],
            },
        }],
    };
    engine.layout_frame(&snapshot, 1, None).unwrap();
    let before = snapshot.clone();
    snapshot
        .font_alternatives
        .set_family_alist(vec![vec!["missing".into(), "monospace".into()]]);
    snapshot.font_alternatives.set_family_alist(vec![]);
    assert_eq!(
        snapshot.font_alternatives.family_alist(),
        before.font_alternatives.family_alist()
    );
    assert_ne!(
        snapshot.font_alternatives.revision(),
        before.font_alternatives.revision()
    );
    engine.layout_frame(&snapshot, 2, None).unwrap();
    assert_eq!(
        engine.resolver.alternative_state(),
        &snapshot.font_alternatives,
        "LAYOUT_ADOPTS_ALTERNATIVE_REVISION_ON_SKIPPED_ROUND_TRIP"
    );
    assert_eq!(
        before.font_alternatives.revision(),
        0,
        "previous snapshot is immutable"
    );
}
