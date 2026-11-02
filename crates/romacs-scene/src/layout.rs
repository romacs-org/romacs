//! Layout of one placement: taffy over the scene tree (§2.2).
//!
//! A flex `group` lays out its children with taffy; any other group places
//! its children at its own origin, where their transforms move them. Leaves
//! with an `Auto` size take their measured content: text through [`Measure`]
//! (the frame's font resolver at wiring time), cells as a grid of the font's
//! cell, paths as their bounds, images at their natural size. An anchored
//! node is placed at its anchor's box, which `layout2` publishes per window.
//!
//! Boxes are relative to the parent's box. Layout reads properties, never
//! motion: a size motion moves the drawn box only (§2.3), so a running motion
//! costs no layout.

use std::collections::BTreeMap;
use std::sync::Arc;

use kurbo::{Rect, Shape, Size};
use taffy::prelude::{
    AvailableSpace, Dimension, Display, FlexDirection, LengthPercentage, LengthPercentageAuto,
    Position, TaffyMaxContent, TaffyTree,
};
use taffy::style::{AlignItems, Style};

use crate::node::{
    Align, AnchorId, Dim, Direction, Font, ImageSource, Kind, Node, NodeId, TextNode,
};
use crate::scene::Scene;

/// One laid-out line of a text node.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: Arc<str>,
    /// Baseline offset from the top of the node's box.
    pub baseline: f64,
    pub width: f64,
}

/// A text node measured at a width.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measured {
    pub size: Size,
    pub lines: Vec<Line>,
}

/// Content measurement, supplied by the host's font resolver.
pub trait Measure {
    /// Shape `text`, wrapping at `max_width` when the node wraps.
    fn text(&mut self, text: &TextNode, max_width: Option<f64>) -> Measured;
    /// The advance of one cell of a grid in `font`.
    fn cell(&mut self, font: &Font) -> Size;
    /// An image's natural size, when known.
    fn image(&mut self, source: &ImageSource) -> Option<Size>;
}

/// What lowering needs beyond a node's box.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Lines(Vec<Line>),
    Cell(Size),
}

/// One placement's solved layout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Solved {
    pub size: Size,
    /// Parent-relative boxes. A node without one (an absent anchor, or below
    /// such a node) is not drawn.
    pub boxes: BTreeMap<NodeId, Rect>,
    pub content: BTreeMap<NodeId, Content>,
}

impl Solved {
    /// A node's box in scene coordinates, ignoring transforms.
    #[must_use]
    pub fn in_scene(&self, scene: &Scene, id: NodeId) -> Option<Rect> {
        let own = *self.boxes.get(&id)?;
        let offset = crate::scene::ancestors(scene.nodes(), id)
            .skip(1)
            .filter_map(|a| self.boxes.get(&a))
            .fold(kurbo::Vec2::ZERO, |acc, r| acc + r.origin().to_vec2());
        Some(own + offset)
    }
}

/// Lay out `scene` for a placement of `size` with the window's `anchors`.
pub fn solve(
    scene: &Scene,
    size: Size,
    anchors: &BTreeMap<AnchorId, Rect>,
    measure: &mut dyn Measure,
) -> Solved {
    let mut tree: TaffyTree<NodeId> = TaffyTree::new();
    tree.disable_rounding();
    let mut ids = BTreeMap::new();
    let mut texts = Texts::new();
    let root = build(&mut tree, scene, NodeId::ROOT, None, anchors, &mut ids);
    let _ = tree.set_style(
        root,
        Style {
            size: taffy::Size {
                width: Dimension::length(size.width as f32),
                height: Dimension::length(size.height as f32),
            },
            ..tree.style(root).cloned().unwrap_or_default()
        },
    );
    let _ = tree.compute_layout_with_measure(
        root,
        taffy::Size::MAX_CONTENT,
        |known, available, _, ctx, _| {
            let Some(id) = ctx.copied() else {
                return taffy::Size::ZERO;
            };
            let Some(node) = scene.node(id) else {
                return taffy::Size::ZERO;
            };
            let width = known.width.map(f64::from).or(match available.width {
                AvailableSpace::Definite(w) => Some(f64::from(w)),
                _ => None,
            });
            let s = intrinsic(id, node, width, measure, &mut texts);
            taffy::Size {
                width: known.width.unwrap_or(s.width as f32),
                height: known.height.unwrap_or(s.height as f32),
            }
        },
    );
    let mut solved = Solved {
        size,
        ..Solved::default()
    };
    collect(
        &tree,
        scene,
        NodeId::ROOT,
        &ids,
        &mut solved,
        &mut |id, node, width| content(id, node, width, measure, &mut texts),
    );
    solved
}

/// Text measured per node and wrap width: flex layout asks for the same
/// leaf several times per pass, and lowering asks once more for its lines.
type Texts = std::collections::HashMap<(NodeId, Option<u64>), Measured>;

fn measure_text<'t>(
    id: NodeId,
    t: &TextNode,
    width: Option<f64>,
    measure: &mut dyn Measure,
    texts: &'t mut Texts,
) -> &'t Measured {
    let width = width.filter(|_| t.wrap);
    texts
        .entry((id, width.map(f64::to_bits)))
        .or_insert_with(|| measure.text(t, width))
}

