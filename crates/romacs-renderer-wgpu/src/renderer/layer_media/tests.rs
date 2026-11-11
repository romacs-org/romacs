#[cfg(feature = "video")]
use romacs_video::{VideoGeometry, VideoRotation};

#[cfg(feature = "video")]
use super::video_quad_vertices;

#[test]
#[cfg(feature = "video")]
fn video_quad_uses_the_native_sampling_transform_instead_of_full_uvs() {
    let mut geometry = VideoGeometry::packed(4, 2);
    geometry.rotation = VideoRotation::Clockwise90;
    let vertices = video_quad_vertices(
        3.0,
        5.0,
        20.0,
        10.0,
        geometry
            .sampling_transform()
            .coordinates_for_destination_rect(0.0, 1.0, 0.25, 0.75),
        0.5,
    );

    assert_eq!(
        vertices.map(|vertex| vertex.tex_coords),
        [
            [0.25, 1.0],
            [0.25, 0.0],
            [0.75, 0.0],
            [0.25, 1.0],
            [0.75, 0.0],
            [0.75, 1.0],
        ]
    );
    assert!(vertices.iter().all(|vertex| vertex.color[3] == 0.5));
}
