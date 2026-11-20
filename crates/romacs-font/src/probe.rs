//! Pixel-size font metric probe, porting GNU's cairo freetype driver.
//!
//! `font-info` on a font ENTITY opens the font at the entity's size — for a
//! scalable entity, `font_open_entity` (GNU src/font.c) bumps pixel size 0
//! upward until `average_width > 0 && height > 0`, which lands at 1px for
//! ordinary scalable fonts. The metrics GNU reports come from
//! `ftcrfont_open` (src/ftcrfont.c): a cairo scaled font's per-glyph
//! `x_advance` rounded via `lround` (cairo hint-metrics rounds advances to
//! integer pixels over FreeType's hinted loads) and `lround`ed cairo font
//! extents for ascent/descent.
//!
//! This module reproduces that with FreeType directly:
//! - per-glyph width = hinted `FT_Load_Char(FT_LOAD_DEFAULT)` advance,
//!   rounded from 26.6 to integer pixels (cairo hint-metrics equivalent);
//! - ascent/descent = rounded FT size metrics (what cairo's font extents
//!   report for an FT backend with hint-metrics on).
//!
//! Byte-exactness is enforced by tests against captured GNU output for
//! concrete font files; if a future font/hinting configuration diverges,
//! the tests say so instead of the probe silently guessing.
//!
//! The GSUB/GPOS `otf_capability` reader uses ttf-parser.

use romacs_display_protocol::font::FontVariationCoord;

/// Metrics of one font file probed at an exact pixel size, shaped like the
/// `font-info` elements GNU fills in `ftcrfont_open` + `font_open_entity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontPxMetrics {
    /// The pixel size the font actually opened at (`font_open_entity` may
    /// bump the requested size upward until the font is "manageable").
    pub pixel_size: u32,
    pub height: i32,
    pub ascent: i32,
    pub descent: i32,
    pub max_width: i32,
    pub space_width: i32,
    pub average_width: i32,
    /// ftcrfont_open's `underline_position' and `underline_thickness'.
    pub underline: romacs_display_protocol::font::FontUnderline,
}

/// Hinted horizontal advances measured in device pixels for ASCII printable
/// glyphs of one exact opened font instance.
///
/// GNU's Cairo display backend stores these integer advances on the opened
/// font and xdisp uses them directly for ordinary characters.  Keeping the
/// coordinate domain in the type prevents a HiDPI device width from being
/// mistaken for logical layout geometry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceAsciiAdvances {
    device_pixel_size: u32,
    widths: [i32; 128],
}

impl DeviceAsciiAdvances {
    #[must_use]
    pub const fn device_pixel_size(&self) -> u32 {
        self.device_pixel_size
    }

    /// Convert one printable ASCII advance to the logical-pixel domain used
    /// by the display matrix.
    #[must_use]
    pub fn logical_advance(
        &self,
        ch: char,
        device_scale: romacs_display_protocol::geometry::DeviceScale,
    ) -> Option<f32> {
        let index = usize::try_from(u32::from(ch)).ok()?;
        let width = *self.widths.get(index)?;
        (width > 0).then(|| width as f32 / device_scale.get())
    }
}

use freetype::Library;
use freetype::face::LoadFlag;
use freetype::freetype_sys as ft;

#[expect(unsafe_code, reason = "FreeType's variation calls, which freetype-rs does not wrap")]
mod variations;

/// Probe `file`[`face_index`] like GNU `font_open_entity`: try
/// `pixel_size`, bumping upward (at most 15 times) until average
/// width and height are positive.
fn probe_px_metrics(
    file: &str,
    face_index: u32,
    pixel_size: u32,
    wght: Option<f32>,
) -> Option<FontPxMetrics> {
    let library = Library::init().ok()?;
    let mut face = library.new_face(file, face_index as isize).ok()?;
    // Variable fonts: GNU probes the MATCHED named instance (the
    // cairo scaled font carries the fontconfig pattern's wght), not
    // the default instance — thin/bold advances differ from regular
    // at real sizes.
    if let Some(wght) = wght {
        apply_wght_axis(&library, &mut face, wght);
    }
    let start = pixel_size.max(1);
    for psize in start..=start + 15 {
        if let Some(metrics) = probe_at_exact_px(&face, psize)
            && metrics.average_width > 0
            && metrics.height > 0
        {
            return Some(metrics);
        }
    }
    None
}

