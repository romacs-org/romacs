//! F0a acceptance: text whose faces are plain data reaches the real renderer
//! with every glyph's font chosen by `romacs-font`, and none by the
//! renderer's emergency path.
//!
//! This test binary's dependency graph is `romacs-font` +
//! `romacs-renderer-wgpu` (+ protocol, materializer, fixtures): no evaluator.
//! A missing GPU adapter FAILS the test rather than skipping it.

use romacs_display_protocol::frame_glyphs::{FrameGlyphBuffer, GlyphRowRole};
use romacs_display_protocol::types::{Color, DisplayWindowId, FaceId};
use romacs_display_protocol::{
    DeviceScale, GeometrySize, LogicalPixels, PresentMapping, PresentationExtent, SurfaceState,
};
use romacs_font::{
    CharResolutionSource, FontRequest, FontResolver, FontsetEntry, FontsetFontSpec, FontsetMode,
    FontsetPolicy, FontsetTarget, publish_face_text,
};
use romacs_renderer_wgpu::{WgpuGlyphAtlas, WgpuRenderer};

const W: u32 = 480;
const H: u32 = 160;
const FAMILY: &str = "JetBrainsMono Nerd Font";
const PX: f32 = 16.0;

const ENGLISH: &str = "Hello, world";
const GREEK: &str = "Ωμέγα αλφάβητο ΆΈΉ ϊΰ";
const CYRILLIC: &str = "Привет, мир ЁЖЩЮЯ ёжщюя";

fn fixtures() {
    for path in [
        romacs_test_fonts::jetbrains_mono_nerd_font_regular_3_4_0(),
        romacs_test_fonts::noto_color_emoji_2_051(),
    ] {
        assert!(
            romacs_font::register_font_file(path),
            "Fontconfig refused fixture {path:?}"
        );
    }
}

/// GNU 32.0.50's `char-script-table` ranges for the `emoji` script, as
/// `map-char-table` reports them (`lane_emacs --batch`, 2026-10-18). A
/// runtime expands a script symbol into ranges before publishing a policy.
const GNU_EMOJI_SCRIPT: &[(u32, u32)] = &[
    (0x231A, 0x231B),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0xFE0F, 0xFE0F),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F1E6, 0x1F1FF),
    (0x1F201, 0x1F201),
    (0x1F21A, 0x1F21A),
    (0x1F22F, 0x1F22F),
    (0x1F232, 0x1F236),
    (0x1F238, 0x1F23A),
    (0x1F250, 0x1F251),
    (0x1F300, 0x1F320),
    (0x1F32D, 0x1F335),
    (0x1F337, 0x1F37C),
    (0x1F37E, 0x1F393),
    (0x1F3A0, 0x1F3CA),
    (0x1F3CF, 0x1F3D3),
    (0x1F3E0, 0x1F3F0),
    (0x1F3F4, 0x1F3F4),
    (0x1F3F8, 0x1F43E),
    (0x1F440, 0x1F440),
    (0x1F442, 0x1F4FC),
    (0x1F4FF, 0x1F53D),
    (0x1F54B, 0x1F54E),
    (0x1F550, 0x1F567),
    (0x1F57A, 0x1F57A),
    (0x1F595, 0x1F596),
    (0x1F5A4, 0x1F5A4),
    (0x1F5FB, 0x1F64F),
    (0x1F680, 0x1F6C5),
    (0x1F6CC, 0x1F6CC),
    (0x1F6D0, 0x1F6D2),
    (0x1F6D5, 0x1F6D8),
    (0x1F6DC, 0x1F6DF),
    (0x1F6EB, 0x1F6EC),
    (0x1F6F4, 0x1F6FC),
    (0x1F7E0, 0x1F7EB),
    (0x1F7F0, 0x1F7F0),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1F9FF),
    (0x1FA70, 0x1FA7C),
    (0x1FA80, 0x1FA8A),
    (0x1FA8E, 0x1FAC6),
    (0x1FAC8, 0x1FAC8),
    (0x1FACD, 0x1FADC),
    (0x1FADF, 0x1FAEA),
    (0x1FAEF, 0x1FAF8),
];

