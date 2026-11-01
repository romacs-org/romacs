//! Fixture scenes: one per node kind plus a dashboard, as create diffs and
//! as lowered draw lists at a step of the deterministic frame clock.
//!
//! The renderer's headless tests draw these; this crate's tests pin their
//! `VM2GFX01` bytes. Text is measured by [`FixedMeasure`] (a fixed advance
//! per character), so a draw list depends on no font.

use std::sync::Arc;
use std::time::Duration;

use kurbo::{BezPath, Size};
use romacs_display_protocol::motion_spec::{MotionDuration, MotionSpec, TweenSpec};
use romacs_display_protocol::scroll_animation::TransitionEasing;

use crate::diff::{Diff, Op, Rev};
use crate::draw::{DrawList, Source};
use crate::layout::{self, Line, Measure, Measured};
use crate::lower::{self, Resolve};
use crate::motion::{Animate, Policy, Repeat};
use crate::node::{
    Align, Camera, Cells, Channel, Color, Dim, Direction, Flex, Font, Glow, ImageNode, ImageSource,
    Kind, ModelNode, NodeId, NodeSpec, PathNode, RectNode, SceneId, ShaderNode, Shadow, Stroke,
    TextNode, default_font,
};
use crate::prop::{Prop, Value};
use crate::scene::Scene;

/// One frame of `ROMACS_FRAME_CLOCK=step:16667us`.
pub const STEP: Duration = Duration::from_micros(16_667);

/// Text and cells measured at `0.6 em` per character and `1.25 em` per line.
#[derive(Clone, Copy, Debug, Default)]
pub struct FixedMeasure;

impl FixedMeasure {
    fn advance(font: &Font) -> f64 {
        font.size * 0.6
    }

    fn line(font: &Font) -> f64 {
        font.size * 1.25
    }
}

impl Measure for FixedMeasure {
    fn text(&mut self, text: &TextNode, max_width: Option<f64>) -> Measured {
        let (advance, line) = (Self::advance(&text.font), Self::line(&text.font));
        let fits = max_width.map_or(usize::MAX, |w| ((w / advance).floor() as usize).max(1));
        let lines = wrap(&text.text, fits);
        let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * advance;
        Measured {
            size: Size::new(width, line * lines.len() as f64),
            lines: lines
                .into_iter()
                .enumerate()
                .map(|(i, l)| Line {
                    width: l.chars().count() as f64 * advance,
                    text: Arc::from(l),
                    baseline: line * i as f64 + text.font.size,
                })
                .collect(),
        }
    }

    fn cell(&mut self, font: &Font) -> Size {
        Size::new(Self::advance(font), Self::line(font))
    }

    fn image(&mut self, _: &ImageSource) -> Option<Size> {
        Some(Size::new(64.0, 48.0))
    }
}

