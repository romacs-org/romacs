//! Fontconfig enumeration, without any evaluator state.
//!
//! GNU's `ftfont_list` enumerates entities with `FcFontList` and scores them
//! itself; generic families go through `ftfont_resolve_generic_family`'s
//! Fontconfig match. This module answers exactly those two questions and
//! nothing about policy.

use romacs_display_protocol::font::FontSlantKind;
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
    // SAFETY: a null config names the current configuration; `file` is a
    // NUL-terminated path that outlives the call.
    unsafe {
        fontconfig_sys::FcConfigAppFontAddFile(std::ptr::null_mut(), file.as_ptr().cast()) != 0
    }
}

/// What Fontconfig's answers depend on: its configuration files and
/// directories and the font directories it scans, as it names them.
pub(crate) fn configuration_inputs() -> Vec<String> {
    if handle().is_none() {
        return vec![];
    }
    let _serial = serialize();
    let mut inputs = vec![];
    // SAFETY: a null config names the current configuration; each list is
    // walked to its end and released, and its strings are copied first.
    unsafe {
        for list in [fontconfig_sys::FcConfigGetConfigFiles(std::ptr::null_mut()),
                     fontconfig_sys::FcConfigGetConfigDirs(std::ptr::null_mut()),
                     fontconfig_sys::FcConfigGetFontDirs(std::ptr::null_mut())] {
            if list.is_null() {
                continue;
            }
            loop {
                let next = fontconfig_sys::FcStrListNext(list);
                if next.is_null() {
                    break;
                }
                inputs.push(CStr::from_ptr(next.cast()).to_string_lossy().into_owned());
            }
            fontconfig_sys::FcStrListDone(list);
        }
    }
    inputs
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

struct Owned<T> {
    ptr: *mut T,
    drop: unsafe extern "C" fn(*mut T),
}

impl<T> Owned<T> {
    fn new(ptr: *mut T, drop: unsafe extern "C" fn(*mut T)) -> Option<Self> {
        (!ptr.is_null()).then_some(Self { ptr, drop })
    }
}

impl<T> Drop for Owned<T> {
    fn drop(&mut self) {
        // SAFETY: `ptr` is non-null, owned, and dropped exactly once with its
        // matching Fontconfig destructor.
        unsafe { (self.drop)(self.ptr) }
    }
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
    // SAFETY (whole function): every pointer comes from Fontconfig, is
    // null-checked, and is released by its owner guard; the pattern takes
    // copies of added values.
    let pattern = Owned::new(
        unsafe { fontconfig_sys::FcPatternCreate() },
        fontconfig_sys::FcPatternDestroy,
    )?;
    if let Some(family) = family {
        let value = CString::new(family).ok()?;
        if unsafe {
            fontconfig_sys::FcPatternAddString(
                pattern.ptr,
                fontconfig::FC_FAMILY.as_ptr(),
                value.as_ptr().cast(),
            )
        } == 0
        {
            return None;
        }
    }
    let _charset = match required {
        Some(ch) => {
            let charset = Owned::new(
                unsafe { fontconfig_sys::FcCharSetCreate() },
                fontconfig_sys::FcCharSetDestroy,
            )?;
            if unsafe { fontconfig_sys::FcCharSetAddChar(charset.ptr, u32::from(ch)) } == 0
                || unsafe {
                    fontconfig_sys::FcPatternAddCharSet(
                        pattern.ptr,
                        fontconfig::FC_CHARSET.as_ptr(),
                        charset.ptr,
                    )
                } == 0
            {
                return None;
            }
            Some(charset)
        }
        None => None,
    };
    let objects = Owned::new(
        unsafe { fontconfig_sys::FcObjectSetCreate() },
        fontconfig_sys::FcObjectSetDestroy,
    )?;
    for key in [
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
    ] {
        if unsafe { fontconfig_sys::FcObjectSetAdd(objects.ptr, key.as_ptr()) } == 0 {
            return None;
        }
    }
    let set = Owned::new(
        unsafe { fontconfig_sys::FcFontList(std::ptr::null_mut(), pattern.ptr, objects.ptr) },
        fontconfig_sys::FcFontSetDestroy,
    )?;
    let count = usize::try_from(unsafe { (*set.ptr).nfont }).ok()?;
    let fonts = unsafe { (*set.ptr).fonts };
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let face = unsafe { *fonts.add(index) };
        if let Some(listed) = listed_face(face, family) {
            out.push(listed);
        }
    }
    Some(out)
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
    set: Owned<fontconfig_sys::FcFontSet>,
    faces: Vec<(usize, ListedFace)>,
}

