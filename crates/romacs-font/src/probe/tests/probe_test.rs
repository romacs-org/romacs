use super::*;

/// Ground truth captured from the GNU Emacs 31.0.50 cairo build on this
/// machine (Xvfb, `(font-info (find-font (font-spec :family ...)))`); see
/// the render-boundary design notes. Each test skips when the nix-store
/// font file is absent so the suite stays green on other machines.
fn assert_gnu_probe(file: &str, size: u32, expect: FontPxMetrics) {
    if !std::path::Path::new(file).exists() {
        eprintln!("skipping: {file} not present");
        return;
    }
    let got = probe_font_px_metrics(file, 0, size, None).expect("probe succeeds");
    assert_eq!(got, expect, "probe of {file} at px={size}");
}

#[test]
fn gnu_parity_noto_sans_px1() {
    // GNU font-info: "... 1 3 0 0 0 1 2 1 0 1" => size 1 height 3
    // max-width 1 ascent 2 descent 1 space-width 0 average-width 1.
    assert_gnu_probe(
        "/nix/store/7lrhms8rphrd8ywphjbvjyll57pkim64-noto-fonts-2025.11.01/share/fonts/noto/NotoSans[wdth,wght].ttf",
        1,
        FontPxMetrics { underline: romacs_display_protocol::font::FontUnderline { position: 0, thickness: 0 },
            pixel_size: 1,
            height: 3,
            ascent: 2,
            descent: 1,
            max_width: 1,
            space_width: 0,
            average_width: 1,
        },
    );
}

#[test]
fn gnu_parity_hack_px1() {
    // GNU font-info: "... 1 2 0 0 0 1 1 1 1 1".
    assert_gnu_probe(
        "/nix/store/b7ybgcl00ak8q66bc0w15vfnyly4g13k-hack-font-3.003/share/fonts/truetype/Hack-Italic.ttf",
        1,
        FontPxMetrics { underline: romacs_display_protocol::font::FontUnderline { position: 0, thickness: 0 },
            pixel_size: 1,
            height: 2,
            ascent: 1,
            descent: 1,
            max_width: 1,
            space_width: 1,
            average_width: 1,
        },
    );
}

#[test]
fn gnu_parity_dejavu_sans_mono_px1() {
    // GNU font-info: "... 1 2 0 0 0 1 1 1 1 1".
    assert_gnu_probe(
        "/nix/store/b5gf37jp4y3965bp6x9wanzqchkkvbvs-dejavu-fonts-2.37/share/fonts/truetype/DejaVuSansMono-BoldOblique.ttf",
        1,
        FontPxMetrics { underline: romacs_display_protocol::font::FontUnderline { position: 0, thickness: 0 },
            pixel_size: 1,
            height: 2,
            ascent: 1,
            descent: 1,
            max_width: 1,
            space_width: 1,
            average_width: 1,
        },
    );
}

#[test]
fn noto_sans_named_instances_cover_thin_to_black() {
    let file = "/nix/store/7lrhms8rphrd8ywphjbvjyll57pkim64-noto-fonts-2025.11.01/share/fonts/noto/NotoSans[wdth,wght].ttf";
    if !std::path::Path::new(file).exists() {
        eprintln!("skipping: {file} not present");
        return;
    }
    let weights = named_instance_wght_values(file, 0);
    // GNU/fontconfig snap requests to these; the fvar table must expose the
    // standard weight ladder (no synthesized 350/950).
    for w in [100u16, 200, 300, 400, 500, 600, 700, 800, 900] {
        assert!(
            weights.contains(&w),
            "expected instance weight {w} in {weights:?}"
        );
    }
    assert!(!weights.contains(&350), "350 is not a named instance");
    assert!(!weights.contains(&950), "950 is not a named instance");
}

/// The variation data FreeType reads from a pinned variable font: its named
/// instances' weights and axis tuples, and the `wght` the probe applies,
/// seen in the glyph outline it draws.
#[test]
fn noto_sans_mono_variations_reach_freetype() {
    let file = romacs_test_fonts::noto_sans_mono_variable().to_str().unwrap();
    assert_eq!(named_instance_wght_values(file, 0), [100, 200, 300, 400, 500, 600, 700, 800, 900]);
    let wght = u32::from_be_bytes(*b"wght");
    let coords = |instance: u32| -> Vec<(u32, u32)> {
        named_instance_variation_coords(file, instance << 16).iter().map(|coord| (coord.tag(), coord.value_bits())).collect()
    };
    assert_eq!(coords(1), [(wght, 100f32.to_bits())]);
    assert_eq!(coords(5), [(wght, 500f32.to_bits())]);
    assert_eq!(coords(9), [(wght, 900f32.to_bits())]);
    assert_eq!(coords(10), Vec::new(), "there are nine named instances");
    // The stem of `l` thickens with the weight applied.
    let stem = |weight: Option<f32>| {
        let library = Library::init().unwrap();
        let mut face = library.new_face(file, 0).unwrap();
        if let Some(weight) = weight {
            apply_wght_axis(&library, &mut face, weight);
        }
        face.set_pixel_sizes(64, 64).unwrap();
        face.load_char(usize::from(b'l'), LoadFlag::NO_HINTING).unwrap();
        let metrics = face.glyph().metrics();
        (metrics.width, metrics.horiBearingX)
    };
    assert_eq!([None, Some(100.0), Some(700.0), Some(900.0)].map(stem),
               [(1884, 328), (1761, 410), (2007, 246), (2130, 164)]);
}