/// A user's init.el: `(set-fontset-font t 'emoji "Noto Color Emoji" nil
/// 'prepend)` and `(set-fontset-font t #x1f5dc "Noto Color Emoji" nil
/// 'prepend)`, as plain data.
fn owner_policy() -> FontsetPolicy {
    let mut policy = FontsetPolicy::new();
    for &(from, to) in GNU_EMOJI_SCRIPT.iter().chain(&[(0x1F5DC, 0x1F5DC)]) {
        policy.set_font(
            FontsetTarget::Range(from, to),
            FontsetEntry::Font(FontsetFontSpec::family("Noto Color Emoji")),
            FontsetMode::Prepend,
        );
    }
    policy
}

struct Scene {
    frame: FrameGlyphBuffer,
    /// (y0, y1) pixel band of each drawn line.
    bands: Vec<(u32, u32)>,
}

/// Lay LINES out on fixed rows, each `(face, clusters)`; all ink is the
/// face's foreground. Advances come from the published font answers.
fn scene(resolver: &mut FontResolver, policy: &FontsetPolicy, lines: &[Vec<&str>]) -> Scene {
    let mut frame = FrameGlyphBuffer::with_size(W as f32, H as f32);
    frame.background = Color::BLACK;
    let face = FaceId::new(1);
    frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
    frame.set_face_with_font(
        face,
        Color::WHITE,
        None,
        FAMILY,
        400,
        false,
        PX,
        0,
        None,
        0,
        None,
        0,
        None,
        false,
    );
    let request = FontRequest::new(FAMILY, PX);
    let all: String = lines.iter().flatten().copied().collect();
    let published = publish_face_text(resolver, &mut frame, face, &request, policy, &all)
        .expect("the fixture family resolves");
    assert!(
        published.unresolved.is_empty(),
        "unresolved characters: {:?}",
        published.unresolved
    );

    let primary = frame.fonts[&published.primary].clone();
    let line_height = (primary.ascent_px + primary.descent_px).ceil();
    let cell = primary
        .glyph_advance
        .cell_advance_px()
        .expect("the fixture is a monospace font");
    let mut bands = Vec::new();
    for (row, clusters) in lines.iter().enumerate() {
        let y = row as f32 * line_height;
        let mut x = 0.0;
        for cluster in clusters {
            let mut chars = cluster.chars();
            let first = chars.next().expect("non-empty cluster");
            let advance = frame
                .char_fonts
                .get(&face)
                .and_then(|by_char| by_char.get(&first))
                .map_or(cell, |glyph| glyph.advance_px.max(cell));
            if chars.next().is_some() {
                frame.add_composed_char(
                    cluster,
                    first,
                    x,
                    y,
                    advance,
                    line_height,
                    primary.ascent_px,
                    false,
                );
            } else {
                frame.add_char(first, x, y, advance, line_height, primary.ascent_px, false);
            }
            x += advance;
        }
        bands.push((y as u32, (y + line_height) as u32));
    }
    Scene { frame, bands }
}

fn clusters(text: &str) -> Vec<&str> {
    text.char_indices()
        .map(|(start, ch)| &text[start..start + ch.len_utf8()])
        .collect()
}

struct Rendered {
    pixels: Vec<u8>,
    unresolved: u64,
}

