//! O1b-1a commit 1a-1: a pane picture round-trips through its codec with
//! every window id relabelled to the master's, and decoding refuses what a
//! receiver must not draw (design O1b.md rev 3, §2.3, §5).
//!
//! The interface these tests drive, in `crate::pane_picture`:
//!
//! ```ignore
//! /// The format word: bytes 0..2 of every encoding, little-endian, before
//! /// the Postcard body.
//! pub const PANE_PICTURE_FORMAT: u16;
//! /// The largest width or height a picture's box may have, logical px.
//! pub const PANE_BOX_LIMIT: u32 = 16_384;
//!
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
//! pub struct PaneHead { pub owner: u64, pub window: u64, pub view: u64,
//!                       pub request: u64, pub v: u64, pub c: u64 }
//!
//! /// What the owner laid out for one pane: the frame, and what travels
//! /// beside it (`thread_comm`'s text animations; the coverage receipt).
//! pub struct PaneFrame {
//!     pub frame: FrameGlyphBuffer,
//!     pub animations: Vec<(u64, TextAnimation)>,
//!     pub submitted: Option<SubmittedWindow>,
//! }
//!
//! #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
//! pub struct PanePicture { /* the fields of O1b.md §2.3, all pub */ }
//!
//! #[derive(Debug)]
//! pub enum PictureError {
//!     Malformed(..),            // payload the writer's choice; truncation lands here
//!     UnknownFormat(u16),
//!     MissingFace(FaceId),
//!     FontOutsideSlot(ResolvedFontId),
//!     ImageOutsideSlot(ImageId),
//!     BoxTooLarge { width: u32, height: u32 },
//! }
//!
//! impl PanePicture {
//!     /// OWN_ID relabelled to `head.window` in glyph `window_id`,
//!     /// `slot_id.window_id`, the cursor, each animation's `window` and
//!     /// its key's low 32 bits, and `submitted.window_id`. The cursor is
//!     /// the frame's `window_cursors` entry for OWN_ID.
//!     pub fn from_frame(head: PaneHead, frame: PaneFrame, own_id: DisplayWindowId) -> Self;
//!     /// The frame a pane draws: the carried fields, the cursor as the one
//!     /// `window_cursors` entry.
//!     pub fn into_frame(self) -> FrameGlyphBuffer;
//!     /// Unvalidated: format word, then Postcard.
//!     pub fn encode(&self) -> Vec<u8>;
//!     /// Validated against owner slot SLOT: every glyph face id in
//!     /// `faces`; every font id (`fonts` keys and ids, `char_fonts`,
//!     /// `shaped_clusters`, faces' `default_resolved_font_id`) and image
//!     /// id in `(SLOT+1) << 24 .. (SLOT+2) << 24`; the box at most
//!     /// `PANE_BOX_LIMIT` each way.
//!     pub fn decode(bytes: &[u8], slot: u8) -> Result<Self, PictureError>;
//! }
//! ```
use crate::face::Face;
use crate::font::{
    CharFontTable, FontCatalogGeneration, FontFileAsset, FontOutlineAsset, FontReplay,
    FontSlantKind, FontUnderline, ResolvedCharGlyph, ResolvedFont, ResolvedFontAdvance,
    ResolvedFontId, ResolvedFontIdentity,
    ResolvedFontTable, ResolvedGlyph, ResolvedGlyphId, ShapedClusterTable,
};
use crate::pane_picture::{
    PaneFrame, PaneHead, PanePicture, PictureError, PANE_BOX_LIMIT, PANE_PICTURE_FORMAT,
};
use crate::submission::SubmittedWindow;
use crate::text_animation::{LogoColours, TextAnimation, TextAnimationKind};
use crate::{
    BoxVerticalEdges, Color, CursorStyle, DisplaySlotId, DisplayWindowId, FaceId, FrameGlyph,
    FrameGlyphBuffer, FrameSpace, FringeBitmapData, FringeSide, GeometryPoint, GlyphRowRole,
    ImageId, ImageSourceRect, LogicalPixels, Px, Rect, SurfaceId, VideoId, WebViewId,
    WindowCursor, XwidgetContentExtent, XwidgetId, XwidgetLayoutAdvance,
    XwidgetPresentationGeometry,
};
use rustc_hash::FxHashMap as HashMap;

