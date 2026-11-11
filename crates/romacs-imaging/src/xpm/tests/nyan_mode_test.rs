//! The issue's upstream artwork, pinned rather than vendored.
//!
//! `nyan.xpm` is TeMPOraL/nyan-mode's, so a copy in this tree would be a fork
//! of it: `romacs-test-fonts`' fixtures.json pins its URL and SHA-256, and
//! tools/fetch-test-fixtures fetches it into the test cache. The report's claim was that
//! "gray details turn black" in this file, and its palette is where the
//! ladder lives -- fourteen `grayNN` names, nine of which the pixel data paints.

use super::*;

/// The named entries this file's own pixel data paints, in the order X11's
/// `grayNN` ladder defines them: `grayNN` is `255 * N / 100`, rounded.
const NYAN_MODE_PAINTED_GRAYS: [&str; 9] = [
    "gray15", "gray20", "gray22", "gray31", "gray38", "gray40", "gray59", "gray60", "gray81",
];

/// A fallback the artwork never uses: every key in its palette resolves, and
/// one that stopped resolving would paint this instead of collapsing into a
/// gray, so the ladder assertions below fail either way.
const FALLBACK: [u8; 3] = [0xff, 0x00, 0xff];

#[test]
fn nyan_mode_xpm_keeps_its_gray_ladder() {
    let data = std::fs::read(romacs_test_fonts::nyan_mode_xpm()).expect("the pinned file is readable");
    let (width, height, rgba) = decode_xpm_data(&data, FALLBACK).expect("nyan.xpm decodes");
    assert_eq!((width, height), (25, 15));

    // The expectation comes from the shared database the decoder itself reads,
    // and that database is pinned against etc/rgb.txt (drift guard) and against
    // GNU's own `color-values` (the XPM named-colors GUI test), so agreement
    // here is agreement with GNU.
    let painted: std::collections::HashSet<[u8; 3]> = rgba
        .chunks_exact(4)
        .filter(|pixel| pixel[3] == 255)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    for name in NYAN_MODE_PAINTED_GRAYS {
        let (red, green, blue) = romacs_display_protocol::x11_color_lookup(name)
            .expect("the ladder is in the database");
        assert!(
            painted.contains(&[red, green, blue]),
            "{name} is not painted as ({red}, {green}, {blue}) in nyan.xpm -- \
             the gray ladder has collapsed"
        );
    }

    // `None` is transparency, and the artwork leans on it for its silhouette.
    assert!(
        rgba.chunks_exact(4).any(|pixel| pixel[3] == 0),
        "nyan.xpm's None pixels must be transparent"
    );
}
