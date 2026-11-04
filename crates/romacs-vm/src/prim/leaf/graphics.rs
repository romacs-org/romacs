//! GNU image.c for Lisp over the image cache (graphics.rs): image specs as
//! `parse_image_spec' and each type's `*_image_p' validate them, their
//! loading through the display host, and the primitives' bodies
//! (`prims/graphics.rs').
use super::*;
use crate::graphics::{Dependency, ImageEntry, ImageFacts, ImageKey, ImageQuery, ImageSource, KeyValue};
use romacs_display_protocol::{
    AxisSize, ImageColorContext, ImageEmbeddedMetadata, ImageFrameIndex, ImageHeuristicMask, ImageMaskPolicy,
    ImageNativeExtent, ImageRealization, ImageRotation, ImageSizeLimit, ImageSizeSpec,
};

/// GNU `enum image_value_type'.
#[derive(Clone, Copy)]
enum Check {
    Any,
    String,
    StringOrNil,
    Symbol,
    PositiveInteger,
    NonNegativeIntegerOrPair,
    NonNegativeInteger,
    Ascent,
    Integer,
}

/// The keywords common to every raster loader (png_format and its kin).
const COMMON: [(&str, Check, bool); 8] = [
    (":type", Check::Symbol, true), (":file", Check::String, false),
    (":ascent", Check::Ascent, false), (":margin", Check::NonNegativeIntegerOrPair, false),
    (":relief", Check::Integer, false), (":heuristic-mask", Check::Any, false), (":mask", Check::Any, false),
    (":background", Check::StringOrNil, false),
];

/// TYPE's keywords beyond `COMMON' (image.c's `*_format' tables), and
/// whether its `:data' is checked as a string there.
fn keywords(kind: &str) -> Option<&'static [(&'static str, Check, bool)]> {
    Some(match kind {
        "png" => &[(":conversion", Check::Any, false)],
        "jpeg" => &[(":conversions", Check::Any, false)],
        "tiff" => &[(":conversions", Check::Any, false), (":index", Check::NonNegativeInteger, false)],
        "gif" | "webp" => &[(":conversion", Check::Any, false), (":index", Check::NonNegativeInteger, false)],
        "pbm" => &[(":conversion", Check::Any, false), (":foreground", Check::StringOrNil, false)],
        "xpm" => &[(":conversion", Check::Any, false), (":color-symbols", Check::Any, false)],
        "svg" => &[(":conversion", Check::Any, false), (":base-uri", Check::String, false), (":css", Check::String, false),
                   (":foreground", Check::StringOrNil, false)],
        "xbm" => &[(":conversion", Check::Any, false), (":width", Check::PositiveInteger, false),
                   (":height", Check::PositiveInteger, false), (":stride", Check::PositiveInteger, false),
                   (":foreground", Check::StringOrNil, false), (":data-width", Check::PositiveInteger, false),
                   (":data-height", Check::PositiveInteger, false)],
        _ => return None,
    })
}

/// The types the display host decodes: `image-types'.
pub(crate) const IMAGE_TYPES: [&str; 9] = ["svg", "webp", "png", "gif", "tiff", "jpeg", "xpm", "xbm", "pbm"];

/// A valid spec's keywords and values, by name.
pub(crate) struct Spec {
    pub kind: String,
    values: Vec<(String, Value)>,
}

impl Spec {
    pub(crate) fn get(&self, key: &str) -> Option<Value> {
        self.values.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
    }
}

/// What a spec's look depends on besides its plist: the frame's and its
/// default face's (GNU's `lookup_image' arguments).
pub(crate) struct ImageEnv {
    /// The frame's stable id: a frame object moves in a collection.
    pub frame: u64,
    pub foreground: u32,
    pub background: u32,
    pub font_size: u32,
    pub font_height: u32,
    pub font_width: u32,
    pub font_family: String,
    pub column: u32,
    pub line: u32,
    pub frame_pixels: (u32, u32),
}

