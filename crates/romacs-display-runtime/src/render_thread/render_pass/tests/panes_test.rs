//! O1b-3 pane pass on a real GPU: a pane draws its owner's picture in its
//! rect with its own font tables, the root's tables come back after it,
//! `cursor_off` hides its cursor, a stale catalog draws nothing, and the
//! over-pane hook runs once per entry (design O1b.md rev 3, §6.2).
use super::render_pane_layers;
use crate::pane_mailbox::Posted;
use crate::render_thread::frame_windows::GuiFrameRenderState;
use crate::render_thread::pane_layers::{DrawnPane, OverPane, SharedOverPane};
use romacs_display_protocol::font::FontCatalogGeneration;
use romacs_display_protocol::foreign_pane::ForeignPaneEntry;
use romacs_display_protocol::frame_time::observe_platform_now;
use romacs_display_protocol::pane_picture::{PaneFrame, PaneHead, PanePicture};
use romacs_display_protocol::pane_revisions::PaneRevisions;
use romacs_display_protocol::pane_sink::PaneState;
use romacs_display_protocol::{
    Color, DeviceScale, DisplayWindowId, FrameGlyphBuffer, GeometrySize, LogicalPixels, PresentMapping,
    PresentationExtent, Rect, SurfaceState,
};
use romacs_layout::LayoutEngine;
use romacs_layout_input::{
    BandStretches, CursorKinds, Decor, Face, FaceHandle, FontAlternativeState, FontSelectionState,
    FrameChrome, FrameSnapshot, Hscroll, LineWrap, NoFont, PlacedWindowSnapshot, TextSlice,
    UnderlineRules, WindowChrome, WindowSnapshot,
};
use romacs_renderer_wgpu::WgpuRenderer;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const W: u32 = 240;
const H: u32 = 120;
/// The pane's rect in the root (`in_pane` in whole px), and its owner's
/// window.
const PANE: Rect = Rect::new(120.0, 40.0, 120.0, 60.0);
const OWNER: u64 = 41;
const WINDOW: u64 = 900;
const VIEW: u64 = 11;
const OWN: u64 = 5;
const SLOT: u8 = 3;

/// A frame of one window ID, WIDTH by HEIGHT, showing TEXT with point at
/// its start.
fn snapshot(id: u64, width: u32, height: u32, text: &str) -> FrameSnapshot {
    let face = Face { decor: Decor::default(), foreground: [255; 3], background: [0; 3], family: "Liberation Mono".into(), pixel_size: 20.0, weight: 400, italic: false, width: 5 };
    let text = TextSlice { extends: vec![], raw: vec![], utf8: text.into(), face_runs: vec![] };
    let window = WindowSnapshot { underline_rules: UnderlineRules::default(), band_stretches: BandStretches::default(), id, width, height, start_byte: 0, point_byte: 0, buffer_version: 1,
        mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0), vscroll_px: 0, hscroll: Hscroll::default(),
        tab_width: 8, wrap: LineWrap::default(), cursor_off: false, cursor: CursorKinds::default(), stretches: vec![], images: vec![], wrap_flips: vec![], line_numbers: None, chrome: WindowChrome::default(),
        text, faces: vec![face] };
    FrameSnapshot { font_selection: FontSelectionState::default(), font_alternatives: FontAlternativeState::default(), fontsets: Arc::default(), width, height,
        windows: vec![PlacedWindowSnapshot { x: 0, y: 0, snapshot: window }], background_alpha: 1.0, children: vec![], tab_bar: None,
        chrome: FrameChrome::default(), status_bar: None, no_font: NoFont::default() }
}

/// The master's root: its own text, the pane's entry.
fn root(cursor_off: bool) -> FrameGlyphBuffer {
    let mut engine = LayoutEngine::new().unwrap();
    let mut root = engine.layout_frame(&snapshot(1, W, H, "root text"), 1, None).unwrap().0;
    root.foreign_panes = vec![ForeignPaneEntry { rect: PANE, window: WINDOW, owner: OWNER, view: VIEW, request: 3, cursor_off }];
    root
}

