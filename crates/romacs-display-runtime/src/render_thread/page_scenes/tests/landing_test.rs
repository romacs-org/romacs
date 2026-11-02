use super::*;
use romacs_renderer_wgpu::shader_surface::{validate_surface_wgsl, SurfaceContract};
use romacs_scene::draw::Paint;
use romacs_scene::layout::{self, Measure, Measured};
use romacs_scene::lower::{self, Resolve};
use romacs_scene::motion::Policy;
use romacs_scene::node::{Font, ImageSource, SceneId, TextNode};
use romacs_scene::scene::Scene;

const BACKGROUND: Color = Color::rgb8(0x0a, 0x0a, 0x0a);

fn view() -> romacs_logo::View {
    romacs_logo::View { cols: 110, rows: 36, cell_aspect: 0.45 }
}

fn cell() -> Size {
    Size::new(9.0, 20.0)
}

fn geometry_of() -> Geometry {
    geometry(view(), cell()).expect("a landing fits")
}

/// The scene after SECONDS of motion under POLICY.
fn scene_at(g: &Geometry, policy: Policy, seconds: f64) -> Scene {
    Scene::new(SceneId(1), 1)
        .tick(Duration::ZERO, true)
        .apply(&scene(g, BACKGROUND, policy), policy)
        .expect("the landing scene applies")
        .tick(Duration::from_secs_f64(seconds), true)
}

fn uniform(scene: &Scene, i: u8) -> f32 {
    match scene.value(MARK, Prop::Uniform(i)) {
        Some(Value::Num(v)) => v,
        other => panic!("uniform {i}: {other:?}"),
    }
}

fn opacity(scene: &Scene, id: NodeId) -> f32 {
    match scene.value(id, Prop::Opacity) {
        Some(Value::Num(v)) => v,
        other => panic!("opacity: {other:?}"),
    }
}

struct NoText;

impl Measure for NoText {
    fn text(&mut self, _: &TextNode, _: Option<f64>) -> Measured {
        Measured::default()
    }
    fn cell(&mut self, _: &Font) -> Size {
        Size::ZERO
    }
    fn image(&mut self, _: &ImageSource) -> Option<Size> {
        None
    }
}

/// The mark draws into surface MARK_SURFACE.
struct Marks;

const MARK_SURFACE: u32 = 70;
const FIELD_SURFACE: u32 = 71;

impl Resolve for Marks {
    fn image(&mut self, _: SceneId, _: NodeId, _: &ImageSource) -> Option<(romacs_scene::draw::Source, Size)> {
        None
    }
    fn surface(&mut self, _: SceneId, node: NodeId) -> Option<u32> {
        (node == MARK).then_some(MARK_SURFACE)
    }
}

fn list_at(g: &Geometry, scene: &Scene) -> romacs_scene::draw::DrawList {
    let solved = layout::solve(scene, g.placement.size(), &Default::default(), &mut NoText);
    lower::lower(scene, &solved, &mut Marks)
}

#[test]
fn the_landing_sits_where_the_braille_logo_does() {
    let g = geometry_of();
    let placed = romacs_logo::placement(view());
    let mark = Rect::new(placed.mark.0, placed.mark.1, placed.mark.0 + placed.mark.2, placed.mark.1 + placed.mark.3)
        .scale_from_origin(cell().height);
    let block = Rect::new(0.0, 0.0, 110.0 * 9.0, 36.0 * 20.0);
    assert!(block.contains_rect(g.placement), "{:?} in {block:?}", g.placement);
    // The shader box is the mark grown by its margin, in placement terms.
    let shader = g.mark + g.placement.origin().to_vec2();
    assert!((shader.inflate(-g.margin, -g.margin).x0 - mark.x0).abs() < 1e-6);
    assert!((shader.inflate(-g.margin, -g.margin).height() - mark.height()).abs() < 1e-6);
    assert!(g.word.y0 > g.mark.y1 - g.margin, "the wordmark is under the mark");
    assert_eq!(g.dot, Size::new(4.5, 5.0));
}

#[test]
fn a_view_too_small_has_no_landing() {
    assert_eq!(geometry(romacs_logo::View { cols: 4, rows: 2, cell_aspect: 0.5 }, cell()), None);
    assert_eq!(geometry(view(), Size::new(0.0, 20.0)), None);
}

