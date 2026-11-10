//! Fontconfig enumeration, without any evaluator state.
//!
//! GNU's `ftfont_list` enumerates entities with `FcFontList` and scores them
//! itself; generic families go through `ftfont_resolve_generic_family`'s
//! Fontconfig match. This module answers exactly those two questions and
//! nothing about policy.

use romacs_display_protocol::font::FontSlantKind;
use crate::fc::{self, CharSet, FontSet, LangSet, ObjectSet, Pattern, PatternRef};
use std::ffi::{CStr, CString};
use std::sync::OnceLock;

/// One Fontconfig pattern, projected to what scoring and replay need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListedFace {
    pub family: String,
    pub file: String,
    /// Fontconfig `FC_INDEX`: collection face in bits 0-15, FreeType named
    /// instance in bits 16-30.
    pub selector: u32,
    pub postscript_name: Option<String>,
    pub weight: u16,
    pub slant: FontSlantKind,
    pub width: u16,
    pub spacing: Option<i32>,
    /// Fontconfig's `FC_FOUNDRY', and its raw `FC_WEIGHT', `FC_SLANT' and
    /// `FC_WIDTH', as GNU's ftfont.c reads them for a font-entity.
    pub foundry: Option<String>,
    pub fc_style: (i32, i32, i32),
}

impl ListedFace {
    pub fn is_monospace(&self) -> bool {
        self.spacing.is_some_and(|spacing| spacing >= 100)
    }
}

/// Fontconfig's configuration is not safe to extend while another thread
/// queries it (`FcConfigAppFontAddFile` rebuilds the font set in place), so
/// every call this module makes holds this lock.
pub(crate) fn serialize() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn handle() -> Option<&'static fontconfig::Fontconfig> {
    static HANDLE: OnceLock<Option<fontconfig::Fontconfig>> = OnceLock::new();
    HANDLE.get_or_init(fontconfig::Fontconfig::new).as_ref()
}

pub(crate) fn register_font_file(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    if handle().is_none() {
        return false;
    }
    let Ok(file) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let _serial = serialize();
    fc::add_app_font_file(&file)
}

/// What Fontconfig's answers depend on: its configuration files and
/// directories and the font directories it scans, as it names them.
pub(crate) fn configuration_inputs() -> Vec<String> {
    if handle().is_none() {
        return vec![];
    }
    let _serial = serialize();
    fc::configuration_inputs()
}

/// The concrete family Fontconfig's match gives GENERIC (an alias such as
/// `monospace`, or a family that is not installed), if any.
pub(crate) fn match_family(generic: &str) -> Option<String> {
    let fc = handle()?;
    let _serial = serialize();
    let mut pattern = fontconfig::Pattern::new(fc).ok()?;
    let value = CString::new(generic).ok()?;
    pattern.add_string(fontconfig::FC_FAMILY, &value).ok()?;
    let matched = pattern.font_match().ok()?;
    let family = matched.get_string(fontconfig::FC_FAMILY).ok()?.trim();
    (!family.is_empty()).then(|| family.to_owned())
}

/// The properties a listed face carries.
const LISTED: [&CStr; 11] = [
    fontconfig::FC_FAMILY,
    fontconfig::FC_FILE,
    fontconfig::FC_INDEX,
    fontconfig::FC_WEIGHT,
    fontconfig::FC_SLANT,
    fontconfig::FC_WIDTH,
    fontconfig::FC_SPACING,
    fontconfig::FC_SCALABLE,
    fontconfig::FC_VARIABLE,
    fontconfig::FC_POSTSCRIPT_NAME,
    fontconfig::FC_FOUNDRY,
];

/// A pattern naming FAMILY when given.
fn family_pattern(family: Option<&str>) -> Option<Pattern> {
    let mut pattern = Pattern::new()?;
    if let Some(family) = family {
        let value = CString::new(family).ok()?;
        pattern.add_string(fontconfig::FC_FAMILY, &value).then_some(())?;
    }
    Some(pattern)
}

