//! Native model primitives (lisp/romacs/romacs-model.el, `display_host/models.rs`): Lisp's
//! arguments checked and turned into the model protocol's values, then
//! sent to the open display. Mesh IDs stay in Lisp; the renderer sees
//! mesh indices only.
use super::{Fault, Leaf, Value, data};
use crate::prim::Out;
use romacs_display_protocol::model::{
    MODEL_EDGE_RANGE, MODEL_MAX_HIGHLIGHT_LAYERS, MODEL_MAX_MESHES, ModelChange, ModelHighlightLayer, ModelMeshSpec,
    ModelTarget, ModelView,
};

/// The most elements a target plist has: :mesh :kind :face :barycentric
/// :tolerance, each with its value.
const MODEL_TARGET_FIELDS: usize = 10;

fn error(message: impl Into<String>) -> Fault {
    Fault::Error(message.into())
}

fn natural(value: Value, what: &str) -> std::result::Result<u64, Fault> {
    value.integer_value().and_then(|n| u64::try_from(n).ok())
        .ok_or_else(|| error(format!("Model {what} must be a natural number")))
}

fn edge(value: Value, what: &str) -> std::result::Result<u32, Fault> {
    let edge = natural(value, what)?;
    u32::try_from(edge).ok().filter(|edge| MODEL_EDGE_RANGE.contains(edge))
        .ok_or_else(|| error(format!("Model {what} must be {}..{} pixels", MODEL_EDGE_RANGE.start(), MODEL_EDGE_RANGE.end())))
}

fn mesh_index(value: Value, meshes: usize) -> std::result::Result<u16, Fault> {
    natural(value, "mesh index").ok().and_then(|n| u16::try_from(n).ok()).filter(|&n| usize::from(n) < meshes)
        .ok_or_else(|| error("Model mesh index out of range"))
}

/// A fixnum as the protocol's f64, as Lisp's `float` converts it.
#[expect(clippy::cast_precision_loss, reason = "a fixnum past 2^53 rounds, as GNU's float does")]
fn fixnum_f64(n: i64) -> f64 {
    n as f64
}

/// A colour or background component, checked in 0..1 before it narrows.
#[expect(clippy::cast_possible_truncation, reason = "a component in 0..1 keeps its value as f32")]
fn component(value: f64) -> f32 {
    value as f32
}

