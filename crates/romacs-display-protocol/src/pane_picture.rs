//! A pane picture: what an owner VM laid out for one master window, as it
//! crosses to the master, and the one codec for it. The owner lays the
//! window out under its own id and builds the picture with the master's
//! (`PanePicture::from_frame`); the master's receiver decodes it, refusing
//! what it must not draw (`PanePicture::decode`), and a pane draws it as a
//! frame (`PanePicture::into_frame`).
//!
//! The wire form is a little-endian `u16` format word, then the picture in
//! Postcard. The font and image ids a picture names are in its owner's
//! slot k: its fonts from `owner_slot_base(k)`, its images in
//! `ImageIdSpace::Owner(k)`. Fonts and images are separate spaces, each
//! split between the master and the owner slots, and images also with the
//! renderer.
use crate::face::FrameFaceMap;
use crate::font::{
    CharFontTable, FontCatalogGeneration, FontOutlineAsset, ResolvedFontId, ResolvedFontTable,
    ShapedClusterTable,
};
use crate::frame_glyphs::{FrameGlyph, FrameGlyphBuffer, FringeBitmapData, WindowCursor};
use crate::submission::SubmittedWindow;
use crate::text_animation::TextAnimation;
use crate::types::{Color, DisplayWindowId, FaceId, ImageId};
use serde::{Deserialize, Serialize};

/// The format word: bytes 0..2 of every encoding, little-endian, before the
/// Postcard body. A change to the picture's shape takes a new one.
pub const PANE_PICTURE_FORMAT: u16 = 1;

/// The largest width or height a picture's box may have, logical px.
pub const PANE_BOX_LIMIT: u32 = 16_384;

/// How many font ids one owner slot holds.
pub const OWNER_SLOT_SPAN: u32 = 1 << SLOT_SHIFT;
const SLOT_SHIFT: u32 = 24;

/// The last owner slot: slot 255's font ids would pass `u32`.
pub const LAST_OWNER_SLOT: u8 = 254;

/// Owner slot SLOT's first font id, `(SLOT+1) << 24`: its ids run to the
/// next slot's first. The master's are below slot 0's. None for a slot
/// past `LAST_OWNER_SLOT`.
#[must_use]
pub fn owner_slot_base(slot: u8) -> Option<u32> {
    (slot <= LAST_OWNER_SLOT).then(|| (u32::from(slot) + 1) << SLOT_SHIFT)
}

/// Whether font ID is one of owner slot SLOT's.
fn in_slot(id: u32, slot: u8) -> bool {
    id >> SLOT_SHIFT == u32::from(slot) + 1
}

/// Who allocates an image id: the `u32` [`ImageId`] split so that no two
/// allocators share one. The renderer's own (its toolbar) are
/// `[1, 1 << 24)`, the master VM's `[1 << 24, 1 << 31)`, and owner slot
/// k's `[(1 << 31) + k * 2^23, (1 << 31) + (k + 1) * 2^23)`; slot 255 has
/// none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageIdSpace {
    Renderer,
    Master,
    Owner(u8),
}

const MASTER_IMAGE_BASE: u32 = 1 << 24;
const OWNER_IMAGE_BASE: u32 = 1 << 31;
const OWNER_IMAGE_SPAN: u32 = 1 << 23;

impl ImageIdSpace {
    /// The space's first id and how many it holds; None for slot 255.
    fn bounds(self) -> Option<(u32, u32)> {
        match self {
            Self::Renderer => Some((1, MASTER_IMAGE_BASE - 1)),
            Self::Master => Some((MASTER_IMAGE_BASE, OWNER_IMAGE_BASE - MASTER_IMAGE_BASE)),
            Self::Owner(slot) => (slot <= LAST_OWNER_SLOT)
                .then(|| (OWNER_IMAGE_BASE + u32::from(slot) * OWNER_IMAGE_SPAN, OWNER_IMAGE_SPAN)),
        }
    }

    /// The space's id INDEX (from 0), or None past its end.
    #[must_use]
    pub fn id(self, index: u32) -> Option<ImageId> {
        let (first, count) = self.bounds()?;
        (index < count).then(|| ImageId::new(first + index))
    }

    /// Whether ID is one of the space's.
    #[must_use]
    pub fn contains(self, id: ImageId) -> bool {
        self.bounds().is_some_and(|(first, count)| id.get().wrapping_sub(first) < count)
    }
}