#[test]
fn the_mark_assembles_then_turns_and_the_wordmark_rises_in() {
    let g = geometry_of();
    let open = scene_at(&g, Policy::On, 0.0);
    assert_eq!(uniform(&open, 3), 0.0, "assembly starts at nothing");
    assert_eq!(opacity(&open, WORDMARK), 0.0);
    assert_eq!((uniform(&open, 4), uniform(&open, 5)), (4.5, 5.0), "the braille dot pitch");
    let settled = scene_at(&g, Policy::On, 3.2);
    assert_eq!(uniform(&settled, 3), 1.0, "assembled within 3 s");
    assert_eq!(opacity(&settled, WORDMARK), 1.0);
    let (a, b) = (scene_at(&g, Policy::On, 4.0), scene_at(&g, Policy::On, 5.0));
    assert_ne!(uniform(&a, 0), uniform(&b, 0), "the yaw keeps swinging");
    assert!(uniform(&a, 0).abs() <= 17.0 && uniform(&b, 1).abs() <= 9.0);
    assert!(scene_at(&g, Policy::On, 600.0).animating(), "the loop never settles");
}

#[test]
fn reduced_motion_is_the_still_frame_at_once() {
    let g = geometry_of();
    let still = scene_at(&g, Policy::Reduced, 0.0);
    assert_eq!(uniform(&still, 3), 1.0, "assembled");
    assert_eq!((uniform(&still, 0), uniform(&still, 1)), (YAW.1, PITCH.1), "posed");
    assert_eq!(uniform(&still, 2), 1.0, "no band of light");
    let later = scene_at(&g, Policy::Reduced, 30.0);
    assert!(!later.animating(), "nothing moves under reduced motion");
    assert_eq!(list_at(&g, &still).targets.len(), 1);
    let list = list_at(&g, &still);
    let items = &list.targets[0].items;
    assert!(matches!(items.last().map(|i| &i.paint), Some(Paint::Path(_))), "the wordmark is drawn");
    assert_eq!(items.last().map(|i| i.opacity), Some(1.0));
}

#[test]
fn the_placement_draws_backdrop_mark_and_wordmark_in_order() {
    let g = geometry_of();
    let list = list_at(&g, &scene_at(&g, Policy::On, 3.0));
    let root = list.targets.last().expect("a root target");
    assert_eq!(root.size, g.placement.size());
    let kinds: Vec<_> = root.items.iter().map(|i| match &i.paint {
        Paint::Rect(r) => format!("rect {:?}", r.fill.map(|c| c.a)),
        Paint::Sample(s) => format!("sample {:?}", s.source),
        Paint::Path(_) => "path".to_owned(),
        other => format!("{other:?}"),
    }).collect();
    assert_eq!(kinds, [
        "rect Some(1.0)".to_owned(),
        format!("sample {:?}", romacs_scene::draw::Source::Surface(MARK_SURFACE)),
        "path".to_owned(),
    ]);
    let drawn = root.items[1].transform.transform_rect_bbox(Rect::from_origin_size((0.0, 0.0), g.mark.size()));
    assert!((drawn.x0 - g.mark.x0).abs() < 1e-3 && (drawn.y1 - g.mark.y1).abs() < 1e-3, "{drawn:?} at {:?}", g.mark);
}

#[test]
fn a_refit_moves_the_scene_to_new_geometry_and_keeps_its_motion() {
    let g = geometry_of();
    let small = geometry(romacs_logo::View { cols: 80, rows: 18, cell_aspect: 0.45 }, cell()).expect("fits");
    let before = scene_at(&g, Policy::On, 1.0);
    let after = before.apply(&refit(&small, BACKGROUND, before.rev()), Policy::On).expect("the refit applies");
    let revision = |s: &Scene| s.node(WORDMARK).map(|n| n.revision);
    assert!(revision(&after) > revision(&before), "the wordmark's tessellation input changed");
    assert_eq!(uniform(&after, 3), uniform(&before, 3), "the assembly runs on from where it was");
    assert!(after.tracks_of(MARK).count() >= 4, "turn, light and assembly keep their motions");
    let list = list_at(&small, &after.tick(Duration::from_secs(5), true));
    assert_eq!(list.targets[0].size, small.placement.size());
}

#[test]
fn the_shaders_compile_against_the_surface_contract() {
    validate_surface_wgsl(field_wgsl(), &[], SurfaceContract::V1).expect("field WGSL");
    let uniforms: Vec<(String, u8)> = (0..UNIFORMS).map(|i| (format!("u{i}"), 1)).collect();
    validate_surface_wgsl(mark_wgsl(), &uniforms, SurfaceContract::V1).expect("mark WGSL");
    assert!(field_wgsl().contains("R_SEGMENTS"));
    let (width, height) = field_size();
    assert!(width > height && height == 640, "{width}x{height}");
}

