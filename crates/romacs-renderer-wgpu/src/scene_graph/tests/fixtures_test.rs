//! Screenshot fixtures for the scene pass: one hand-built draw list per node
//! kind, rendered headlessly on the software Vulkan adapter (lavapipe) and
//! compared with the PNGs beside this file.
//!
//! The fixtures are small Ember Lattice pages in the Omarchy "rum" palette:
//! light rather than chrome, one hero effect each, quiet everywhere else.
//!
//! Run with a lavapipe ICD visible to the Vulkan loader, for example
//! `VK_ADD_DRIVER_FILES=$MESA/share/vulkan/icd.d/lvp_icd.x86_64.json`.
//! Without a CPU adapter the comparisons skip (and say so); hardware output
//! differs in the last bits and is not what the references record.
//! `ROMACS_SCENE_BLESS=1` rewrites the references.

use super::draw::*;
use super::*;
use crate::glyph_atlas::WgpuGlyphAtlas;
use crate::image_cache::ImageCache;
use crate::shader_surface::SurfaceShaderLanguage;
use crate::shader_surface_cache::ShaderSurfaceCache;
use kurbo::{Affine, BezPath, Circle, Point, Rect, Shape, Size, Vec2};
use romacs_display_protocol::FaceId;
use romacs_display_protocol::face::Face;
use romacs_display_protocol::font::{
    FontFileAsset, FontOutlineAsset, FontReplay, FontSlantKind, ResolvedFont, ResolvedFontId,
    ResolvedFontIdentity, ResolvedFontTable,
};
use std::path::PathBuf;
use std::sync::Arc;

// ─── The rum palette ────────────────────────────────────────────────

const BG: Color = Color::rgb8(0x00, 0x00, 0x00);
const DARK_BG: Color = Color::rgb8(0x0a, 0x0a, 0x0a);
const DARKER_BG: Color = Color::rgb8(0x05, 0x04, 0x03);
const LIGHTER_BG: Color = Color::rgb8(0x11, 0x11, 0x11);
const FG: Color = Color::rgb8(0xd4, 0xc5, 0xa0);
const DIM_FG: Color = Color::rgb8(0x77, 0x6d, 0x5c);
const BRIGHT: Color = Color::rgb8(0xf4, 0xe7, 0xc3);
const ACCENT: Color = Color::rgb8(0xd4, 0x8a, 0x20);
const SELECTION: Color = Color::rgb8(0x3a, 0x29, 0x14);
const MUTED: Color = Color::rgb8(0x75, 0x6b, 0x59);
const GREEN: Color = Color::rgb8(0x9c, 0xbd, 0x6f);
const RED: Color = Color::rgb8(0xff, 0x5f, 0x59);

const W: f64 = 480.0;
const H: f64 = 300.0;

const MONO: &str = "JetBrainsMono Nerd Font";
const BRAILLE: &str = "PlemolJP Console NF";
const ARABIC: &str = "Noto Sans Arabic";

/// No pinned test font carries Arabic; the joining test uses the system's
/// Noto Sans Arabic when it is installed.
fn arabic_font() -> Option<PathBuf> {
    let path = PathBuf::from("/usr/share/fonts/noto/NotoSansArabic-Regular.ttf");
    path.exists().then_some(path)
}

// ─── Harness ────────────────────────────────────────────────────────

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    atlas: WgpuGlyphAtlas,
    pass: ScenePass,
    fonts: Vec<(Arc<str>, SceneFont)>,
    software: bool,
}

fn software_adapter() -> Option<wgpu::Adapter> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
        .into_iter()
        .find(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
}

/// One software device for the whole test binary: the Vulkan loader crashes
/// when test threads create instances concurrently.
fn software_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    static DEVICE: std::sync::OnceLock<Option<(wgpu::Device, wgpu::Queue)>> =
        std::sync::OnceLock::new();
    DEVICE
        .get_or_init(|| software_adapter().map(|adapter| request_device(&adapter)))
        .clone()
}

fn request_device(adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("scene fixtures"),
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        required_limits: wgpu::Limits::default(),
        memory_hints: Default::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        trace: wgpu::Trace::Off,
    }))
    .expect("scene fixture device")
}

fn hardware_adapter() -> Option<wgpu::Adapter> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .ok()
    .filter(|a| a.get_info().device_type != wgpu::DeviceType::Cpu)
}

fn gpu((device, queue): (wgpu::Device, wgpu::Queue), software: bool) -> Gpu {
    let mut atlas = WgpuGlyphAtlas::new(&device);
    let fonts = install_fonts(&mut atlas);
    let pass = ScenePass::new(&device, &queue, atlas.bind_group_layout());
    Gpu {
        software,
        device,
        queue,
        atlas,
        pass,
        fonts,
    }
}

fn software_gpu() -> Option<Gpu> {
    let device = software_device();
    if device.is_none() {
        eprintln!("SKIP: no software (lavapipe) Vulkan adapter; scene fixtures need one");
    }
    device.map(|device| gpu(device, true))
}

