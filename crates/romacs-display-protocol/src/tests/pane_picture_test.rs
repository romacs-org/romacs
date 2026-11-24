//! O1b-1a commit 1a-1: a pane picture round-trips through its codec with
//! every window id relabelled to the master's, and decoding refuses what a
//! receiver must not draw (design O1b.md rev 3, §2.3, §5).
//!
//! The interface these tests expect in `crate::pane_picture`:
//! - `ImageIdSpace` is a copyable enum with `Renderer`, `Master` and `Owner(u8)`.
//!   Its `id(index: u32) -> Option<ImageId>` uses the zero-based index in
//!   the image ranges of O1b.md §4.4.1 (A2).
//! - `PanePicture::encode(&self) -> Result<Vec<u8>, PictureError>` writes the
//!   little-endian format word, then Postcard.
//! - `PanePicture::decode(bytes: &[u8], slot: u8) -> Result<Self, PictureError>`
//!   validates font and image IDs in their own ranges. It returns
//!   `PictureError::UnsupportedGlyph` for Video, Xwidget, Surface or Terminal,
//!   and `PictureError::MemoryFont` for a memory-backed outline in the font table.
//! - `FrameGlyph::image_id(&self) -> Option<ImageId>` returns an image glyph's ID.
use crate::face::Face;
use crate::font::{
    CharFontTable, FontBackendKind, FontCatalogGeneration, FontFileAsset, FontMemoryAsset,
    FontOutlineAsset, FontReplay,
    FontSlantKind, FontUnderline, ResolvedCharGlyph, ResolvedFont, ResolvedFontAdvance,
    ResolvedFontId, ResolvedFontIdentity,
    ResolvedFontTable, ResolvedGlyph, ResolvedGlyphId, ShapedClusterTable,
};
use crate::pane_picture::{
    ImageIdSpace, PaneFrame, PaneHead, PanePicture, PictureError, PANE_BOX_LIMIT, PANE_PICTURE_FORMAT,
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
use std::sync::Arc;

/// The owner's slot and its first font ID.
const SLOT: u8 = 3;
const BASE: u32 = (SLOT as u32 + 1) << 24;
/// The window as the owner laid it out, and as the master knows it: both
/// under 32 bits and different in their low halves.
const OWN: DisplayWindowId = DisplayWindowId::new(5);
const MASTER: DisplayWindowId = DisplayWindowId::new(900);

const PLAIN: FaceId = FaceId::new(0);
const EMOJI: FaceId = FaceId::new(21);
const MASTER_IMAGE_BASE: u32 = 1 << 24;
const OWNER_IMAGE_BASE: u32 = 1 << 31;
const IMAGE_SLOT_SPAN: u32 = 1 << 23;
const IMAGE: ImageId = ImageId::new(OWNER_IMAGE_BASE + SLOT as u32 * IMAGE_SLOT_SPAN + 7);
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

fn image_glyph(window: DisplayWindowId, image_id: ImageId) -> FrameGlyph {
    FrameGlyph::Image {
        window_id: window, row_role: GlyphRowRole::Text,
        clip_rect: Some(Rect::new(0.0, 0.0, 320.0, 184.0)), slot_id: Some(slot(window, 1, 0)),
        image_id, source_rect: ImageSourceRect::FULL,
        slot_rect: Rect::new(0.0, 16.0, 32.0, 32.0), box_rect: Rect::new(0.0, 16.0, 32.0, 32.0),
        x: 0.0, y: 16.0, width: 32.0, height: 32.0, face_id: PLAIN, box_vertical_edges: BoxVerticalEdges::Unboxed,
    }
}

/// A frame of one window, WINDOW, with supported glyphs, their faces,
/// fonts in `SLOT`, a fringe bitmap and its cursor.
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
        image_glyph(window, IMAGE),
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
    PanePicture::decode(&picture.encode().unwrap(), SLOT)
}

#[test]
fn a_pane_picture_round_trips_with_every_id_the_masters() {
    let picture = picture();
    let bytes = picture.encode().unwrap();
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
    let bytes = picture().encode().unwrap();
    assert!(PanePicture::decode(&bytes, SLOT).is_ok(), "the whole picture decodes");
    for cut in [0, 1, 2, bytes.len() / 2, bytes.len() - 1] {
        let result = PanePicture::decode(&bytes[..cut], SLOT);
        assert!(matches!(result, Err(PictureError::Malformed { .. })), "cut at {cut}: {result:?}");
    }
}