impl FaceList {
    /// The listed faces having CH.
    pub fn covering(&self, ch: char) -> Vec<ListedFace> {
        let _serial = serialize();
        // SAFETY: the set lives as long as self, and each index is one it
        // listed; the charset belongs to its pattern.
        unsafe {
            let fonts = (*self.set.ptr).fonts;
            self.faces.iter().filter(|(index, _)| {
                let mut charset = std::ptr::null_mut();
                fontconfig_sys::FcPatternGetCharSet(*fonts.add(*index), fontconfig::FC_CHARSET.as_ptr(), 0, &mut charset)
                    == fontconfig_sys::FcResultMatch && fontconfig_sys::FcCharSetHasChar(charset, u32::from(ch)) != 0
            }).map(|(_, face)| face.clone()).collect()
        }
    }
}

pub(crate) fn list_constrained(spec: &ListSpec) -> Option<FaceList> {
    handle()?;
    let _serial = serialize();
    // SAFETY (whole function): as `list_faces_inner`.
    let pattern = Owned::new(unsafe { fontconfig_sys::FcPatternCreate() }, fontconfig_sys::FcPatternDestroy)?;
    for (key, value) in [(fontconfig::FC_FAMILY, &spec.family), (fontconfig::FC_FOUNDRY, &spec.foundry)] {
        if let Some(value) = value {
            let value = CString::new(value.as_str()).ok()?;
            if unsafe { fontconfig_sys::FcPatternAddString(pattern.ptr, key.as_ptr(), value.as_ptr().cast()) } == 0 {
                return None;
            }
        }
    }
    let charset = Owned::new(unsafe { fontconfig_sys::FcCharSetCreate() }, fontconfig_sys::FcCharSetDestroy)?;
    if !spec.chars.is_empty() {
        for &ch in &spec.chars {
            if unsafe { fontconfig_sys::FcCharSetAddChar(charset.ptr, ch) } == 0 {
                return None;
            }
        }
        if unsafe { fontconfig_sys::FcPatternAddCharSet(pattern.ptr, fontconfig::FC_CHARSET.as_ptr(), charset.ptr) } == 0 {
            return None;
        }
    }
    let langs = Owned::new(unsafe { fontconfig_sys::FcLangSetCreate() }, fontconfig_sys::FcLangSetDestroy)?;
    if !spec.langs.is_empty() {
        for lang in &spec.langs {
            let lang = CString::new(lang.as_str()).ok()?;
            if unsafe { fontconfig_sys::FcLangSetAdd(langs.ptr, lang.as_ptr().cast()) } == 0 {
                return None;
            }
        }
        if unsafe { fontconfig_sys::FcPatternAddLangSet(pattern.ptr, fontconfig::FC_LANG.as_ptr(), langs.ptr) } == 0 {
            return None;
        }
    }
    let objects = Owned::new(unsafe { fontconfig_sys::FcObjectSetCreate() }, fontconfig_sys::FcObjectSetDestroy)?;
    for key in [fontconfig::FC_FAMILY, fontconfig::FC_FILE, fontconfig::FC_INDEX, fontconfig::FC_WEIGHT, fontconfig::FC_SLANT,
                fontconfig::FC_WIDTH, fontconfig::FC_SPACING, fontconfig::FC_SCALABLE, fontconfig::FC_VARIABLE,
                fontconfig::FC_POSTSCRIPT_NAME, fontconfig::FC_FOUNDRY, fontconfig::FC_CHARSET] {
        if unsafe { fontconfig_sys::FcObjectSetAdd(objects.ptr, key.as_ptr()) } == 0 {
            return None;
        }
    }
    let set = Owned::new(unsafe { fontconfig_sys::FcFontList(std::ptr::null_mut(), pattern.ptr, objects.ptr) }, fontconfig_sys::FcFontSetDestroy)?;
    let count = usize::try_from(unsafe { (*set.ptr).nfont }).ok()?;
    let fonts = unsafe { (*set.ptr).fonts };
    let faces = (0..count).filter_map(|index| Some((index, listed_face(unsafe { *fonts.add(index) }, spec.family.as_deref())?))).collect();
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
    // SAFETY (whole function): as `list_faces_inner`.
    let pattern = Owned::new(
        unsafe { fontconfig_sys::FcPatternCreate() },
        fontconfig_sys::FcPatternDestroy,
    )?;
    let objects = Owned::new(
        unsafe { fontconfig_sys::FcObjectSetCreate() },
        fontconfig_sys::FcObjectSetDestroy,
    )?;
    if unsafe { fontconfig_sys::FcObjectSetAdd(objects.ptr, fontconfig::FC_FAMILY.as_ptr()) } == 0 {
        return None;
    }
    let set = Owned::new(
        unsafe { fontconfig_sys::FcFontList(std::ptr::null_mut(), pattern.ptr, objects.ptr) },
        fontconfig_sys::FcFontSetDestroy,
    )?;
    let count = usize::try_from(unsafe { (*set.ptr).nfont }).ok()?;
    let fonts = unsafe { (*set.ptr).fonts };
    Some((0..count).filter_map(|index| get_string(unsafe { *fonts.add(index) }, fontconfig::FC_FAMILY)).collect())
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
    // SAFETY (whole function): as `list_faces_inner`; patterns in the sorted
    // set are owned by that set and read only while it lives.
    let pattern = Owned::new(
        unsafe { fontconfig_sys::FcPatternCreate() },
        fontconfig_sys::FcPatternDestroy,
    )?;
    let value = CString::new(family).ok()?;
    if unsafe {
        fontconfig_sys::FcPatternAddString(
            pattern.ptr,
            fontconfig::FC_FAMILY.as_ptr(),
            value.as_ptr().cast(),
        )
    } == 0
    {
        return None;
    }
    let charset = Owned::new(
        unsafe { fontconfig_sys::FcCharSetCreate() },
        fontconfig_sys::FcCharSetDestroy,
    )?;
    if unsafe { fontconfig_sys::FcCharSetAddChar(charset.ptr, u32::from(ch)) } == 0
        || unsafe {
            fontconfig_sys::FcPatternAddCharSet(
                pattern.ptr,
                fontconfig::FC_CHARSET.as_ptr(),
                charset.ptr,
            )
        } == 0
    {
        return None;
    }
    unsafe {
        fontconfig_sys::FcConfigSubstitute(
            std::ptr::null_mut(),
            pattern.ptr,
            fontconfig_sys::FcMatchPattern,
        );
        fontconfig_sys::FcDefaultSubstitute(pattern.ptr);
    }
    let mut result = fontconfig_sys::FcResultNoMatch;
    let set = Owned::new(
        unsafe {
            fontconfig_sys::FcFontSort(
                std::ptr::null_mut(),
                pattern.ptr,
                0,
                std::ptr::null_mut(),
                &mut result,
            )
        },
        fontconfig_sys::FcFontSetDestroy,
    )?;
    let count = usize::try_from(unsafe { (*set.ptr).nfont }).ok()?;
    let fonts = unsafe { (*set.ptr).fonts };
    let mut out = Vec::new();
    for index in 0..count {
        let face = unsafe { *fonts.add(index) };
        let mut faces_charset: *mut fontconfig_sys::FcCharSet = std::ptr::null_mut();
        let has = unsafe {
            fontconfig_sys::FcPatternGetCharSet(
                face,
                fontconfig::FC_CHARSET.as_ptr(),
                0,
                &mut faces_charset,
            ) == fontconfig_sys::FcResultMatch
                && !faces_charset.is_null()
                && fontconfig_sys::FcCharSetHasChar(faces_charset, u32::from(ch)) != 0
        };
        if has && let Some(listed) = listed_face(face, None) {
            out.push(listed);
        }
    }
    Some(out)
}