/// A charset of CHARS.
fn charset_of(chars: impl IntoIterator<Item = u32>) -> Option<CharSet> {
    let mut charset = CharSet::new()?;
    for ch in chars {
        charset.add(ch).then_some(())?;
    }
    Some(charset)
}

/// `FcFontList` over FAMILY (any family when `None`), restricted to faces
/// whose charset contains REQUIRED when given. Variable-font default
/// patterns without a weight are skipped, as layout's adapter does; their
/// named instances are listed individually. Only scalable faces are returned
/// (bitmap strikes are out of this slice's scope).
pub(crate) fn list_faces(family: Option<&str>, required: Option<char>) -> Vec<ListedFace> {
    if handle().is_none() {
        return Vec::new();
    }
    let _serial = serialize();
    list_faces_inner(family, required).unwrap_or_default()
}

fn list_faces_inner(family: Option<&str>, required: Option<char>) -> Option<Vec<ListedFace>> {
    let mut pattern = family_pattern(family)?;
    if let Some(ch) = required {
        let charset = charset_of([u32::from(ch)])?;
        pattern.add_charset(fontconfig::FC_CHARSET, &charset).then_some(())?;
    }
    let set = FontSet::list(&pattern, &ObjectSet::of(&LISTED)?)?;
    let mut out: Vec<_> = set.patterns().filter_map(|face| listed_face(face, family)).collect();
    registered_first(&mut out, |face| &face.file);
    Some(out)
}

/// LISTED with the faces of fonts Romacs registered first, each group in
/// Fontconfig's order, so a font Romacs ships wins a tie with a system
/// face of the same family and style (`best_face' keeps the first best):
/// the bundled Iosevka, not a system copy of another version.
fn registered_first<T>(listed: &mut [T], file: impl Fn(&T) -> &str) {
    let registered: std::collections::HashSet<String> = fc::application_files().into_iter().collect();
    if !registered.is_empty() {
        listed.sort_by_key(|face| !registered.contains(file(face)));
    }
}

/// What ftfont_spec_pattern asks of a listing for a font-spec.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct ListSpec {
    pub family: Option<String>,
    pub foundry: Option<String>,
    /// Characters every face must have, and languages it must cover.
    pub chars: Vec<u32>,
    pub langs: Vec<String>,
}

/// `FcFontList' for a [`ListSpec`], with each face's charset, so one listing
/// answers for any character, as GNU caches font_list_entities and tests
/// has_char.
pub(crate) struct FaceList {
    set: FontSet,
    faces: Vec<(usize, ListedFace)>,
}

impl FaceList {
    /// The listed faces having CH.
    pub fn covering(&self, ch: char) -> Vec<ListedFace> {
        let _serial = serialize();
        self.faces
            .iter()
            .filter(|(index, _)| self.set.get(*index).is_some_and(|face| face.has_char(ch)))
            .map(|(_, face)| face.clone())
            .collect()
    }
}

pub(crate) fn list_constrained(spec: &ListSpec) -> Option<FaceList> {
    handle()?;
    let _serial = serialize();
    let mut pattern = family_pattern(spec.family.as_deref())?;
    if let Some(foundry) = &spec.foundry {
        let value = CString::new(foundry.as_str()).ok()?;
        pattern.add_string(fontconfig::FC_FOUNDRY, &value).then_some(())?;
    }
    if !spec.chars.is_empty() {
        let charset = charset_of(spec.chars.iter().copied())?;
        pattern.add_charset(fontconfig::FC_CHARSET, &charset).then_some(())?;
    }
    if !spec.langs.is_empty() {
        let mut langs = LangSet::new()?;
        for lang in &spec.langs {
            langs.add(&CString::new(lang.as_str()).ok()?).then_some(())?;
        }
        pattern.add_langset(fontconfig::FC_LANG, &langs).then_some(())?;
    }
    let mut objects = LISTED.to_vec();
    objects.push(fontconfig::FC_CHARSET);
    let set = FontSet::list(&pattern, &ObjectSet::of(&objects)?)?;
    let mut faces: Vec<_> = set
        .patterns()
        .enumerate()
        .filter_map(|(index, face)| Some((index, listed_face(face, spec.family.as_deref())?)))
        .collect();
    registered_first(&mut faces, |(_, face)| &face.file);
    Some(FaceList { set, faces })
}