/// The owner's picture of the pane, laid out in slot `SLOT`, with its
/// cursor when CURSOR.
fn picture(cursor: bool) -> PanePicture {
    let mut engine = LayoutEngine::with_font_base(SLOT).unwrap();
    let cursor = cursor.then_some((OWN, Color::from_pixel(0x00ff_ffff)));
    let frame = engine.layout_frame(&snapshot(OWN, 120, 60, "pane"), 1, cursor).unwrap().0;
    let head = PaneHead { owner: OWNER, window: WINDOW, view: VIEW, request: 3, v: 1, c: 1 };
    PanePicture::from_frame(head, PaneFrame { frame, animations: Vec::new(), submitted: None }, DisplayWindowId::new(OWN.cast_signed()))
}

fn shown(render: &mut GuiFrameRenderState, root: &FrameGlyphBuffer, picture: PanePicture) {
    let posted = Posted { owner: OWNER, picture, revisions: Arc::new(PaneRevisions::default()) };
    assert!(render.compositor.pane_layers.ingest(Some(root), vec![posted], &mut |_, _| {}));
}

fn mapping(frame: &FrameGlyphBuffer) -> PresentMapping {
    let SurfaceState::Drawable(surface) = SurfaceState::from_device_size(W, H, DeviceScale::new(1.0).unwrap()).unwrap() else {
        unreachable!()
    };
    PresentMapping::top_left_clip(surface, PresentationExtent::new(frame.presentation_id, GeometrySize::<LogicalPixels>::from_px(frame.width, frame.height).unwrap()))
}

fn target(renderer: &WgpuRenderer) -> wgpu::Texture {
    renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("pane pass target"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// ROOT drawn as the root pass draws it, with its panes when PANES; its
/// pixels, BGRA rows.
fn draw(renderer: &mut WgpuRenderer, render: &mut GuiFrameRenderState, root: &FrameGlyphBuffer, panes: bool) -> Vec<u8> {
    let target = target(renderer);
    let view = target.create_view(&Default::default());
    let atlas = render.compositor.glyph_atlas.as_mut().unwrap();
    atlas.set_current_frame_fonts(root.font_bindings());
    renderer.render_frame_glyphs(&view, root, atlas, mapping(root), false, None, None, None, None);
    if panes {
        render_pane_layers(renderer, render, &view, root, (W, H)).unwrap();
    }
    pixels(renderer, &target)
}

fn pixels(renderer: &WgpuRenderer, target: &wgpu::Texture) -> Vec<u8> {
    let padded = (W * 4).div_ceil(256) * 256;
    let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("pane pass readback"),
        size: u64::from(padded * H),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer.device().create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(H) } },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    renderer.queue().submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    renderer.device().poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(3)) }).unwrap();
    let data = slice.get_mapped_range().unwrap();
    (0..H).flat_map(|row| data[(row * padded) as usize..(row * padded + W * 4) as usize].to_vec()).collect()
}

/// Whether pixel (X, Y) lies in the pane's rect.
fn in_pane(x: u32, y: u32) -> bool {
    (120..240).contains(&x) && (40..100).contains(&y)
}

/// The pixels of DATA inside the pane's rect, or outside it.
fn region(data: &[u8], inside: bool) -> Vec<u8> {
    (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).filter(|&(x, y)| in_pane(x, y) == inside)
        .flat_map(|(x, y)| data[((y * W + x) * 4) as usize..((y * W + x) * 4 + 4) as usize].to_vec()).collect()
}

/// HOOK as the app holds one.
fn shared(hook: OverPane) -> SharedOverPane {
    Arc::new(Mutex::new(hook))
}

fn gpu() -> (WgpuRenderer, GuiFrameRenderState) {
    let renderer = romacs_renderer_wgpu::gpu_policy::test_gpu::renderer(W, H).expect("GPU required for the pane pass");
    let render = GuiFrameRenderState::new(1, renderer.device(), 1.0, false, observe_platform_now());
    (renderer, render)
}

#[test]
fn a_pane_draws_its_slots_picture_in_its_rect_and_the_root_keeps_its_own_fonts() {
    let (mut renderer, mut render) = gpu();
    let root = root(false);
    let before = draw(&mut renderer, &mut render, &root, false);
    shown(&mut render, &root, picture(false));
    let with_pane = draw(&mut renderer, &mut render, &root, true);
    assert_eq!(region(&with_pane, false), region(&before, false), "nothing drawn outside the pane");
    assert_ne!(region(&with_pane, true), region(&before, true), "the pane's text drawn in its rect");
    // Drawn again without rebinding: the pane pass left the root's tables
    // bound and the atlas whole.
    let target = target(&renderer);
    let view = target.create_view(&Default::default());
    renderer.render_frame_glyphs(&view, &root, render.compositor.glyph_atlas.as_mut().unwrap(), mapping(&root), false, None, None, None, None);
    assert_eq!(pixels(&renderer, &target), before);
}

