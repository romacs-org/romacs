//! Face font attributes as plain data, and GNU's entity scoring over them.
//!
//! GNU `font_score` (src/font.c) compares GNU style numerics — the
//! `weight_table`/`slant_table`/`width_table` values — and orders properties
//! lexicographically by `face-font-selection-order` (default: width, size,
//! weight, slant). The inherited `romacs-layout-engine/src/font/selection.rs`
//! encodes the same policy over core's style enums; a parity test in
//! layout-engine pins the two together until those enums leave core.

use romacs_display_protocol::font::FontSlantKind;

/// A realized face's font attributes. Plain data: no symbol, no face handle.
#[derive(Clone, Debug, PartialEq)]
pub struct FontRequest {
    /// Family as the face names it. Generic names (`monospace`, `sans`,
    /// `serif`) resolve through the platform's alias configuration.
    pub family: String,
    /// CSS weight (400 normal, 700 bold).
    pub weight: u16,
    pub slant: FontSlantKind,
    /// OS/2 `usWidthClass` (1..=9, 5 normal).
    pub width: u16,
    /// Logical pixel size the face is realized at.
    pub pixel_size: f32,
}

impl FontRequest {
    /// Regular upright normal-width request.
    pub fn new(family: impl Into<String>, pixel_size: f32) -> Self {
        Self {
            family: family.into(),
            weight: 400,
            slant: FontSlantKind::Normal,
            width: 5,
            pixel_size,
        }
    }

    pub fn with_weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    pub fn with_slant(mut self, slant: FontSlantKind) -> Self {
        self.slant = slant;
        self
    }

    pub fn with_width(mut self, width: u16) -> Self {
        self.width = width;
        self
    }

    pub(crate) fn cache_key(&self) -> RequestKey {
        RequestKey {
            family: self.family.clone(),
            weight: self.weight,
            slant: self.slant,
            width: self.width,
            pixel_size_bits: self.pixel_size.to_bits(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RequestKey {
    family: String,
    weight: u16,
    slant: FontSlantKind,
    width: u16,
    pixel_size_bits: u32,
}

/// GNU `weight_table` numeric for a CSS weight, via the same CSS buckets as
/// `romacs_core::face::FontWeight::from_css_weight`.
pub(crate) fn gnu_weight(css: u16) -> u16 {
    match css {
        0..=150 => 0,
        151..=250 => 40,
        251..=325 => 50,
        326..=375 => 55,
        376..=450 => 80,
        451..=550 => 100,
        551..=650 => 180,
        651..=750 => 200,
        751..=850 => 205,
        851..=925 => 210,
        _ => 250,
    }
}

/// GNU `width_table` numeric for an OS/2 width class.
pub(crate) fn gnu_width(class: u16) -> u16 {
    match class {
        0 | 1 => 50,
        2 => 63,
        3 => 75,
        4 => 87,
        5 => 100,
        6 => 113,
        7 => 125,
        8 => 150,
        _ => 200,
    }
}

/// OS/2 width class for a Fontconfig `FC_WIDTH` value (the same numerics as
/// GNU's `width_table`), bucketed as layout's Fontconfig adapter does.
pub(crate) fn width_class_from_fontconfig(width: i32) -> u16 {
    match width {
        i32::MIN..=56 => 1,
        57..=69 => 2,
        70..=81 => 3,
        82..=93 => 4,
        94..=106 => 5,
        107..=119 => 6,
        120..=137 => 7,
        138..=175 => 8,
        _ => 9,
    }
}

/// CSS weight for a Fontconfig `FC_WEIGHT` value.
pub(crate) fn css_weight_from_fontconfig(weight: i32) -> u16 {
    match weight {
        i32::MIN..=20 => 100,
        21..=45 => 200,
        46..=62 => 300,
        63..=89 => 400,
        90..=139 => 500,
        140..=189 => 600,
        190..=204 => 700,
        205..=209 => 800,
        _ => 900,
    }
}

/// GNU `font_score` slant distance, as `selection.rs` encodes it.
fn slant_distance(requested: FontSlantKind, candidate: FontSlantKind) -> u32 {
    use FontSlantKind::{Italic, Normal, Oblique};
    match (requested, candidate) {
        (Normal, Normal) | (Italic | Oblique, Italic | Oblique) => 0,
        (Normal, _) => 350,
        (_, Normal) => 250,
    }
}

/// Lexicographic score: lower is better. Field order is the policy.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StyleScore {
    /// Non-style compatibility (family affinity, spacing), compared first.
    compatibility: u32,
    width: u32,
    weight: u32,
    slant: u32,
}

impl StyleScore {
    /// GNU font.c:2133-2142 packs all style distances with bitwise OR.
    /// Shifts can overlap after duplicate orders; height remains deferred/zero.
    pub fn ordered(self, state: romacs_layout_input::FontSelectionState) -> (u32, u32) {
        let [width, _height, weight, slant] = state.shifts();
        (self.compatibility,
            (self.width.min(127) << width)
                | (self.weight.min(127) << weight)
                | (self.slant.min(127) << slant))
    }
}

/// Score one scalable candidate against a request (CSS weights, OS/2
/// width classes). Size does not
/// participate: every candidate the resolver scores is scalable, for which
/// GNU's size distance is zero.
pub fn style_score(
    compatibility: u32,
    weight: u16,
    slant: FontSlantKind,
    width: u16,
    candidate_weight: u16,
    candidate_slant: FontSlantKind,
    candidate_width: u16,
) -> StyleScore {
    StyleScore {
        compatibility,
        width: u32::from(gnu_width(candidate_width).abs_diff(gnu_width(width))),
        weight: u32::from(
            gnu_weight(candidate_weight)
                .abs_diff(gnu_weight(weight))
                .min(127),
        ),
        slant: slant_distance(slant, candidate_slant),
    }
}

#[cfg(test)]
#[path = "style/tests/style_test.rs"]
mod tests;
