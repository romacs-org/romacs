//! Colour arithmetic for rlisp/romacs-theme.el, which keeps the rules: sRGB,
//! CIE XYZ and Oklab as GNU's lisp/color.el computes them, in its order of
//! operations, and WCAG 2's relative luminance.  A theme's contrast floors
//! ask for thousands of these when it loads.
//!
//! Portions Copyright (C) 2010-2026 Free Software Foundation, Inc.
use super::leaf_scalars::number;
use super::orderless::{elements, list};
use super::*;

type Triple = [f64; 3];

/// HEX, "#rrggbb", as (R G B), each from 0.0 to 1.0.
fn channels(cx: &mut Leaf<'_>, hex: Value) -> std::result::Result<Triple, Fault> {
    let bytes = cx.string_view(hex)?.bytes().to_vec();
    if bytes.len() < 7 {
        let data = list(cx, &[hex]);
        return Err(Fault::Signal(cx.intern("args-out-of-range"), data));
    }
    let channel = |at: usize| {
        let digits = std::str::from_utf8(&bytes[at..at + 2]).ok();
        f64::from(digits.and_then(|d| u8::from_str_radix(d, 16).ok()).unwrap_or(0)) / 255.0
    };
    Ok([channel(1), channel(3), channel(5)])
}

/// sRGB channel C made linear, as color.el's `color-srgb-to-xyz'.
fn linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.95 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Linear channel C as sRGB, as color.el's `color-xyz-to-srgb'.
fn gamma(c: f64) -> f64 {
    if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// The real cube root of F.
fn cbrt(f: f64) -> f64 {
    if f < 0.0 { -(-f).powf(1.0 / 3.0) } else { f.powf(1.0 / 3.0) }
}

/// LAB as linear sRGB (R G B), not clamped, as color.el's
/// `color-oklab-to-xyz' and `color-xyz-to-srgb' before the transfer curve.
fn oklab_linear([big_l, a, b]: Triple) -> Triple {
    let l = (1.0 * big_l + 0.39633779 * a + 0.21580376 * b).powf(3.0);
    let m = (1.00000001 * big_l + -0.10556134 * a + -0.06385417 * b).powf(3.0);
    let s = (1.00000005 * big_l + -0.08948418 * a + -1.29148554 * b).powf(3.0);
    let x = l * 1.22701385 + m * -0.55779998 + s * 0.28125615;
    let y = l * -0.04058018 + m * 1.11225687 + s * -0.07167668;
    let z = l * -0.07638128 + m * -0.42148198 + s * 1.58616322;
    [3.2404542 * x + -1.5371385 * y + -0.4985314 * z,
     -0.9692660 * x + 1.8760108 * y + 0.0415560 * z,
     0.0556434 * x + -0.2040259 * y + 1.0572252 * z]
}

fn in_gamut(lab: Triple) -> bool {
    oklab_linear(lab).iter().all(|c| (-0.0001..=1.0001).contains(c))
}

fn triple(cx: &mut Leaf<'_>, values: Triple) -> Value {
    let floats = values.map(|v| cx.float(v));
    list(cx, &floats)
}

fn lab_of(cx: &mut Leaf<'_>, lab: Value) -> std::result::Result<Triple, Fault> {
    let items = elements(cx, lab)?;
    let component = |at: usize| number(cx, items.get(at).copied().unwrap_or(Value::NIL));
    Ok([component(0)?, component(1)?, component(2)?])
}

// HEX as Oklab (L A B), through CIE XYZ as color.el's `color-srgb-to-oklab'.
defsubr! { name = "romacs-color-oklab", gnu = "romacs:faces", effect = Pure,
fn romacs_color_oklab(cx, hex: Value) {
    let [r, g, b] = channels(cx, hex)?.map(linear);
    let x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b;
    let y = 0.21266729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = 0.0193339 * r + 0.1191920 * g + 0.9503041 * b;
    let l = cbrt(x * 0.8189330101 + y * 0.3618667424 + z * -0.1288597137);
    let m = cbrt(x * 0.0329845436 + y * 0.9293118715 + z * 0.0361456387);
    let s = cbrt(x * 0.0482003018 + y * 0.2643662691 + z * 0.6338517070);
    Ok(triple(cx, [l * 0.2104542553 + m * 0.7936177850 + s * -0.0040720468,
                   l * 1.9779984951 + m * -2.4285922050 + s * 0.4505937099,
                   l * 0.0259040371 + m * 0.7827717662 + s * -0.8086757660]))
}}

// LAB as "#rrggbb", each channel clamped into sRGB.
defsubr! { name = "romacs-color-from-oklab", gnu = "romacs:faces", effect = Pure,
fn romacs_color_from_oklab(cx, lab: Value) {
    let lab = lab_of(cx, lab)?;
    let [r, g, b] = oklab_linear(lab).map(|c| (255.0 * gamma(c.clamp(0.0, 1.0)).clamp(0.0, 1.0)).round_ties_even() as u8);
    Ok(cx.string(&format!("#{r:02x}{g:02x}{b:02x}")))
}}

// Whether LAB lies inside sRGB.
defsubr! { name = "romacs-color--in-gamut", gnu = "romacs:faces", effect = Pure,
fn romacs_color_in_gamut(cx, lab: Value) {
    let lab = lab_of(cx, lab)?;
    Ok(if in_gamut(lab) { Value::TRUE } else { Value::NIL })
}}

// The Oklab colour of lightness L, CHROMA and HUE (radians), brought into
// sRGB by reducing its chroma, hue and lightness kept.
defsubr! { name = "romacs-color--oklch", gnu = "romacs:faces", effect = Pure,
fn romacs_color_oklch(cx, l: Value, chroma: Value, hue: Value) {
    let (l, mut chroma, hue) = (number(cx, l)?, number(cx, chroma)?, number(cx, hue)?);
    let at = |chroma: f64| [l, chroma * hue.cos(), chroma * hue.sin()];
    let mut lab = at(chroma);
    while chroma > 0.0001 && !in_gamut(lab) {
        chroma *= 0.95;
        lab = at(chroma);
    }
    Ok(triple(cx, lab))
}}

// HEX's relative luminance, as WCAG 2 defines it.
defsubr! { name = "romacs-color-luminance", gnu = "romacs:faces", effect = Pure,
fn romacs_color_luminance(cx, hex: Value) {
    let [r, g, b] = channels(cx, hex)?.map(|c| if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) });
    Ok(cx.float(0.2126 * r + 0.7152 * g + 0.0722 * b))
}}

#[cfg(test)]
mod tests {
    #[test]
    fn a_short_colour_signals() {
        let mut vm = crate::Vm::new();
        let got = vm.eval_source_raw(r##"(list (condition-case e (romacs-color-oklab "#12") (error (car e)))
                                               (condition-case e (romacs-color-luminance "") (error (car e)))
                                               (condition-case e (romacs-color-from-oklab nil) (error (car e)))
                                               (condition-case e (romacs-color--in-gamut '(0.5 0.1)) (error (car e)))
                                               (romacs-color-from-oklab (romacs-color-oklab "#50269a")))"##).unwrap();
        assert_eq!(vm.display(got).unwrap(),
                   r##"(args-out-of-range args-out-of-range wrong-type-argument wrong-type-argument "#50269a")"##);
    }
}