impl Leaf<'_> {
    /// `romacs-model--available-p`: whether the open display has a live
    /// GPU renderer.
    pub(crate) fn model_available(&mut self) -> Value {
        let live = self.vm.window_system_host().is_some_and(|host| host.model_available());
        if live { Value::TRUE } else { Value::NIL }
    }

    /// A number: a fixnum or a float.
    fn model_number(&self, value: Value, what: &str) -> std::result::Result<f64, Fault> {
        value.integer_value().map(fixnum_f64)
            .map_or_else(|| self.float_value(value), Ok)
            .map_err(|_| error(format!("Model {what} must be a number")))
    }

    /// VALUE's elements, a proper list of at most MAX: a longer, dotted or
    /// circular one is refused before any element is converted, having
    /// walked no more than MAX links.
    fn model_items(&self, value: Value, what: &str, max: usize) -> std::result::Result<Vec<Value>, Fault> {
        let refuse = || error(format!("Model {what} must be a list of at most {max}"));
        let mut items = Vec::new();
        let mut rest = value;
        while rest != Value::NIL {
            if items.len() == max { return Err(refuse()); }
            let (item, next) = self.vm.cons_pair(rest).map_err(|_| refuse())?;
            items.push(item);
            rest = next;
        }
        Ok(items)
    }

    fn model_view(&self, yaw: Value, pitch: Value, zoom: Value) -> std::result::Result<ModelView, Fault> {
        ModelView { yaw: self.model_number(yaw, "yaw")?, pitch: self.model_number(pitch, "pitch")?, zoom: self.model_number(zoom, "zoom")? }
            .validated().map_err(error)
    }


    /// One mesh, (PATH SHA256-OR-NIL): an absolute local path, and
    /// optionally the lowercase hex digest the loader checks.
    fn model_mesh(&self, value: Value) -> std::result::Result<ModelMeshSpec, Fault> {
        let parts = self.model_items(value, "mesh", 2)?;
        let path = parts.first().and_then(|&p| self.string_text(p).ok())
            .ok_or_else(|| error("Model mesh needs a file name"))?;
        let path = std::path::PathBuf::from(path);
        if !path.is_absolute() { return Err(error("Model mesh file names must be absolute")); }
        let sha256 = match parts.get(1).copied().filter(|&v| v != Value::NIL) {
            None => None,
            Some(digest) => {
                let text = self.string_text(digest).map_err(|_| error("Model mesh digest must be a string"))?;
                let bytes = (text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
                    .then(|| (0..32).map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<_>>());
                Some(bytes.and_then(|b| <[u8; 32]>::try_from(b).ok())
                    .ok_or_else(|| error("Model mesh digest must be 64 lowercase hex digits"))?)
            }
        };
        Ok(ModelMeshSpec { path, sha256 })
    }

    fn model_background(&self, value: Value) -> std::result::Result<[f32; 4], Fault> {
        let parts = self.model_items(value, "background", 3)?;
        let rgb = parts.iter().map(|&c| self.model_number(c, "background")).collect::<std::result::Result<Vec<_>, _>>()?;
        match rgb[..] {
            [r, g, b] if rgb.iter().all(|c| (0.0..=1.0).contains(c)) => Ok([component(r), component(g), component(b), 1.0]),
            _ => Err(error("Model background must be three numbers in 0..1")),
        }
    }

    /// `romacs-model--create` MESHES VIEW BACKGROUND WIDTH HEIGHT: a new
    /// model's surface id. It loads asynchronously; failures come back as
    /// its `failed` event.
    pub(crate) fn model_create(&mut self, meshes: Value, view: Value, background: Value, width: Value, height: Value) -> Out {
        let meshes = self.model_items(meshes, "meshes", MODEL_MAX_MESHES)?.into_iter().map(|m| self.model_mesh(m)).collect::<std::result::Result<Vec<_>, _>>()?;
        if meshes.is_empty() {
            return Err(error(format!("A model needs 1..{MODEL_MAX_MESHES} meshes")));
        }
        let view = match self.model_items(view, "view", 3)?[..] {
            [yaw, pitch, zoom] => self.model_view(yaw, pitch, zoom)?,
            _ => return Err(error("Model view must be (YAW PITCH ZOOM)")),
        };
        let background = self.model_background(background)?;
        let size = (edge(width, "width")?, edge(height, "height")?);
        let count = meshes.len();
        let display = self.vm.display.as_mut().filter(|d| d.open)
            .ok_or_else(|| error("Native models need a graphical display"))?;
        if !display.host.model_available() { return Err(error("No GPU renderer is live to draw native models")); }
        let id = display.surfaces.allocate();
        display.host.create_model(id, crate::ModelCreate { meshes, view, background, size }).map_err(error)?;
        display.surfaces.insert(id, count);
        Ok(Value::integer(i64::from(id)).expect("a surface id is a fixnum"))
    }

    /// Send CHANGE as request REQ to live model ID.
    fn model_update(&mut self, id: Value, req: Value, change: impl FnOnce(&Self, usize) -> std::result::Result<ModelChange, Fault>) -> Out {
        let id = u32::try_from(natural(id, "id")?).map_err(|_| error("Model id out of range"))?;
        let req = natural(req, "request")?;
        let meshes = self.vm.display.as_ref().and_then(|d| d.surfaces.meshes(id))
            .ok_or_else(|| error("Model is destroyed"))?;
        let change = change(self, meshes)?;
        let display = self.vm.display.as_mut().filter(|d| d.open).ok_or_else(|| error("Native models need a graphical display"))?;
        display.host.update_model(id, req, change).map_err(error)?;
        Ok(Value::NIL)
    }

    pub(crate) fn model_set_view(&mut self, id: Value, req: Value, yaw: Value, pitch: Value, zoom: Value) -> Out {
        self.model_update(id, req, |cx, _| Ok(ModelChange::View(cx.model_view(yaw, pitch, zoom)?)))
    }


    fn model_plist_get(&self, plist: &[Value], key: &str) -> Option<Value> {
        plist.chunks(2).find(|pair| self.symbol_name(pair[0]).ok().as_deref() == Some(key)).and_then(|pair| pair.get(1).copied())
    }

    /// A highlight TARGET: (:mesh INDEX :kind object), (:mesh INDEX :kind
    /// point :face N :barycentric [A B C] :tolerance R) or (:mesh INDEX
    /// :kind region :faces (N ...)).
    fn model_target(&self, value: Value, meshes: usize) -> std::result::Result<ModelTarget, Fault> {
        let plist = self.model_items(value, "target", MODEL_TARGET_FIELDS)?;
        let field = |key: &str| self.model_plist_get(&plist, key).ok_or_else(|| error(format!("Model target needs {key}")));
        let mesh = mesh_index(field(":mesh")?, meshes)?;
        let face = |value| natural(value, "face").ok().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| error("Model face out of range"));
        let target = match self.symbol_name(field(":kind")?)?.as_str() {
            "object" => ModelTarget::Object { mesh },
            "point" => {
                let weights = match self.vm.object_if(field(":barycentric")?) {
                    // Its length first: no work in proportion to a long vector.
                    Some(data::Object::Vector(items)) if items.len() <= 3 => items.clone(),
                    Some(data::Object::Vector(_)) => return Err(error("Model barycentric needs three weights")),
                    _ => self.model_items(field(":barycentric")?, "barycentric", 3)?,
                };
                let weights = weights.iter().map(|&w| self.model_number(w, "barycentric weight")).collect::<std::result::Result<Vec<_>, _>>()?;
                let barycentric = <[f64; 3]>::try_from(weights).map_err(|_| error("Model barycentric needs three weights"))?;
                ModelTarget::Point { mesh, face: face(field(":face")?)?, barycentric, tolerance: self.model_number(field(":tolerance")?, "tolerance")? }
            }
            "region" => {
                let faces = self.model_items(field(":faces")?, "faces", self.vm.list_limit())?.into_iter().map(face).collect::<std::result::Result<Vec<_>, _>>()?;
                ModelTarget::Region { mesh, faces }
            }
            other => return Err(error(format!("Unknown model target kind {other}"))),
        };
        target.validated().map_err(error)
    }

    /// `romacs-model--set-highlight` ID REQ TARGET SELECTED.
    pub(crate) fn model_set_highlight(&mut self, id: Value, req: Value, target: Value, selected: Value) -> Out {
        self.model_update(id, req, |cx, meshes| {
            let target = (target != Value::NIL).then(|| cx.model_target(target, meshes)).transpose()?;
            let selected = (selected != Value::NIL).then(|| mesh_index(selected, meshes)).transpose()?;
            Ok(ModelChange::Highlight { target, selected })
        })
    }

    /// `romacs-model--set-highlight-layers` ID REQ LAYERS, each (TARGET R G B).
    pub(crate) fn model_set_highlight_layers(&mut self, id: Value, req: Value, layers: Value) -> Out {
        self.model_update(id, req, |cx, meshes| {
            let layers = cx.model_items(layers, "layers", MODEL_MAX_HIGHLIGHT_LAYERS)?;
            let layers = layers.into_iter().map(|layer| {
                let parts = cx.model_items(layer, "layer", 4)?;
                let (&target, rgb) = parts.split_first().ok_or_else(|| error("Model layer needs a target"))?;
                let rgb = rgb.iter().map(|&c| cx.model_number(c, "colour").map(component)).collect::<std::result::Result<Vec<_>, _>>()?;
                let color = <[f32; 3]>::try_from(rgb).map_err(|_| error("Model layer colour needs three components"))?;
                ModelHighlightLayer { target: cx.model_target(target, meshes)?, color }.validated().map_err(error)
            }).collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(ModelChange::HighlightLayers(layers))
        })
    }

    pub(crate) fn model_resize(&mut self, id: Value, req: Value, width: Value, height: Value) -> Out {
        self.model_update(id, req, |_, _| Ok(ModelChange::Resize { width: edge(width, "width")?, height: edge(height, "height")? }))
    }

    pub(crate) fn model_freeze(&mut self, id: Value, req: Value) -> Out {
        self.model_update(id, req, |_, _| Ok(ModelChange::Freeze))
    }

    /// The slot a `(surface :id ID :width WIDTH :height HEIGHT)` display
    /// spec shows: WIDTH x HEIGHT pixels standing on the baseline, holding
    /// live surface ID.  None for a malformed spec or a freed surface,
    /// whose text then shows instead.
    pub(crate) fn surface_glyph(&self, spec: Value) -> Option<romacs_layout_input::InlineImage> {
        let plist = self.vm.list_values(spec).ok()?;
        let plist = plist.get(1..)?;
        let id = u32::try_from(self.model_plist_get(plist, ":id")?.integer_value()?).ok()?;
        let edge = |key| self.model_plist_get(plist, key).and_then(|v| edge(v, key).ok()).map(i64::from);
        let (width, height) = (edge(":width")?, edge(":height")?);
        self.vm.display.as_ref()?.surfaces.meshes(id)?;
        Some(romacs_layout_input::InlineImage {
            byte: 0, image: 0, surface: id, width, ascent: height, descent: 0, body_x: 0, body_y: 0,
            source: (0, 0, width, height), full: (width, height), relief: 0, relief_background: None,
        })
    }

    /// `romacs-surface-destroy` ID: free a live surface; a freed or unknown
    /// one is left alone, so a finalizer after an explicit destroy is safe.
    /// ID must be a fixnum.
    pub(crate) fn surface_destroy(&mut self, id: Value) -> Out {
        let n = id.integer_value().ok_or(Fault::Wrong("fixnump", id))?;
        let Ok(id) = u32::try_from(n) else { return Ok(Value::NIL) };
        if let Some(display) = self.vm.display.as_mut() && display.surfaces.remove(id) {
            display.host.free_surface(id);
            // Its glyphs show their text from the next redisplay: every
            // window republishes, as after `force-window-update' nil.
            self.vm.display_state.free_realized_faces();
            return Ok(Value::TRUE);
        }
        Ok(Value::NIL)
    }
}