/// The owner's slot, and the first id it may use.
const SLOT: u8 = 3;
const BASE: u32 = (SLOT as u32 + 1) << 24;
/// The window as the owner laid it out, and as the master knows it: both
/// under 32 bits and different in their low halves.
const OWN: DisplayWindowId = DisplayWindowId::new(5);
const MASTER: DisplayWindowId = DisplayWindowId::new(900);

const PLAIN: FaceId = FaceId::new(0);
const EMOJI: FaceId = FaceId::new(21);
const IMAGE: ImageId = ImageId::new(BASE + 7);
const HEART: &str = "\u{2764}\u{FE0F}";

fn head() -> PaneHead {
    PaneHead { owner: 41, window: MASTER.get().cast_unsigned(), view: 11, request: 13, v: 17, c: 19 }
}

fn slot(window: DisplayWindowId, row: u32, col: u16) -> DisplaySlotId {
    DisplaySlotId { window_id: window, row, col }
}

fn font(id: u32, file: &str) -> ResolvedFont {
    ResolvedFont {
        id: ResolvedFontId(id),
        identity: ResolvedFontIdentity::from_file(file, 0, None),
        replay: FontReplay::Swash {
            asset: FontOutlineAsset::File(FontFileAsset::new(file, 0).unwrap()),
        },
        family: "Test".into(),
        full_name: None,
        postscript_name: None,
        weight: 400,
        slant: FontSlantKind::Normal,
        width: 5,
        pixel_size: 14.0,
        ascent_px: 11.0,
        descent_px: 3.0,
        space_advance_px: 8.0,
        glyph_advance: ResolvedFontAdvance::default(),
        underline: FontUnderline::default(),
    }
}

fn cursor(window: DisplayWindowId) -> WindowCursor {
    WindowCursor {
        window_id: window,
        slot_id: slot(window, 0, 0),
        x: 0.0,
        y: 0.0,
        width: 8.0,
        height: 16.0,
        style: CursorStyle::FilledBox,
        color: Color::WHITE,
        cursor_fg: Color::BLACK,
        ascent: 12.0,
        active: true,
    }
}