/// Open one exact Fontconfig/FreeType face at its device-pixel size
/// and capture the per-glyph advances GNU's Cairo backend gives xdisp
/// for ASCII printables.
pub fn probe_device_ascii_advances(
    file: &str,
    face_selector: u32,
    device_pixel_size: u32,
    wght: Option<f32>,
) -> Option<DeviceAsciiAdvances> {
    let library = Library::init().ok()?;
    let mut face = library.new_face(file, face_selector as isize).ok()?;
    if let Some(wght) = wght {
        apply_wght_axis(&library, &mut face, wght);
    }
    let device_pixel_size = device_pixel_size.max(1);
    face.set_pixel_sizes(device_pixel_size, device_pixel_size).ok()?;

    let mut widths = [0; 128];
    let load_flags = LoadFlag::TARGET_LIGHT;
    for byte in 32u8..127 {
        if face.load_char(usize::from(byte), load_flags).is_err()
            && face.load_glyph(0, load_flags).is_err()
        {
            continue;
        }
        let advance = face.glyph().advance().x;
        widths[usize::from(byte)] = ((advance + 32) >> 6) as i32;
    }
    Some(DeviceAsciiAdvances {
        device_pixel_size,
        widths,
    })
}

/// The `wght` design-axis values of a variable font's fvar NAMED
/// INSTANCES (OT axis units == CSS weight), sorted ascending and
/// deduped. Empty for a non-variable font or one whose instances omit
/// a `wght` axis.
///
/// GNU/fontconfig expose named instances as separate font entities,
/// so weight resolution snaps a request to the nearest instance
/// rather than synthesizing an arbitrary axis value
/// (font_match::resolve_requested_weight).
pub fn named_instance_wght_values(file: &str, face_index: u32) -> Vec<u16> {
    let Ok(library) = Library::init() else {
        return Vec::new();
    };
    let Ok(mut face) = library.new_face(file, face_index as isize) else {
        return Vec::new();
    };
    let Some(var) = variations::of(&library, &mut face) else {
        return Vec::new();
    };
    let Some(wght) = var.axes.iter().position(|axis| axis.tag == WGHT) else {
        return Vec::new();
    };
    let mut weights: Vec<u16> = var
        .named_styles
        .iter()
        .filter_map(|coords| coords.get(wght))
        // FT_Fixed 16.16 → integer CSS weight.
        .map(|&value| (value as f64 / 65536.0).round())
        .filter(|value| (1.0..=1000.0).contains(value))
        .map(|value| value as u16)
        .collect();
    weights.sort_unstable();
    weights.dedup();
    weights
}

/// Decode the complete axis tuple selected by a Fontconfig/FreeType
/// encoded face selector. FreeType numbers named instances from one in
/// bits 16-30; the raw collection face remains in bits 0-15.
pub fn named_instance_variation_coords(
    file: &str,
    face_selector: u32,
) -> Vec<FontVariationCoord> {
    let named_instance = ((face_selector >> 16) & 0x7fff) as usize;
    if named_instance == 0 {
        return Vec::new();
    }
    let Ok(library) = Library::init() else {
        return Vec::new();
    };
    let Ok(mut face) = library.new_face(file, (face_selector & 0xffff) as isize) else {
        return Vec::new();
    };
    let Some(var) = variations::of(&library, &mut face) else {
        return Vec::new();
    };
    let Some(coords) = var.named_styles.get(named_instance - 1) else {
        return Vec::new();
    };
    var.axes
        .iter()
        .zip(coords)
        .filter(|&(axis, value)| *value != axis.default)
        .filter_map(|(axis, value)| FontVariationCoord::try_new(axis.tag as u32, *value as f32 / 65536.0))
        .collect()
}