/// Greedy word wrap at `fits` characters; a longer word gets its own line.
fn wrap(text: &str, fits: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let needed =
                line.chars().count() + usize::from(!line.is_empty()) + word.chars().count();
            if !line.is_empty() && needed > fits {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

/// Image 1 at 64×48 for files, surface 2 for other scenes, surface
/// `1000 + node` for shaders and models.
#[derive(Clone, Copy, Debug, Default)]
pub struct FixtureResources;

impl Resolve for FixtureResources {
    fn image(&mut self, _: SceneId, _: NodeId, source: &ImageSource) -> Option<(Source, Size)> {
        Some(match source {
            ImageSource::File(_) => (Source::Image(1), Size::new(64.0, 48.0)),
            ImageSource::Scene(_) => (Source::Surface(2), Size::new(64.0, 48.0)),
        })
    }

    fn surface(&mut self, _: SceneId, node: NodeId) -> Option<u32> {
        Some(1000 + node.0)
    }
}

/// Palette colours (Ember Lattice, §3.2), sRGB.
pub const BACKGROUND: Color = Color::rgb8(0x12, 0x14, 0x18);
pub const ACCENT: Color = Color::rgb8(0xd4, 0x8a, 0x20);
pub const FOREGROUND: Color = Color::rgb8(0xe6, 0xe1, 0xd6);

/// Colours cross the wire as RGBA8, so fixtures use alphas it can carry.
fn alpha(c: Color, a: u8) -> Color {
    c.with_alpha(f32::from(a) / 255.0)
}

fn sized(spec: NodeSpec, w: f32, h: f32) -> NodeSpec {
    spec.with(|c| {
        c.width = Dim::Px(w);
        c.height = Dim::Px(h);
    })
}

fn at(spec: NodeSpec, x: f32, y: f32) -> NodeSpec {
    spec.with(|c| {
        c.transform.x = x;
        c.transform.y = y;
    })
}

fn font(size: f64) -> Font {
    Font {
        size,
        ..default_font()
    }
}

fn create(nodes: Vec<NodeSpec>) -> Diff {
    Diff {
        base: Rev(0),
        ops: nodes
            .into_iter()
            .enumerate()
            .map(|(i, node)| Op::Insert {
                parent: NodeId::ROOT,
                index: i as u32,
                node,
            })
            .collect(),
    }
}

fn backdrop() -> NodeSpec {
    sized(
        NodeSpec::new(
            1,
            Kind::Rect(RectNode {
                fill: BACKGROUND,
                ..RectNode::default()
            }),
        ),
        320.0,
        200.0,
    )
}

fn rect() -> NodeSpec {
    at(
        sized(
            NodeSpec::new(
                2,
                Kind::Rect(RectNode {
                    radius: 12.0,
                    fill: alpha(ACCENT, 217),
                    stroke: Stroke {
                        color: FOREGROUND,
                        width: 2.0,
                    },
                    shadow: Shadow {
                        color: alpha(Color::rgb8(0, 0, 0), 128),
                        blur: 8.0,
                        spread: 0.0,
                        dx: 0.0,
                        dy: 4.0,
                    },
                    glow: Glow {
                        color: ACCENT,
                        radius: 10.0,
                    },
                }),
            ),
            160.0,
            90.0,
        ),
        80.0,
        55.0,
    )
}

fn sparkline() -> Arc<BezPath> {
    let mut p = BezPath::new();
    p.move_to((0.0, 40.0));
    for i in 1..=60 {
        let x = f64::from(i) * 4.0;
        p.line_to((x, (40.0 + 30.0 * (x * 0.07).sin()).round()));
    }
    Arc::new(p)
}

fn path() -> NodeSpec {
    let mut panel = BezPath::new();
    panel.move_to((0.0, 0.0));
    panel.line_to((240.0, 0.0));
    panel.quad_to((260.0, 40.0), (240.0, 80.0));
    panel.curve_to((160.0, 100.0), (80.0, 60.0), (0.0, 80.0));
    panel.close_path();
    let fill = at(
        NodeSpec::new(
            3,
            Kind::Path(PathNode {
                data: Arc::new(panel),
                fill: alpha(ACCENT, 64),
                ..PathNode::default()
            }),
        ),
        30.0,
        20.0,
    );
    let line = at(
        NodeSpec::new(
            4,
            Kind::Path(PathNode {
                data: sparkline(),
                stroke: Stroke {
                    color: ACCENT,
                    width: 2.0,
                },
                join: crate::node::Join::Round,
                cap: crate::node::Cap::Round,
                dash: Arc::from([6.0, 3.0]),
                ..PathNode::default()
            }),
        ),
        40.0,
        110.0,
    );
    NodeSpec::new(5, Kind::group()).child(fill).child(line)
}

fn text() -> NodeSpec {
    let node = NodeSpec::new(
        6,
        Kind::Text(TextNode {
            text: Arc::from("Romacs scenes draw text through the glyph atlas"),
            font: font(16.0),
            color: FOREGROUND,
            wrap: true,
        }),
    );
    at(node.with(|c| c.width = Dim::Px(280.0)), 20.0, 40.0)
}

fn image() -> NodeSpec {
    at(
        sized(
            NodeSpec::new(
                7,
                Kind::Image(ImageNode {
                    source: ImageSource::File(Arc::from("fixtures/ember.png")),
                    fit: crate::node::Fit::Contain,
                }),
            ),
            200.0,
            100.0,
        ),
        60.0,
        50.0,
    )
}

fn shader() -> NodeSpec {
    sized(
        NodeSpec::new(
            8,
            Kind::Shader(ShaderNode {
                source: Arc::from(
                    "fn mainImage(uv: vec2f) -> vec4f { return vec4f(uv, 0.5, 1.0); }",
                ),
                uniforms: vec![0.25, 1.0],
                fps: 0,
                channels: Vec::new(),
            }),
        ),
        320.0,
        200.0,
    )
}

fn model() -> NodeSpec {
    at(
        sized(
            NodeSpec::new(
                9,
                Kind::Model(ModelNode {
                    mesh: 1,
                    camera: Camera {
                        yaw: 30.0,
                        pitch: 15.0,
                        distance: 3.0,
                    },
                }),
            ),
            160.0,
            160.0,
        ),
        80.0,
        20.0,
    )
}

fn cells() -> NodeSpec {
    let rows = ["⣿⣷⣶⣤⣀  ⣀⣤⣶⣷⣿", "▀▄▀▄▀▄▀▄▀▄▀▄", " .:-=+*#%@  "];
    let glyphs: Arc<[char]> = rows.iter().flat_map(|r| r.chars()).collect();
    let shades: Arc<[u8]> = (0..glyphs.len()).map(|i| (i % 3) as u8).collect();
    at(
        NodeSpec::new(
            10,
            Kind::Cells(Cells {
                cols: 12,
                rows: 3,
                glyphs,
                shades,
                palette: Arc::from([ACCENT, FOREGROUND, alpha(ACCENT, 128)]),
                font: font(14.0),
            }),
        ),
        40.0,
        60.0,
    )
}

/// A flex column of rows with a gap and padding.
fn flex() -> NodeSpec {
    let card = NodeSpec::new(
        11,
        Kind::Group {
            flex: Some(Flex {
                direction: Direction::Column,
                gap: 8.0,
                padding: [12.0; 4],
                align: Align::Stretch,
            }),
            target: false,
        },
    );
    let card = sized(card, 200.0, 160.0);
    (0..4).fold(at(card, 60.0, 20.0), |card, i| {
        let row = NodeSpec::new(
            12 + i,
            Kind::Rect(RectNode {
                radius: 4.0,
                fill: alpha(ACCENT, 51 + 51 * i as u8),
                ..RectNode::default()
            }),
        )
        .with(|c| c.height = Dim::Px(24.0));
        card.child(row)
    })
}

/// A shader samples a target group that comes after it among its siblings:
/// the draw list still renders that target first.
fn target() -> Vec<NodeSpec> {
    let sampler = sized(
        NodeSpec::new(
            20,
            Kind::Shader(ShaderNode {
                source: Arc::from(
                    "fn mainImage(uv: vec2f) -> vec4f { return textureSample(iChannel0, uv); }",
                ),
                channels: vec![Channel::Local(NodeId(21))],
                ..ShaderNode::default()
            }),
        ),
        160.0,
        200.0,
    );
    let word = NodeSpec::new(
        22,
        Kind::Text(TextNode {
            text: Arc::from("ROMACS"),
            font: font(24.0),
            color: ACCENT,
            wrap: false,
        }),
    );
    let group = at(
        sized(
            NodeSpec::new(
                21,
                Kind::Group {
                    flex: None,
                    target: true,
                },
            ),
            160.0,
            200.0,
        ),
        160.0,
        0.0,
    )
    .child(at(word, 30.0, 80.0));
    vec![sampler, group]
}

/// Every fixture: its name and create diff (root at 320×200 placements).
#[must_use]
pub fn per_kind() -> Vec<(&'static str, Diff)> {
    vec![
        ("rect", create(vec![backdrop(), rect()])),
        ("path", create(vec![backdrop(), path()])),
        ("text", create(vec![backdrop(), text()])),
        ("image", create(vec![backdrop(), image()])),
        ("shader", create(vec![shader()])),
        ("model", create(vec![backdrop(), model()])),
        ("cells", create(vec![backdrop(), cells()])),
        ("flex", create(vec![backdrop(), flex()])),
        ("target", create([vec![backdrop()], target()].concat())),
    ]
}

/// A dashboard: `cards` flex columns of `rows` text rows, in a flex row.
#[must_use]
pub fn dashboard(cards: u32, rows: u32) -> Diff {
    let card = |c: u32| {
        let group = NodeSpec::new(
            100 + c * 1000,
            Kind::Group {
                flex: Some(Flex {
                    direction: Direction::Column,
                    gap: 2.0,
                    padding: [8.0; 4],
                    align: Align::Stretch,
                }),
                target: false,
            },
        )
        .with(|s| s.grow = 1.0);
        (0..rows).fold(group, |g, r| {
            g.child(NodeSpec::new(
                101 + c * 1000 + r,
                Kind::Text(TextNode {
                    text: Arc::from(format!("metric {r:02} {:>6}", r * 37 + c)),
                    font: font(12.0),
                    color: FOREGROUND,
                    wrap: false,
                }),
            ))
        })
    };
    let row = NodeSpec::new(
        1,
        Kind::Group {
            flex: Some(Flex {
                direction: Direction::Row,
                gap: 12.0,
                padding: [12.0; 4],
                align: Align::Stretch,
            }),
            target: false,
        },
    )
    .with(|c| {
        c.width = Dim::Fraction(1.0);
        c.height = Dim::Fraction(1.0);
    });
    create(vec![(0..cards).fold(row, |row, c| row.child(card(c)))])
}

/// A linear tween of `ms` milliseconds.
#[must_use]
pub fn linear(ms: u64) -> MotionSpec {
    MotionDuration::new(Duration::from_millis(ms)).map_or(MotionSpec::Instant, |duration| {
        MotionSpec::Tween(TweenSpec {
            duration,
            easing: TransitionEasing::Linear,
            bezier: None,
        })
    })
}

/// The `rect` fixture (whose draw lists always run this) with its card gliding 120 px right over 500 ms.
#[must_use]
pub fn glide(base: Rev) -> Diff {
    Diff {
        base,
        ops: vec![Op::Set {
            id: NodeId(2),
            prop: Prop::X,
            value: Some(Value::Num(200.0)),
            animate: Some(Animate::Motion {
                spec: linear(500),
                repeat: Repeat::Once,
            }),
        }],
    }
}

/// A fixture scene after `steps` frames of the step clock, all visible.
#[must_use]
pub fn scene_at(diff: &Diff, motion: Option<fn(Rev) -> Diff>, steps: u32) -> Option<Scene> {
    let scene = Scene::new(SceneId(1), 1)
        .apply(diff, Policy::On)
        .ok()?
        .tick(Duration::ZERO, true);
    let scene = match motion {
        Some(m) => scene.apply(&m(scene.rev()), Policy::On).ok()?,
        None => scene,
    };
    Some((1..=steps).fold(scene, |s, n| s.tick(STEP * n, true)))
}

/// A fixture's draw list in a placement of `size` after `steps` frames.
#[must_use]
pub fn draw_list(name: &str, size: Size, steps: u32) -> Option<DrawList> {
    let (_, diff) = per_kind().into_iter().find(|(n, _)| *n == name)?;
    let motion: Option<fn(Rev) -> Diff> = (name == "rect").then_some(glide as fn(Rev) -> Diff);
    let scene = scene_at(&diff, motion, steps)?;
    let solved = layout::solve(&scene, size, &Default::default(), &mut FixedMeasure);
    Some(lower::lower(&scene, &solved, &mut FixtureResources))
}
