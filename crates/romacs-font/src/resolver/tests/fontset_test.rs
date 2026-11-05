//! fontset_font's order, stop markers, group order and retirement, over the
//! repository's fonts.
use super::*;
use romacs_layout_input::{Alternative, Fontset, FontsetMode, FontsetTarget};
use std::collections::HashMap;

const FAMILY: &str = "JetBrainsMono Nerd Font";
const EMOJI: &str = "Noto Color Emoji";

fn fixtures() -> FontResolver {
    for path in [romacs_test_fonts::jetbrains_mono_nerd_font_regular_3_4_0(), romacs_test_fonts::noto_color_emoji_2_051(),
                 romacs_test_fonts::noto_sans_mono_variable()] {
        assert!(crate::register_font_file(path));
    }
    FontResolver::new().unwrap()
}

fn def(family: &str) -> Alternative {
    Some(Arc::new(FontDef::family(family)))
}

fn registry(name: &str, encoding: u32) -> Alternative {
    Some(Arc::new(FontDef { registry: Some(name.into()), encoding, ..FontDef::default() }))
}

fn publish(resolver: &mut FontResolver, default: Fontset, frame: Option<Fontset>, order: &[(u32, u32)], revision: u64) {
    let default = Arc::new(default);
    let frame = frame.map_or_else(|| Arc::clone(&default), Arc::new);
    let order: HashMap<u32, u32> = order.iter().copied().collect();
    let preferred = order.len() as u32;
    resolver.set_fontsets(Arc::new(FontsetState::new(default, frame, Arc::new(order), preferred, revision)));
}

fn request() -> FontRequest {
    FontRequest::new(FAMILY, 16.0)
}

fn fallback_to_face() -> Fontset {
    let mut frame = Fontset::new("startup");
    frame.add(FontsetTarget::Fallback, registry("iso10646-1", 0), FontsetMode::Overwrite);
    frame
}

#[cfg(target_os = "linux")]
#[test]
fn ascii_is_the_face_font_whatever_the_rules_say() {
    let mut resolver = fixtures();
    let mut rules = Fontset::new("d");
    rules.add(FontsetTarget::Range(0, 0x3FFFFF), def(EMOJI), FontsetMode::Overwrite);
    publish(&mut resolver, rules, None, &[], 1);
    assert_eq!(resolver.resolve_char(&request(), 'a').unwrap().font.family, FAMILY);
}

#[cfg(target_os = "linux")]
#[test]
fn a_nil_alternative_stops_its_fontset_and_its_fallback() {
    let mut resolver = fixtures();
    let mut frame = fallback_to_face();
    frame.add(FontsetTarget::Range(0x416, 0x416), None, FontsetMode::Overwrite);
    let mut default = Fontset::new("d");
    default.add(FontsetTarget::Fallback, def(FAMILY), FontsetMode::Overwrite);
    publish(&mut resolver, default.clone(), Some(frame), &[], 1);
    // The frame's stop skips its own fallback; the default's goes on.
    assert_eq!(resolver.resolve_char(&request(), 'Ж').unwrap().font.family, FAMILY);

    default.add(FontsetTarget::Range(0x416, 0x416), None, FontsetMode::Overwrite);
    let mut frame = fallback_to_face();
    frame.add(FontsetTarget::Range(0x416, 0x416), None, FontsetMode::Overwrite);
    publish(&mut resolver, default, Some(frame), &[], 2);
    assert!(matches!(resolver.resolve_char(&request(), 'Ж'), Err(FontResolveError::NoCoveringFont { .. })));
}

#[cfg(target_os = "linux")]
#[test]
fn unavailable_rules_find_nothing_though_a_font_has_the_character() {
    let mut resolver = fixtures();
    let mut rules = Fontset::new("d");
    rules.add(FontsetTarget::Range(0x416, 0x416), def("No Such Family"), FontsetMode::Overwrite);
    rules.add(FontsetTarget::Fallback, def("Also Missing"), FontsetMode::Overwrite);
    publish(&mut resolver, rules, None, &[], 1);
    assert!(matches!(resolver.resolve_char(&request(), 'Ж'), Err(FontResolveError::NoCoveringFont { .. })));
}

#[cfg(target_os = "linux")]
#[test]
fn a_rule_without_a_family_prefers_the_face_family() {
    let mut resolver = fixtures();
    let mut rules = Fontset::new("d");
    rules.add(FontsetTarget::Range(0x416, 0x416), registry("iso10646-1", 0), FontsetMode::Overwrite);
    publish(&mut resolver, rules, None, &[], 1);
    assert_eq!(resolver.resolve_char(&request(), 'Ж').unwrap().font.family, FAMILY);
    assert_eq!(resolver.resolve_char(&FontRequest::new("Noto Sans Mono", 16.0), 'Ж').unwrap().font.family, "Noto Sans Mono");
}

