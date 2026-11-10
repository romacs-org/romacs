//! Menu-bar pixel regressions using the actual chrome painter, not a mock widget.
use super::*;

#[test]
fn menu_bar_initial_cjk_glyphs_do_not_overlap() {
    assert_menu_heading_pixels(false);
}

#[test]
fn compact_bar_initial_cjk_glyphs_do_not_overlap() {
    assert_menu_heading_pixels(true);
}

fn assert_menu_heading_pixels(compact: bool) {
    use romacs_display_protocol::MenuBarItem;
    let mut h = try_harness().expect("menu bar pixel test requires an offscreen GPU adapter");
    let reference = h
        .atlas
        .get_or_create_composed_atlas(
            h.renderer.device(),
            h.renderer.queue(),
            "中中",
            FaceId::new(0),
            0.0_f32.to_bits(),
            None,
            cosmic_text::SubpixelBin::Zero,
            cosmic_text::SubpixelBin::Zero,
            SubpixelRequest::Disabled,
        )
        .expect("requires a CJK fallback font");
    let advance = reference[0].advance_width / 2.0;
    assert!(advance > h.atlas.default_char_width() + 2.0);
    let mut fonts = romacs_font::FontResolver::new().expect("platform font catalog");
    let items = vec![
        romacs_display_protocol::frame_chrome::PositionedMenuHeading::measure(
            MenuBarItem {
                index: 0,
                key: "file".into(),
                label: "中中".into(),
            },
            0.0,
            32.0,
            8.0,
            |text| {
                let size = h.atlas.default_font_size();
                let resolved = fonts
                    .resolve_char(
                        &romacs_font::FontRequest::new("monospace", size),
                        '中',
                    )
                    .expect("requires a CJK fallback font");
                let glyphs = text
                    .char_indices()
                    .enumerate()
                    .map(
                        |(index, (start, ch))| romacs_display_protocol::font::ResolvedGlyph {
                            resolved_font_id: resolved.font.id,
                            glyph_id: resolved.glyph.glyph_id,
                            x: index as f32 * resolved.glyph.advance_px,
                            y: 0.0,
                            x_advance: resolved.glyph.advance_px,
                            cluster_start: start as u32,
                            cluster_end: (start + ch.len_utf8()) as u32,
                        },
                    )
                    .collect();
                let fonts = vec![resolved.font];
                romacs_display_protocol::frame_chrome::MenuHeadingText::Pixels(
                    romacs_display_protocol::frame_chrome::ResolvedMenuLabel::new(
                        glyphs, fonts, size,
                    ),
                )
            },
        ),
    ];
    if compact {
        let content = romacs_display_protocol::CompactBarContent::new(
            items,
            vec![],
            Color::WHITE,
            Color::BLACK,
            Color::WHITE,
            Color::BLACK,
            16,
            4,
        );
        h.renderer.render_compact_bar(
            &h.view,
            &content,
            FrameRect::new(0.0, 0.0, W as f32, 32.0).unwrap(),
            &std::collections::HashMap::new(),
            None,
            None,
            None,
            None,
            &mut h.atlas,
            W,
            H,
        );
    } else {
        h.renderer.render_menu_bar(
            &h.view,
            &items,
            FrameRect::new(0.0, 0.0, W as f32, 32.0).unwrap(),
            (1.0, 1.0, 1.0),
            (0.0, 0.0, 0.0),
            None,
            None,
            &mut h.atlas,
            W,
            H,
        );
    }
    let pixels = read_back(&h);
    let step = advance.round() as u32;
    assert!((advance - step as f32).abs() < 0.01);
    let ink = |origin| {
        (0..step)
            .flat_map(|x| (0..30).map(move |y| (origin + x, y)))
            .map(|(x, y)| {
                let [r, g, b, _] = px(&pixels, x, y);
                r > 120 && g > 120 && b > 120
            })
            .collect::<Vec<_>>()
    };
    let first = ink(8);
    let second = ink(8 + step);
    assert!(first.iter().any(|v| *v), "heading must be visible");
    assert!(
        first == second,
        "repeated CJK heading glyphs must be one real-font advance apart"
    );
}