/// A frame of one window, WINDOW, with a glyph of every kind, its faces,
/// fonts in `SLOT`, a fringe bitmap and its cursor. Laid out by hand: the
/// codec must carry what any layout makes.
fn frame(window: DisplayWindowId) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(320.0, 200.0);
    let (role, clip, edges) = (GlyphRowRole::Text, Some(Rect::new(0.0, 0.0, 320.0, 184.0)), BoxVerticalEdges::Unboxed);
    let colour = Color::new(0.1, 0.2, 0.3, 1.0);
    frame.background = colour;
    frame.char_width = 8.0;
    frame.char_height = 16.0;
    frame.font_catalog_generation = FontCatalogGeneration::from_raw(7);
    frame.glyphs = vec![
        FrameGlyph::Background { bounds: Rect::new(0.0, 0.0, 320.0, 200.0), color: colour },
        FrameGlyph::Char {
            window_id: window, row_role: role, clip_rect: clip, slot_id: slot(window, 0, 0),
            bidi_level: 0, char: 'a', composed: None, x: 0.0, y: 0.0, baseline: 12.0,
            width: 8.0, height: 16.0, ascent: 12.0, face_id: PLAIN, box_vertical_edges: edges,
        },
        FrameGlyph::Char {
            window_id: window, row_role: role, clip_rect: clip, slot_id: slot(window, 0, 1),
            bidi_level: 1, char: '\u{2764}', composed: Some(HEART.into()), x: 8.0, y: 0.0,
            baseline: 12.0, width: 16.0, height: 16.0, ascent: 12.0, face_id: EMOJI,
            box_vertical_edges: BoxVerticalEdges::Both,
        },
        FrameGlyph::Stretch {
            window_id: window, row_role: role, clip_rect: clip, slot_id: slot(window, 0, 3),
            bidi_level: 0, x: 24.0, y: 0.0, width: 296.0, height: 16.0, bg: colour,
            face_id: PLAIN, box_vertical_edges: edges,
        },
        FrameGlyph::Image {
            window_id: window, row_role: role, clip_rect: clip, slot_id: Some(slot(window, 1, 0)),
            image_id: IMAGE, source_rect: ImageSourceRect::FULL,
            slot_rect: Rect::new(0.0, 16.0, 32.0, 32.0), box_rect: Rect::new(0.0, 16.0, 32.0, 32.0),
            x: 0.0, y: 16.0, width: 32.0, height: 32.0, face_id: PLAIN, box_vertical_edges: edges,
        },
        FrameGlyph::Video {
            window_id: window, row_role: role, clip_rect: clip, slot_id: Some(slot(window, 1, 4)),
            video_id: VideoId::new(2), x: 32.0, y: 16.0, width: 32.0, height: 32.0, opacity: 0.5,
            face_id: PLAIN, box_vertical_edges: edges,
        },
        FrameGlyph::Xwidget {
            window_id: window, row_role: role, slot_id: Some(slot(window, 1, 8)),
            xwidget_id: XwidgetId::new(3), webview_id: WebViewId::new(4),
            presentation: XwidgetPresentationGeometry::new(
                GeometryPoint::<FrameSpace, LogicalPixels>::from_px(64.0, 16.0).unwrap(),
                XwidgetContentExtent::new(60.0, 40.0).unwrap(),
                XwidgetLayoutAdvance::new(Px(64.0)).unwrap(),
                None,
            ),
            face_id: PLAIN, box_vertical_edges: edges,
        },
        FrameGlyph::Surface {
            window_id: window, row_role: role, clip_rect: clip, slot_id: Some(slot(window, 1, 16)),
            surface_id: SurfaceId::new(5), x: 128.0, y: 16.0, width: 32.0, height: 32.0,
            face_id: PLAIN, box_vertical_edges: edges,
        },
        FrameGlyph::FringeBitmap {
            window_id: window, row_role: role, clip_rect: clip, x: 0.0, y: 48.0, width: 8.0,
            height: 16.0, bitmap_index: 1, face_id: PLAIN, side: FringeSide::Left,
        },
        FrameGlyph::Border {
            window_id: window, row_role: GlyphRowRole::ModeLine, clip_rect: None, x: 0.0,
            y: 183.0, width: 320.0, height: 1.0, color: Color::BLACK,
        },
        FrameGlyph::ScrollBar {
            window_id: window, row_role: role, clip_rect: clip, horizontal: false, x: 312.0,
            y: 0.0, width: 8.0, height: 184.0, position: 1, portion: 40, whole: 100,
            thumb_start: 0.0, thumb_size: 70.0, track_color: Color::BLACK, thumb_color: Color::WHITE,
        },
    ];
    let mut plain = Face::new(PLAIN);
    plain.default_resolved_font_id = Some(ResolvedFontId(BASE));
    frame.faces.insert(PLAIN, plain);
    frame.faces.insert(EMOJI, Face::new(EMOJI));
    frame.fonts = ResolvedFontTable::from_iter([
        (ResolvedFontId(BASE), font(BASE, "/fonts/mono.ttf")),
        (ResolvedFontId(BASE + 1), font(BASE + 1, "/fonts/emoji.ttf")),
    ]);
    frame.char_fonts = CharFontTable::from_iter([(PLAIN, HashMap::from_iter([(
        'a',
        ResolvedCharGlyph { resolved_font_id: ResolvedFontId(BASE), glyph_id: ResolvedGlyphId::new(68), advance_px: 8.0 },
    )]))]);
    frame.shaped_clusters = ShapedClusterTable::from_iter([(EMOJI, HashMap::from_iter([(
        Box::<str>::from(HEART),
        vec![ResolvedGlyph {
            resolved_font_id: ResolvedFontId(BASE + 1), glyph_id: ResolvedGlyphId::new(1201),
            x: 0.0, y: 0.0, x_advance: 16.0, cluster_start: 0,
            cluster_end: u32::try_from(HEART.len()).unwrap(),
        }],
    )]))]);
    frame.fringe_bitmaps = HashMap::from_iter([(1, FringeBitmapData { bits: vec![0x8000, 0xc000], width: 2, height: 2, period: 0, align: 0 })]);
    frame.window_cursors = vec![cursor(window)];
    frame
}