/// PostScript name of the exact FreeType face or named instance.
///
/// `face_selector` intentionally keeps Fontconfig's high named-instance
/// bits; opening that selector is what makes FreeType report names such
/// as `NotoSans-Bold` instead of the variable file's default name.
pub fn postscript_name(file: &str, face_selector: u32) -> Option<String> {
    let library = Library::init().ok()?;
    library
        .new_face(file, face_selector as isize)
        .ok()?
        .postscript_name()
}

/// Set the `wght` design axis (OT axis units == CSS weight), leaving
/// all other axes at their defaults. No-op for non-variable fonts.
fn apply_wght_axis(library: &Library, face: &mut freetype::Face, wght: f32) {
    let Some(var) = variations::of(library, face) else {
        return;
    };
    let coords: Vec<ft::FT_Fixed> = var
        .axes
        .iter()
        .map(|axis| if axis.tag == WGHT { (f64::from(wght) * 65536.0) as ft::FT_Fixed } else { axis.default })
        .collect();
    variations::set_design_coordinates(face, &coords);
}

/// The `wght` axis tag.
const WGHT: ft::FT_ULong = u32::from_be_bytes(*b"wght") as ft::FT_ULong;

fn probe_at_exact_px(face: &freetype::Face, pixel_size: u32) -> Option<FontPxMetrics> {
    face.set_pixel_sizes(pixel_size, pixel_size).ok()?;

    // ASCII printables loop (ftcrfont.c ftcrfont_open): per-glyph
    // width is the hinted advance rounded to integer pixels (cairo
    // lround of x_advance with hint-metrics on). Glyphs a char is
    // missing fall back to glyph id 0, mirroring the cairo
    // text_to_glyphs failure path.
    let mut max_width = 0i32;
    let mut space_width = 0i32;
    let mut average_width = 0i64;
    let mut n = 0i64;
    // Cairo under fontconfig's default hintstyle=hintslight loads
    // with FT_LOAD_TARGET_LIGHT: vertical-only hinting, so horizontal
    // advances stay fractional and hint-metrics rounding (lround)
    // decides the pixel width. Full bytecode hinting (LOAD_DEFAULT)
    // would widen some glyphs (Noto '@'-class) to 2px where GNU
    // reports 1.
    let load_flags = LoadFlag::TARGET_LIGHT;
    for c in 32u8..127 {
        if face.load_char(c as usize, load_flags).is_err()
            && face.load_glyph(0, load_flags).is_err()
        {
            continue;
        }
        // 26.6 fixed-point hinted advance → integer pixels, round
        // half up (lround semantics for the non-negative advances
        // fonts produce).
        let advance = face.glyph().advance().x;
        let this_width = ((advance + 32) >> 6) as i32;
        if this_width > 0 {
            if this_width > max_width {
                max_width = this_width;
            }
            if c == 32 {
                space_width = this_width;
            }
            average_width += this_width as i64;
            n += 1;
        }
    }
    if n > 0 {
        average_width /= n;
    }

    // Font extents (cairo_scaled_font_extents → lround): for an FT
    // backend with hint-metrics on these are the grid-fitted size
    // metrics.
    let size_metrics = face.size_metrics()?;
    let ascent = ((size_metrics.ascender + 32) >> 6) as i32;
    let descent = ((-size_metrics.descender + 32) >> 6) as i32;

    // ftcrfont_open: a scalable face's underline metrics, scaled to
    // the size, a thick line raised by half its thickness; none for
    // a fixed-size one.
    let raw = face.raw();
    let scalable = raw.face_flags & freetype::freetype_sys::FT_FACE_FLAG_SCALABLE != 0;
    let underline = if scalable && raw.units_per_EM > 0 {
        let scale = |units: i64| (units * i64::from(pixel_size) / i64::from(raw.units_per_EM)) as i32;
        romacs_display_protocol::font::FontUnderline::scaled(
            scale(-i64::from(raw.underline_position)), scale(i64::from(raw.underline_thickness)))
    } else {
        romacs_display_protocol::font::FontUnderline::default()
    };
    Some(FontPxMetrics {
        pixel_size,
        height: ascent + descent,
        ascent,
        descent,
        max_width,
        space_width,
        average_width: average_width as i32,
        underline,
    })
}