/// Record the boxes of a displayed subtree; a hidden node hides its subtree.
fn collect(
    tree: &TaffyTree<NodeId>,
    scene: &Scene,
    id: NodeId,
    ids: &BTreeMap<NodeId, taffy::NodeId>,
    solved: &mut Solved,
    content: &mut dyn FnMut(NodeId, &Node, f64) -> Option<Content>,
) {
    let (Some(t), Some(node)) = (ids.get(&id), scene.node(id)) else {
        return;
    };
    if tree.style(*t).is_ok_and(|s| s.display == Display::None) {
        return;
    }
    let Ok(layout) = tree.layout(*t) else { return };
    let r = Rect::from_origin_size(
        (f64::from(layout.location.x), f64::from(layout.location.y)),
        (f64::from(layout.size.width), f64::from(layout.size.height)),
    );
    solved.boxes.insert(id, r);
    if let Some(c) = content(id, node, r.width()) {
        solved.content.insert(id, c);
    }
    for c in &node.children {
        collect(tree, scene, *c, ids, solved, content);
    }
}

fn build(
    tree: &mut TaffyTree<NodeId>,
    scene: &Scene,
    id: NodeId,
    parent_flex: Option<bool>,
    anchors: &BTreeMap<AnchorId, Rect>,
    ids: &mut BTreeMap<NodeId, taffy::NodeId>,
) -> taffy::NodeId {
    let node = &scene.nodes()[&id];
    let flex = node.kind.flex();
    let children: Vec<_> = node
        .children
        .iter()
        .map(|c| build(tree, scene, *c, Some(flex.is_some()), anchors, ids))
        .collect();
    let style = style(node, parent_flex, anchors);
    let t = if children.is_empty() && !node.kind.is_group() {
        tree.new_leaf_with_context(style, id)
    } else {
        tree.new_with_children(style, &children)
    }
    .expect("taffy node");
    ids.insert(id, t);
    t
}

fn style(node: &Node, parent_flex: Option<bool>, anchors: &BTreeMap<AnchorId, Rect>) -> Style {
    let c = &node.common;
    let dim = |d: Dim| match d {
        Dim::Auto => Dimension::auto(),
        Dim::Px(v) => Dimension::length(v),
        Dim::Fraction(v) => Dimension::percent(v),
    };
    let mut style = Style {
        display: Display::Flex,
        size: taffy::Size {
            width: dim(c.width),
            height: dim(c.height),
        },
        flex_grow: c.grow,
        flex_shrink: 0.0,
        ..Style::default()
    };
    if let Some(f) = node.kind.flex() {
        style.flex_direction = match f.direction {
            Direction::Row => FlexDirection::Row,
            Direction::Column => FlexDirection::Column,
        };
        style.gap = taffy::Size {
            width: LengthPercentage::length(f.gap),
            height: LengthPercentage::length(f.gap),
        };
        let [top, right, bottom, left] = f.padding.map(LengthPercentage::length);
        style.padding = taffy::Rect {
            left,
            right,
            top,
            bottom,
        };
        style.align_items = Some(match f.align {
            Align::Stretch => AlignItems::STRETCH,
            Align::Start => AlignItems::FLEX_START,
            Align::Center => AlignItems::CENTER,
            Align::End => AlignItems::FLEX_END,
        });
    }
    let at = |x: f64, y: f64| taffy::Rect {
        left: LengthPercentageAuto::length(x as f32),
        top: LengthPercentageAuto::length(y as f32),
        right: LengthPercentageAuto::auto(),
        bottom: LengthPercentageAuto::auto(),
    };
    match (c.anchor, parent_flex) {
        (Some(a), _) => match anchors.get(&a) {
            Some(r) => {
                style.position = Position::Absolute;
                style.inset = at(r.x0, r.y0);
                if c.width == Dim::Auto {
                    style.size.width = Dimension::length(r.width() as f32);
                }
                if c.height == Dim::Auto {
                    style.size.height = Dimension::length(r.height() as f32);
                }
            }
            None => style.display = Display::None,
        },
        (None, Some(false)) => {
            style.position = Position::Absolute;
            style.inset = at(0.0, 0.0);
        }
        _ => {}
    }
    style
}

/// A leaf's content size at an available width.
fn intrinsic(
    id: NodeId,
    node: &Node,
    width: Option<f64>,
    measure: &mut dyn Measure,
    texts: &mut Texts,
) -> Size {
    match &node.kind {
        Kind::Text(t) => measure_text(id, t, width, measure, texts).size,
        Kind::Cells(g) => {
            let cell = measure.cell(&g.font);
            Size::new(
                cell.width * f64::from(g.cols),
                cell.height * f64::from(g.rows),
            )
        }
        Kind::Path(p) => {
            let b = p.data.bounding_box();
            if p.data.elements().is_empty() {
                Size::ZERO
            } else {
                Size::new(b.x1.max(0.0), b.y1.max(0.0))
            }
        }
        Kind::Image(i) => measure.image(&i.source).unwrap_or(Size::ZERO),
        _ => Size::ZERO,
    }
}

fn content(
    id: NodeId,
    node: &Node,
    width: f64,
    measure: &mut dyn Measure,
    texts: &mut Texts,
) -> Option<Content> {
    match &node.kind {
        Kind::Text(t) => Some(Content::Lines(
            measure_text(id, t, Some(width), measure, texts)
                .lines
                .clone(),
        )),
        Kind::Cells(g) => Some(Content::Cell(measure.cell(&g.font))),
        _ => None,
    }
}