impl Leaf<'_> {
    /// GNU `valid_image_p': SPEC is `(image . PLIST)' whose `:type' names a
    /// type this display decodes and whose plist that type's
    /// `parse_image_spec' and `*_image_p' accept.
    pub(crate) fn image_spec(&mut self, spec: Value) -> std::result::Result<Option<Spec>, Fault> {
        let Some((head, plist)) = self.pair(spec) else { return Ok(None) };
        if head != self.intern("image") { return Ok(None) }
        // The type: the first `:type' in the plist.
        let mut kind = None;
        let mut tail = plist;
        for _ in 0..self.list_limit() {
            let Some((key, rest)) = self.pair(tail) else { break };
            let Some((value, rest)) = self.pair(rest) else { break };
            if key == self.intern(":type") {
                if self.kind(value) == Kind::Symbol { kind = Some(self.symbol_name(value)?); }
                break;
            }
            tail = rest;
        }
        let Some(kind) = kind else { return Ok(None) };
        let Some(own) = keywords(&kind) else { return Ok(None) };
        // `:data' is a string but for XBM, whose raw forms its own check reads.
        let data = [(":data", if kind == "xbm" { Check::Any } else { Check::String }, false)];
        let table: Vec<(&str, Check, bool)> = COMMON.iter().chain(&data).chain(own).copied().collect();
        // parse_image_spec: each known keyword once, its value of its type.
        let mut values: Vec<(String, Value)> = vec![];
        let mut tail = plist;
        for _ in 0..self.list_limit() {
            if tail == Value::NIL { break }
            let Some((key, rest)) = self.pair(tail) else { return Ok(None) };
            if self.kind(key) != Kind::Symbol { return Ok(None) }
            let Some((value, rest)) = self.pair(rest) else { return Ok(None) };
            let name = self.symbol_name(key)?;
            if let Some(&(_, check, _)) = table.iter().find(|(k, ..)| *k == name) {
                if values.iter().any(|(k, _)| *k == name) { return Ok(None) }
                if !self.image_value_ok(check, value) { return Ok(None) }
                if name == ":type" && self.symbol_name(value)? != kind { return Ok(None) }
            }
            if !values.iter().any(|(k, _)| *k == name) { values.push((name, value)); }
            tail = rest;
        }
        if table.iter().any(|(k, _, mandatory)| *mandatory && !values.iter().any(|(n, _)| n == k)) { return Ok(None) }
        let spec = Spec { kind, values };
        Ok(self.image_source_ok(&spec)?.then_some(spec))
    }

    fn image_value_ok(&self, check: Check, value: Value) -> bool {
        let int = |v: Value| v.integer_value();
        match check {
            Check::Any => true,
            Check::String => self.vm.str_view(value).is_some(),
            Check::StringOrNil => value == Value::NIL || self.vm.str_view(value).is_some(),
            Check::Symbol => self.kind(value) == Kind::Symbol,
            Check::PositiveInteger => int(value).is_some_and(|n| (1..=i64::from(i32::MAX)).contains(&n)),
            Check::NonNegativeInteger => int(value).is_some_and(|n| n >= 0),
            Check::NonNegativeIntegerOrPair => int(value).is_some_and(|n| (0..=i64::from(i32::MAX)).contains(&n))
                || self.pair(value).is_some_and(|(a, d)| [a, d].iter().all(|v| int(*v).is_some_and(|n| (0..=i64::from(i32::MAX)).contains(&n)))),
            Check::Ascent => value == self.vm_symbol("center") || int(value).is_some_and(|n| (0..=100).contains(&n)),
            Check::Integer => int(value).is_some_and(|n| i32::try_from(n).is_ok()),
        }
    }

    fn vm_symbol(&self, name: &str) -> Value {
        self.vm.names.get(name).map_or(Value::UNBOUND, |&slot| Value::reference(slot + 1, 7))
    }

    /// Each type's `*_image_p' beyond its keywords: exactly one of `:file'
    /// and `:data', XBM's raw `:data' forms of their dimensions, XPM's
    /// `:color-symbols' an alist of strings.
    fn image_source_ok(&mut self, spec: &Spec) -> std::result::Result<bool, Fault> {
        let (file, data) = (spec.get(":file"), spec.get(":data"));
        if spec.kind != "xbm" {
            if file.is_some() == data.is_some() { return Ok(false) }
            if spec.kind == "xpm" && let Some(symbols) = spec.get(":color-symbols") {
                let mut tail = symbols;
                for _ in 0..self.list_limit() {
                    if tail == Value::NIL { break }
                    let Some((entry, rest)) = self.pair(tail) else { return Ok(false) };
                    let Some((name, color)) = self.pair(entry) else { return Ok(false) };
                    if self.vm.str_view(name).is_none() || self.vm.str_view(color).is_none() { return Ok(false) }
                    tail = rest;
                }
            }
            return Ok(true);
        }
        // xbm_image_p.
        if file.is_some() { return Ok(data.is_none()) }
        let Some(data) = data else { return Ok(false) };
        if self.vm.str_view(data).is_some_and(|s| xbm_file_p(s.bytes())) { return Ok(true) }
        let dim = |key: &str| spec.get(key).and_then(|v| v.integer_value());
        let (Some(width), Some(height)) = (dim(":data-width"), dim(":data-height")) else { return Ok(false) };
        let stride = dim(":stride").unwrap_or(width);
        Ok(match self.kind(data) {
            Kind::Vector => {
                let rows = self.vector_values(data)?;
                rows.len() as i64 >= height && rows[..height as usize].iter().all(|&row| {
                    if let Some(s) = self.vm.str_view(row) { (s.chars as i64) >= stride / 8 }
                    else { self.bool_vector_size(row).is_some_and(|n| n >= width) }
                })
            }
            _ if self.vm.str_view(data).is_some() => self.vm.str_view(data).unwrap().chars as i64 >= stride / 8 * height,
            _ => match self.bool_vector_size(data) {
                Some(n) => !(height > 1 && stride != (width + 7) / 8 * 8) && n / height >= stride,
                None => false,
            },
        })
    }

    fn bool_vector_size(&self, v: Value) -> Option<i64> {
        match self.vm.object_if(v) { Some(crate::data::Object::BoolVector(b)) => Some(b.len as i64), _ => None }
    }

    /// The environment GNU's `lookup_image' sees for FRAME (nil, the
    /// selected frame): a window-system frame, else GNU's error.
    pub(crate) fn image_env(&mut self, frame: Value) -> std::result::Result<ImageEnv, Fault> {
        if !self.vm.window_system_open() { return Err(Fault::Error("Window system frame should be used".into())) }
        let frame = self.face_frame(frame, true)?;
        let default = self.intern("default");
        let face = self.face_lookup(default, frame)?;
        let attribute = |cx: &mut Self, name: &str| -> std::result::Result<Value, Fault> {
            let index = romacs_face::registry::LFACE_ATTRIBUTES.iter().position(|k| *k == name).unwrap() + 1;
            if face == Value::NIL { return Ok(Value::NIL) }
            cx.vector_get(face, Value::integer(index as i64).unwrap())
        };
        let color = |cx: &mut Self, name: &str, fallback: u32| -> std::result::Result<u32, Fault> {
            let value = attribute(cx, name)?;
            Ok(cx.vm.str_view(value).and_then(|s| color_pixel(&String::from_utf8_lossy(s.bytes()))).unwrap_or(fallback))
        };
        let foreground = color(self, ":foreground", 0x000000)?;
        let background = color(self, ":background", 0xffffff)?;
        let family = attribute(self, ":family")?;
        let font_family = self.vm.str_view(family).map(|s| String::from_utf8_lossy(s.bytes()).into_owned()).unwrap_or_default();
        let (column, line) = (self.vm.window_state.column.max(1), self.vm.window_state.line.max(1));
        let frame_pixels = (self.vm.window_state.width, self.vm.window_state.height);
        // GNU's `img->face_font_size', `_height', `_width': the default
        // face's realized font's pixel size, height and average width.
        let font = self.default_font_request().ok()
            .and_then(|request| self.font_host()?.char_font(&request, ' ')).map(|(font, _)| font);
        let (font_size, font_height, font_width) = font.map_or((line, line, column), |f| {
            let width = if f.average_width > 0 { f.average_width } else { f.space_width };
            (f.pixel_size.max(1) as u32, (f.ascent + f.descent).max(1) as u32, width.max(1) as u32)
        });
        let frame = self.vm.frame(frame).map_err(Fault::Error)?.id;
        Ok(ImageEnv { frame, foreground, background, font_size, font_height, font_width, font_family,
                      column, line, frame_pixels })
    }

    /// GNU `image_compute_scale', and whether `image-scaling-factor' `auto'
    /// gave it.
    fn image_scale(&mut self, spec: &Spec, env: &ImageEnv) -> std::result::Result<(f64, bool), Fault> {
        let value = spec.get(":scale").unwrap_or(Value::NIL);
        if value == self.intern("default") {
            let factor = self.intern("image-scaling-factor");
            let factor = self.variable_value(factor)?.unwrap_or(Value::NIL);
            if let Some(n) = self.number_value(factor).filter(|_| factor != self.intern("auto")) { return Ok((n, false)) }
            return Ok((if env.column > 10 { f64::from(env.column) / 10.0 } else { 1.0 }, true));
        }
        Ok((self.number_value(value).filter(|n| *n >= 0.0).unwrap_or(1.0), false))
    }

    /// GNU `image_get_dimension': a natural number of pixels, or `(N . em)',
    /// `(N . ch)', `(N . cw)' of the face's font.
    fn image_dimension(&mut self, spec: &Spec, key: &str, env: &ImageEnv) -> std::result::Result<Option<u32>, Fault> {
        let Some(value) = spec.get(key) else { return Ok(None) };
        if let Some(n) = value.integer_value().filter(|n| *n >= 0) { return Ok(Some(n.min(i64::from(i32::MAX)) as u32)) }
        let Some((n, unit)) = self.pair(value) else { return Ok(None) };
        let Some(n) = self.number_value(n) else { return Ok(None) };
        let base = if unit == self.intern("em") { env.font_size } else if unit == self.intern("ch") { env.font_height }
            else if unit == self.intern("cw") { env.font_width } else { return Ok(None) };
        Ok(Some((f64::from(base) * n).ceil().max(0.0) as u32))
    }

    /// A copy of a Lisp value for a key.
    fn key_value(&self, v: Value, depth: usize) -> KeyValue {
        if v == Value::NIL { return KeyValue::Nil }
        if depth > 32 { return KeyValue::Object(v.0) }
        if let Some(n) = v.integer_value() { return KeyValue::Integer(n) }
        if let Some(s) = self.vm.str_view(v) { return KeyValue::bytes(s.bytes()) }
        match self.vm.object_if(v) {
            Some(crate::data::Object::Float(f)) => return KeyValue::Float(f.to_bits()),
            Some(crate::data::Object::BoolVector(_)) => {
                let bits = self.bool_vector_bits(v);
                let packed: Vec<u8> = bits.chunks(8).map(|c| c.iter().rev().fold(0, |byte, &b| byte << 1 | u8::from(b))).collect();
                return KeyValue::Bits(bits.len(), Box::new(KeyValue::bytes(&packed)));
            }
            Some(crate::data::Object::Vector(items)) =>
                return KeyValue::Vector(items.clone().into_iter().map(|i| self.key_value(i, depth + 1)).collect()),
            _ => {}
        }
        if self.kind(v) == Kind::Symbol { return self.symbol_name(v).map_or(KeyValue::Object(v.0), KeyValue::Symbol) }
        if let Some((a, d)) = self.pair(v) { return KeyValue::Cons(Box::new(self.key_value(a, depth + 1)), Box::new(self.key_value(d, depth + 1))) }
        KeyValue::Object(v.0)
    }

    /// SPEC's cache key under ENV and SCALE (GNU's `filter_image_spec' and
    /// `search_image_cache').
    fn image_key(&self, spec: &Spec, env: &ImageEnv, scale: f64) -> ImageKey {
        let plist = spec.values.iter()
            .filter(|(k, _)| ![":animate-buffer", ":animate-tardiness", ":animate-position"].contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), self.key_value(*v, 0))).collect();
        ImageKey { plist, frame: env.frame, scale: scale.to_bits(), foreground: env.foreground, background: env.background,
                   font_size: env.font_size, font_family: env.font_family.clone() }
    }

    /// GNU `lookup_image': SPEC's cached image on ENV's frame, loaded
    /// through the display host when not cached.  A load that fails is
    /// cached as GNU's placeholder, `:width' x `:height' or 30 x 30.
    /// The loader's message when this lookup failed, for `image_error''s log.
    pub(crate) fn image_lookup(&mut self, spec: &Spec, env: &ImageEnv)
                               -> std::result::Result<(ImageKey, ImageEntry, Option<String>), Fault> {
        let (scale, auto) = self.image_scale(spec, env)?;
        let key = self.image_key(spec, env, scale);
        if let Some(entry) = self.vm.images.get(&key).filter(|e| !e.failed).cloned() { return Ok((key, entry, None)) }
        let mut dependencies = if auto { vec![Dependency::Auto] } else { vec![] };
        let loaded = match self.image_query(spec, env, scale)? {
            Ok((query, file)) => {
                if let Some(file) = file { dependencies.push(Dependency::File(file)); }
                match self.vm.window_system_host() {
                    Some(host) => host.decode_image(query),
                    None => Err("Window system frame should be used".into()),
                }
            }
            Err(message) => Err(message),
        };
        let mut error = None;
        let (facts, failed) = match loaded {
            Ok(facts) => (facts, false),
            Err(message) => {
                error = Some(message);
                let dim = |key: &str| spec.get(key).and_then(|v| v.integer_value()).filter(|n| *n >= 0).map_or(30, |n| n as u32);
                (ImageFacts { id: 0, width: dim(":width"), height: dim(":height"), mask: false, metadata: ImageEmbeddedMetadata::EMPTY }, true)
            }
        };
        let margins = if failed { (0, 0) } else { self.image_margins(spec) };
        let extension = if failed || spec.kind != "gif" { vec![] } else { self.gif_extension(spec)? };
        let entry = ImageEntry { facts, margins, failed, dependencies, extension };
        self.vm.images.insert(key.clone(), entry.clone());
        Ok((key, entry, error))
    }

    /// A GIF's extension blocks before its `:index' frame, last first, as
    /// GNU's gif_load lists them for `image-metadata''s `extension-data':
    /// each block's first sub-block under its label, the rest under 0.
    fn gif_extension(&mut self, spec: &Spec) -> std::result::Result<Vec<(u8, Vec<u8>)>, Fault> {
        let bytes = if let Some(file) = spec.get(":file") {
            let name = String::from_utf8_lossy(self.vm.str_view(file).unwrap().bytes()).into_owned();
            match self.image_find_file(&name)?.and_then(|path| crate::host::read_file(path).ok()) { Some(b) => b, None => return Ok(vec![]) }
        } else {
            spec.get(":data").and_then(|d| self.vm.str_view(d)).map_or(vec![], |s| s.bytes().to_vec())
        };
        let index = spec.get(":index").and_then(|v| v.integer_value()).unwrap_or(0).max(0) as usize;
        Ok(gif_frame_extensions(&bytes, index))
    }

    /// Whether the selected frame is a window-system frame.
    pub(crate) fn window_system_frame_p(&self) -> bool {
        self.vm.window_system_open()
    }

    /// Drop the cached images REMOVE says to, forget their pixels in the
    /// display host, and have the windows showing them redrawn.
    pub(crate) fn image_uncache(&mut self, remove: impl Fn(&ImageKey, &ImageEntry) -> bool) {
        let released = self.vm.images.retain(|k, e| !remove(k, e));
        if let Some(host) = self.vm.window_system_host() { for id in released { host.release_image(id); } }
        self.vm.display_state.version += 1;
    }

    pub(crate) fn image_cache_bytes(&self) -> u64 {
        self.vm.images.size_in_bytes()
    }

    pub(crate) fn image_cache_id(&self, key: &ImageKey) -> u64 {
        self.vm.images.id(key).unwrap_or(0)
    }

    /// `:margin' plus `:relief''s magnitude on each side (lookup_image).
    fn image_margins(&self, spec: &Spec) -> (u32, u32) {
        let (mut h, mut v) = match spec.get(":margin") {
            Some(m) if m.integer_value().is_some() => { let n = m.integer_value().unwrap() as u32; (n, n) }
            Some(m) => self.pair(m).map_or((0, 0), |(a, d)| (a.integer_value().unwrap_or(0) as u32, d.integer_value().unwrap_or(0) as u32)),
            None => (0, 0),
        };
        if let Some(r) = spec.get(":relief").and_then(|r| r.integer_value()) {
            let r = r.unsigned_abs().min(u64::from(u32::MAX)) as u32;
            h = h.saturating_add(r);
            v = v.saturating_add(r);
        }
        (h, v)
    }

    /// The decode SPEC asks for, and the absolute file it reads; the
    /// loader's message when its source cannot be found.
    fn image_query(&mut self, spec: &Spec, env: &ImageEnv, scale: f64)
                   -> std::result::Result<std::result::Result<(ImageQuery, Option<String>), String>, Fault> {
        let (source, file) = if let Some(file) = spec.get(":file") {
            let name = String::from_utf8_lossy(self.vm.str_view(file).unwrap().bytes()).into_owned();
            match self.image_find_file(&name)? {
                Some(path) => (ImageSource::File(path.clone().into()), Some(path)),
                None => return Ok(Err(format!("Cannot find image file `{name}'"))),
            }
        } else {
            let data = spec.get(":data").unwrap();
            let bytes = match self.vm.str_view(data) {
                Some(s) if spec.kind != "xbm" || xbm_file_p(s.bytes()) => s.bytes().to_vec(),
                _ => self.xbm_bits_file(spec, data)?,
            };
            let base_uri = spec.get(":base-uri").and_then(|v| self.vm.str_view(v)).map(|s| String::from_utf8_lossy(s.bytes()).into_owned());
            (ImageSource::Data { bytes, base_uri }, None)
        };
        let size = ImageSizeSpec::new(
            AxisSize::resolve(self.image_dimension(spec, ":width", env)?, self.image_dimension(spec, ":max-width", env)?),
            AxisSize::resolve(self.image_dimension(spec, ":height", env)?, self.image_dimension(spec, ":max-height", env)?));
        let rotation = spec.get(":rotation").and_then(|v| self.number_value(v)).map_or(ImageRotation::None, ImageRotation::from_degrees);
        let colors = {
            let pick = |cx: &Self, key: &str, fallback: u32| spec.get(key).and_then(|v| cx.vm.str_view(v))
                .and_then(|s| color_pixel(&String::from_utf8_lossy(s.bytes()))).unwrap_or(fallback);
            ImageColorContext::from_pixels(pick(self, ":foreground", env.foreground), pick(self, ":background", env.background))
        };
        let mask = self.image_mask_policy(spec);
        let frame = ImageFrameIndex::new(spec.get(":index").and_then(|v| v.integer_value()).filter(|n| *n >= 0).unwrap_or(0) as u64);
        let limit = {
            let max = self.intern("max-image-size");
            let max = self.variable_value(max)?.unwrap_or(Value::NIL);
            if let Some(n) = max.integer_value() { ImageSizeLimit::from_axis_pixels(n) }
            else if let Some(r) = self.number_value(max) {
                ImageSizeLimit::from_frame_ratio(r, Some(ImageNativeExtent::new(env.frame_pixels.0, env.frame_pixels.1)))
            } else { ImageSizeLimit::UNLIMITED }
        };
        let query = ImageQuery { kind: spec.kind.clone(), source, size, rotation,
                                 realization: ImageRealization::new(scale as f32, 1.0, 1.0), colors, mask, frame, limit };
        Ok(Ok((query, file)))
    }

    /// `:mask' and `:heuristic-mask' as the loaders read them (image.c
    /// postprocess_image): absent keeps the image's own transparency.
    fn image_mask_policy(&mut self, spec: &Spec) -> ImageMaskPolicy {
        let heuristic = self.intern("heuristic");
        let corners = |cx: &mut Self, color: Option<Value>| match color.and_then(|c| cx.vm.str_view(c))
            .and_then(|s| romacs_display_protocol::x11_colors::x11_color_spec_16bit(s.bytes())) {
            Some((r, g, b)) => ImageMaskPolicy::Heuristic(ImageHeuristicMask::Rgb16([r, g, b])),
            None => ImageMaskPolicy::Heuristic(ImageHeuristicMask::FourCorners),
        };
        if let Some(mask) = spec.get(":mask") {
            if mask == heuristic { return corners(self, None) }
            if let Some((head, rest)) = self.pair(mask) && head == heuristic {
                let color = self.pair(rest).map(|(c, _)| c);
                return corners(self, color);
            }
            if mask == Value::NIL { return ImageMaskPolicy::Suppress }
        }
        match spec.get(":heuristic-mask") {
            Some(v) if v == Value::TRUE => corners(self, None),
            Some(v) if v != Value::NIL => corners(self, Some(v)),
            _ => ImageMaskPolicy::Preserve,
        }
    }

    /// GNU `image_find_image_fd': an absolute FILE as it is, a relative one
    /// in `data-directory''s images/, then `x-bitmap-file-path'.
    fn image_find_file(&mut self, name: &str) -> std::result::Result<Option<String>, Fault> {
        let expand = |dir: &str| std::path::Path::new(dir).join(name);
        if std::path::Path::new(name).is_absolute() || name.starts_with('~') {
            let path = if let Some(rest) = name.strip_prefix("~/") {
                crate::host::getenv("HOME").map(|h| std::path::Path::new(&h).join(rest)).unwrap_or_else(|| name.into())
            } else { name.into() };
            return Ok(crate::host::probe(crate::host::Probe::IsFile, &path).then(|| path.to_string_lossy().into_owned()));
        }
        let mut dirs = vec![];
        let data = self.intern("data-directory");
        if let Some(d) = self.variable_value(data)?.and_then(|d| self.vm.str_view(d).map(|s| String::from_utf8_lossy(s.bytes()).into_owned())) {
            dirs.push(std::path::Path::new(&d).join("images").to_string_lossy().into_owned());
        }
        let path = self.intern("x-bitmap-file-path");
        let mut tail = self.variable_value(path)?.unwrap_or(Value::NIL);
        for _ in 0..self.list_limit() {
            let Some((dir, rest)) = self.pair(tail) else { break };
            if let Some(s) = self.vm.str_view(dir) { dirs.push(String::from_utf8_lossy(s.bytes()).into_owned()); }
            tail = rest;
        }
        Ok(dirs.iter().map(|d| expand(d)).find(|p| crate::host::probe(crate::host::Probe::IsFile, p)).map(|p| p.to_string_lossy().into_owned()))
    }

    /// XBM raw `:data' (a packed-bit string, a bool-vector, or a vector of
    /// rows) as an in-memory XBM file the host's XBM loader reads.
    fn xbm_bits_file(&mut self, spec: &Spec, data: Value) -> std::result::Result<Vec<u8>, Fault> {
        let dim = |key: &str| spec.get(key).and_then(|v| v.integer_value()).unwrap_or(0) as usize;
        let (width, height) = (dim(":data-width"), dim(":data-height"));
        let stride = spec.get(":stride").and_then(|v| v.integer_value()).map_or(width, |n| n as usize);
        let row_bytes = width.div_ceil(8);
        // Each row's bits, least significant bit first as XBM stores them.
        let bit_row = |bits: &dyn Fn(usize) -> bool| -> Vec<u8> {
            (0..row_bytes).map(|byte| (0..8).fold(0u8, |acc, bit| {
                let x = byte * 8 + bit;
                if x < width && bits(x) { acc | (1 << bit) } else { acc }
            })).collect()
        };
        let mut rows: Vec<Vec<u8>> = vec![];
        if self.kind(data) == Kind::Vector {
            for row in self.vector_values(data)?.into_iter().take(height) {
                if let Some(s) = self.vm.str_view(row) {
                    let b = s.bytes().to_vec();
                    rows.push(bit_row(&|x| b.get(x / 8).is_some_and(|byte| byte & (1 << (x % 8)) != 0)));
                } else {
                    let b = self.bool_vector_bits(row);
                    rows.push(bit_row(&|x| b.get(x).copied().unwrap_or(false)));
                }
            }
        } else if let Some(s) = self.vm.str_view(data) {
            let b = s.bytes().to_vec();
            let stride_bytes = stride / 8;
            for y in 0..height {
                let start = y * stride_bytes;
                rows.push(bit_row(&|x| b.get(start + x / 8).is_some_and(|byte| byte & (1 << (x % 8)) != 0)));
            }
        } else {
            let b = self.bool_vector_bits(data);
            for y in 0..height { rows.push(bit_row(&|x| b.get(y * stride + x).copied().unwrap_or(false))); }
        }
        let bytes: Vec<String> = rows.concat().iter().map(|b| format!("0x{b:02x}")).collect();
        Ok(format!("#define image_width {width}\n#define image_height {height}\nstatic char image_bits[] = {{\n{}}};\n",
                   bytes.join(", ")).into_bytes())
    }

    fn bool_vector_bits(&self, v: Value) -> Vec<bool> {
        match self.vm.object_if(v) { Some(crate::data::Object::BoolVector(b)) => (0..b.len).map(|i| b.get(i).unwrap_or(false)).collect(), _ => vec![] }
    }
}