#[test]
fn an_unknown_format_word_is_refused() {
    let mut bytes = picture().encode().unwrap();
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

fn assert_image_space(space: ImageIdSpace, first: u32, last: u32) {
    assert_eq!(space.id(0), Some(ImageId::new(first)));
    assert_eq!(space.id(last - first), Some(ImageId::new(last)));
    assert_eq!(space.id(last - first + 1), None);
}

#[test]
fn renderer_images_stay_below_master_images() {
    assert_image_space(ImageIdSpace::Renderer, 1, MASTER_IMAGE_BASE - 1);
}

#[test]
fn master_images_stay_between_renderer_and_owner_images() {
    assert_image_space(ImageIdSpace::Master, MASTER_IMAGE_BASE, OWNER_IMAGE_BASE - 1);
}

#[test]
fn owner_image_slots_have_distinct_bounds_and_refuse_slot_255() {
    for slot in 0..254 {
        let first = OWNER_IMAGE_BASE + u32::from(slot) * IMAGE_SLOT_SPAN;
        assert_image_space(ImageIdSpace::Owner(slot), first, first + IMAGE_SLOT_SPAN - 1);
    }
    assert_image_space(ImageIdSpace::Owner(254), 0xff00_0000, 0xff7f_ffff);
    assert_eq!(ImageIdSpace::Owner(255).id(0), None);
}

fn decode_image_for_slot(image: ImageId, slot: u8) -> Result<PanePicture, PictureError> {
    let mut frame = FrameGlyphBuffer::with_size(320.0, 200.0);
    frame.faces.insert(PLAIN, Face::new(PLAIN));
    frame.glyphs.push(image_glyph(OWN, image));
    let picture = PanePicture::from_frame(
        head(),
        PaneFrame { frame, animations: vec![], submitted: None },
        OWN,
    );
    PanePicture::decode(&picture.encode().unwrap(), slot)
}

fn assert_image_refused(image: ImageId, slot: u8) {
    let result = decode_image_for_slot(image, slot);
    assert!(matches!(result, Err(PictureError::ImageOutsideSlot(id)) if id == image), "{image:?}: {result:?}");
}

#[test]
fn an_image_id_outside_the_owners_slot_is_refused() {
    assert!(decode_image_for_slot(IMAGE, SLOT).is_ok());
    assert_image_refused(ImageId::new(MASTER_IMAGE_BASE + 1), SLOT);
    assert_image_refused(ImageId::new(IMAGE.get() + IMAGE_SLOT_SPAN), SLOT);
}

#[test]
fn an_image_id_in_the_owners_font_range_is_refused() {
    assert_image_refused(ImageId::new(BASE + 7), SLOT);
}

#[test]
fn master_image_ids_are_refused_in_owner_slot_127() {
    assert_image_refused(ImageId::new(MASTER_IMAGE_BASE + 1), 127);
    // The previous master allocator issued this ID in slot 127's font range.
    assert_image_refused(ImageId::new(OWNER_IMAGE_BASE + 1), 127);
}

fn assert_unsupported_glyph(glyph: FrameGlyph) {
    let mut picture = picture();
    picture.glyphs.push(glyph);
    let result = through_wire(&picture);
    assert!(matches!(result, Err(PictureError::UnsupportedGlyph)), "{result:?}");
}

#[test]
fn a_video_glyph_is_refused() {
    assert_unsupported_glyph(FrameGlyph::Video {
        window_id: MASTER, row_role: GlyphRowRole::Text, clip_rect: None, slot_id: None,
        video_id: VideoId::new(2), x: 32.0, y: 16.0, width: 32.0, height: 32.0, opacity: 0.5,
        face_id: PLAIN, box_vertical_edges: BoxVerticalEdges::Unboxed,
    });
}

#[test]
fn an_xwidget_glyph_is_refused() {
    assert_unsupported_glyph(FrameGlyph::Xwidget {
        window_id: MASTER, row_role: GlyphRowRole::Text, slot_id: None,
        xwidget_id: XwidgetId::new(3), webview_id: WebViewId::new(4),
        presentation: XwidgetPresentationGeometry::new(
            GeometryPoint::<FrameSpace, LogicalPixels>::from_px(64.0, 16.0).unwrap(),
            XwidgetContentExtent::new(60.0, 40.0).unwrap(),
            XwidgetLayoutAdvance::new(Px(64.0)).unwrap(),
            None,
        ),
        face_id: PLAIN, box_vertical_edges: BoxVerticalEdges::Unboxed,
    });
}

#[test]
fn a_surface_glyph_is_refused() {
    assert_unsupported_glyph(FrameGlyph::Surface {
        window_id: MASTER, row_role: GlyphRowRole::Text, clip_rect: None, slot_id: None,
        surface_id: SurfaceId::new(5), x: 128.0, y: 16.0, width: 32.0, height: 32.0,
        face_id: PLAIN, box_vertical_edges: BoxVerticalEdges::Unboxed,
    });
}

#[cfg(feature = "romacs-term")]
#[test]
fn a_terminal_glyph_is_refused() {
    assert_unsupported_glyph(FrameGlyph::Terminal {
        terminal_id: 1, x: 0.0, y: 0.0, width: 8.0, height: 16.0,
    });
}

#[test]
fn a_memory_backed_font_is_refused() {
    let mut picture = picture();
    let font = picture.fonts.get_mut(&ResolvedFontId(BASE)).unwrap();
    font.identity = ResolvedFontIdentity::from_memory(FontBackendKind::CoreText, "test-font".into(), 0, None);
    let asset = FontMemoryAsset::new("test-font", Arc::new(vec![0, 1, 0, 0]), 0).unwrap();
    font.replay = FontReplay::Swash { asset: FontOutlineAsset::Memory(asset) };
    let result = through_wire(&picture);
    assert!(matches!(result, Err(PictureError::MemoryFont)), "{result:?}");
}

#[test]
fn image_id_names_the_image_and_is_absent_on_a_char() {
    let picture = picture();
    let image = picture.glyphs.iter().find(|glyph| matches!(glyph, FrameGlyph::Image { .. })).unwrap();
    let character = picture.glyphs.iter().find(|glyph| matches!(glyph, FrameGlyph::Char { .. })).unwrap();
    assert_eq!(image.image_id(), Some(IMAGE));
    assert_eq!(character.image_id(), None);
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