/// Pinned fonts installed the way a frame's layout installs its fonts.
fn install_fonts(atlas: &mut WgpuGlyphAtlas) -> Vec<(Arc<str>, SceneFont)> {
    let specs = [
        (
            MONO,
            romacs_test_fonts::jetbrains_mono_nerd_font_regular_3_4_0().to_path_buf(),
            ResolvedFontId(901),
            0.6,
        ),
        (
            BRAILLE,
            romacs_test_fonts::plemol_jp_console_nf_regular().to_path_buf(),
            ResolvedFontId(902),
            0.5,
        ),
    ]
    .into_iter()
    .chain(arabic_font().map(|path| (ARABIC, path, ResolvedFontId(903), 0.3)))
    .collect::<Vec<_>>();
    let mut table = ResolvedFontTable::default();
    for (family, path, id, _) in &specs {
        let identity = ResolvedFontIdentity::from_file(&path.to_string_lossy(), 0, None);
        table.insert(
            *id,
            ResolvedFont {
                id: *id,
                replay: FontReplay::Swash {
                    asset: FontOutlineAsset::File(
                        FontFileAsset::from_identity(&identity).expect("file-backed font"),
                    ),
                },
                identity,
                family: (*family).to_owned(),
                full_name: None,
                postscript_name: None,
                weight: 400,
                slant: FontSlantKind::Normal,
                width: 5,
                pixel_size: 16.0,
                ascent_px: 13.0,
                descent_px: 3.0,
                space_advance_px: 8.0,
                glyph_advance: Default::default(),
            },
        );
    }
    atlas.set_current_frame_fonts(
        romacs_display_protocol::FrameGlyphBuffer::default().font_bindings(),
    );
    atlas.install_frame_fonts(
        &Default::default(),
        &table,
        &Default::default(),
        &Default::default(),
    );
    specs
        .iter()
        .enumerate()
        .map(|(n, (family, _, id, advance))| {
            let mut face = Face::new(FaceId::new(900 + n as u32));
            face.font_family = (*family).to_owned();
            face.default_resolved_font_id = Some(*id);
            let font = SceneFont {
                face,
                ascent: 0.0,
                space_advance: *advance,
            };
            (Arc::<str>::from(*family), font)
        })
        .collect()
}

/// Metrics scale with the requested size: `ascent` and `space_advance` are
/// stored per pixel of size.
fn lookup(fonts: &[(Arc<str>, SceneFont)]) -> impl Fn(&Font) -> Option<SceneFont> + '_ {
    |font: &Font| {
        let (_, found) = fonts.iter().find(|(family, _)| **family == *font.family)?;
        let mut realised = found.clone();
        realised.face.font_size = font.size as f32;
        realised.ascent = (font.size * 0.8) as f32;
        realised.space_advance = (font.size * f64::from(found.space_advance)) as f32;
        Some(realised)
    }
}

struct Rendered {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    stats: SceneStats,
}

fn render(gpu: &mut Gpu, list: &DrawList, scale: f32, sources: &dyn SceneSources) -> Rendered {
    gpu.atlas.set_scale_factor(scale);
    let fonts = lookup(&gpu.fonts);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let stats = gpu
        .pass
        .render(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            list,
            scale,
            SceneInputs {
                atlas: &mut gpu.atlas,
                fonts: &fonts,
                sources,
            },
        )
        .expect("fixture list is valid");
    let root = list.targets.last().expect("root").id;
    let texture = gpu.pass.texture(root).expect("root texture").clone();
    let (width, height) = (texture.width(), texture.height());
    let row = (width * 4).div_ceil(256) * 256;
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scene fixture readback"),
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    gpu.queue.submit([encoder.finish()]);
    gpu.pass.submitted();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    wait(&gpu.device);
    let mapped = readback.slice(..).get_mapped_range().expect("mapped");
    let rgba = mapped
        .chunks(row as usize)
        .flat_map(|line| &line[..(width * 4) as usize])
        .copied()
        .collect();
    Rendered {
        width,
        height,
        rgba,
        stats,
    }
}

fn wait(device: &wgpu::Device) {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll");
}

fn reference_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/scene_graph/tests/fixtures")
        .join(format!("{name}.png"))
}

fn write_png(path: &std::path::Path, image: &Rendered) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("fixture dir");
    let file = std::fs::File::create(path).expect("png file");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(&image.rgba).expect("png data");
}

fn read_png(path: &std::path::Path) -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

/// Pixels whose largest channel difference exceeds this count as changed.
const CHANNEL_TOLERANCE: u8 = 4;
/// Share of changed pixels a comparison allows.
const PIXEL_TOLERANCE: f64 = 0.002;

/// Render `list` twice (the second must be byte-identical: no hidden state)
/// and compare it with the checked-in reference.
fn check(name: &str, gpu: &mut Gpu, list: &DrawList, sources: &dyn SceneSources) {
    let first = render(gpu, list, 1.0, sources);
    let second = render(gpu, list, 1.0, sources);
    assert!(
        first.rgba == second.rgba,
        "{name}: re-rendering changed pixels"
    );
    assert!(
        first.stats.skipped.is_empty(),
        "{name}: skipped {:?}",
        first.stats.skipped
    );
    compare(name, &first);
}

fn check_fixture(name: &str, list: &DrawList) {
    if let Some(mut gpu) = software_gpu() {
        check(name, &mut gpu, list, &());
    }
}

fn compare(name: &str, image: &Rendered) {
    let path = reference_path(name);
    if std::env::var_os("ROMACS_SCENE_BLESS").is_some() {
        write_png(&path, image);
        eprintln!("blessed {}", path.display());
    }
    let (width, height, reference) =
        read_png(&path).unwrap_or_else(|| panic!("{name}: no reference at {}", path.display()));
    assert_eq!((width, height), (image.width, image.height), "{name}: size");
    let changed = reference
        .chunks(4)
        .zip(image.rgba.chunks(4))
        .filter(|(a, b)| {
            a.iter()
                .zip(*b)
                .any(|(x, y)| x.abs_diff(*y) > CHANNEL_TOLERANCE)
        })
        .count();
    let share = changed as f64 / f64::from(width * height);
    if share > PIXEL_TOLERANCE {
        let actual = std::env::temp_dir().join(format!("romacs-scene-{name}.png"));
        write_png(&actual, image);
        panic!(
            "{name}: {changed} pixels ({:.3}%) differ from {}; actual at {}",
            share * 100.0,
            path.display(),
            actual.display()
        );
    }
}

// ─── Draw-list builders ─────────────────────────────────────────────

fn item(node: NodeId, paint: Paint) -> Item {
    Item {
        node,
        transform: Affine::IDENTITY,
        opacity: 1.0,
        clip: None,
        paint,
    }
}

