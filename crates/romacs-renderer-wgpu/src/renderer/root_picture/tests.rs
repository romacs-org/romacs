use romacs_display_protocol::types::Rect;

use super::{Reach, pixel_region, pixel_regions};

#[test]
fn a_region_grows_to_whole_device_pixels_within_the_surface() {
    let (region, scissor) = pixel_region(Rect::new(10.3, 20.0, 5.0, 19.9), 1.0, (100, 100)).unwrap();
    assert_eq!(scissor, (10, 20, 6, 20));
    assert_eq!(region, Rect::new(10.0, 20.0, 6.0, 20.0));
    let (region, scissor) = pixel_region(Rect::new(10.0, 20.0, 5.0, 5.0), 1.5, (100, 100)).unwrap();
    assert_eq!((scissor, region.y), ((15, 30, 8, 8), 20.0));
    assert_eq!(
        pixel_region(Rect::new(-5.0, 98.0, 4.0, 10.0), 1.0, (100, 100)),
        None,
        "left of the surface"
    );
    assert_eq!(
        pixel_region(Rect::new(0.0, 98.0, 4.0, 10.0), 1.0, (100, 100))
            .unwrap()
            .1,
        (0, 98, 4, 2)
    );
}

#[test]
fn regions_grown_by_reach_and_to_pixels_join_where_they_then_meet() {
    // At 1.5, rows 10..20 and 20.5..30 grow to device rows 15..30 and 30..45:
    // they meet, and are one region; one further down is another.
    let regions = pixel_regions(
        &[
            Rect::new(0.0, 10.0, 50.0, 10.0),
            Rect::new(0.0, 20.5, 50.0, 9.5),
            Rect::new(0.0, 60.0, 50.0, 10.0),
        ],
        Reach::default(),
        1.5,
        (75, 150),
    );
    assert_eq!(
        regions.iter().map(|(_, s)| *s).collect::<Vec<_>>(),
        vec![(0, 15, 75, 30), (0, 90, 75, 15)]
    );
    // Ink two pixels above its rows grows each up by two, and joins them.
    let reach = Reach {
        up: 2.0,
        ..Reach::default()
    };
    let regions = pixel_regions(
        &[Rect::new(0.0, 10.0, 50.0, 10.0), Rect::new(0.0, 20.0, 50.0, 10.0)],
        reach,
        1.0,
        (50, 100),
    );
    assert_eq!(
        regions.iter().map(|(_, s)| *s).collect::<Vec<_>>(),
        vec![(0, 8, 50, 22)]
    );
}
