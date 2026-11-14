use super::guide_stops;

#[test]
fn a_row_s_guides_are_its_tab_stops_on_the_frame_before_its_text() {
    let stops: Vec<(f32, u64)> = guide_stops(0.0, 16.0, 63.0, 96.0).collect();
    assert_eq!(stops, [(16.0, 0), (32.0, 1), (48.0, 2)]);
    // A row starting left of the frame: its stops on the frame, in phase,
    // deeper by those off it.
    let stops: Vec<(f32, u64)> = guide_stops(-40.0, 16.0, 30.0, 96.0).collect();
    assert_eq!(stops, [(8.0, 2), (24.0, 3)]);
    // Text past the frame: the stops end at its edge.
    assert_eq!(guide_stops(0.0, 16.0, 1000.0, 96.0).count(), 5, "16 to 80");
}

#[test]
#[expect(clippy::cast_precision_loss, reason = "a handful of stops")]
fn guides_end_however_far_a_row_lies() {
    // f32 steps would stall here: a stop added to the start rounds back.
    for (start, tab) in [(-1_073_741_824.0, 16.0), (-18_014_398_509_481_984.0, 31.2), (f32::MIN, 7.8)] {
        let stops: Vec<(f32, u64)> = guide_stops(start, tab, 79.0, 96.0).collect();
        assert!(stops.len() as f32 <= (96.0 / tab).ceil() + 1.0, "{start}: {} stops", stops.len());
        assert!(stops.iter().all(|&(x, _)| (-2.0..79.0).contains(&x)), "{start}: {stops:?}");
    }
    assert_eq!(guide_stops(f32::NAN, 16.0, 79.0, 96.0).count(), 0);
}