fn rect(r: Rect, radius: f64, fill: Color) -> RectPaint {
    RectPaint {
        rect: r,
        radius,
        fill: Some(fill),
        stroke: None,
        shadow: None,
        glow: None,
    }
}

fn soft_shadow() -> Shadow {
    Shadow {
        offset: Vec2::new(0.0, 10.0),
        blur: 28.0,
        spread: -2.0,
        color: BG.with_alpha(0.9),
    }
}

fn text(node: NodeId, x: f64, baseline: f64, size: f64, color: Color, s: &str) -> Item {
    item(
        node,
        Paint::Text(TextPaint {
            origin: Point::new(x, baseline),
            text: s.into(),
            font: mono(size),
            color,
        }),
    )
}

fn mono(size: f64) -> Font {
    Font {
        family: MONO.into(),
        size,
        weight: 400,
        italic: false,
    }
}

fn root(items: Vec<Item>) -> Target {
    Target {
        id: TargetId(0),
        size: Size::new(W, H),
        clear: BG,
        items,
    }
}

/// A faint ambient wash so pure black reads as depth, not void.
fn ambient(node: NodeId) -> Item {
    item(
        node,
        Paint::Rect(RectPaint {
            rect: Rect::new(-40.0, H * 0.55, W + 40.0, H + 80.0),
            radius: 120.0,
            fill: None,
            stroke: None,
            shadow: Some(Shadow {
                offset: Vec2::ZERO,
                blur: 160.0,
                spread: 0.0,
                color: SELECTION.with_alpha(0.2),
            }),
            glow: None,
        }),
    )
}

/// Two layer cards: an idle one, and a focused one that emits light.
fn rect_fixture() -> DrawList {
    let card = |x: f64, focused: bool| RectPaint {
        rect: Rect::new(x, 44.0, x + 196.0, 256.0),
        radius: 14.0,
        fill: Some(DARK_BG),
        stroke: Some(Stroke {
            width: 1.0,
            color: if focused {
                ACCENT.with_alpha(0.45)
            } else {
                LIGHTER_BG
            },
        }),
        shadow: Some(soft_shadow()),
        glow: focused.then_some(Glow {
            radius: 18.0,
            color: ACCENT.with_alpha(0.12),
        }),
    };
    let line = |node, x: f64, y: f64, w: f64, color: Color| {
        item(
            node,
            Paint::Rect(rect(Rect::new(x, y, x + w, y + 6.0), 3.0, color)),
        )
    };
    let mut items = vec![
        ambient(1),
        item(2, Paint::Rect(card(24.0, false))),
        item(3, Paint::Rect(card(260.0, true))),
    ];
    for (n, (w, alpha)) in [(132.0, 0.55), (96.0, 0.35), (150.0, 0.35), (80.0, 0.25)]
        .into_iter()
        .enumerate()
    {
        let y = 76.0 + n as f64 * 22.0;
        items.push(line(10 + n as u32, 44.0, y, w, MUTED.with_alpha(alpha)));
        items.push(line(20 + n as u32, 280.0, y, w, MUTED.with_alpha(alpha)));
    }
    // The selection band in the focused card: warm, lit from within.
    items.push(item(
        30,
        Paint::Rect(RectPaint {
            rect: Rect::new(272.0, 158.0, 444.0, 182.0),
            radius: 6.0,
            fill: Some(SELECTION),
            stroke: None,
            shadow: None,
            glow: Some(Glow {
                radius: 8.0,
                color: ACCENT.with_alpha(0.14),
            }),
        }),
    ));
    items.push(line(31, 284.0, 167.0, 110.0, FG.with_alpha(0.85)));
    // Status lights: ok glows, danger stays a plain dot.
    let dot = |node, cx: f64, color: Color, glow: bool| {
        item(
            node,
            Paint::Rect(RectPaint {
                rect: Rect::new(cx - 4.0, 226.0, cx + 4.0, 234.0),
                radius: 4.0,
                fill: Some(color),
                stroke: None,
                shadow: None,
                glow: glow.then_some(Glow {
                    radius: 9.0,
                    color: color.with_alpha(0.5),
                }),
            }),
        )
    };
    items.push(dot(40, 288.0, GREEN, true));
    items.push(dot(41, 304.0, RED, false));
    items.push(dot(42, 52.0, MUTED, false));
    DrawList {
        targets: vec![root(items)],
    }
}

fn spark_points() -> Vec<Point> {
    (0..48)
        .map(|i| {
            let t = f64::from(i);
            let v = 0.55 + 0.22 * (t * 0.31).sin() + 0.12 * (t * 0.83 + 1.0).sin() - 0.004 * t;
            Point::new(44.0 + t * 5.6, 70.0 + (1.0 - v) * 140.0)
        })
        .collect()
}

fn polyline(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for p in &points[1..] {
        path.line_to(*p);
    }
    path
}

fn stroke(width: f64, color: Color) -> PathStroke {
    PathStroke {
        width,
        color,
        join: Join::Round,
        cap: Cap::Round,
        dash: Vec::new(),
        dash_offset: 0.0,
    }
}

fn path_item(node: NodeId, path: BezPath, fill: Option<Fill>, stroke: Option<PathStroke>) -> Item {
    item(
        node,
        Paint::Path(PathPaint {
            path: Arc::new(path),
            revision: 1,
            fill,
            stroke,
        }),
    )
}

fn arc(center: Point, radius: f64, from_deg: f64, sweep_deg: f64) -> BezPath {
    kurbo::Arc::new(
        center,
        Vec2::new(radius, radius),
        from_deg.to_radians(),
        sweep_deg.to_radians(),
        0.0,
    )
    .into_path(0.05)
}

