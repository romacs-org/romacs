use super::*;

#[test]
fn width_dominates_weight_which_dominates_slant() {
    // Iosevka ships Regular and Extended in one family; GNU's default
    // `face-font-selection-order` puts width first, so a normal-width
    // request must not take an Extended face even with an exact weight.
    let normal_width_bold = style_score(
        0,
        400,
        FontSlantKind::Normal,
        5,
        700,
        FontSlantKind::Normal,
        5,
    );
    let extended_regular = style_score(
        0,
        400,
        FontSlantKind::Normal,
        5,
        400,
        FontSlantKind::Normal,
        7,
    );
    assert!(normal_width_bold < extended_regular);

    let regular_italic = style_score(
        0,
        400,
        FontSlantKind::Normal,
        5,
        400,
        FontSlantKind::Italic,
        5,
    );
    let semibold_upright = style_score(
        0,
        400,
        FontSlantKind::Normal,
        5,
        600,
        FontSlantKind::Normal,
        5,
    );
    // Weight outranks slant: GNU prefers the exact weight in the wrong
    // slant over the right slant at another weight.
    assert!(regular_italic < semibold_upright);
}

#[test]
fn compatibility_is_compared_before_style() {
    let affine = style_score(
        0,
        400,
        FontSlantKind::Normal,
        5,
        900,
        FontSlantKind::Italic,
        9,
    );
    let exact_style_other_family = style_score(
        1,
        400,
        FontSlantKind::Normal,
        5,
        400,
        FontSlantKind::Normal,
        5,
    );
    assert!(affine < exact_style_other_family);
}

#[test]
fn italic_and_oblique_satisfy_each_other_as_in_gnu() {
    assert_eq!(
        slant_distance(FontSlantKind::Italic, FontSlantKind::Oblique),
        0
    );
    assert_eq!(
        slant_distance(FontSlantKind::Oblique, FontSlantKind::Italic),
        0
    );
    assert_eq!(
        slant_distance(FontSlantKind::Normal, FontSlantKind::Italic),
        350
    );
    assert_eq!(
        slant_distance(FontSlantKind::Italic, FontSlantKind::Normal),
        250
    );
}

#[test]
fn fontconfig_numerics_round_trip_to_gnu_buckets() {
    assert_eq!(css_weight_from_fontconfig(80), 400);
    assert_eq!(css_weight_from_fontconfig(200), 700);
    assert_eq!(width_class_from_fontconfig(100), 5);
    assert_eq!(width_class_from_fontconfig(125), 7);
    assert_eq!(gnu_width(width_class_from_fontconfig(87)), 87);
    assert_eq!(gnu_weight(400), 80);
    assert_eq!(gnu_weight(700), 200);
}