// The probe tests exercise the FreeType functions against real fonts.
#[cfg(test)]
#[path = "probe/tests/probe_test.rs"]
mod tests;

/// One langsys of an OpenType script: `None` tag = the default langsys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtfLangSys {
    pub tag: Option<String>,
    pub features: Vec<String>,
}

/// One script of a GSUB/GPOS table with its langsyses (default first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtfScript {
    pub tag: String,
    pub lang_syses: Vec<OtfLangSys>,
}

/// GSUB/GPOS capability of a font file, shaped like GNU's
/// `hbfont_otf_capability` (src/hbfont.c): per table, scripts in table
/// order; per script the default langsys first (tag `None`) then named
/// langsyses in table order, langsyses without features skipped; features
/// are the langsys's feature indices mapped to tags in index order. Tags
/// keep their trailing spaces ("MKD ").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OtfCapability {
    pub gsub: Vec<OtfScript>,
    pub gpos: Vec<OtfScript>,
}

/// Observations of font files by path and arguments, each kept only while
/// its file's identity is unchanged. Every observation has one process-wide
/// cache (`probe_font_px_metrics`, `otf_capability`), so the display host,
/// the layout engines and their resolvers observe a font once between them
/// (twice at most when two threads miss the same entry at once).
pub struct FileCache<A, V> {
    entries: std::collections::HashMap<(String, A), (FontFileIdentity, V)>,
    cap: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FontFileIdentity {
    len: u64,
    modified: std::time::SystemTime,
    device: u64,
    inode: u64,
    changed: (i64, i64),
}

impl FontFileIdentity {
    fn for_file(file: &str) -> Option<Self> {
        // Inode and device distinguish replacement, and ctime catches
        // in-place changes even if mtime is restored.
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(file).ok()?;
        if !metadata.is_file() {
            return None;
        }
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok()?,
            device: metadata.dev(),
            inode: metadata.ino(),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

impl<A: Clone + Eq + std::hash::Hash, V: Clone> FileCache<A, V> {
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self { entries: std::collections::HashMap::new(), cap }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The observation of FILE at ARGS while FILE is still IDENTITY. A
    /// missing or changed file retires the earlier observation.
    fn hit(&mut self, file: &str, args: &A, identity: Option<&FontFileIdentity>) -> Option<V> {
        let key = (file.to_owned(), args.clone());
        match (identity, self.entries.get(&key)) {
            (Some(identity), Some((cached, value))) if identity == cached => Some(value.clone()),
            _ => {
                self.entries.remove(&key);
                None
            }
        }
    }

    fn keep(&mut self, file: &str, args: A, identity: FontFileIdentity, value: V) {
        if self.entries.len() >= self.cap {
            self.clear();
        }
        self.entries.insert((file.to_owned(), args), (identity, value));
    }
}

/// FILE's observation at ARGS from CACHE, or OBSERVE's, made without holding
/// CACHE's lock. OBSERVE returns `None` for a failure worth retrying (an
/// unreadable file), which is never cached; an observation is cached only
/// if FILE's identity is the same after it as before. This is a metadata
/// observation, not an atomic defense against adversarial replacement.
pub fn observe_file<A: Clone + Eq + std::hash::Hash, V: Clone>(
    cache: &std::sync::Mutex<FileCache<A, V>>,
    file: &str,
    args: A,
    observe: impl FnOnce() -> Option<V>,
) -> Option<V> {
    let lock = || cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let identity = FontFileIdentity::for_file(file);
    if let Some(value) = lock().hit(file, &args, identity.as_ref()) {
        return Some(value);
    }
    let value = observe()?;
    if let Some(identity) = identity
        && Some(&identity) == FontFileIdentity::for_file(file).as_ref()
    {
        lock().keep(file, args, identity, value.clone());
    }
    Some(value)
}

type PxMetricsCache = std::sync::Mutex<FileCache<(u32, u32, Option<u32>), FontPxMetrics>>;
type OtfCapabilityCache = std::sync::Mutex<FileCache<u32, Option<OtfCapability>>>;

fn px_metrics_cache() -> &'static PxMetricsCache {
    static CACHE: std::sync::OnceLock<PxMetricsCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(FileCache::new(256)))
}

fn otf_capability_cache() -> &'static OtfCapabilityCache {
    static CACHE: std::sync::OnceLock<OtfCapabilityCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(FileCache::new(64)))
}

