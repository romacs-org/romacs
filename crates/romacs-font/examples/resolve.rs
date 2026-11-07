//! Print how a family resolves on this machine, per character.
//!
//! `cargo run -p romacs-font --example resolve -- Iosevka 16 'Ωμέγα Привет Hello 😀'`
//!
//! With `--owner-emoji` the policy carries
//! `(set-fontset-font t 'emoji "Noto Color Emoji" nil 'prepend)`, approximated by
//! U+1F300..U+1FAFF (the test carries GNU's exact emoji ranges). `--bold` and `--italic` change the requested style.
//! With `--bundled` the fonts Romacs ships (assets/fonts) are registered first,
//! as the GUI registers them, so the primary line names the file that wins.

use romacs_font::{
    FontDef, FontRequest, FontResolver, Fontset, FontsetMode, FontsetState, FontsetTarget,
};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |args: &mut Vec<String>, name: &str| {
        let present = args.iter().any(|arg| arg == name);
        args.retain(|arg| arg != name);
        present
    };
    let owner_emoji = flag(&mut args, "--owner-emoji");
    let bold = flag(&mut args, "--bold");
    let italic = flag(&mut args, "--italic");
    if flag(&mut args, "--bundled") {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
        let mut fonts: Vec<_> = std::fs::read_dir(&dir).expect("assets/fonts").filter_map(|entry| Some(entry.ok()?.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "ttf")).collect();
        fonts.sort();
        for font in fonts {
            assert!(romacs_font::register_font_file(&font), "Fontconfig refused {font:?}");
        }
    }
    let family = args.first().map_or("monospace", String::as_str);
    let size: f32 = args
        .get(1)
        .and_then(|size| size.parse().ok())
        .unwrap_or(16.0);
    let text = args.get(2).map_or("Ωμέγα Привет Hello", String::as_str);

    let mut fontset = Fontset::new("example");
    if owner_emoji {
        fontset.add(
            FontsetTarget::Range(0x1F300, 0x1FAFF),
            Some(std::sync::Arc::new(FontDef::family("Noto Color Emoji"))),
            FontsetMode::Prepend,
        );
    }
    let mut resolver = FontResolver::new().expect("platform font catalog");
    let fontset = std::sync::Arc::new(fontset);
    resolver.set_fontsets(std::sync::Arc::new(FontsetState::new(fontset.clone(), fontset, Default::default(), 0, 1)));
    let mut request = FontRequest::new(family, size);
    if bold {
        request = request.with_weight(700);
    }
    if italic {
        request = request.with_slant(romacs_display_protocol::font::FontSlantKind::Italic);
    }
    match resolver.resolve_primary(&request) {
        Ok(font) => println!(
            "primary {family:?} -> {} | {} | w{} {:?} wc{} | asc {} desc {} space {} | {:?}",
            font.family,
            font.identity.stable_key,
            font.weight,
            font.slant,
            font.width,
            font.ascent_px,
            font.descent_px,
            font.space_advance_px,
            font.glyph_advance
        ),
        Err(error) => {
            println!("primary {family:?} -> ERROR {error}");
            return;
        }
    }
    let mut seen = Vec::new();
    for ch in text
        .chars()
        .filter(|ch| !ch.is_ascii() && !ch.is_whitespace())
    {
        if seen.contains(&ch) {
            continue;
        }
        seen.push(ch);
        match resolver.resolve_char(&request, ch) {
            Ok(found) => println!(
                "U+{:04X} {ch} -> {:?} {} glyph {} adv {}",
                u32::from(ch),
                found.source,
                found.font.family,
                found.glyph.glyph_id.get(),
                found.glyph.advance_px
            ),
            Err(error) => println!("U+{:04X} {ch} -> ERROR {error}", u32::from(ch)),
        }
    }
}
