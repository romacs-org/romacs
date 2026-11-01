//! The armillary sphere (src/lib.rs) draws the frames of the site
//! it was ported from: tests/fixtures/armillary-site.jsonl holds views and
//! the frames thanosapollo.org's ascii-home.js `B()` gave for them
//! (tools/armillary-site-frames.mjs).  ROMACS_ARMILLARY_FRAMES names
//! another such file to compare as well.
use romacs_armillary::{frame, View};

fn view(json: &serde_json::Value) -> View {
    let number = |key: &str| json[key].as_f64().unwrap();
    View {
        cols: number("columns") as usize, rows: number("rows") as usize,
        pitch: number("pitch"), yaw: number("yaw"), roll: number("roll"), zoom: number("zoom"),
        cell_aspect: number("cellAspect"), stars: json["stars"].as_bool().unwrap(),
    }
}

fn compare(frames: &str) -> usize {
    let mut count = 0;
    for line in frames.lines().filter(|l| !l.is_empty()) {
        let case: serde_json::Value = serde_json::from_str(line).unwrap();
        let view = view(&case["view"]);
        assert!(view.valid(), "{view:?}");
        assert_eq!(frame(&view), case["frame"].as_str().unwrap(), "{view:?}");
        count += 1;
    }
    count
}

#[test]
fn frames_match_the_site() {
    assert_eq!(compare(include_str!("fixtures/armillary-site.jsonl")), 12);
    if let Ok(path) = std::env::var("ROMACS_ARMILLARY_FRAMES") {
        let more = compare(&std::fs::read_to_string(&path).unwrap());
        eprintln!("{more} frames of {path} match");
    }
}
