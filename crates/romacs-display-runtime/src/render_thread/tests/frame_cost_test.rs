use super::*;

#[test]
fn a_line_reports_every_stretch_and_every_count_of_the_renderer() {
    let counts: Vec<_> = names().map(|name| (name, 0)).collect();
    let said = line(7, "rebuild", SubmissionResult::Submitted, 1, &counts);
    for wanted in [
        "ingest_instructions",
        "frame_instructions",
        "acquire_instructions",
        "present_instructions",
        "submit_instructions",
        "rows",
        "rows_tessellated",
        "rows_reused",
        "rows_unmarked",
        "glyphs_walked",
        "font_identities",
        "bytes_uploaded",
    ] {
        assert_eq!(said[wanted], 0, "{wanted}");
    }
    assert_eq!(said["diagnostic"], "frame-cost");
    assert_eq!((&said["presentation"], &said["work"], &said["result"], &said["scenes"]), (&7.into(), &"rebuild".into(), &"Submitted".into(), &1.into()));
    assert_eq!(said.as_object().map(serde_json::Map::len), Some(5 + counts.len()), "no count shadows another or a heading");
}

#[test]
fn a_line_counts_from_the_line_before() {
    let last = HashMap::from([(FRAME, 100), (romacs_renderer_wgpu::cost::ROWS, 45)]);
    let now = [(FRAME, 130), (romacs_renderer_wgpu::cost::ROWS, 90), (INGEST, 5)];
    assert_eq!(added(&now, &last), [(FRAME, 30), (romacs_renderer_wgpu::cost::ROWS, 45), (INGEST, 5)], "a count the line before had not seen starts at nothing");
}