/// A dashboard panel: a review streak sparkline and a retention gauge.
fn path_fixture() -> DrawList {
    let points = spark_points();
    let mut area = polyline(&points);
    area.line_to(Point::new(points.last().expect("points").x, 220.0));
    area.line_to(Point::new(points[0].x, 220.0));
    area.close_path();
    let mut baseline = BezPath::new();
    baseline.move_to((44.0, 220.0));
    baseline.line_to((312.0, 220.0));
    let mut dashed = stroke(1.0, MUTED.with_alpha(0.6));
    dashed.cap = Cap::Butt;
    dashed.dash = vec![3.0, 5.0];
    let gauge = Point::new(392.0, 150.0);
    // An even-odd orbit ring: one path, two circles, a hole.
    let mut ring = Circle::new((392.0, 150.0), 30.0).to_path(0.05);
    ring.extend(Circle::new((392.0, 150.0), 26.0).to_path(0.05));
    let last = *points.last().expect("points");
    let items = vec![
        ambient(1),
        item(
            2,
            Paint::Rect(RectPaint {
                rect: Rect::new(20.0, 28.0, 460.0, 272.0),
                radius: 16.0,
                fill: Some(DARKER_BG),
                stroke: Some(Stroke {
                    width: 1.0,
                    color: LIGHTER_BG,
                }),
                shadow: Some(soft_shadow()),
                glow: None,
            }),
        ),
        path_item(3, baseline, None, Some(dashed)),
        path_item(
            4,
            area,
            Some(Fill {
                color: ACCENT.with_alpha(0.025),
                rule: FillRule::NonZero,
            }),
            None,
        ),
        path_item(5, polyline(&points), None, Some(stroke(2.0, ACCENT))),
        item(
            6,
            Paint::Rect(RectPaint {
                rect: Rect::new(last.x - 4.0, last.y - 4.0, last.x + 4.0, last.y + 4.0),
                radius: 4.0,
                fill: Some(BRIGHT),
                stroke: None,
                shadow: None,
                glow: Some(Glow {
                    radius: 12.0,
                    color: ACCENT.with_alpha(0.6),
                }),
            }),
        ),
        path_item(
            7,
            arc(gauge, 52.0, 135.0, 270.0),
            None,
            Some(stroke(7.0, MUTED.with_alpha(0.25))),
        ),
        path_item(
            8,
            arc(gauge, 52.0, 135.0, 270.0 * 0.72),
            None,
            Some(stroke(7.0, ACCENT)),
        ),
        path_item(
            9,
            ring,
            Some(Fill {
                color: FG.with_alpha(0.35),
                rule: FillRule::EvenOdd,
            }),
            None,
        ),
    ];
    DrawList {
        targets: vec![root(items)],
    }
}

/// A landing page's text: wordmark, tagline, one lit command row.
fn text_fixture() -> DrawList {
    let items = vec![
        ambient(1),
        text(2, 40.0, 92.0, 44.0, BRIGHT, "ROMACS"),
        text(3, 42.0, 124.0, 15.0, FG, "a lisp machine, lit from within"),
        item(
            4,
            Paint::Rect(RectPaint {
                rect: Rect::new(32.0, 160.0, 448.0, 188.0),
                radius: 6.0,
                fill: Some(SELECTION),
                stroke: None,
                shadow: None,
                glow: Some(Glow {
                    radius: 8.0,
                    color: ACCENT.with_alpha(0.12),
                }),
            }),
        ),
        text(5, 44.0, 179.0, 14.0, FG, "find-file"),
        text(6, 360.0, 179.0, 14.0, ACCENT, "C-x C-f"),
        text(7, 44.0, 215.0, 14.0, DIM_FG, "switch-to-buffer"),
        text(8, 360.0, 215.0, 14.0, MUTED, "C-x b"),
        text(9, 44.0, 251.0, 14.0, DIM_FG, "project-find-file"),
        text(10, 360.0, 251.0, 14.0, MUTED, "C-x p f"),
    ];
    DrawList {
        targets: vec![root(items)],
    }
}

/// Braille rings of an armillary sphere: three tilted ellipses on a dot
/// grid, shaded by depth (behind the sphere dim, in front bright).
fn armillary_cells(cols: usize, rows: usize) -> (Vec<char>, Vec<u8>) {
    let (dots_w, dots_h) = (cols * 2, rows * 4);
    let mut depth = vec![None::<f64>; dots_w * dots_h];
    let (cx, cy) = (dots_w as f64 / 2.0, dots_h as f64 / 2.0);
    let sphere = dots_h as f64 * 0.42;
    // (tilt, yaw, radius): a ring of tilt 0 faces the viewer, π/2 lies flat.
    // Equator, two meridians a quarter turn apart, and the ecliptic band
    // reaching past the sphere; the whole sphere pitched toward the viewer.
    // Braille dots are square at a 1:2 cell.
    use std::f64::consts::FRAC_PI_2;
    let rings = [
        (FRAC_PI_2, 0.0_f64, sphere),
        (0.0, 0.5, sphere),
        (0.0, 0.5 + FRAC_PI_2, sphere),
        (FRAC_PI_2 - 0.41, 0.3, sphere * 1.45),
    ];
    let pitch = 0.38_f64;
    for (tilt, yaw, radius) in rings {
        for step in 0..2000 {
            let t = f64::from(step) / 2000.0 * std::f64::consts::TAU;
            let (x, y, z) = (t.cos(), t.sin() * tilt.cos(), t.sin() * tilt.sin());
            let (x, z) = (
                x * yaw.cos() + z * yaw.sin(),
                -x * yaw.sin() + z * yaw.cos(),
            );
            let (y, z) = (
                y * pitch.cos() - z * pitch.sin(),
                y * pitch.sin() + z * pitch.cos(),
            );
            let px = (cx + x * radius).round() as isize;
            let py = (cy - y * radius).round() as isize;
            if px < 0 || py < 0 || px as usize >= dots_w || py as usize >= dots_h {
                continue;
            }
            let slot = &mut depth[py as usize * dots_w + px as usize];
            *slot = Some(slot.map_or(z, |d: f64| d.max(z)));
        }
    }
    // Braille dot numbering: column-major 1-2-3-7 / 4-5-6-8.
    const BITS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    let mut glyphs = Vec::with_capacity(cols * rows);
    let mut shades = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        for col in 0..cols {
            let mut bits = 0;
            let mut nearest = f64::NEG_INFINITY;
            for (dy, line) in BITS.iter().enumerate() {
                for (dx, bit) in line.iter().enumerate() {
                    if let Some(z) = depth[(row * 4 + dy) * dots_w + col * 2 + dx] {
                        bits |= bit;
                        nearest = nearest.max(z);
                    }
                }
            }
            glyphs.push(if bits == 0 {
                ' '
            } else {
                char::from_u32(0x2800 + bits).expect("braille")
            });
            shades.push(match nearest {
                z if z > 0.35 => 2,
                z if z > -0.35 => 1,
                _ => 0,
            });
        }
    }
    (glyphs, shades)
}