fn listed_face(
    pattern: *mut fontconfig_sys::FcPattern,
    requested_family: Option<&str>,
) -> Option<ListedFace> {
    if pattern.is_null() || get_bool(pattern, fontconfig::FC_SCALABLE) != Some(true) {
        return None;
    }
    let weight = get_int(pattern, fontconfig::FC_WEIGHT);
    if get_bool(pattern, fontconfig::FC_VARIABLE) == Some(true) && weight.is_none() {
        return None;
    }
    let families = get_strings(pattern, fontconfig::FC_FAMILY);
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
        file: get_string(pattern, fontconfig::FC_FILE)?,
        selector: u32::try_from(get_int(pattern, fontconfig::FC_INDEX)?).ok()?,
        postscript_name: get_string(pattern, fontconfig::FC_POSTSCRIPT_NAME),
        weight: crate::style::css_weight_from_fontconfig(weight.unwrap_or(80)),
        slant: match get_int(pattern, fontconfig::FC_SLANT) {
            Some(fontconfig::FC_SLANT_ITALIC) => FontSlantKind::Italic,
            Some(fontconfig::FC_SLANT_OBLIQUE) => FontSlantKind::Oblique,
            _ => FontSlantKind::Normal,
        },
        width: crate::style::width_class_from_fontconfig(
            get_int(pattern, fontconfig::FC_WIDTH).unwrap_or(100),
        ),
        spacing: get_int(pattern, fontconfig::FC_SPACING),
        foundry: get_string(pattern, fontconfig::FC_FOUNDRY),
        fc_style: (weight.unwrap_or(80), get_int(pattern, fontconfig::FC_SLANT).unwrap_or(0),
                   get_int(pattern, fontconfig::FC_WIDTH).unwrap_or(100)),
    })
}