/// The extension blocks before frame INDEX of GIF BYTES, last first.
fn gif_frame_extensions(bytes: &[u8], index: usize) -> Vec<(u8, Vec<u8>)> {
    if bytes.len() < 13 || &bytes[..3] != b"GIF" { return vec![] }
    let table = |flags: u8| if flags & 0x80 != 0 { 3 * (1usize << ((flags & 7) + 1)) } else { 0 };
    let mut at = 13 + table(bytes[10]);
    let (mut frame, mut blocks) = (0, vec![]);
    // The sub-blocks from AT: their data, and where they end.
    let sub_blocks = |mut at: usize| -> (Vec<Vec<u8>>, usize) {
        let mut out = vec![];
        while let Some(&n) = bytes.get(at) {
            at += 1;
            if n == 0 { break }
            out.push(bytes.get(at..at + n as usize).map_or(vec![], <[u8]>::to_vec));
            at += n as usize;
        }
        (out, at)
    };
    while let Some(&kind) = bytes.get(at) {
        match kind {
            0x21 => {
                let label = bytes.get(at + 1).copied().unwrap_or(0);
                let (subs, end) = sub_blocks(at + 2);
                for (i, data) in subs.into_iter().enumerate() { blocks.push((if i == 0 { label } else { 0 }, data)); }
                at = end;
            }
            0x2c => {
                if frame == index { blocks.reverse(); return blocks }
                let flags = bytes.get(at + 9).copied().unwrap_or(0);
                let (_, end) = sub_blocks(at + 10 + table(flags) + 1);
                at = end;
                frame += 1;
                blocks.clear();
            }
            _ => break,
        }
    }
    vec![]
}