fn cells_fixture() -> DrawList {
    let (cols, rows) = (48, 14);
    let (glyphs, shades) = armillary_cells(cols, rows);
    let cell = Size::new(8.0, 16.0);
    let grid_w = cols as f64 * cell.width;
    let items = vec![
        ambient(1),
        item(
            2,
            Paint::Cells(CellsPaint {
                origin: Point::new((W - grid_w) / 2.0, 22.0),
                cols: cols as u16,
                rows: rows as u16,
                cell,
                glyphs: glyphs.into(),
                shades: shades.into(),
                palette: vec![MUTED.with_alpha(0.55), ACCENT, BRIGHT].into(),
                font: Font {
                    family: BRAILLE.into(),
                    size: 16.0,
                    weight: 400,
                    italic: false,
                },
            }),
        ),
        text(3, 177.0, 280.0, 14.0, DIM_FG, "preparing world"),
    ];
    DrawList {
        targets: vec![root(items)],
    }
}

const PANEL: TargetId = TargetId(2);
const BADGE: TargetId = TargetId(3);

/// Target groups: a panel group that samples its *later* sibling, a badge
/// group, so the list carries the badge first; the root shows the panel and
/// a reflection of it.
fn group_fixture(badge_color: Color) -> DrawList {
    let badge = Target {
        id: BADGE,
        size: Size::new(96.0, 40.0),
        clear: Color::TRANSPARENT,
        items: vec![
            item(
                31,
                Paint::Rect(RectPaint {
                    rect: Rect::new(4.0, 4.0, 92.0, 36.0),
                    radius: 16.0,
                    fill: Some(badge_color.with_alpha(0.16)),
                    stroke: Some(Stroke {
                        width: 1.0,
                        color: badge_color.with_alpha(0.7),
                    }),
                    shadow: None,
                    glow: None,
                }),
            ),
            text(32, 22.0, 25.0, 13.0, badge_color, "λ live"),
        ],
    };
    let panel = Target {
        id: PANEL,
        size: Size::new(320.0, 120.0),
        clear: Color::TRANSPARENT,
        items: vec![
            item(
                21,
                Paint::Rect(RectPaint {
                    rect: Rect::new(8.0, 8.0, 312.0, 112.0),
                    radius: 14.0,
                    fill: Some(DARK_BG),
                    stroke: Some(Stroke {
                        width: 1.0,
                        color: ACCENT.with_alpha(0.45),
                    }),
                    shadow: None,
                    glow: None,
                }),
            ),
            text(22, 28.0, 52.0, 22.0, BRIGHT, "*scratch*"),
            text(23, 28.0, 84.0, 13.0, DIM_FG, "lisp-interaction-mode"),
            item(
                24,
                Paint::Sample(SamplePaint {
                    source: Source::Target(BADGE),
                    rect: Rect::new(204.0, 64.0, 300.0, 104.0),
                    radius: 0.0,
                    channels: Arc::default(),
                }),
            ),
        ],
    };
    let shown = Rect::new(80.0, 40.0, 400.0, 160.0);
    let mirror =
        Affine::translate((0.0, 2.0 * shown.y1 + 8.0)) * Affine::scale_non_uniform(1.0, -1.0);
    let sample = |node, transform, opacity| Item {
        node,
        transform,
        opacity,
        clip: None,
        paint: Paint::Sample(SamplePaint {
            source: Source::Target(PANEL),
            rect: shown,
            radius: 14.0,
            channels: Arc::default(),
        }),
    };
    let mut reflection = sample(3, mirror, 0.18);
    reflection.clip = Some(Rect::new(0.0, shown.y1 + 8.0, W, shown.y1 + 70.0));
    let items = vec![
        ambient(1),
        item(
            2,
            Paint::Rect(RectPaint {
                rect: shown.inset(-8.0),
                radius: 14.0,
                fill: None,
                stroke: None,
                shadow: Some(soft_shadow()),
                glow: Some(Glow {
                    radius: 16.0,
                    color: ACCENT.with_alpha(0.11),
                }),
            }),
        ),
        sample(4, Affine::IDENTITY, 1.0),
        reflection,
    ];
    DrawList {
        targets: vec![badge, panel, root(items)],
    }
}

/// A Bayer-dithered warm gradient, as an image cache would hold a decoded
/// picture: muted ink on near black.
fn dither_image(width: u32, height: u32) -> Vec<u8> {
    const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let (u, v) = (
                f64::from(x) / f64::from(width),
                f64::from(y) / f64::from(height),
            );
            let d = ((u - 0.3).powi(2) + (v - 0.35).powi(2)).sqrt();
            let level = (1.0 - d * 1.4).clamp(0.0, 1.0);
            let threshold = f64::from(BAYER[(y % 4) as usize][(x % 4) as usize]) / 16.0;
            let on = level > threshold;
            let [r, g, b] = if on {
                [0x75, 0x6b, 0x59]
            } else {
                [0x0a, 0x0a, 0x0a]
            };
            rgb.extend([r, g, b]);
        }
    }
    rgb
}

