//! Owned and borrowed handles over the Fontconfig C API, so the catalog
//! (`fontconfig_catalog`) is safe code. Each foreign call is made here,
//! once. Every function reads or changes Fontconfig's current
//! configuration (a null `FcConfig`), so callers hold
//! `fontconfig_catalog::serialize()` while they use these handles.

use fontconfig_sys as sys;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::ptr::{NonNull, null_mut};

/// A pointer Fontconfig allocated, released once by its destructor.
struct Owned<T> {
    ptr: NonNull<T>,
    destroy: unsafe extern "C" fn(*mut T),
}

impl<T> Owned<T> {
    fn new(ptr: *mut T, destroy: unsafe extern "C" fn(*mut T)) -> Option<Self> {
        NonNull::new(ptr).map(|ptr| Self { ptr, destroy })
    }

    fn get(&self) -> *mut T {
        self.ptr.as_ptr()
    }
}

impl<T> Drop for Owned<T> {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from the allocator `destroy` pairs with, and
        // this owner, which nothing copies, releases it once.
        unsafe { (self.destroy)(self.ptr.as_ptr()) }
    }
}

/// A pattern this process built.
pub(crate) struct Pattern(Owned<sys::FcPattern>);

impl Pattern {
    pub fn new() -> Option<Self> {
        // SAFETY: FcPatternCreate takes nothing; a null result is refused.
        Owned::new(unsafe { sys::FcPatternCreate() }, sys::FcPatternDestroy).map(Self)
    }

    /// Add VALUE under KEY; false when Fontconfig refuses.
    pub fn add_string(&mut self, key: &CStr, value: &CStr) -> bool {
        // SAFETY: the pattern is live; KEY and VALUE are NUL-terminated,
        // and Fontconfig copies VALUE.
        unsafe { sys::FcPatternAddString(self.0.get(), key.as_ptr(), value.as_ptr().cast()) != 0 }
    }

    pub fn add_charset(&mut self, key: &CStr, charset: &CharSet) -> bool {
        // SAFETY: both are live and KEY is NUL-terminated; Fontconfig takes
        // its own reference to the charset.
        unsafe { sys::FcPatternAddCharSet(self.0.get(), key.as_ptr(), charset.0.get()) != 0 }
    }

    pub fn add_langset(&mut self, key: &CStr, langs: &LangSet) -> bool {
        // SAFETY: both are live and KEY is NUL-terminated; Fontconfig copies
        // the language set.
        unsafe { sys::FcPatternAddLangSet(self.0.get(), key.as_ptr(), langs.0.get()) != 0 }
    }

    /// Apply the configuration's match rules, then the defaults, as a
    /// pattern is prepared for matching or sorting.
    pub fn substitute(&mut self) {
        // SAFETY: the pattern is live; a null config names the current one.
        unsafe {
            sys::FcConfigSubstitute(null_mut(), self.0.get(), sys::FcMatchPattern);
            sys::FcDefaultSubstitute(self.0.get());
        }
    }
}

pub(crate) struct CharSet(Owned<sys::FcCharSet>);

impl CharSet {
    pub fn new() -> Option<Self> {
        // SAFETY: FcCharSetCreate takes nothing; a null result is refused.
        Owned::new(unsafe { sys::FcCharSetCreate() }, sys::FcCharSetDestroy).map(Self)
    }

    pub fn add(&mut self, ch: u32) -> bool {
        // SAFETY: the charset is live.
        unsafe { sys::FcCharSetAddChar(self.0.get(), ch) != 0 }
    }
}

pub(crate) struct LangSet(Owned<sys::FcLangSet>);

impl LangSet {
    pub fn new() -> Option<Self> {
        // SAFETY: FcLangSetCreate takes nothing; a null result is refused.
        Owned::new(unsafe { sys::FcLangSetCreate() }, sys::FcLangSetDestroy).map(Self)
    }

    pub fn add(&mut self, lang: &CStr) -> bool {
        // SAFETY: the set is live and LANG NUL-terminated; Fontconfig copies it.
        unsafe { sys::FcLangSetAdd(self.0.get(), lang.as_ptr().cast()) != 0 }
    }
}

/// The properties a listing's patterns carry.
pub(crate) struct ObjectSet(Owned<sys::FcObjectSet>);

impl ObjectSet {
    pub fn of(keys: &[&CStr]) -> Option<Self> {
        // SAFETY: FcObjectSetCreate takes nothing; a null result is refused.
        let set = Owned::new(unsafe { sys::FcObjectSetCreate() }, sys::FcObjectSetDestroy)?;
        for key in keys {
            // SAFETY: the set is live; KEY is NUL-terminated and, being a
            // Fontconfig object name, static.
            if unsafe { sys::FcObjectSetAdd(set.get(), key.as_ptr()) } == 0 {
                return None;
            }
        }
        Some(Self(set))
    }
}

/// A font set this process owns: a listing or a sort.
pub(crate) struct FontSet(Owned<sys::FcFontSet>);

impl FontSet {
    /// `FcFontList`: the faces matching PATTERN, carrying OBJECTS.
    pub fn list(pattern: &Pattern, objects: &ObjectSet) -> Option<Self> {
        // SAFETY: both are live; a null config names the current one.
        let set = unsafe { sys::FcFontList(null_mut(), pattern.0.get(), objects.0.get()) };
        Owned::new(set, sys::FcFontSetDestroy).map(Self)
    }

    /// `FcFontSort` without trimming: every face, in the configuration's
    /// preference order for PATTERN.
    pub fn sort(pattern: &Pattern) -> Option<Self> {
        let mut result = sys::FcResultNoMatch;
        // SAFETY: the pattern is live, `result` a live local, and the null
        // config and charset pointer ask for the current configuration and
        // no coverage union.
        let set = unsafe { sys::FcFontSort(null_mut(), pattern.0.get(), 0, null_mut(), &mut result) };
        Owned::new(set, sys::FcFontSetDestroy).map(Self)
    }