/// Which picture this is, and what it answers: also the publication
/// frame's header, readable without the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneHead {
    /// The owner's incarnation; the receiver checks it against the socket's.
    pub owner: u64,
    /// The master's window id.
    pub window: u64,
    /// The master view generation the request named.
    pub view: u64,
    /// The request revision laid out for.
    pub request: u64,
    /// The visual seq, monotone per incarnation; also the token.
    pub v: u64,
    /// The committed seq reflected.
    pub c: u64,
}

/// What the owner laid out for one pane: the frame, and what travels
/// beside it (its text animations, keyed `(key << 32) | window`, and the
/// coverage receipt).
#[derive(Clone, Debug)]
pub struct PaneFrame {
    pub frame: FrameGlyphBuffer,
    pub animations: Vec<(u64, TextAnimation)>,
    pub submitted: Option<SubmittedWindow>,
}

/// One master window as its owner laid it out, every window id in it the
/// master's and every coordinate pane-local.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanePicture {
    pub head: PaneHead,
    /// The request's box echoed, logical px.
    pub width: u32,
    pub height: u32,
    /// The request's font catalog echoed.
    pub catalog: FontCatalogGeneration,
    /// Key calls first shown here.
    pub keys: Vec<u64>,
    pub background: Color,
    pub char_width: f32,
    pub char_height: f32,
    pub glyphs: Vec<FrameGlyph>,
    pub cursor: Option<WindowCursor>,
    /// The pane's own face ids.
    pub faces: FrameFaceMap,
    /// Font ids in the owner's slot.
    pub fonts: ResolvedFontTable,
    pub char_fonts: CharFontTable,
    pub shaped_clusters: ShapedClusterTable,
    /// By bitmap index, in order.
    pub fringe_bitmaps: Vec<(u16, FringeBitmapData)>,
    /// Keyed `(key << 32) | window`.
    pub animations: Vec<(u64, TextAnimation)>,
    /// What the scene shows of exactly these glyphs.
    pub submitted: Option<SubmittedWindow>,
    /// Owner image ids drawn.
    pub images: Vec<u32>,
    /// Owner image ids the owner's cache let go.
    pub released: Vec<u32>,
}

/// Why a receiver refuses a picture's bytes.
#[derive(Debug)]
pub enum PictureError {
    /// Not a picture: too short for its format word, or a body Postcard
    /// cannot read.
    Malformed(postcard::Error),
    /// Postcard could not write the picture.
    Unencodable(postcard::Error),
    UnknownFormat(u16),
    /// A video, xwidget, surface or terminal glyph: what a pane does not
    /// draw.
    UnsupportedGlyph,
    /// A font whose outline is held in memory, whose bytes would travel in
    /// every picture.
    MemoryFont,
    /// A glyph's face is not in the picture's faces.
    MissingFace(FaceId),
    FontOutsideSlot(ResolvedFontId),
    ImageOutsideSlot(ImageId),
    /// A box wider or higher than `PANE_BOX_LIMIT`.
    BoxTooLarge { width: u32, height: u32 },
}

impl std::fmt::Display for PictureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(error) => write!(f, "malformed pane picture: {error}"),
            Self::Unencodable(error) => write!(f, "pane picture could not be encoded: {error}"),
            Self::UnknownFormat(word) => write!(f, "unknown pane picture format {word}"),
            Self::UnsupportedGlyph => write!(f, "pane picture has a video, xwidget, surface or terminal glyph"),
            Self::MemoryFont => write!(f, "pane picture has a memory-backed font"),
            Self::MissingFace(face) => write!(f, "pane picture glyph names face {face}, not in its faces"),
            Self::FontOutsideSlot(font) => write!(f, "pane picture font {} is outside its owner's slot", font.0),
            Self::ImageOutsideSlot(image) => write!(f, "pane picture image {image} is outside its owner's slot"),
            Self::BoxTooLarge { width, height } => write!(f, "pane picture box {width}x{height} is over {PANE_BOX_LIMIT} px"),
        }
    }
}

impl std::error::Error for PictureError {}