/// GNU `xbm_file_p': DATA reads as an XBM file (its `#define's).
fn xbm_file_p(data: &[u8]) -> bool {
    let text = String::from_utf8_lossy(data);
    text.contains("#define") && text.contains("_width") && text.contains("_height")
}

/// A colour name or spec as 0xRRGGBB.
fn color_pixel(name: &str) -> Option<u32> {
    let (r, g, b) = romacs_display_protocol::x11_colors::x11_color_value(name)?;
    Some(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
}

#[cfg(test)]
mod tests {
    use super::gif_frame_extensions;

    const ANIM: &[u8] = include_bytes!("../../../../romacs-vm-gui/harness/fixtures/images/anim.gif");

    #[test]
    fn gif_extension_data_is_the_frames_blocks_last_first() {
        // Pinned GNU: (extension-data (249 "\0\n\0\0" 0 "\1\0\0" 255 "NETSCAPE2.0")).
        let first = vec![(249, b"\0\n\0\0".to_vec()), (0, b"\x01\0\0".to_vec()), (255, b"NETSCAPE2.0".to_vec())];
        assert_eq!(gif_frame_extensions(ANIM, 0), first);
        assert_eq!(gif_frame_extensions(ANIM, 1), vec![(249, b"\0\x1e\0\0".to_vec())]);
        assert_eq!(gif_frame_extensions(ANIM, 9), vec![]);
        assert_eq!(gif_frame_extensions(&ANIM[..20], 0), vec![]);
    }
}