/// Probe `file`[`face_index`] like GNU `font_open_entity` (see
/// `probe_px_metrics`), once per unchanged file and arguments (see
/// `FileCache`).
pub fn probe_font_px_metrics(
    file: &str,
    face_index: u32,
    pixel_size: u32,
    wght: Option<f32>,
) -> Option<FontPxMetrics> {
    observe_file(px_metrics_cache(), file, (face_index, pixel_size, wght.map(f32::to_bits)), || {
        probe_px_metrics(file, face_index, pixel_size, wght)
    })
}

/// The GSUB/GPOS capability of `file`[`face_index`], read once per
/// unchanged file. A malformed but readable file caches `None`; a read
/// error stays retryable.
#[must_use]
pub fn otf_capability(file: &str, face_index: u32) -> Option<OtfCapability> {
    otf_capability_in(otf_capability_cache(), file, face_index, |file| read_capability(file, face_index))
}

/// The largest layout table `read_capability` reads: a font's run to a few
/// megabytes, and a larger one (a sparse file's, say) is not allocated.
const LAYOUT_TABLE_LIMIT: u64 = 64 << 20;

/// The capability of `file`'s face `face_index`, read table by table rather than
/// whole: a font collection runs to tens of megabytes, of which the table
/// directory and the tables a face needs are a small part. Ok(None) for a
/// face ttf-parser refuses, as `otf_capability_from_bytes` would.
fn read_capability(file: &str, face_index: u32) -> std::io::Result<Option<OtfCapability>> {
    use std::os::unix::fs::FileExt;
    let file = std::fs::File::open(file)?;
    let len = file.metadata()?.len();
    // The first NEED of FILE's LENGTH bytes at OFFSET; None where the
    // LENGTH bytes run past its end.
    let read = |offset: u64, length: u64, need: u64| -> std::io::Result<Option<Vec<u8>>> {
        if offset.checked_add(length).is_none_or(|end| end > len) {
            return Ok(None);
        }
        let mut data = vec![0; usize::try_from(length.min(need)).map_err(std::io::Error::other)?];
        file.read_exact_at(&mut data, offset)?;
        Ok(Some(data))
    };
    let be32 = |bytes: &[u8], at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    // Where the face's directory starts, as `RawFace::parse` finds it: a
    // collection's face at its offset, which follows the whole offset array.
    let Some(header) = read(0, 12, 12)? else { return Ok(None) };
    let start = if &header[..4] == b"ttcf" {
        let faces = be32(&header, 8);
        let first = 12 + 4 * u64::from(faces);
        if face_index >= faces || first > len {
            return Ok(None);
        }
        let Some(offset) = read(12 + 4 * u64::from(face_index), 4, 4)? else { return Ok(None) };
        let offset = u64::from(be32(&offset, 0));
        if offset < first {
            return Ok(None);
        }
        offset
    } else if face_index == 0 {
        0
    } else {
        return Ok(None);
    };
    // The face's directory alone; its records hold offsets in the file.
    let Some(face_header) = read(start, 12, 12)? else { return Ok(None) };
    if &face_header[..4] == b"ttcf" {
        return Ok(None);
    }
    let records = u64::from(u16::from_be_bytes([face_header[4], face_header[5]]));
    let directory_length = 12 + 16 * records;
    let Some(directory) = read(start, directory_length, directory_length)? else { return Ok(None) };
    let Ok(raw) = ttf_parser::RawFace::parse(&directory, 0) else { return Ok(None) };
    // The first NEED bytes of each table as `Face::parse` collects it from
    // the whole file: the directory's last record of the tag, absent when it
    // runs past the end. head, hhea and maxp are parsed from their first 54,
    // 36 and 6 bytes; a layout table is read whole, and one larger than any
    // font's (a sparse file's, say) is absent rather than allocated.
    let record = |tag: &[u8; 4]| {
        let record = raw.table_records.into_iter().filter(|record| &record.tag.to_bytes() == tag).last()?;
        Some((u64::from(record.offset), u64::from(record.length)))
    };
    let prefix = |tag, need| record(tag).map_or(Ok(None), |(offset, length)| read(offset, length, need));
    let whole = |tag| match record(tag) {
        Some((offset, length)) if length <= LAYOUT_TABLE_LIMIT => read(offset, length, length),
        _ => Ok(None),
    };
    let (Some(head), Some(hhea), Some(maxp)) = (prefix(b"head", 54)?, prefix(b"hhea", 36)?, prefix(b"maxp", 6)?) else {
        return Ok(None);
    };
    let (gsub, gpos) = (whole(b"GSUB")?, whole(b"GPOS")?);
    let tables = ttf_parser::RawFaceTables {
        head: &head,
        hhea: &hhea,
        maxp: &maxp,
        gsub: gsub.as_deref(),
        gpos: gpos.as_deref(),
        ..Default::default()
    };
    Ok(ttf_parser::Face::from_raw_tables(tables).ok().map(|face| capability_of(&face)))
}