fn get_string(pattern: *mut fontconfig_sys::FcPattern, key: &CStr) -> Option<String> {
    get_string_at(pattern, key, 0)
}

fn get_strings(pattern: *mut fontconfig_sys::FcPattern, key: &CStr) -> Vec<String> {
    (0..)
        .map_while(|n| get_string_at(pattern, key, n))
        .collect()
}

fn get_string_at(pattern: *mut fontconfig_sys::FcPattern, key: &CStr, n: i32) -> Option<String> {
    let mut value: *mut fontconfig_sys::FcChar8 = std::ptr::null_mut();
    // SAFETY: Fontconfig writes a pointer into pattern-owned storage, valid
    // while the pattern lives; it is copied out immediately.
    let ok = unsafe { fontconfig_sys::FcPatternGetString(pattern, key.as_ptr(), n, &mut value) };
    if ok != fontconfig_sys::FcResultMatch || value.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(value.cast()) }
        .to_str()
        .ok()?
        .trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn get_int(pattern: *mut fontconfig_sys::FcPattern, key: &CStr) -> Option<i32> {
    let mut value = 0;
    // SAFETY: reads one integer property of a live pattern.
    let ok = unsafe { fontconfig_sys::FcPatternGetInteger(pattern, key.as_ptr(), 0, &mut value) };
    (ok == fontconfig_sys::FcResultMatch).then_some(value)
}

fn get_bool(pattern: *mut fontconfig_sys::FcPattern, key: &CStr) -> Option<bool> {
    let mut value = 0;
    // SAFETY: reads one boolean property of a live pattern.
    let ok = unsafe { fontconfig_sys::FcPatternGetBool(pattern, key.as_ptr(), 0, &mut value) };
    (ok == fontconfig_sys::FcResultMatch).then_some(value != 0)
}
