//! Native 3D models drawn by the GPU renderer (rlisp/romacs-model.el,
//! prim/leaf/model.rs). Each model is an integer surface id that Lisp's
//! handle owns; its events come back as `(romacs-model ID TYPE PLIST)`
//! special events.
use super::Value;

// Whether the open display has a live GPU renderer that can draw models.
defsubr! { name = "romacs-model--available-p", gnu = "romacs:display", effect = Reads,
fn romacs_model_available_p(cx) { Ok(cx.model_available()) }}

// Create a model of MESHES, each (FILE SHA256-OR-NIL), at VIEW (YAW PITCH
// ZOOM) on BACKGROUND (R G B in 0..1), WIDTH x HEIGHT logical pixels: its
// surface id.  It loads asynchronously; a failure comes as its `failed' event.
defsubr! { name = "romacs-model--create", gnu = "romacs:display", effect = Global,
fn romacs_model_create(cx, meshes: Value, view: Value, background: Value, width: Value, height: Value) {
    cx.model_create(meshes, view, background, width, height)
}}

// Request REQ: camera YAW PITCH ZOOM for model ID.
defsubr! { name = "romacs-model--set-view", gnu = "romacs:display", effect = Global,
fn romacs_model_set_view(cx, id: Value, req: Value, yaw: Value, pitch: Value, zoom: Value) {
    cx.model_set_view(id, req, yaw, pitch, zoom)
}}

// Request REQ: model ID's highlight TARGET (a plist naming a mesh index) or
// nil, and SELECTED mesh index or nil.
defsubr! { name = "romacs-model--set-highlight", gnu = "romacs:display", effect = Global,
fn romacs_model_set_highlight(cx, id: Value, req: Value, target: Value, selected: Value) {
    cx.model_set_highlight(id, req, target, selected)
}}

// Request REQ: model ID's highlight as LAYERS, each (TARGET R G B).
defsubr! { name = "romacs-model--set-highlight-layers", gnu = "romacs:display", effect = Global,
fn romacs_model_set_highlight_layers(cx, id: Value, req: Value, layers: Value) {
    cx.model_set_highlight_layers(id, req, layers)
}}

// Request REQ: model ID drawn WIDTH x HEIGHT logical pixels.
defsubr! { name = "romacs-model--resize", gnu = "romacs:display", effect = Global,
fn romacs_model_resize(cx, id: Value, req: Value, width: Value, height: Value) {
    cx.model_resize(id, req, width, height)
}}

// Request REQ: keep model ID's last picture and stop its interaction.
defsubr! { name = "romacs-model--freeze", gnu = "romacs:display", effect = Global,
fn romacs_model_freeze(cx, id: Value, req: Value) { cx.model_freeze(id, req) }}

// Free surface ID now: t if it was live, nil if already freed or unknown.
defsubr! { name = "romacs-surface-destroy", gnu = "romacs:display", effect = Global,
fn romacs_surface_destroy(cx, id: Value) { cx.surface_destroy(id) }}
