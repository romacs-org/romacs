//! Draw-list colours (sRGB, straight alpha) to shader colours (linear,
//! premultiplied). Scene targets are sRGB textures, so blending happens in
//! linear light and the hardware encodes on store.

use super::draw::Color;

pub(super) fn linear_premultiplied(color: Color, opacity: f32) -> [f32; 4] {
    let alpha = (color.a * opacity).clamp(0.0, 1.0);
    [
        srgb_to_linear(color.r) * alpha,
        srgb_to_linear(color.g) * alpha,
        srgb_to_linear(color.b) * alpha,
        alpha,
    ]
}

/// A target clear colour: premultiplied linear, as `wgpu::Color` wants.
pub(super) fn clear_color(color: Color) -> wgpu::Color {
    let [r, g, b, a] = linear_premultiplied(color, 1.0);
    wgpu::Color {
        r: f64::from(r),
        g: f64::from(g),
        b: f64::from(b),
        a: f64::from(a),
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_premultiplies_and_srgb_decodes() {
        let [r, g, b, a] = linear_premultiplied(Color::rgba(1.0, 0.5, 0.0, 1.0), 0.5);
        assert_eq!((r, b, a), (0.5, 0.0, 0.5));
        assert!((g - 0.214_041_14 * 0.5).abs() < 1e-6);
    }

    #[test]
    fn components_are_clamped() {
        assert_eq!(
            linear_premultiplied(Color::rgba(2.0, -1.0, 0.0, 3.0), 1.0),
            [1.0, 0.0, 0.0, 1.0]
        );
    }
}