    pub fn patterns(&self) -> impl Iterator<Item = PatternRef<'_>> {
        pattern_slice(self.0.get()).iter().filter_map(|&pattern| PatternRef::new(pattern))
    }

    pub fn get(&self, index: usize) -> Option<PatternRef<'_>> {
        pattern_slice(self.0.get()).get(index).and_then(|&pattern| PatternRef::new(pattern))
    }
}

/// The patterns of SET, borrowed for as long as the caller keeps SET.
fn pattern_slice<'a>(set: *mut sys::FcFontSet) -> &'a [*mut sys::FcPattern] {
    // SAFETY: SET is a live font set; Fontconfig keeps `nfont` pattern
    // pointers at `fonts`, unchanged until the set is destroyed or, for the
    // configuration's own sets, the configuration changes, which the
    // caller's `serialize` guard excludes.
    unsafe {
        let count = usize::try_from((*set).nfont).unwrap_or(0);
        if count == 0 || (*set).fonts.is_null() {
            &[]
        } else {
            std::slice::from_raw_parts((*set).fonts, count)
        }
    }
}

/// The files of the application font set: the fonts this process added
/// (`add_app_font_file`).
pub(crate) fn application_files() -> Vec<String> {
    // SAFETY: a null config names the current one, which owns the set.
    let set = unsafe { sys::FcConfigGetFonts(null_mut(), sys::FcSetApplication) };
    if set.is_null() {
        return Vec::new();
    }
    pattern_slice(set)
        .iter()
        .filter_map(|&pattern| PatternRef::new(pattern)?.string(fontconfig::FC_FILE))
        .collect()
}

/// A pattern owned by a font set.
#[derive(Clone, Copy)]
pub(crate) struct PatternRef<'a> {
    ptr: NonNull<sys::FcPattern>,
    owner: PhantomData<&'a ()>,
}

impl PatternRef<'_> {
    fn new(ptr: *mut sys::FcPattern) -> Option<Self> {
        NonNull::new(ptr).map(|ptr| Self { ptr, owner: PhantomData })
    }

    /// KEY's Nth string, trimmed; None when absent, empty or not UTF-8.
    pub fn string_at(self, key: &CStr, n: c_int) -> Option<String> {
        let mut value: *mut sys::FcChar8 = null_mut();
        // SAFETY: the pattern is live while its set is; Fontconfig points
        // VALUE into the pattern's own storage, which is copied out at once.
        let text = unsafe {
            if sys::FcPatternGetString(self.ptr.as_ptr(), key.as_ptr(), n, &mut value)
                != sys::FcResultMatch
                || value.is_null()
            {
                return None;
            }
            CStr::from_ptr(value.cast())
        };
        let text = text.to_str().ok()?.trim();
        (!text.is_empty()).then(|| text.to_owned())
    }

    pub fn string(self, key: &CStr) -> Option<String> {
        self.string_at(key, 0)
    }

    pub fn strings(self, key: &CStr) -> Vec<String> {
        (0..).map_while(|n| self.string_at(key, n)).collect()
    }

    pub fn int(self, key: &CStr) -> Option<i32> {
        let mut value = 0;
        // SAFETY: the pattern is live; `value` is a live local.
        let found = unsafe { sys::FcPatternGetInteger(self.ptr.as_ptr(), key.as_ptr(), 0, &mut value) };
        (found == sys::FcResultMatch).then_some(value)
    }

    pub fn bool(self, key: &CStr) -> Option<bool> {
        let mut value = 0;
        // SAFETY: the pattern is live; `value` is a live local.
        let found = unsafe { sys::FcPatternGetBool(self.ptr.as_ptr(), key.as_ptr(), 0, &mut value) };
        (found == sys::FcResultMatch).then_some(value != 0)
    }

    /// Whether the pattern's charset has CH; false without a charset.
    pub fn has_char(self, ch: char) -> bool {
        let mut charset: *mut sys::FcCharSet = null_mut();
        // SAFETY: the pattern is live; the charset it yields is the
        // pattern's own and is only read while the pattern lives.
        unsafe {
            sys::FcPatternGetCharSet(self.ptr.as_ptr(), fontconfig::FC_CHARSET.as_ptr(), 0, &mut charset)
                == sys::FcResultMatch
                && !charset.is_null()
                && sys::FcCharSetHasChar(charset, u32::from(ch)) != 0
        }
    }
}

/// Add FILE's faces to the application font set.
pub(crate) fn add_app_font_file(file: &CStr) -> bool {
    // SAFETY: a null config names the current one; FILE is NUL-terminated.
    unsafe { sys::FcConfigAppFontAddFile(null_mut(), file.as_ptr().cast()) != 0 }
}

/// The configuration's files, its configuration directories and its font
/// directories, as it names them.
pub(crate) fn configuration_inputs() -> Vec<String> {
    let mut inputs = vec![];
    // SAFETY: a null config names the current one. Each list is walked to
    // its end and then released; each string is copied before the next.
    unsafe {
        for list in [
            sys::FcConfigGetConfigFiles(null_mut()),
            sys::FcConfigGetConfigDirs(null_mut()),
            sys::FcConfigGetFontDirs(null_mut()),
        ] {
            if list.is_null() {
                continue;
            }
            loop {
                let next = sys::FcStrListNext(list);
                if next.is_null() {
                    break;
                }
                inputs.push(CStr::from_ptr(next.cast()).to_string_lossy().into_owned());
            }
            sys::FcStrListDone(list);
        }
    }
    inputs
}
