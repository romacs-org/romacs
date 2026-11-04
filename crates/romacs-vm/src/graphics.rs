//! GNU image.c's image cache for Lisp: the facts of every image a frame
//! has loaded, by a canonical key of its spec, and the plain data the VM
//! asks the display host to decode with.
//!
//! The VM owns no pixels.  `DisplayHost::decode_image' decodes a source as
//! GNU's loaders do (romacs-vm-gui backs it with the renderer's
//! `SemanticImageDecoder'), keeps the pixels under an id, and answers with
//! the image's facts: its size, whether it has a mask, its metadata.  The
//! primitives over this cache are in `prim/leaf/graphics.rs'.
//!
//! A key never holds a Lisp value: it is a Rust copy of the spec's
//! plist, a long string as its length and 128-bit hash, so an entry
//! survives every collection and a mutated spec is looked up afresh.
use romacs_display_protocol::{
    ImageColorContext, ImageEmbeddedMetadata, ImageFrameIndex, ImageMaskPolicy, ImageRealization,
    ImageRotation, ImageSizeLimit, ImageSizeSpec,
};
use std::collections::HashMap;

/// What an image is decoded from, owned by the VM for the length of a load.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageSource {
    /// A local file, found as GNU's loaders find it.
    File(std::path::PathBuf),
    /// Encoded bytes (`:data'), with the base URI an SVG resolves against.
    Data { bytes: Vec<u8>, base_uri: Option<String> },
}

/// One decode the VM asks of the display host: GNU's `load_img'.
#[derive(Clone, Debug)]
pub struct ImageQuery {
    /// The image type's name: `png', `jpeg', `svg' ...
    pub kind: String,
    pub source: ImageSource,
    pub size: ImageSizeSpec,
    pub rotation: ImageRotation,
    pub realization: ImageRealization,
    pub colors: ImageColorContext,
    pub mask: ImageMaskPolicy,
    pub frame: ImageFrameIndex,
    pub limit: ImageSizeLimit,
}

/// What the host reports of a decoded image, whose pixels it keeps under ID.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageFacts {
    pub id: u64,
    /// The image's own size in pixels, before margins (GNU's `img->width',
    /// `img->height').
    pub width: u32,
    pub height: u32,
    /// GNU's `img->mask': the image has a clipping mask.
    pub mask: bool,
    pub metadata: ImageEmbeddedMetadata,
    /// GNU's IMAGE_BACKGROUND (its four corners' guess, 0xRRGGBB) when
    /// that is not transparent: a relief's colors start from it.
    pub background: Option<u32>,
}

/// A Lisp value of a spec's plist, copied: a string longer than
/// `INLINE` bytes is its length and xxh3-128 hash.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum KeyValue {
    Nil,
    Symbol(String),
    Integer(i64),
    Float(u64),
    Bytes(Vec<u8>),
    Digest(usize, u128),
    /// A bool-vector: its length and packed bits.
    Bits(usize, Box<KeyValue>),
    Cons(Box<KeyValue>, Box<KeyValue>),
    Vector(Vec<KeyValue>),
    /// Another object, by identity.
    Object(u64),
}

impl KeyValue {
    pub(crate) const INLINE: usize = 256;
    pub(crate) fn bytes(bytes: &[u8]) -> Self {
        if bytes.len() <= Self::INLINE {
            KeyValue::Bytes(bytes.to_vec())
        } else {
            KeyValue::Digest(bytes.len(), twox_hash::XxHash3_128::oneshot(bytes))
        }
    }
}

/// An image's identity in the cache: GNU's `search_image_cache' match.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ImageKey {
    /// The spec's plist but `:animate-buffer', `:animate-tardiness',
    /// `:animate-position' (GNU's `filter_image_spec'), `:index' kept.
    pub plist: Vec<(String, KeyValue)>,
    pub frame: u64,
    /// The resolved `:scale', as bits.
    pub scale: u64,
    pub foreground: u32,
    pub background: u32,
    pub font_size: u32,
    pub font_family: String,
}

/// One cached image.
#[derive(Clone, Debug)]
pub(crate) struct ImageEntry {
    pub facts: ImageFacts,
    /// GNU's `img->hmargin', `img->vmargin': `:margin' plus `:relief'.
    pub margins: (u32, u32),
    /// The decode failed: GNU's `load_failed_p'; FACTS hold the
    /// placeholder size and no host id.
    pub failed: bool,
    /// GNU's `img->dependencies': the absolute file names it was read from,
    /// and `auto' when its scale came from `image-scaling-factor' `auto'.
    pub dependencies: Vec<Dependency>,
    /// A GIF's extension blocks for `image-metadata' (graphics' `extension-data').
    pub extension: Vec<(u8, Vec<u8>)>,
    /// When it was last looked up (GNU's `img->timestamp').
    pub used: std::time::Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Dependency {
    File(String),
    Auto,
}

/// GNU's per-frame image cache, every frame's together.
#[derive(Default)]
pub(crate) struct ImageCache {
    entries: HashMap<ImageKey, ImageEntry>,
    /// The last lookup environment, under what it was computed from.
    env: Option<([u64; 7], crate::prim::ImageEnv)>,
    /// The next id for `lookup-image' (GNU's cache slot index).
    ids: HashMap<ImageKey, u64>,
    next_id: u64,
}

impl ImageCache {
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub(crate) fn env(&self, stamp: &[u64; 7]) -> Option<crate::prim::ImageEnv> {
        self.env.as_ref().filter(|(s, _)| s == stamp).map(|(_, env)| env.clone())
    }
    pub(crate) fn keep_env(&mut self, stamp: [u64; 7], env: crate::prim::ImageEnv) {
        self.env = Some((stamp, env));
    }
    pub(crate) fn get(&mut self, key: &ImageKey) -> Option<&ImageEntry> {
        let entry = self.entries.get_mut(key)?;
        entry.used = std::time::Instant::now();
        Some(entry)
    }
    /// Cache ENTRY under KEY, and its `lookup-image' id.
    pub(crate) fn insert(&mut self, key: ImageKey, entry: ImageEntry) -> u64 {
        let id = *self.ids.entry(key.clone()).or_insert_with(|| {
            self.next_id += 1;
            self.next_id
        });
        self.entries.insert(key, entry);
        id
    }
    pub(crate) fn id(&self, key: &ImageKey) -> Option<u64> {
        self.ids.get(key).copied()
    }
    /// Remove every entry KEEP says no to; the host ids of those it removed.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&ImageKey, &ImageEntry) -> bool) -> Vec<u64> {
        let mut released = vec![];
        self.entries.retain(|key, entry| {
            let kept = keep(key, entry);
            if !kept && !entry.failed { released.push(entry.facts.id); }
            kept
        });
        let entries = &self.entries;
        self.ids.retain(|key, _| entries.contains_key(key));
        released
    }
    /// GNU's `image-cache-size': the bytes the cached images' pixels take.
    pub(crate) fn size_in_bytes(&self) -> u64 {
        self.entries.values().filter(|e| !e.failed)
            .map(|e| u64::from(e.facts.width) * u64::from(e.facts.height) * 4)
            .sum()
    }
}