fn otf_capability_in(
    cache: &OtfCapabilityCache,
    file: &str,
    face_index: u32,
    read: impl FnOnce(&str) -> std::io::Result<Option<OtfCapability>>,
) -> Option<OtfCapability> {
    observe_file(cache, file, face_index, || read(file).ok()).flatten()
}

#[cfg(test)]
#[path = "probe/tests/otf_cache_test.rs"]
mod otf_cache_tests;

#[must_use]
pub fn otf_capability_from_bytes(data: &[u8], face_index: u32) -> Option<OtfCapability> {
    ttf_parser::Face::parse(data, face_index).ok().map(|face| capability_of(&face))
}

fn capability_of(face: &ttf_parser::Face) -> OtfCapability {
    let tables = face.tables();
    OtfCapability {
        gsub: tables.gsub.map(otf_table_scripts).unwrap_or_default(),
        gpos: tables.gpos.map(otf_table_scripts).unwrap_or_default(),
    }
}

fn otf_table_scripts(table: ttf_parser::opentype_layout::LayoutTable) -> Vec<OtfScript> {
    // GNU only reports a table with at least one feature tag.
    if table.features.is_empty() {
        return Vec::new();
    }
    let feature_tag = |index: u16| -> Option<String> {
        table
            .features
            .get(index)
            .map(|feature| tag_string(feature.tag))
    };
    let mut scripts = Vec::new();
    for script in table.scripts {
        let mut lang_syses = Vec::new();
        let mut push_langsys =
            |tag: Option<String>, lang: ttf_parser::opentype_layout::LanguageSystem| {
                let features: Vec<String> = lang
                    .feature_indices
                    .into_iter()
                    .filter_map(feature_tag)
                    .collect();
                if !features.is_empty() {
                    lang_syses.push(OtfLangSys { tag, features });
                }
            };
        if let Some(default) = script.default_language {
            push_langsys(None, default);
        }
        for lang in script.languages {
            push_langsys(Some(tag_string(lang.tag)), lang);
        }
        scripts.push(OtfScript {
            tag: tag_string(script.tag),
            lang_syses,
        });
    }
    scripts
}

fn tag_string(tag: ttf_parser::Tag) -> String {
    String::from_utf8_lossy(&tag.to_bytes()).into_owned()
}