/// Every family Fontconfig lists, as GNU's ftfont_list_family asks: one
/// `FcFontList' pattern per distinct family set, its first name each,
/// scalable or not.
pub(crate) fn list_families() -> Vec<String> {
    if handle().is_none() {
        return Vec::new();
    }
    let _serial = serialize();
    list_families_inner().unwrap_or_default()
}

fn list_families_inner() -> Option<Vec<String>> {
    let set = FontSet::list(&Pattern::new()?, &ObjectSet::of(&[fontconfig::FC_FAMILY])?)?;
    Some(set.patterns().filter_map(|face| face.string(fontconfig::FC_FAMILY)).collect())
}

/// Fontconfig's own preference order for FAMILY's pattern with CH required
/// (`FcConfigSubstitute` + `FcDefaultSubstitute` + `FcFontSort`), keeping only
/// scalable faces whose charset has CH. This is the order the platform's
/// alias and preference rules (for example "emoji prefers a colour font")
/// produce; enumeration order carries no such preference.
pub(crate) fn sort_faces(family: &str, ch: char) -> Vec<ListedFace> {
    if handle().is_none() {
        return Vec::new();
    }
    let _serial = serialize();
    sort_faces_inner(family, ch).unwrap_or_default()
}

fn sort_faces_inner(family: &str, ch: char) -> Option<Vec<ListedFace>> {
    let mut pattern = family_pattern(Some(family))?;
    let charset = charset_of([u32::from(ch)])?;
    pattern.add_charset(fontconfig::FC_CHARSET, &charset).then_some(())?;
    pattern.substitute();
    let set = FontSet::sort(&pattern)?;
    Some(
        set.patterns()
            .filter(|face| face.has_char(ch))
            .filter_map(|face| listed_face(face, None))
            .collect(),
    )
}

fn listed_face(pattern: PatternRef<'_>, requested_family: Option<&str>) -> Option<ListedFace> {
    if pattern.bool(fontconfig::FC_SCALABLE) != Some(true) {
        return None;
    }
    let weight = pattern.int(fontconfig::FC_WEIGHT);
    if pattern.bool(fontconfig::FC_VARIABLE) == Some(true) && weight.is_none() {
        return None;
    }
    let families = pattern.strings(fontconfig::FC_FAMILY);
    // A pattern lists every family name it answers to; report the one the
    // caller asked for so policy sees its own spelling.
    let family = requested_family
        .and_then(|wanted| {
            families
                .iter()
                .find(|name| name.eq_ignore_ascii_case(wanted))
        })
        .or_else(|| families.first())?
        .clone();
    Some(ListedFace {
        family,
        file: pattern.string(fontconfig::FC_FILE)?,
        selector: u32::try_from(pattern.int(fontconfig::FC_INDEX)?).ok()?,
        postscript_name: pattern.string(fontconfig::FC_POSTSCRIPT_NAME),
        weight: crate::style::css_weight_from_fontconfig(weight.unwrap_or(80)),
        slant: match pattern.int(fontconfig::FC_SLANT) {
            Some(fontconfig::FC_SLANT_ITALIC) => FontSlantKind::Italic,
            Some(fontconfig::FC_SLANT_OBLIQUE) => FontSlantKind::Oblique,
            _ => FontSlantKind::Normal,
        },
        width: crate::style::width_class_from_fontconfig(
            pattern.int(fontconfig::FC_WIDTH).unwrap_or(100),
        ),
        spacing: pattern.int(fontconfig::FC_SPACING),
        foundry: pattern.string(fontconfig::FC_FOUNDRY),
        fc_style: (weight.unwrap_or(80), pattern.int(fontconfig::FC_SLANT).unwrap_or(0),
                   pattern.int(fontconfig::FC_WIDTH).unwrap_or(100)),
    })
}