#[test]
fn cursor_off_hides_the_panes_cursor() {
    let (mut renderer, mut render) = gpu();
    let shows = |cursor_off: bool, cursor: bool, renderer: &mut WgpuRenderer, render: &mut GuiFrameRenderState| {
        let root = root(cursor_off);
        shown(render, &root, picture(cursor));
        draw(renderer, render, &root, true)
    };
    let none = shows(false, false, &mut renderer, &mut render);
    let on = shows(false, true, &mut renderer, &mut render);
    let off = shows(true, true, &mut renderer, &mut render);
    assert_ne!(on, none, "the cursor draws");
    assert_eq!(off, none, "cursor_off hides it");
}

#[test]
fn the_hook_runs_once_per_entry_and_a_stale_catalog_draws_no_picture() {
    let (mut renderer, mut render) = gpu();
    let mut root = root(false);
    root.foreign_panes.push(ForeignPaneEntry { rect: Rect::new(0.0, 0.0, 60.0, 30.0), window: WINDOW + 1, owner: OWNER, view: VIEW, request: 3, cursor_off: false });
    let before = draw(&mut renderer, &mut render, &root, false);
    shown(&mut render, &root, picture(false));
    let state = PaneState { code: PaneState::BUSY | PaneState::QUITS, since: observe_platform_now(), slot: SLOT };
    let mut states = crate::render_thread::pane_layers::PaneStates::default();
    states.update(vec![(OWNER, state)], |_| true);
    render.compositor.pane_layers.mirror(&states);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let record = calls.clone();
    render.compositor.pane_layers.share(&shared(Box::new(move |_, _, pane: &DrawnPane, _| {
        assert_eq!(pane.owner, OWNER);
        record.lock().unwrap().push((pane.window, pane.rect.width(), pane.picture.map(|head| head.v), pane.state));
        false
    })));
    // The root moves to another catalog: the shown picture is stale.
    let mut stale = root.clone();
    stale.font_catalog_generation = FontCatalogGeneration::from_raw(9);
    draw(&mut renderer, &mut render, &stale, true);
    assert_eq!(*calls.lock().unwrap(), vec![(WINDOW, 120.0, None, Some(state)), (WINDOW + 1, 60.0, None, Some(state))]);
    calls.lock().unwrap().clear();
    let drawn = draw(&mut renderer, &mut render, &root, true);
    assert_eq!(*calls.lock().unwrap(), vec![(WINDOW, 120.0, Some(1), Some(state)), (WINDOW + 1, 60.0, None, Some(state))], "a waiting pane gets the hook too");
    assert_ne!(region(&drawn, true), region(&before, true));
}

/// A hook still animating over a waiting pane says so, for the frame's
/// demand; one at rest does not.
#[test]
fn a_hook_still_animating_asks_for_the_next_frame() {
    let (mut renderer, mut render) = gpu();
    let root = root(false);
    let target = target(&renderer);
    let view = target.create_view(&Default::default());
    for animating in [true, false] {
        render.compositor.pane_layers.share(&shared(Box::new(move |_, _, pane: &DrawnPane, _| {
            assert_eq!(pane.picture, None, "the pane waits");
            animating
        })));
        assert_eq!(render_pane_layers(&mut renderer, &mut render, &view, &root, (W, H)).unwrap(), animating);
    }
}

/// A frame with an owner's pane never takes the retained-static path: the
/// over-pane hook runs every frame, a compositor-only one too.
#[test]
fn a_root_with_panes_never_takes_the_retained_static_path() {
    let mut render = GuiFrameRenderState::new_without_device(1, false, observe_platform_now());
    let with_panes = root(false);
    let mut without = with_panes.clone();
    without.foreign_panes.clear();
    render.compositor.current_frame = Some(Arc::new(without));
    assert!(super::super::retained_static::is_eligible(true, &[], &render), "without panes the path is open");
    render.compositor.current_frame = Some(Arc::new(with_panes));
    assert!(!super::super::retained_static::is_eligible(true, &[], &render));
}