/// A static shader surface: slow plasma bands quantised to three rum tones.
const PLASMA_WGSL: &str = "
fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
    let uv = fragCoord / u.iResolution.xy;
    let v = sin(uv.x * 9.0 + sin(uv.y * 5.0) * 1.7) + sin((uv.x + uv.y) * 6.0);
    let band = floor((v * 0.25 + 0.5) * 3.0);
    let ink = select(vec3<f32>(0.0015, 0.0012, 0.0009),
                     select(vec3<f32>(0.042, 0.022, 0.007), vec3<f32>(0.254, 0.102, 0.031), band > 1.5),
                     band > 0.5);
    return vec4<f32>(ink, 1.0);
}";

const IMAGE_ID: u32 = 71;
const SURFACE_ID: u32 = 72;

fn media_fixture() -> DrawList {
    let frame = |node, x: f64| {
        item(
            node,
            Paint::Rect(RectPaint {
                rect: Rect::new(x, 48.0, x + 200.0, 228.0),
                radius: 14.0,
                fill: None,
                stroke: None,
                shadow: Some(soft_shadow()),
                glow: None,
            }),
        )
    };
    let picture = |node, x: f64, source| {
        item(
            node,
            Paint::Sample(SamplePaint {
                source,
                rect: Rect::new(x, 48.0, x + 200.0, 228.0),
                radius: 14.0,
                channels: Arc::default(),
            }),
        )
    };
    let items = vec![
        ambient(1),
        frame(2, 28.0),
        frame(3, 252.0),
        picture(4, 28.0, Source::Image(IMAGE_ID)),
        picture(5, 252.0, Source::Surface(SURFACE_ID)),
        text(6, 32.0, 262.0, 13.0, DIM_FG, "image"),
        text(7, 256.0, 262.0, 13.0, DIM_FG, "shader"),
    ];
    DrawList {
        targets: vec![root(items)],
    }
}

/// The media fixture's sources, made through the renderer's own caches.
fn media_sources(gpu: &Gpu) -> (ImageCache, ShaderSurfaceCache) {
    let mut images = ImageCache::new(&gpu.device);
    let id = romacs_display_protocol::ImageId::new(IMAGE_ID);
    images.load_raw_rgb24_with_id(
        romacs_display_protocol::ImageLoadToken::new(
            id,
            romacs_display_protocol::ImageLoadAttempt::new(1).expect("attempt"),
        ),
        &dither_image(200, 180),
        200,
        180,
        600,
    );
    for _ in 0..500 {
        images.process_pending(&gpu.device, &gpu.queue);
        if images.is_ready(id) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(images.is_ready(id), "fixture image decoded");
    let mut surfaces = ShaderSurfaceCache::new(&gpu.device);
    surfaces
        .create_shader(
            &gpu.device,
            images.bind_group_layout(),
            images.sampler(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
            SURFACE_ID,
            SurfaceShaderLanguage::Wgsl,
            PLASMA_WGSL,
            &[],
            200,
            180,
            1.0,
            false,
            None,
            None,
        )
        .expect("plasma surface");
    let now = romacs_display_protocol::frame_time::observe_platform_now();
    surfaces.render_pending(
        &gpu.device,
        &gpu.queue,
        &Default::default(),
        romacs_display_protocol::frame_time::FrameSample::new(now, std::time::Duration::ZERO),
    );
    (images, surfaces)
}

// ─── Fixtures ───────────────────────────────────────────────────────

#[test]
fn rect_fixture_matches_reference() {
    check_fixture("rect", &rect_fixture());
}

#[test]
fn path_fixture_matches_reference() {
    check_fixture("path", &path_fixture());
}

#[test]
fn text_fixture_matches_reference() {
    check_fixture("text", &text_fixture());
}

#[test]
fn cells_fixture_matches_reference() {
    check_fixture("cells", &cells_fixture());
}

#[test]
fn group_fixture_matches_reference() {
    check_fixture("group", &group_fixture(GREEN));
}

#[test]
fn media_fixture_matches_reference() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    let (images, surfaces) = media_sources(&gpu);
    let sources = CacheSources {
        images: Some(&images),
        surfaces: Some(&surfaces),
    };
    check("media", &mut gpu, &media_fixture(), &sources);
}

// ─── Behaviour ──────────────────────────────────────────────────────

fn pixel(image: &Rendered, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * image.width + x) * 4) as usize;
    image.rgba[i..i + 4].try_into().expect("pixel")
}

/// A target group sampling its later sibling sees the sibling's current
/// frame on first paint and after the sibling changes.
#[test]
fn a_group_sees_its_later_siblings_current_frame() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    // The badge's fill sits at panel (204..300, 64..104) + root offset (80, 40).
    let probe = (80 + 214, 40 + 84);
    let green = render(&mut gpu, &group_fixture(GREEN), 1.0, &());
    let red = render(&mut gpu, &group_fixture(RED), 1.0, &());
    let [gr, gg, _, _] = pixel(&green, probe.0, probe.1);
    let [rr, rg, _, _] = pixel(&red, probe.0, probe.1);
    assert!(gg > gr, "first paint shows the green badge: {gr},{gg}");
    assert!(rr > rg, "the update shows the red badge: {rr},{rg}");
}

#[test]
fn a_list_out_of_dependency_order_is_refused_whole() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    let mut list = group_fixture(GREEN);
    list.targets.swap(0, 1);
    let fonts = lookup(&gpu.fonts);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let refused = gpu.pass.render(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &list,
        1.0,
        SceneInputs {
            atlas: &mut gpu.atlas,
            fonts: &fonts,
            sources: &(),
        },
    );
    assert_eq!(
        refused,
        Err(SceneError::SampleOrder {
            target: PANEL,
            node: 24,
            sampled: BADGE
        })
    );
    assert!(gpu.pass.view(PANEL).is_none(), "nothing was allocated");
}