impl PanePicture {
    /// The picture of FRAME, laid out under the owner's window `own_id`,
    /// for HEAD: `own_id` becomes the master's `head.window` in the
    /// glyphs' window and slot ids, the cursor (the frame's cursor for
    /// `own_id`), and the coverage receipt. Only that window's animations
    /// are the pane's, each its window and its key's low half relabelled.
    #[must_use]
    pub fn from_frame(head: PaneHead, frame: PaneFrame, own_id: DisplayWindowId) -> Self {
        let PaneFrame { frame: mut laid, animations, submitted } = frame;
        let master = DisplayWindowId::new(head.window.cast_signed());
        for glyph in &mut laid.glyphs {
            relabel_glyph(glyph, own_id, master);
        }
        let cursor = std::mem::take(&mut laid.window_cursors)
            .into_iter()
            .find(|cursor| cursor.window_id == own_id)
            .map(|mut cursor| {
                relabel_cursor(&mut cursor, master);
                cursor
            });
        let mut fringe_bitmaps: Vec<_> = std::mem::take(&mut laid.fringe_bitmaps).into_iter().collect();
        fringe_bitmaps.sort_unstable_by_key(|&(index, _)| index);
        let (own, window) = (own_id.get().cast_unsigned(), head.window);
        Self {
            head,
            width: whole_px(laid.width),
            height: whole_px(laid.height),
            catalog: laid.font_catalog_generation,
            keys: Vec::new(),
            background: laid.background,
            char_width: laid.char_width,
            char_height: laid.char_height,
            glyphs: std::mem::take(&mut laid.glyphs),
            cursor,
            faces: std::mem::take(&mut laid.faces),
            fonts: std::mem::take(&mut laid.fonts),
            char_fonts: std::mem::take(&mut laid.char_fonts),
            shaped_clusters: std::mem::take(&mut laid.shaped_clusters),
            fringe_bitmaps,
            animations: animations.into_iter().filter_map(|entry| relabel_animation(entry, own, window)).collect(),
            submitted: submitted.map(|receipt| relabel_submitted(receipt, own, window)),
            images: Vec::new(),
            released: Vec::new(),
        }
    }

    /// The frame a pane draws: the picture's box, colours, cell, catalog,
    /// glyphs, tables and fringe bitmaps, and its cursor as the frame's one
    /// cursor.
    #[must_use]
    pub fn into_frame(self) -> FrameGlyphBuffer {
        let mut frame = FrameGlyphBuffer::with_glyphs(px(self.width), px(self.height), self.glyphs);
        frame.background = self.background;
        frame.char_width = self.char_width;
        frame.char_height = self.char_height;
        frame.font_catalog_generation = self.catalog;
        frame.window_cursors = self.cursor.into_iter().collect();
        frame.faces = self.faces;
        frame.fonts = self.fonts;
        frame.char_fonts = self.char_fonts;
        frame.shaped_clusters = self.shaped_clusters;
        frame.fringe_bitmaps = self.fringe_bitmaps.into_iter().collect();
        frame
    }

    /// The picture's bytes: the format word, then Postcard. Unvalidated.
    ///
    /// # Errors
    ///
    /// `Unencodable` when Postcard cannot write it.
    pub fn encode(&self) -> Result<Vec<u8>, PictureError> {
        postcard::to_extend(self, PANE_PICTURE_FORMAT.to_le_bytes().to_vec()).map_err(PictureError::Unencodable)
    }

    /// The picture BYTES encode, as owner slot SLOT's receiver takes it.
    ///
    /// # Errors
    ///
    /// Refused: bytes too short or unreadable, another format word, a box
    /// over `PANE_BOX_LIMIT` either way, a video, xwidget, surface or
    /// terminal glyph, a glyph's face not in `faces`, a memory-backed
    /// font, a font id (in `fonts`' keys and records, `char_fonts`,
    /// `shaped_clusters`, faces' primary fonts) outside slot SLOT's, or an
    /// image id (drawn, `images`, `released`) outside
    /// `ImageIdSpace::Owner(SLOT)`.
    pub fn decode(bytes: &[u8], slot: u8) -> Result<Self, PictureError> {
        let (word, body) = bytes
            .split_first_chunk::<2>()
            .ok_or(PictureError::Malformed(postcard::Error::DeserializeUnexpectedEnd))?;
        let word = u16::from_le_bytes(*word);
        if word != PANE_PICTURE_FORMAT {
            return Err(PictureError::UnknownFormat(word));
        }
        let picture: Self = postcard::from_bytes(body).map_err(PictureError::Malformed)?;
        picture.check_box()?;
        picture.check_glyphs()?;
        picture.check_faces()?;
        picture.check_fonts(slot)?;
        picture.check_images(slot)?;
        Ok(picture)
    }

    fn check_box(&self) -> Result<(), PictureError> {
        let (width, height) = (self.width, self.height);
        if width > PANE_BOX_LIMIT || height > PANE_BOX_LIMIT {
            return Err(PictureError::BoxTooLarge { width, height });
        }
        Ok(())
    }