/// The scenes a pane's animations hold leave with its layers, for the
/// renderer to free.
#[test]
fn tearing_the_panes_down_gives_back_their_animation_scenes() {
    use romacs_display_protocol::text_animation::{LogoColours, TextAnimation, TextAnimationKind};
    let (mut renderer, mut render) = gpu();
    let root = root(false);
    let mut picture = picture(false);
    let kind = TextAnimationKind::Logo { cell_aspect: 0.5, epoch: 1, colours: LogoColours::default() };
    picture.animations = vec![((1 << 32) | WINDOW, TextAnimation { window: WINDOW, top: 0, cols: 4, rows: 2, anchor: None, rise: None, fps: 25, moving: true, still: false, kind })];
    shown(&mut render, &root, picture);
    for shown in render.compositor.pane_layers.shown_all_mut() {
        shown.animations.prepare_scenes(&shown.frame, true, &mut renderer);
    }
    let settled = Arc::new(Mutex::new(0));
    let count = settled.clone();
    let scenes = render.compositor.pane_layers.clear(|_| true, &mut |_, _| *count.lock().unwrap() += 1);
    assert_eq!(scenes.len(), 1, "the logo's scene");
    assert_eq!(*settled.lock().unwrap(), 0, "a shown picture was settled when it was taken");
    for scene in scenes {
        scene.free(&mut renderer);
    }
    assert!(render.compositor.pane_layers.shown_all_mut().next().is_none());
}

// --- contract A4: an owner's death ---

/// A render thread on a GPU whose root shows two panes, the old owner's
/// (slot `SLOT`) and a new owner's: the old owner's picture drawn (its
/// fonts installed) and an image of its slot loaded; its producer, its
/// mailbox wired to the command channel as `PaintSender::pane_sink`
/// wires it, and the heads it settled.
struct DeadSlot {
    app: crate::render_thread::RenderApp,
    producer: crate::thread_comm::EmacsComms,
    font: romacs_display_protocol::font::ResolvedFontId,
    image: romacs_display_protocol::ImageId,
    settled: Arc<Mutex<Vec<u64>>>,
}

fn dead_slot_app() -> DeadSlot {
    use romacs_display_protocol::pane_picture::ImageIdSpace;
    use romacs_display_protocol::pane_sink::PaneSink;
    use romacs_display_protocol::{ImageLoadAttempt, ImageLoadToken};
    let mut app = crate::render_thread::tests::make_test_app();
    let (producer, render) = crate::thread_comm::ThreadComms::new().split();
    app.comms = render;
    assert!(app.panes.connect(producer.cmd_tx.clone()));
    let settled = Arc::new(Mutex::new(Vec::new()));
    let record = settled.clone();
    assert!(app.panes.install(Box::new(move |_, head: PaneHead| record.lock().unwrap().push(head.v))));
    let mut renderer = romacs_renderer_wgpu::gpu_policy::test_gpu::renderer(W, H).expect("GPU required for the pane pass");
    let mut root = root(false);
    let newcomer = ForeignPaneEntry { window: WINDOW + 1, owner: OWNER + 1, ..root.foreign_panes[0] };
    root.foreign_panes.push(newcomer);
    let compositor = &mut app.frame_windows.primary_window_mut().unwrap().render.compositor;
    compositor.glyph_atlas = Some(romacs_renderer_wgpu::WgpuGlyphAtlas::new(renderer.device()));
    compositor.current_frame = Some(Arc::new(root));
    let old = picture(false);
    let font = *old.fonts.keys().next().expect("the picture names a font");
    app.panes.post(OWNER, old, Arc::default());
    app.process_commands();
    let compositor = &mut app.frame_windows.primary_window_mut().unwrap().render.compositor;
    let frame = Arc::clone(&compositor.pane_layers.shown_mut(WINDOW).expect("the old owner's pane").frame);
    compositor.glyph_atlas.as_mut().unwrap().set_current_frame_fonts(frame.font_bindings());
    let image = ImageIdSpace::Owner(SLOT).id(7).unwrap();
    renderer.load_image_argb32_with_id(ImageLoadToken::new(image, ImageLoadAttempt::new(1).unwrap()), &[0; 4], 1, 1, 4);
    app.renderer = Some(renderer);
    DeadSlot { app, producer, font, image, settled }
}

fn dead() -> PaneState {
    PaneState { code: PaneState::DEAD, since: observe_platform_now(), slot: SLOT }
}