/// Twice the device scale doubles the texture, and the shapes land where
/// the logical picture puts them.
#[test]
fn device_scale_doubles_the_target() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    let one = render(&mut gpu, &rect_fixture(), 1.0, &());
    let two = render(&mut gpu, &rect_fixture(), 2.0, &());
    assert_eq!((two.width, two.height), (one.width * 2, one.height * 2));
    // The focused card's fill, well inside, is the same colour at both scales.
    assert_eq!(pixel(&one, 430, 240), pixel(&two, 860, 480));
    // Text rasterises at the device scale: the wordmark's ink doubles.
    let heading = |image: &Rendered, k: u32| {
        ink_bounds(image, [0, 40 * k, image.width, 100 * k]).expect("wordmark ink")
    };
    let one = heading(&render(&mut gpu, &text_fixture(), 1.0, &()), 1);
    let two = heading(&render(&mut gpu, &text_fixture(), 2.0, &()), 2);
    for (a, b) in one.iter().zip(two) {
        assert!(
            (i64::from(*a) * 2 - i64::from(b)).abs() <= 3,
            "{one:?} vs {two:?}"
        );
    }
}

/// `[x0, y0, x1, y1]` of pixels brighter than the background within
/// `region` (`[x0, y0, x1, y1]`, exclusive end).
fn ink_bounds(image: &Rendered, region: [u32; 4]) -> Option<[u32; 4]> {
    let mut bounds: Option<[u32; 4]> = None;
    for y in region[1]..region[3].min(image.height) {
        for x in region[0]..region[2].min(image.width) {
            let [r, g, b, _] = pixel(image, x, y);
            if r.max(g).max(b) > 100 {
                let [x0, y0, x1, y1] = bounds.unwrap_or([x, y, x, y]);
                bounds = Some([x0.min(x), y0.min(y), x1.max(x), y1.max(y)]);
            }
        }
    }
    bounds
}

fn one_target(clear: Color, items: Vec<Item>) -> DrawList {
    DrawList {
        targets: vec![Target {
            id: TargetId(0),
            size: Size::new(200.0, 60.0),
            clear,
            items,
        }],
    }
}

/// A combining sequence is one cluster: shaped together, one advance.
#[test]
fn a_combining_sequence_draws_as_its_precomposed_form() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    let line = |s: &str| one_target(BG, vec![text(1, 10.0, 40.0, 24.0, BRIGHT, s)]);
    let full = [0, 0, 200, 60];
    let decomposed = ink_bounds(&render(&mut gpu, &line("e\u{301}x"), 1.0, &()), full);
    let precomposed = ink_bounds(&render(&mut gpu, &line("\u{e9}x"), 1.0, &()), full);
    let plain = ink_bounds(&render(&mut gpu, &line("ex"), 1.0, &()), full);
    let (decomposed, precomposed, plain) = (
        decomposed.expect("ink"),
        precomposed.expect("ink"),
        plain.expect("ink"),
    );
    assert!(
        decomposed[1] + 3 < plain[1],
        "the accent is drawn: {decomposed:?} {plain:?}"
    );
    for (a, b) in decomposed.iter().zip(precomposed) {
        assert!(a.abs_diff(b) <= 1, "{decomposed:?} vs {precomposed:?}");
    }
}

/// A joining script is shaped as a line: the joined word is narrower than its
/// letters set one by one in their isolated forms.
#[test]
fn a_joining_script_is_shaped_as_a_line() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    if arabic_font().is_none() {
        eprintln!("SKIP: no Noto Sans Arabic on this system");
        return;
    }
    let width = |gpu: &mut Gpu, s: &str| {
        let list = one_target(
            BG,
            vec![item(
                1,
                Paint::Text(TextPaint {
                    origin: Point::new(20.0, 40.0),
                    text: s.into(),
                    font: Font {
                        family: ARABIC.into(),
                        size: 28.0,
                        weight: 400,
                        italic: false,
                    },
                    color: BRIGHT,
                }),
            )],
        );
        let [x0, _, x1, _] =
            ink_bounds(&render(gpu, &list, 1.0, &()), [0, 0, 200, 60]).expect("arabic ink");
        x1 - x0 + 1
    };
    let word = "سلام";
    let joined = width(&mut gpu, word);
    let isolated: u32 = word.chars().map(|c| width(&mut gpu, &c.to_string())).sum();
    assert!(
        f64::from(joined) < f64::from(isolated) * 0.85,
        "joined {joined} px vs isolated letters {isolated} px"
    );
}

/// A shaped line wider than an atlas page still draws: word by word, and a
/// word wider than a page cluster by cluster.
#[test]
fn a_shaped_line_too_long_for_one_raster_still_draws() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    if arabic_font().is_none() {
        eprintln!("SKIP: no Noto Sans Arabic on this system");
        return;
    }
    for separator in [" ", ""] {
        let line = std::iter::repeat_n("سلام", 60)
            .collect::<Vec<_>>()
            .join(separator);
        let [x0, x1] = long_line_ink(&mut gpu, line);
        assert!(
            x1 - x0 > 2048,
            "{separator:?}: the line spans past one atlas page: {x0}..{x1}"
        );
    }
}

/// Horizontal ink extent of `line` drawn at device scale 2.
fn long_line_ink(gpu: &mut Gpu, line: String) -> [u32; 2] {
    let list = DrawList {
        targets: vec![Target {
            id: TargetId(0),
            size: Size::new(3000.0, 60.0),
            clear: BG,
            items: vec![item(
                1,
                Paint::Text(TextPaint {
                    origin: Point::new(10.0, 40.0),
                    text: line.into(),
                    font: Font {
                        family: ARABIC.into(),
                        size: 28.0,
                        weight: 400,
                        italic: false,
                    },
                    color: BRIGHT,
                }),
            )],
        }],
    };
    let image = render(gpu, &list, 2.0, &());
    let [x0, _, x1, _] = ink_bounds(&image, [0, 0, image.width, image.height]).expect("ink");
    [x0, x1]
}

