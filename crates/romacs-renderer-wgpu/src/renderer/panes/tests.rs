//! Pane geometry at the effective scales a display gives (1, 1.25, 1.5
//! and 2): device-pixel edges rounded once, a hairline and focus edge one
//! device pixel wide, corners as squares of the radius.
use super::*;

const SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];
const BACKGROUND: Color = Color { r: 0.1, g: 0.1, b: 0.1, a: 1.0 };
const ACCENT: Color = Color { r: 0.8, g: 0.5, b: 0.1, a: 1.0 };
const LINE: Color = Color { r: 0.3, g: 0.3, b: 0.3, a: 1.0 };

/// A frame 1216 wide split right with dividers and border 6: boxes of
/// 602 and 586 pixels (frame_chrome.gnu), a 6-pixel divider between.
fn split(radius: f32) -> [Pane; 2] {
    [Pane { bounds: Rect::new(6.0, 6.0, 596.0, 839.0), radius, edge: Some(ACCENT), dim: 0.0 },
     Pane { bounds: Rect::new(608.0, 6.0, 602.0, 839.0), radius, edge: Some(LINE), dim: 0.12 }]
}

fn device(rect: Rect, scale: f32) -> [f32; 4] {
    [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height].map(|e| e * scale)
}

fn is_whole(n: f32) -> bool { (n - n.round()).abs() < 1e-3 }

#[test]
fn a_shared_boundary_lands_on_one_device_column() {
    for scale in SCALES {
        let panes = split(6.0);
        let g = pane_geometry(&panes, BACKGROUND, scale);
        let dim = device(g.dims[0].0, scale);
        assert!(dim.iter().all(|&e| is_whole(e)), "scale {scale}: {dim:?}");
        // The dimmed pane's box starts at the device column its API edge
        // rounds to, the one the divider before it ends at.
        assert_eq!(dim[0].round(), device_edge(608.0, scale), "scale {scale}");
        assert_eq!(dim[2].round(), device_edge(1210.0, scale), "scale {scale}");
        // Every device column from the first pane's box to the second's is
        // either a box's or the divider's, never both.
        let [first, second] = panes.map(|p| (device_edge(p.bounds.x, scale), device_edge(p.bounds.x + p.bounds.width, scale)));
        assert!(first.1 <= second.0, "scale {scale}: {first:?} {second:?}");
    }
}

#[test]
fn the_hairline_and_focus_edge_are_one_device_pixel() {
    for scale in SCALES {
        let g = pane_geometry(&split(6.0), BACKGROUND, scale);
        for (rect, color) in &g.edges {
            let [x0, y0, x1, y1] = device(*rect, scale);
            assert!([x0, y0, x1, y1].iter().all(|&e| is_whole(e)), "scale {scale}: {rect:?}");
            assert!(((x1 - x0) - 1.0).abs() < 1e-3 || ((y1 - y0) - 1.0).abs() < 1e-3, "scale {scale}: {rect:?}");
            assert!(*color == ACCENT || *color == LINE);
        }
        // The selected pane's left edge is the accent, on its first device column.
        let left = g.edges.iter().find(|(r, c)| *c == ACCENT && r.height > r.width).unwrap().0;
        assert_eq!(device(left, scale)[0].round(), device_edge(6.0, scale), "scale {scale}");
        for corner in &g.corners {
            let (_, width) = corner.edge.unwrap();
            assert!(((width * scale) - 1.0).abs() < 1e-3, "scale {scale}");
        }
    }
}

#[test]
fn corners_are_squares_of_the_radius_inside_the_box() {
    for scale in SCALES {
        let g = pane_geometry(&split(6.0), BACKGROUND, scale);
        assert_eq!(g.corners.len(), 8);
        for corner in &g.corners {
            let side = device_edge(6.0, scale) / scale;
            assert!((corner.square.width - side).abs() < 1e-3 && (corner.square.height - side).abs() < 1e-3);
            let (s, p) = (corner.square, corner.pane);
            assert!(s.x >= p.x && s.y >= p.y && s.x + s.width <= p.x + p.width + 1e-3 && s.y + s.height <= p.y + p.height + 1e-3);
        }
        // Square panes have no corners, and their edge still closes.
        let square = pane_geometry(&split(0.0), BACKGROUND, scale);
        assert!(square.corners.is_empty());
        assert_eq!(square.edges.len(), 8);
    }
}

#[test]
fn only_an_inactive_pane_is_dimmed_toward_the_background() {
    let g = pane_geometry(&split(6.0), BACKGROUND, 1.0);
    assert_eq!(g.dims.len(), 1);
    let (rect, color) = g.dims[0];
    assert_eq!((rect.x, rect.width), (608.0, 602.0));
    assert_eq!((color.r, color.g, color.b, color.a), (BACKGROUND.r, BACKGROUND.g, BACKGROUND.b, 0.12));
}