/// A load of IMAGE (attempt ATTEMPT), as an owner's receiver sends one.
fn load(image: romacs_display_protocol::ImageId, attempt: u64) -> crate::thread_comm::RenderCommand {
    use romacs_display_protocol::{ImageLoadAttempt, ImageLoadToken};
    let load = ImageLoadToken::new(image, ImageLoadAttempt::new(attempt).unwrap());
    crate::thread_comm::RenderCommand::Asset(crate::thread_comm::AssetCommand::ImageLoadDecoded { load, decoded: Arc::new(decoded()) })
}

/// Whether IMAGE is resident once every retirement is released.
fn resident(app: &mut crate::render_thread::RenderApp, image: romacs_display_protocol::ImageId) -> bool {
    let renderer = app.renderer.as_mut().unwrap();
    renderer.synchronize_retained_images(romacs_display_protocol::RetainedImageSet::default());
    renderer.image_cache().get(image).is_some()
}

/// In one round the old owner dies and the new owner of its slot posts a
/// picture naming the same font ids.
fn die_and_reuse(app: &mut crate::render_thread::RenderApp) {
    use romacs_display_protocol::pane_sink::PaneSink;
    app.panes.state(OWNER, dead());
    let mut reused = picture(false);
    reused.head.owner = OWNER + 1;
    reused.head.window = WINDOW + 1;
    app.panes.post(OWNER + 1, reused, Arc::default());
    app.process_commands();
}

#[test]
fn a_dead_slots_fonts_and_images_are_gone_before_a_new_owner_of_the_slot_draws() {
    let DeadSlot { mut app, font, image, producer: _producer, .. } = dead_slot_app();
    let atlas_has = |app: &crate::render_thread::RenderApp| {
        app.frame_windows.primary_window().unwrap().render.compositor.glyph_atlas.as_ref().unwrap().resolved_font(font).is_some()
    };
    assert!(atlas_has(&app) && app.renderer.as_ref().unwrap().image_cache().get_state(image).is_some(), "the old owner's font and image");
    die_and_reuse(&mut app);
    assert!(!atlas_has(&app), "the dead slot's font is gone before the new owner's picture draws");
    assert!(!resident(&mut app, image), "and its image");
    let layers = &mut app.frame_windows.primary_window_mut().unwrap().render.compositor.pane_layers;
    assert!(layers.shown_mut(WINDOW + 1).is_some(), "the new owner's picture is taken");
}

/// The dead owner's pane draws empty, its composed picture gone, while the
/// hook still runs over it with its DEAD state: the veil.
#[test]
fn a_dead_owner_draws_empty_under_the_hook_with_its_state() {
    let DeadSlot { mut app, producer: _producer, .. } = dead_slot_app();
    die_and_reuse(&mut app);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let record = calls.clone();
    *app.over_pane.lock().unwrap() = Box::new(move |_, _, pane: &DrawnPane, _| {
        record.lock().unwrap().push((pane.window, pane.picture.is_some(), pane.state.map(|state| state.kind())));
        false
    });
    let root = app.frame_windows.primary_window().unwrap().render.compositor.current_frame.clone().unwrap();
    let mut renderer = app.renderer.take().unwrap();
    let target = target(&renderer);
    let view = target.create_view(&Default::default());
    let render = &mut app.frame_windows.primary_window_mut().unwrap().render;
    assert!(render.compositor.pane_layers.shown_mut(WINDOW).is_none(), "the dead owner's picture is dropped");
    render_pane_layers(&mut renderer, render, &view, &root, (W, H)).unwrap();
    assert_eq!(*calls.lock().unwrap(), vec![(WINDOW, false, Some(PaneState::DEAD)), (WINDOW + 1, true, None)]);
}

/// Pixels decoded as the image path decodes them: a 4x3 SVG.
fn decoded() -> romacs_renderer_wgpu::image_cache::SemanticImageDecoded {
    use romacs_display_protocol::image_diagnostic::{ImageDiagnosticSubject, ImageFormatName, ImageLoadIdentity};
    use romacs_display_protocol::*;
    use romacs_renderer_wgpu::image_cache::{SemanticImageDecoder, SemanticImageRequest, SemanticImageSource};
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="3"/>"#.to_vec();
    SemanticImageDecoder::default().decode_pixels(SemanticImageRequest {
        source: SemanticImageSource::Data { data: EncodedBytes::new(svg), resources: romacs_renderer_wgpu::svg::SvgResourceContext::Isolated },
        size: ImageSizeSpec::new(AxisSize::resolve(None, None), AxisSize::resolve(None, None)), rotation: ImageRotation::None,
        realization: ImageRealization::new(1.0, 1.0, 1.0), colors: ImageColorContext::from_pixels(0, 0xffffff),
        mask: ImageMaskPolicy::Preserve, animation: ImageAnimationPolicy::disabled(), frame: ImageFrameIndex::new(0),
        sequence: ImageSequenceId::new(1).unwrap(), limit: ImageSizeLimit::UNLIMITED,
        identity: ImageLoadIdentity::new(ImageFormatName::from_lisp_type("svg"), ImageDiagnosticSubject::Spec("test".into())),
    }).unwrap()
}