/// Filtering a straight-alpha source premultiplies first: halfway between
/// opaque red and transparent black is half-covered red, not dark red.
#[test]
fn straight_alpha_sources_filter_without_dark_fringes() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    let images = ImageCache::new(&gpu.device);
    let mut surfaces = ShaderSurfaceCache::new(&gpu.device);
    surfaces
        .create_pixels(
            &gpu.device,
            &gpu.queue,
            images.bind_group_layout(),
            images.sampler(),
            SURFACE_ID,
            &[255, 0, 0, 255, 0, 0, 0, 0],
            2,
            1,
        )
        .expect("pixel surface");
    let sources = CacheSources {
        images: None,
        surfaces: Some(&surfaces),
    };
    let list = one_target(
        BG,
        vec![item(
            1,
            Paint::Sample(SamplePaint {
                source: Source::Surface(SURFACE_ID),
                rect: Rect::new(0.0, 0.0, 200.0, 60.0),
                radius: 0.0,
                channels: Arc::default(),
            }),
        )],
    );
    let image = render(&mut gpu, &list, 1.0, &sources);
    // Texel centres sit at x = 50 and 150; x = 100 is the midpoint. Half
    // coverage of linear red is 0.5, sRGB 188; filtering straight colour
    // first gives 0.25, sRGB 137.
    let [r, g, b, _] = pixel(&image, 100, 30);
    assert!(r > 175 && g < 8 && b < 8, "midpoint {r},{g},{b}");
}

/// A translucent stroke draws over the fill, not instead of it.
#[test]
fn a_translucent_stroke_composites_over_the_fill() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    let list = one_target(
        Color::TRANSPARENT,
        vec![item(
            1,
            Paint::Rect(RectPaint {
                rect: Rect::new(20.0, 10.0, 180.0, 50.0),
                radius: 0.0,
                fill: Some(Color::rgb8(255, 255, 255)),
                stroke: Some(Stroke {
                    width: 6.0,
                    color: Color::rgba(1.0, 0.0, 0.0, 0.5),
                }),
                shadow: None,
                glow: None,
            }),
        )],
    );
    let image = render(&mut gpu, &list, 1.0, &());
    let [r, g, _, a] = pixel(&image, 23, 30);
    assert_eq!(a, 255, "the band is as opaque as the fill under it");
    assert!(r == 255 && g > 150, "half red over white: {r},{g}");
}

/// Moving a path reuses its tessellation; changing its revision does not.
#[test]
fn moving_a_path_does_not_retessellate() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    let first = render(&mut gpu, &path_fixture(), 1.0, &());
    let mut moved = path_fixture();
    for item in &mut moved.targets[0].items {
        item.transform = Affine::translate((3.0, 0.0));
    }
    let again = render(&mut gpu, &moved, 1.0, &());
    assert!(first.stats.tessellations > 0);
    assert_eq!(again.stats.tessellations, 0);
}

/// An unresolvable font or a missing source skips that item and names it.
#[test]
fn unresolved_inputs_skip_only_their_item() {
    let Some(mut gpu) = software_gpu() else {
        return;
    };
    let mut list = text_fixture();
    list.targets[0].items.push(item(
        99,
        Paint::Text(TextPaint {
            origin: Point::new(0.0, 20.0),
            text: "x".into(),
            font: Font {
                family: "No Such Family".into(),
                size: 12.0,
                weight: 400,
                italic: false,
            },
            color: FG,
        }),
    ));
    list.targets[0].items.push(item(
        98,
        Paint::Sample(SamplePaint {
            source: Source::Image(12345),
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            radius: 0.0,
            channels: Arc::default(),
        }),
    ));
    let image = render(&mut gpu, &list, 1.0, &());
    assert_eq!(
        image.stats.skipped,
        [Skipped::UnresolvedFont(99), Skipped::MissingSource(98)]
    );
}

/// GPU time per fixture 2560 pixels wide against the design's 1.0 ms scene
/// budget (1.5 ms with glow), on the hardware adapter with timestamp queries. Prints the times;
/// fails only when the budget is exceeded.
#[test]
#[ignore = "needs a hardware GPU with timestamp queries; run explicitly"]
fn fixture_gpu_times_within_budget() {
    let Some(adapter) = hardware_adapter() else {
        eprintln!("SKIP: no hardware adapter");
        return;
    };
    eprintln!("scene GPU adapter: {:?}", adapter.get_info().name);
    let mut gpu = gpu(request_device(&adapter), false);
    assert!(!gpu.software);
    assert!(
        gpu.pass.timing_available(),
        "device lacks timestamp queries"
    );
    let scale = (2560.0 / W) as f32;
    let fixtures = [
        ("rect", rect_fixture()),
        ("path", path_fixture()),
        ("text", text_fixture()),
        ("cells", cells_fixture()),
        ("group", group_fixture(GREEN)),
    ];
    for (name, list) in fixtures {
        let mut best = u64::MAX;
        for _ in 0..20 {
            render(&mut gpu, &list, scale, &());
            wait(&gpu.device);
            if let Some(timings) = gpu.pass.take_timings() {
                best = best.min(timings.iter().map(|t| t.gpu_ns).sum());
            }
        }
        eprintln!(
            "scene GPU {name}: {:.3} ms at {}x{}",
            best as f64 / 1e6,
            (W * f64::from(scale)) as u32,
            (H * f64::from(scale)) as u32
        );
        assert!(
            best < 1_500_000,
            "{name}: {best} ns exceeds the scene budget"
        );
    }
}