    /// Every glyph is one a pane draws.
    fn check_glyphs(&self) -> Result<(), PictureError> {
        if self.glyphs.iter().all(drawn_in_panes) { Ok(()) } else { Err(PictureError::UnsupportedGlyph) }
    }

    /// Every inline glyph's face (`FrameGlyph::face_id`) is in `faces`.
    fn check_faces(&self) -> Result<(), PictureError> {
        match self.glyphs.iter().filter_map(FrameGlyph::face_id).find(|id| !self.faces.contains_key(id)) {
            Some(id) => Err(PictureError::MissingFace(id)),
            None => Ok(()),
        }
    }

    fn check_fonts(&self, slot: u8) -> Result<(), PictureError> {
        if self.fonts.values().any(|font| matches!(font.replay.outline_asset(), Some(FontOutlineAsset::Memory(_)))) {
            return Err(PictureError::MemoryFont);
        }
        let table = self.fonts.iter().flat_map(|(key, font)| [*key, font.id]);
        let chars = self.char_fonts.values().flat_map(|chars| chars.values().map(|glyph| glyph.resolved_font_id));
        let clusters = self.shaped_clusters.values().flat_map(|clusters| clusters.values().flatten().map(|glyph| glyph.resolved_font_id));
        let faces = self.faces.values().filter_map(|face| face.default_resolved_font_id);
        match table.chain(chars).chain(clusters).chain(faces).find(|id| !in_slot(id.0, slot)) {
            Some(id) => Err(PictureError::FontOutsideSlot(id)),
            None => Ok(()),
        }
    }

    fn check_images(&self, slot: u8) -> Result<(), PictureError> {
        let drawn = self.glyphs.iter().filter_map(FrameGlyph::image_id);
        let listed = self.images.iter().chain(&self.released).map(|&id| ImageId::new(id));
        match drawn.chain(listed).find(|&id| !ImageIdSpace::Owner(slot).contains(id)) {
            Some(id) => Err(PictureError::ImageOutsideSlot(id)),
            None => Ok(()),
        }
    }
}

/// Whether a pane draws GLYPH: not a video, xwidget, surface or terminal,
/// whose players and views live in the master.
fn drawn_in_panes(glyph: &FrameGlyph) -> bool {
    match glyph {
        FrameGlyph::Char { .. }
        | FrameGlyph::Stretch { .. }
        | FrameGlyph::Image { .. }
        | FrameGlyph::FringeBitmap { .. }
        | FrameGlyph::Border { .. }
        | FrameGlyph::ScrollBar { .. }
        | FrameGlyph::Background { .. } => true,
        FrameGlyph::Video { .. } | FrameGlyph::Xwidget { .. } | FrameGlyph::Surface { .. } => false,
        #[cfg(feature = "romacs-term")]
        FrameGlyph::Terminal { .. } => false,
    }
}

/// GLYPH's window and slot ids that are OWN, made MASTER.
fn relabel_glyph(glyph: &mut FrameGlyph, own: DisplayWindowId, master: DisplayWindowId) {
    if let Some(window) = glyph.window_id_mut().filter(|window| **window == own) {
        *window = master;
    }
    if let Some(slot) = glyph.slot_id_mut().filter(|slot| slot.window_id == own) {
        slot.window_id = master;
    }
}

fn relabel_cursor(cursor: &mut WindowCursor, master: DisplayWindowId) {
    cursor.window_id = master;
    cursor.slot_id.window_id = master;
}

/// ENTRY, an animation and its key, moved from window OWN to MASTER; None
/// for another window's.
fn relabel_animation((key, animation): (u64, TextAnimation), own: u64, master: u64) -> Option<(u64, TextAnimation)> {
    const LOW: u64 = 0xffff_ffff;
    (animation.window == own).then_some(((key & !LOW) | (master & LOW), TextAnimation { window: master, ..animation }))
}

fn relabel_submitted(receipt: SubmittedWindow, own: u64, master: u64) -> SubmittedWindow {
    let window_id = if receipt.window_id == own { master } else { receipt.window_id };
    SubmittedWindow { window_id, ..receipt }
}

/// V logical px as a box's whole px.
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a frame's box is whole px far below `u32::MAX`; a negative or NaN one saturates to 0")]
pub(crate) fn whole_px(v: f32) -> u32 {
    v.round() as u32
}

/// N whole px as a frame's.
#[expect(clippy::cast_precision_loss, reason = "a drawn box is far below 2^24 px")]
fn px(n: u32) -> f32 {
    n as f32
}