/// Window WINDOW's text animations, one of each kind, keyed as
/// `romacs-vm`'s display keys them: (key << 32) | the window's low half.
fn animations(window: DisplayWindowId) -> Vec<(u64, TextAnimation)> {
    let w = window.get().cast_unsigned();
    vec![
        ((1 << 32) | (w & 0xffff_ffff), TextAnimation {
            window: w, top: 0, cols: 4, rows: 2, anchor: None, rise: None, fps: 25, moving: true,
            still: false,
            kind: TextAnimationKind::Logo { cell_aspect: 0.5, epoch: 1, colours: LogoColours::default() },
        }),
        ((2 << 32) | (w & 0xffff_ffff), TextAnimation {
            window: w, top: 1, cols: 8, rows: 1, anchor: Some(3), rise: Some((16.0, 16.0)), fps: 30,
            moving: true, still: false,
            kind: TextAnimationKind::Working {
                work: romacs_logo::working::Work::Running, beats: 3, epoch: 2, colour: [1, 2, 3],
                bead: [4, 5, 6],
            },
        }),
    ]
}

fn submitted(window: DisplayWindowId) -> SubmittedWindow {
    SubmittedWindow {
        window_id: window.get().cast_unsigned(),
        view_generation: 11,
        text_revision: 12,
        visible_text_bytes: vec![0..4, 8..12],
        cursor_byte: Some(1),
    }
}

/// The owner's picture of its frame, laid out under OWN.
fn picture() -> PanePicture {
    PanePicture::from_frame(
        head(),
        PaneFrame { frame: frame(OWN), animations: animations(OWN), submitted: Some(submitted(OWN)) },
        OWN,
    )
}

/// PICTURE through the wire, as slot `SLOT`'s receiver decodes it.
fn through_wire(picture: &PanePicture) -> Result<PanePicture, PictureError> {
    PanePicture::decode(&picture.encode(), SLOT)
}

#[test]
fn a_pane_picture_round_trips_with_every_id_the_masters() {
    let picture = picture();
    let bytes = picture.encode();
    assert_eq!(bytes[..2], PANE_PICTURE_FORMAT.to_le_bytes(), "the format word comes first");
    let decoded = PanePicture::decode(&bytes, SLOT).expect("a well-formed picture decodes");
    assert_eq!(decoded, picture);
    assert_eq!(decoded.head, head());
    assert_eq!((decoded.width, decoded.height), (320, 200), "the box echoed");
    assert_eq!(decoded.catalog, FontCatalogGeneration::from_raw(7));

    // The one relabel: the master's id everywhere the owner's was.
    for glyph in &decoded.glyphs {
        assert!(glyph.window_id().is_none_or(|id| id == MASTER), "{glyph:?}");
        assert!(glyph.slot_id().is_none_or(|slot| slot.window_id == MASTER), "{glyph:?}");
    }
    assert_eq!(decoded.glyphs, frame(MASTER).glyphs);
    assert_eq!(decoded.cursor, Some(cursor(MASTER)));
    assert_eq!(decoded.animations, animations(MASTER), "each animation's window and key's low half");
    assert_eq!(decoded.submitted, Some(submitted(MASTER)));

    // A pane draws the frame the owner laid out, under the master's id.
    let (expected, drawn) = (frame(MASTER), decoded.into_frame());
    assert_eq!(drawn.glyphs, expected.glyphs);
    assert_eq!(drawn.window_cursors, vec![cursor(MASTER)]);
    assert_eq!(drawn.faces, expected.faces);
    assert_eq!(drawn.fonts, expected.fonts);
    assert_eq!(drawn.char_fonts, expected.char_fonts);
    assert_eq!(drawn.shaped_clusters, expected.shaped_clusters);
    assert_eq!(drawn.fringe_bitmaps, expected.fringe_bitmaps);
    assert_eq!(drawn.font_catalog_generation, expected.font_catalog_generation);
    assert_eq!(
        (drawn.width, drawn.height, drawn.background, drawn.char_width, drawn.char_height),
        (expected.width, expected.height, expected.background, expected.char_width, expected.char_height),
    );
}