/// The DEAD lands just after a round's take: its slot's retirement and
/// then the new owner's load of a reused image id drain in that round, and
/// the DEAD itself is taken in the next. The retirement rides the command
/// channel with the uploads, so the new owner's image stays.
#[test]
fn a_death_after_the_take_keeps_the_new_owners_load() {
    use romacs_display_protocol::pane_sink::PaneSink;
    let DeadSlot { mut app, producer, image, .. } = dead_slot_app();
    let (mailbox, commands) = (app.panes.clone(), producer.cmd_tx.clone());
    let mut once = Some(());
    *app.panes.after_take.lock().unwrap() = Some(Box::new(move || {
        if once.take().is_some() {
            mailbox.state(OWNER, dead());
            commands.try_send(load(image, 2)).unwrap();
        }
    }));
    app.process_commands();
    app.process_commands();
    assert!(resident(&mut app, image), "the new owner's image is resident");
}

/// The dead owner's upload, picture and DEAD all precede one take: its
/// pixels are retired after they land, and the picture is settled.
#[test]
fn a_death_behind_its_own_upload_retires_the_upload() {
    use romacs_display_protocol::pane_picture::ImageIdSpace;
    use romacs_display_protocol::pane_sink::PaneSink;
    let DeadSlot { mut app, producer, settled, .. } = dead_slot_app();
    let fresh = ImageIdSpace::Owner(SLOT).id(8).unwrap();
    producer.cmd_tx.try_send(load(fresh, 1)).unwrap();
    let mut late = picture(false);
    late.head.v = 2;
    app.panes.post(OWNER, late, Arc::default());
    app.panes.state(OWNER, dead());
    app.process_commands();
    assert!(!resident(&mut app, fresh), "the dead owner's pixels are gone");
    assert!(settled.lock().unwrap().contains(&2), "its picture is settled");
}

/// The DEAD lands just after a round's take; that round's drain meets the
/// death, then the new owner's load of a reused image id, while the root
/// still names the dead owner's stand-in. From the death on, no frame
/// draws the dead owner's picture: its pane draws nothing over the new
/// owner's pixels.
#[test]
fn no_frame_after_a_death_draws_the_dead_owners_picture() {
    use romacs_display_protocol::pane_sink::PaneSink;
    let DeadSlot { mut app, producer, image, .. } = dead_slot_app();
    let (mailbox, commands) = (app.panes.clone(), producer.cmd_tx.clone());
    let mut once = Some(());
    *app.panes.after_take.lock().unwrap() = Some(Box::new(move || {
        if once.take().is_some() {
            mailbox.state(OWNER, dead());
            commands.try_send(load(image, 2)).unwrap();
        }
    }));
    app.process_commands();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let record = calls.clone();
    *app.over_pane.lock().unwrap() = Box::new(move |_, _, pane: &DrawnPane, _| {
        record.lock().unwrap().push((pane.window, pane.picture.is_some()));
        false
    });
    let root = app.frame_windows.primary_window().unwrap().render.compositor.current_frame.clone().unwrap();
    assert!(root.foreign_panes.iter().any(|entry| entry.owner == OWNER), "the root still names the dead owner's stand-in");
    let mut renderer = app.renderer.take().unwrap();
    let target = target(&renderer);
    let view = target.create_view(&Default::default());
    let render = &mut app.frame_windows.primary_window_mut().unwrap().render;
    render_pane_layers(&mut renderer, render, &view, &root, (W, H)).unwrap();
    assert!(calls.lock().unwrap().contains(&(WINDOW, false)), "the dead owner's pane draws no picture");
    app.renderer = Some(renderer);
    assert!(resident(&mut app, image), "the new owner's pixels are resident");
}