#[cfg(target_os = "linux")]
#[test]
fn a_new_revision_retires_every_answer() {
    let mut resolver = fixtures();
    let mut rules = fallback_to_face();
    rules.add(FontsetTarget::Range(0x2600, 0x26FF), None, FontsetMode::Overwrite);
    publish(&mut resolver, rules.clone(), None, &[], 1);
    assert!(resolver.resolve_char(&request(), '☀').is_err());
    rules.add(FontsetTarget::Range(0x2600, 0x26FF), def(EMOJI), FontsetMode::Overwrite);
    publish(&mut resolver, rules, None, &[], 2);
    assert_eq!(resolver.resolve_char(&request(), '☀').unwrap().font.family, EMOJI);
    assert_eq!(resolver.realized.len(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn an_icon_answer_belongs_to_its_family_and_a_stop_reaches_it() {
    let mut resolver = fixtures();
    let icon = '\u{F0001}';
    let mut rules = Fontset::new("d");
    rules.add(FontsetTarget::Range(u32::from(icon), u32::from(icon)), registry("iso10646-1", 0), FontsetMode::Overwrite);
    publish(&mut resolver, rules.clone(), None, &[], 1);
    let nerd = resolver.resolve_sole(&request(), icon).map(|found| found.font.family.clone()).ok();
    let mono = FontRequest::new("Noto Sans Mono", 16.0);
    let other = resolver.resolve_sole(&mono, icon).map(|found| found.font.family.clone()).ok();
    assert_eq!(nerd.as_deref(), Some(FAMILY));
    assert_eq!(other, resolver.resolve_char(&mono, icon).map(|found| found.font.family).ok());
    rules.add(FontsetTarget::Range(u32::from(icon), u32::from(icon)), None, FontsetMode::Prepend);
    publish(&mut resolver, rules, None, &[], 2);
    assert!(resolver.resolve_sole(&request(), icon).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn a_group_follows_the_charset_order() {
    let mut resolver = fixtures();
    let (first, second) = (5, 9);
    let wanted = |family: &str, encoding| Some(Arc::new(FontDef { family: Some(family.into()), encoding, ..FontDef::default() }));
    let mut rules = Fontset::new("d");
    let range = FontsetTarget::Range(0x416, 0x416);
    rules.add(range, wanted("Noto Sans Mono", first), FontsetMode::Overwrite);
    rules.add(range, wanted(FAMILY, second), FontsetMode::Append);
    let request = FontRequest::new("No Such Face", 16.0);
    publish(&mut resolver, rules.clone(), None, &[(first, 0), (second, 1)], 1);
    assert_eq!(resolver.resolve_char(&request, 'Ж').unwrap().font.family, "Noto Sans Mono");
    publish(&mut resolver, rules, None, &[(second, 0), (first, 1)], 2);
    assert_eq!(resolver.resolve_char(&request, 'Ж').unwrap().font.family, FAMILY);
}

#[cfg(target_os = "linux")]
#[test]
fn a_size_in_the_spec_beats_the_faces() {
    let mut resolver = fixtures();
    let sized = FontDef { family: Some(EMOJI.into()), pixel_size: Some(40), ..FontDef::default() };
    let mut rules = Fontset::new("d");
    rules.add(FontsetTarget::Range(0x2600, 0x26FF), Some(Arc::new(sized)), FontsetMode::Overwrite);
    publish(&mut resolver, rules, None, &[], 1);
    assert_eq!(resolver.resolve_char(&request(), '☀').unwrap().font.pixel_size, 40.0);
}

#[cfg(target_os = "linux")]
#[test]
fn a_size_of_its_own_gets_no_ascii_preference() {
    let mut resolver = fixtures();
    let (first, second) = (5, 9);
    let sized = FontDef { family: Some(FAMILY.into()), encoding: second, pixel_size: Some(40), ..FontDef::default() };
    let plain = FontDef { family: Some("Noto Sans Mono".into()), encoding: first, ..FontDef::default() };
    let mut rules = Fontset::new("d");
    let range = FontsetTarget::Range(0x416, 0x416);
    rules.add(range, Some(Arc::new(sized)), FontsetMode::Overwrite);
    rules.add(range, Some(Arc::new(plain)), FontsetMode::Append);
    publish(&mut resolver, rules, None, &[(first, 0), (second, 1)], 1);
    assert_eq!(resolver.resolve_char(&request(), 'Ж').unwrap().font.family, "Noto Sans Mono");
}

#[test]
fn registries_match_as_ftfont_get_charset() {
    let def = |registry: &str| FontDef { registry: Some(registry.into()), ..FontDef::default() };
    assert_eq!(registry_constraints(&def("iso10646-1")), Some((vec![], None)));
    assert_eq!(registry_constraints(&def("iso8859-7")), Some((vec![0x00A0, 0x0384], None)));
    assert_eq!(registry_constraints(&def("gb2312.1980*-*")), Some((vec![0x4E13], Some("zh-cn"))));
    assert_eq!(registry_constraints(&def("jisx0208*-*")), Some((vec![0x4E55], Some("ja"))));
    assert_eq!(registry_constraints(&def("no-such-registry")), None);
}