/// The landing drawn by the renderer's own shader-surface cache and scene
/// pass on the software Vulkan adapter (lavapipe), as a frame shows it.
mod gpu {
    use super::*;
    use romacs_renderer_wgpu::image_cache::ImageCache;
    use romacs_renderer_wgpu::scene_graph::{CacheSources, SceneInputs, ScenePass};
    use romacs_renderer_wgpu::shader_surface::{SurfaceChannelSource, SurfaceShaderLanguage, SurfaceUniformInit};
    use romacs_renderer_wgpu::shader_surface_cache::ShaderSurfaceCache;
    use romacs_renderer_wgpu::WgpuGlyphAtlas;
    use std::path::{Path, PathBuf};

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    pub(super) struct Gpu {
        pub device: wgpu::Device,
        queue: wgpu::Queue,
        images: ImageCache,
        surfaces: ShaderSurfaceCache,
        atlas: WgpuGlyphAtlas,
        pub pass: ScenePass,
        mark_size: Option<(u32, u32)>,
    }

    fn adapter(software: bool) -> Option<wgpu::Adapter> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .find(|a| (a.get_info().device_type == wgpu::DeviceType::Cpu) == software)
    }

    /// One device per adapter kind for the test binary: the Vulkan loader
    /// crashes when test threads create instances concurrently.
    type Device = (wgpu::Device, wgpu::Queue);

    fn device(software: bool) -> Option<Device> {
        static DEVICES: std::sync::Mutex<Vec<(bool, Option<Device>)>> = std::sync::Mutex::new(Vec::new());
        let mut devices = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, found)) = devices.iter().find(|(s, _)| *s == software) {
            return found.clone();
        }
        let made = adapter(software).map(|adapter| {
            eprintln!("landing GPU adapter: {}", adapter.get_info().name);
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("landing"),
                required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            }))
            .expect("landing device")
        });
        devices.push((software, made.clone()));
        made
    }

    pub(super) fn gpu(software: bool) -> Option<Gpu> {
        let (device, queue) = device(software)?;
        let images = ImageCache::new(&device);
        let mut surfaces = ShaderSurfaceCache::new(&device);
        let (width, height) = field_size();
        surfaces
            .create_shader(&device, images.bind_group_layout(), images.sampler(), FORMAT, FIELD_SURFACE,
                           SurfaceShaderLanguage::Wgsl, field_wgsl(), &[], width, height, 1.0, false, None, None)
            .expect("field surface");
        render_surfaces(&device, &queue, &mut surfaces);
        let atlas = WgpuGlyphAtlas::new(&device);
        let pass = ScenePass::new(&device, &queue, atlas.bind_group_layout());
        Some(Gpu { device, queue, images, surfaces, atlas, pass, mark_size: None })
    }

    fn render_surfaces(device: &wgpu::Device, queue: &wgpu::Queue, surfaces: &mut ShaderSurfaceCache) {
        let now = romacs_display_protocol::frame_time::observe_platform_now();
        surfaces.render_pending(device, queue, &Default::default(),
                                romacs_display_protocol::frame_time::FrameSample::new(now, Duration::ZERO));
    }

    pub(super) fn wait(device: &wgpu::Device) {
        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(120)) })
            .expect("device poll");
    }

    pub(super) struct Picture {
        pub width: u32,
        pub height: u32,
        pub rgba: Vec<u8>,
    }

    impl Picture {
        /// The share of pixels whose colour is far from the background and
        /// red-dominant: the lit R.
        fn red(&self) -> f64 {
            let red = self.rgba.chunks(4).filter(|p| p[0] > 90 && p[0] > p[1].saturating_add(40)).count();
            red as f64 / f64::from(self.width * self.height)
        }

        /// The share of pixels lit well above the background.
        fn ink(&self) -> f64 {
            let lit = self.rgba.chunks(4).filter(|p| p[..3].iter().any(|&c| c > 60)).count();
            lit as f64 / f64::from(self.width * self.height)
        }

        fn differs(&self, other: &Self) -> usize {
            self.rgba.chunks(4).zip(other.rgba.chunks(4))
                .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 8))
                .count()
        }
    }

    /// Bring the mark surface to SCENE's uniforms, render it, then the
    /// placement; with READ, read the placement back.
    pub(super) fn frame(gpu: &mut Gpu, g: &Geometry, scene: &Scene, read: bool) -> Option<Picture> {
        let size = (g.mark.width().round() as u32, g.mark.height().round() as u32);
        if gpu.mark_size != Some(size) {
            let uniforms: Vec<SurfaceUniformInit> = (0..UNIFORMS)
                .map(|i| SurfaceUniformInit { name: format!("u{i}"), value: [0.0; 4], components: 1 })
                .collect();
            gpu.surfaces
                .create_shader(&gpu.device, gpu.images.bind_group_layout(), gpu.images.sampler(), FORMAT,
                               MARK_SURFACE, SurfaceShaderLanguage::Wgsl, mark_wgsl(), &uniforms, size.0, size.1,
                               1.0, false, None, Some(SurfaceChannelSource::Surface(FIELD_SURFACE)))
                .expect("mark surface");
            gpu.mark_size = Some(size);
        }
        for i in 0..UNIFORMS as u8 {
            gpu.surfaces.set_uniform(MARK_SURFACE, &format!("u{i}"), [uniform(scene, i), 0.0, 0.0, 0.0]);
        }
        render_surfaces(&gpu.device, &gpu.queue, &mut gpu.surfaces);
        let list = list_at(g, scene);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let sources = CacheSources { images: Some(&gpu.images), surfaces: Some(&gpu.surfaces) };
        let stats = gpu.pass
            .render(&gpu.device, &gpu.queue, &mut encoder, &list, 1.0,
                    SceneInputs { atlas: &mut gpu.atlas, fonts: &|_| None, sources: &sources })
            .expect("the landing list is valid");
        assert!(stats.skipped.is_empty(), "skipped {:?}", stats.skipped);
        if !read {
            gpu.queue.submit([encoder.finish()]);
            gpu.pass.submitted();
            return None;
        }
        let texture = gpu.pass.texture(list.targets.last()?.id)?.clone();
        let (width, height) = (texture.width(), texture.height());
        let row = (width * 4).div_ceil(256) * 256;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("landing readback"),
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) },
            },
            texture.size(),
        );
        gpu.queue.submit([encoder.finish()]);
        gpu.pass.submitted();
        readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        wait(&gpu.device);
        let mapped = readback.slice(..).get_mapped_range().expect("mapped");
        let rgba = mapped.chunks(row as usize).flat_map(|line| &line[..(width * 4) as usize]).copied().collect();
        Some(Picture { width, height, rgba })
    }

    pub(super) fn write_png(path: &Path, picture: &Picture) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("png dir");
        let file = std::fs::File::create(path).expect("png file");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), picture.width, picture.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(&picture.rgba).expect("png data");
    }

    fn read_png(path: &Path) -> Option<Picture> {
        let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        buf.truncate(info.buffer_size());
        Some(Picture { width: info.width, height: info.height, rgba: buf })
    }

    fn reference(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/render_thread/page_scenes/tests/fixtures")
            .join(format!("{name}.png"))
    }

    /// Software frames at moments of the open and the loop: the dots, the
    /// solid, the turned object, and the still frame of reduced motion
    /// against its reference.  `ROMACS_SCENE_BLESS=1` rewrites it.
    #[test]
    fn the_landing_draws_on_the_software_adapter() {
        let Some(mut gpu) = gpu(true) else {
            eprintln!("SKIP: no software (lavapipe) Vulkan adapter");
            return;
        };
        let g = geometry_of();
        let at = |gpu: &mut Gpu, policy, t| frame(gpu, &g, &scene_at(&g, policy, t), true).expect("a picture");
        let still = at(&mut gpu, Policy::Reduced, 0.0);
        let raining = at(&mut gpu, Policy::On, 0.9);
        let solid = at(&mut gpu, Policy::On, 1.8);
        let turned = at(&mut gpu, Policy::On, 6.0);
        let later = at(&mut gpu, Policy::On, 6.5);
        if let Some(dir) = std::env::var_os("ROMACS_LANDING_FRAMES") {
            for (name, picture) in [("still", &still), ("raining", &raining), ("solid", &solid), ("turned", &turned)] {
                write_png(&PathBuf::from(&dir).join(format!("software-{name}.png")), picture);
            }
        }
        assert_eq!(still.differs(&at(&mut gpu, Policy::Reduced, 0.0)), 0, "a still frame is stable");
        assert!(still.red() > 0.08, "the lit R fills its place: {:.3}", still.red());
        assert!(raining.ink() > 0.01 && raining.red() < solid.red(),
                "dots before the solid: ink {:.3}, red {:.3} then {:.3}", raining.ink(), raining.red(), solid.red());
        assert!(turned.differs(&later) > 2000, "the loop moves");
        let path = reference("landing-still");
        if std::env::var_os("ROMACS_SCENE_BLESS").is_some() {
            write_png(&path, &still);
        }
        let blessed = read_png(&path).unwrap_or_else(|| panic!("no reference at {}", path.display()));
        assert_eq!((blessed.width, blessed.height), (still.width, still.height));
        let changed = still.differs(&blessed);
        assert!(changed as f64 / f64::from(still.width * still.height) < 0.002,
                "{changed} pixels differ from {}", path.display());
    }

    /// A frame sequence of the open and the loop for review, on the hardware
    /// adapter when there is one: `ROMACS_LANDING_FRAMES=DIR`.
    #[test]
    #[ignore = "writes a frame sequence for review; run explicitly"]
    fn landing_frame_sequence() {
        let Some(dir) = std::env::var_os("ROMACS_LANDING_FRAMES").map(PathBuf::from) else {
            eprintln!("SKIP: set ROMACS_LANDING_FRAMES");
            return;
        };
        let Some(mut gpu) = gpu(false).or_else(|| gpu(true)) else { return };
        let g = geometry(romacs_logo::View { cols: 180, rows: 48, cell_aspect: 0.5 }, Size::new(10.0, 20.0))
            .expect("a landing fits");
        let mut times: Vec<f64> = (0..=60).map(|i| f64::from(i) / 20.0).collect();
        times.extend((0..12).map(|i| 4.0 + f64::from(i) * 0.75));
        for (i, t) in times.iter().enumerate() {
            let picture = frame(&mut gpu, &g, &scene_at(&g, Policy::On, *t), true).expect("a picture");
            write_png(&dir.join(format!("frame-{i:03}-t{:05.2}.png", t)), &picture);
        }
        let still = frame(&mut gpu, &g, &scene_at(&g, Policy::Reduced, 0.0), true).expect("a picture");
        write_png(&dir.join("reduced-still.png"), &still);
    }

    /// GPU time per landing frame at 2560x1440 on the hardware adapter
    /// against the 1 ms budget: the mark surface and the scene pass, the
    /// best of three batches of 120 frames of the turning loop submitted
    /// back to back (wall time, so an upper bound), then the scene pass's
    /// own timestamps.  Prints both.
    #[test]
    #[ignore = "needs a hardware GPU; run explicitly"]
    fn landing_gpu_time_within_budget() {
        let Some(mut gpu) = gpu(false) else {
            eprintln!("SKIP: no hardware adapter");
            return;
        };
        // A full-screen window of 11x26 cells at 2560x1440, the landing's
        // block all but its last rows.
        let g = geometry(romacs_logo::View { cols: 232, rows: 50, cell_aspect: 11.0 / 26.0 }, Size::new(11.0, 26.0))
            .expect("a landing fits");
        eprintln!("landing at 2560x1440: placement {:?}, mark box {:?}", g.placement.size(), g.mark.size());
        // Two seconds of frames first, so the GPU's clocks are up.
        for i in 0..120 {
            frame(&mut gpu, &g, &scene_at(&g, Policy::On, 5.0 + f64::from(i) / 60.0), false);
        }
        wait(&gpu.device);
        let frames = 120;
        let per_frame = (0..3).map(|batch| {
            let start = std::time::Instant::now();
            for i in 0..frames {
                frame(&mut gpu, &g, &scene_at(&g, Policy::On, 7.0 + f64::from(batch * frames + i) / 60.0), false);
            }
            wait(&gpu.device);
            start.elapsed().as_secs_f64() * 1e3 / f64::from(frames)
        }).fold(f64::MAX, f64::min);
        let mut pass_ns = u64::MAX;
        for i in 0..10 {
            frame(&mut gpu, &g, &scene_at(&g, Policy::On, 8.0 + f64::from(i) / 60.0), false);
            wait(&gpu.device);
            if let Some(timings) = gpu.pass.take_timings() {
                pass_ns = pass_ns.min(timings.iter().map(|t| t.gpu_ns).sum());
            }
        }
        eprintln!("landing GPU: {per_frame:.3} ms per frame (mark + placement, wall, pipelined); \
                   placement pass {:.3} ms (timestamps)", pass_ns as f64 / 1e6);
        assert!(per_frame < 1.0, "{per_frame:.3} ms per frame exceeds the 1 ms budget");
    }
}