fn render(frame: &FrameGlyphBuffer) -> Rendered {
    // Concurrent Vulkan instance creation crashes inside the NVIDIA ICD
    // loader (SIGSEGV in vkEnumerateInstanceExtensionProperties under
    // `cargo test`'s threads; nextest's process-per-test hides it).
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _gpu = GPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut renderer = WgpuRenderer::new(None, W, H)
        .expect("a wgpu adapter is required; this test does not skip without one");
    let mut atlas = WgpuGlyphAtlas::new(renderer.device());
    atlas.set_current_frame_fonts(frame.font_bindings());
    let target = renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("f0a-target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let SurfaceState::Drawable(surface) =
        SurfaceState::from_device_size(W, H, DeviceScale::new(1.0).unwrap()).unwrap()
    else {
        unreachable!("offscreen targets are drawable")
    };
    let mapping = PresentMapping::top_left_clip(
        surface,
        PresentationExtent::new(
            frame.presentation_id,
            GeometrySize::<LogicalPixels>::from_px(frame.width, frame.height).unwrap(),
        ),
    );
    renderer.render_frame_glyphs(
        &view, frame, &mut atlas, mapping, false, None, None, None, None, None,
    );
    Rendered {
        pixels: read_back(&renderer, &target),
        unresolved: atlas.unresolved_face_text_total(),
    }
}

fn read_back(renderer: &WgpuRenderer, target: &wgpu::Texture) -> Vec<u8> {
    let unpadded = W * 4;
    let padded =
        unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("f0a-readback"),
        size: u64::from(padded * H),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer
        .device()
        .create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue().submit(std::iter::once(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    renderer
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .expect("GPU readback");
    let data = slice.get_mapped_range().expect("mapped readback");
    let mut out = vec![0; (unpadded * H) as usize];
    for row in 0..H {
        let (src, dst) = ((row * padded) as usize, (row * unpadded) as usize);
        out[dst..dst + unpadded as usize].copy_from_slice(&data[src..src + unpadded as usize]);
    }
    out
}

/// Pixels in a band brighter than the black background.
fn ink(pixels: &[u8], (y0, y1): (u32, u32)) -> usize {
    (y0..y1.min(H))
        .flat_map(|y| (0..W).map(move |x| ((y * W + x) * 4) as usize))
        .filter(|&i| pixels[i..i + 3].iter().any(|&channel| channel > 96))
        .count()
}

/// Pixels inside solid colour areas: the pixel and its 8 neighbours all have
/// clearly unequal channels. A white mask glyph's subpixel-AA fringes are one
/// pixel wide and never qualify; a colour emoji's fills do.
fn solid_colour(pixels: &[u8], (y0, y1): (u32, u32)) -> usize {
    let chroma = |x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        let [b, g, r] = [pixels[i], pixels[i + 1], pixels[i + 2]];
        r.max(g).max(b) - r.min(g).min(b) > 64
    };
    (y0.max(1)..y1.min(H - 1))
        .flat_map(|y| (1..W - 1).map(move |x| (x, y)))
        .filter(|&(x, y)| (y - 1..=y + 1).all(|ny| (x - 1..=x + 1).all(|nx| chroma(nx, ny))))
        .count()
}

#[test]
fn greek_english_and_cyrillic_render_with_no_emergency_font_lookup() {
    fixtures();
    let mut resolver = FontResolver::new().expect("Fontconfig");
    let policy = FontsetPolicy::new();
    let lines = vec![clusters(ENGLISH), clusters(GREEK), clusters(CYRILLIC)];
    let Scene { frame, bands } = scene(&mut resolver, &policy, &lines);

    let rendered = render(&frame);
    assert_eq!(
        rendered.unresolved, 0,
        "renderer took its emergency font path"
    );

    // Every non-ASCII scalar has an exact binding with a real glyph, so the
    // renderer's uncounted "render-side exact font missing" path is closed
    // too, not only the counted one.
    let face = FaceId::new(1);
    for ch in GREEK
        .chars()
        .chain(CYRILLIC.chars())
        .filter(|ch| !ch.is_ascii())
    {
        let glyph = frame.char_fonts[&face]
            .get(&ch)
            .unwrap_or_else(|| panic!("{ch} unbound"));
        assert_ne!(glyph.glyph_id.get(), 0, "{ch} bound to .notdef");
        let resolution = resolver
            .resolve_char(&FontRequest::new(FAMILY, PX), ch, &policy)
            .expect("resolved");
        assert_eq!(
            resolution.source,
            CharResolutionSource::Primary,
            "{ch}: the face's own font covers Greek and Cyrillic, so GNU keeps them on it"
        );
    }

    for (name, band) in ["English", "Greek", "Cyrillic"].into_iter().zip(bands) {
        let inked = ink(&rendered.pixels, band);
        assert!(inked > 200, "{name} line painted only {inked} ink pixels");
    }
}

#[test]
fn emoji_zwj_vs16_and_keycap_take_the_fontset_font_without_emergency_lookup() {
    fixtures();
    let mut resolver = FontResolver::new().expect("Fontconfig");
    let policy = owner_policy();
    // 😀, woman-technologist (ZWJ), red heart + VS16, keycap one.
    let emoji = vec!["😀", "👩\u{200D}💻", "❤\u{FE0F}", "1\u{FE0F}\u{20E3}"];
    let lines = vec![clusters("Ωμέγα Hello Привет"), emoji];
    let Scene { frame, bands } = scene(&mut resolver, &policy, &lines);

    let face = FaceId::new(1);
    for ch in ['😀', '👩', '💻', '\u{1F600}'] {
        let glyph = frame.char_fonts[&face][&ch];
        let font = &frame.fonts[&glyph.resolved_font_id];
        assert_eq!(
            font.family,
            "Noto Color Emoji",
            "U+{:04X} took {}",
            u32::from(ch),
            font.family
        );
    }

    let rendered = render(&frame);
    assert_eq!(
        rendered.unresolved, 0,
        "renderer took its emergency font path"
    );
    let painted = solid_colour(&rendered.pixels, bands[1]);
    let text = solid_colour(&rendered.pixels, bands[0]);
    eprintln!("solid colour pixels: emoji line {painted}, text line {text}");
    if let Some(path) = std::env::var_os("F0A_DUMP") {
        let mut ppm = format!("P6 {W} {H} 255\n").into_bytes();
        for px in rendered.pixels.chunks_exact(4) {
            ppm.extend([px[2], px[1], px[0]]);
        }
        std::fs::write(path, ppm).expect("dump");
    }
    assert!(
        painted > 200,
        "emoji line has only {painted} solid colour pixels"
    );
    // A thick stem's LCD fringes can leave an isolated qualifying pixel
    // (measured: 1); a colour font on the text line would give hundreds.
    assert!(
        text * 50 < painted,
        "text line has {text} solid colour pixels; drawn by a colour font?"
    );
}

#[test]
fn without_a_fontset_rule_emoji_still_reach_a_colour_font() {
    // GNU 32.0.50 PGTK, -Q, default family JetBrainsMono Nerd Font, no
    // user rule: `font-at` on 😀 reports Noto Color Emoji.
    fixtures();
    let mut resolver = FontResolver::new().expect("Fontconfig");
    let found = resolver
        .resolve_char(&FontRequest::new(FAMILY, PX), '😀', &FontsetPolicy::new())
        .expect("resolved");
    assert_eq!(found.source, CharResolutionSource::Fallback);
    assert_eq!(found.font.family, "Noto Color Emoji");
}

#[test]
fn a_family_that_is_not_installed_falls_back_as_gnu_does() {
    // Native-X11 GNU 32.0.50, -Q, Xvfb at 96dpi, gate's pinned fontconfig:
    // the genuinely absent family substitutes Liberation Sans for every
    // character of "Καλημέρα Жж A", with Greek width 71 at 16px.
    // DejaVu is installed in this font set, so it is tested separately below.
    // Transcript and reproducer: f0a/gnu/pinned-fonts.txt and pinned-fonts.el.
    fixtures();
    assert_gnu_family_and_width(
        FontRequest::new("No Such Family F0a", PX),
        "Liberation Sans",
        71.0,
    );
}

#[test]
fn an_installed_dejavu_family_resolves_to_itself_as_gnu_does() {
    // Same oracle/environment as the missing-family case: normal width is
    // 81px, condensed selects the semi-condensed face and measures 73px.
    // Transcript: f0a/gnu/pinned-fonts.txt.
    fixtures();
    assert_gnu_family_and_width(FontRequest::new("DejaVu Sans", PX), "DejaVu Sans", 81.0);
    assert_gnu_family_and_width(
        FontRequest::new("DejaVu Sans", PX).with_width(3),
        "DejaVu Sans",
        73.0,
    );
}

fn assert_gnu_family_and_width(request: FontRequest, family: &str, greek_width: f32) {
    let mut resolver = FontResolver::new().expect("Fontconfig");
    let policy = FontsetPolicy::new();
    let font = resolver
        .resolve_primary(&request, &policy)
        .expect("GNU resolves the requested family or its substitute");
    assert_eq!(font.family, family, "{request:?}");
    let mut width = 0.0;
    for ch in "Καλημέρα Жж A".chars() {
        let found = resolver
            .resolve_char(&request, ch, &policy)
            .expect("resolved");
        assert_eq!(found.font.family, family, "{request:?} {ch}");
        if "Καλημέρα".contains(ch) {
            width += found.glyph.advance_px;
        }
    }
    assert_eq!(width, greek_width, "{request:?}: Greek pixel width");
}

#[test]
fn generic_families_resolve_through_fontconfig_aliases() {
    let mut resolver = FontResolver::new().expect("Fontconfig");
    let font = resolver
        .resolve_primary(&FontRequest::new("monospace", PX), &FontsetPolicy::new())
        .expect("`monospace` is an alias every Fontconfig install defines");
    assert!(font.identity.file_path.is_some());
    assert!(
        font.ascent_px > 0.0 && font.space_advance_px > 0.0,
        "{font:?}"
    );
}

#[test]
fn an_explicit_fontset_family_wins_over_the_primary_font() {
    fixtures();
    let mut resolver = FontResolver::new().expect("Fontconfig");
    let request = FontRequest::new(FAMILY, PX);
    let mut policy = FontsetPolicy::new();
    // GNU: (set-fontset-font t 'greek "Noto Color Emoji") is honoured even
    // though the face's own font covers Greek — but this one has no Ω, so
    // the search moves on to the face's font, exactly as GNU's next entry.
    policy.set_font(
        FontsetTarget::Range(0x370, 0x3FF),
        FontsetEntry::Font(FontsetFontSpec::family("Noto Color Emoji")),
        FontsetMode::Overwrite,
    );
    let omega = resolver
        .resolve_char(&request, 'Ω', &policy)
        .expect("resolved");
    assert_eq!(omega.source, CharResolutionSource::Primary);

    // An entry that covers the character is taken before the face's font.
    policy.set_font(
        FontsetTarget::Range(0x2600, 0x26FF),
        FontsetEntry::Font(FontsetFontSpec::family("Noto Color Emoji")),
        FontsetMode::Overwrite,
    );
    let sun = resolver
        .resolve_char(&request, '☀', &policy)
        .expect("resolved");
    assert_eq!(sun.source, CharResolutionSource::Fontset(0));
    assert_eq!(sun.font.family, "Noto Color Emoji");

    // `nil` leaves the range without a font of its own; GNU still draws
    // Ж with the face's font (PGTK transcript, `(set-fontset-font t
    // 'cyrillic nil)` under Iosevka). So does this resolver.
    policy.set_font(
        FontsetTarget::Range(0x416, 0x416),
        FontsetEntry::ExplicitNone,
        FontsetMode::Overwrite,
    );
    let zhe = resolver
        .resolve_char(&request, 'Ж', &policy)
        .expect("resolved");
    assert_eq!(zhe.source, CharResolutionSource::Primary);
}
