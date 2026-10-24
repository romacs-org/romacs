use super::*;

#[test]
fn canonical_icon_renders_the_brush_mark_on_black_without_fonts() {
    for size in [16, 24, 32, 48, 128, 256] {
        let icon = decode_svg_icon(WINDOW_ICON_SVG, size).expect("decode canonical vector icon");
        assert_eq!((icon.width, icon.height), (size, size));
        let pixels: Vec<_> = icon.premultiplied_rgba.chunks_exact(4).collect();
        assert!(pixels.iter().all(|pixel| pixel[3] == 255));
        assert_eq!(pixels[0], [0, 0, 0, 255]);
        assert_eq!(pixels[pixels.len() - 1], [0, 0, 0, 255]);
        assert!(pixels.iter().any(|pixel| **pixel == [255, 95, 89, 255]));
        assert!(
            pixels
                .iter()
                .any(|pixel| { pixel[2] > 100 && u16::from(pixel[2]) > u16::from(pixel[0]) * 2 })
        );
    }
    let icon = load_window_icon().expect("load runtime icon");
    let pixel_at = |x, y| {
        let offset = ((y * icon.width + x) * 4) as usize;
        &icon.premultiplied_rgba[offset..offset + 4]
    };
    assert_eq!(pixel_at(70, 120), [255, 95, 89, 255], "red stem");
    assert_eq!(pixel_at(165, 80), [47, 175, 255, 255], "blue crescent");
    assert_eq!(pixel_at(120, 100), [0, 0, 0, 255], "open counter");
}

#[test]
fn wayland_argb8888_preserves_premultiplied_color_in_native_byte_order() {
    let icon = RasterizedWindowIcon {
        width: 1,
        height: 1,
        premultiplied_rgba: vec![0x11, 0x22, 0x33, 0x44],
    };

    assert_eq!(icon.to_wayland_argb8888(), 0x4411_2233_u32.to_ne_bytes());
}