#[test]
fn truncated_bytes_are_malformed() {
    let bytes = picture().encode();
    assert!(PanePicture::decode(&bytes, SLOT).is_ok(), "the whole picture decodes");
    for cut in [0, 1, 2, bytes.len() / 2, bytes.len() - 1] {
        let result = PanePicture::decode(&bytes[..cut], SLOT);
        assert!(matches!(result, Err(PictureError::Malformed { .. })), "cut at {cut}: {result:?}");
    }
}

#[test]
fn an_unknown_format_word_is_refused() {
    let mut bytes = picture().encode();
    let other = PANE_PICTURE_FORMAT.wrapping_add(1);
    bytes[..2].copy_from_slice(&other.to_le_bytes());
    let result = PanePicture::decode(&bytes, SLOT);
    assert!(matches!(result, Err(PictureError::UnknownFormat(word)) if word == other), "{result:?}");
}

#[test]
fn a_glyph_face_missing_from_faces_is_refused() {
    assert!(through_wire(&picture()).is_ok());
    let mut picture = picture();
    picture.faces.remove(&EMOJI);
    let result = through_wire(&picture);
    assert!(matches!(result, Err(PictureError::MissingFace(face)) if face == EMOJI), "{result:?}");
}

#[test]
fn a_font_id_outside_the_owners_slot_is_refused() {
    assert!(through_wire(&picture()).is_ok());
    let next_slot = ResolvedFontId(BASE + (1 << 24));
    let below = ResolvedFontId(BASE - 1);
    let master = ResolvedFontId(1);
    // A font in the table under the next slot's first id.
    let mut picture_a = picture();
    picture_a.fonts.insert(next_slot, font(next_slot.0, "/fonts/other.ttf"));
    // A character's font in the master's range.
    let mut picture_b = picture();
    picture_b.char_fonts.get_mut(&PLAIN).unwrap().get_mut(&'a').unwrap().resolved_font_id = master;
    // A face's primary font just below the slot.
    let mut picture_c = picture();
    picture_c.faces.get_mut(&EMOJI).unwrap().default_resolved_font_id = Some(below);
    // A shaped cluster's font in the master's range.
    let mut picture_d = picture();
    picture_d.shaped_clusters.get_mut(&EMOJI).unwrap().get_mut(HEART).unwrap()[0].resolved_font_id = master;
    for (picture, id) in [(picture_a, next_slot), (picture_b, master), (picture_c, below), (picture_d, master)] {
        let result = through_wire(&picture);
        assert!(matches!(result, Err(PictureError::FontOutsideSlot(font)) if font == id), "{id:?}: {result:?}");
    }
}

#[test]
fn an_image_id_outside_the_owners_slot_is_refused() {
    assert!(through_wire(&picture()).is_ok());
    for id in [ImageId::new(7), ImageId::new(BASE + (1 << 24))] {
        let mut picture = picture();
        for glyph in &mut picture.glyphs {
            if let FrameGlyph::Image { ref mut image_id, .. } = *glyph {
                *image_id = id;
            }
        }
        let result = through_wire(&picture);
        assert!(matches!(result, Err(PictureError::ImageOutsideSlot(image)) if image == id), "{id:?}: {result:?}");
    }
}

#[test]
fn a_box_over_the_limit_is_refused() {
    assert_eq!(PANE_BOX_LIMIT, 16_384);
    let mut largest = picture();
    (largest.width, largest.height) = (PANE_BOX_LIMIT, PANE_BOX_LIMIT);
    assert!(through_wire(&largest).is_ok(), "a box of the limit is drawn");
    for (width, height) in [(PANE_BOX_LIMIT + 1, 200), (320, PANE_BOX_LIMIT + 1)] {
        let mut picture = picture();
        (picture.width, picture.height) = (width, height);
        let result = through_wire(&picture);
        assert!(
            matches!(result, Err(PictureError::BoxTooLarge { width: w, height: h }) if (w, h) == (width, height)),
            "{width}x{height}: {result:?}"
        );
    }
}
