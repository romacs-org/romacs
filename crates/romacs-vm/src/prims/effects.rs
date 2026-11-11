//! Renderer effects (lisp/romacs/romacs-effects.el): the properties the renderer
//! defines, published to Lisp and set from it.
use super::*;

// Set EFFECT's properties, given as keyword and value pairs; an unknown
// effect or property, or a value of the wrong kind, is an error and
// changes nothing.
defsubr! { name = "romacs--effect-set", gnu = "romacs:display", effect = Global,
fn effect_set(cx, effect: Value; rest properties) { cx.effect_set(effect, properties, false) }}

// Set EFFECT to its defaults with PROPERTIES over them, as one change: the
// whole state of an effect configured one plist at a time.
defsubr! { name = "romacs--effect-replace", gnu = "romacs:display", effect = Global,
fn effect_replace(cx, effect: Value; rest properties) { cx.effect_set(effect, properties, true) }}

// EFFECT's requested properties as a plist.
defsubr! { name = "romacs-effect-get", gnu = "romacs:display", effect = Reads,
fn effect_get(cx, effect: Value) { cx.effect_get(effect) }}

// The effects there are; with SCOPE `shader' only the decorative ones.
defsubr! { name = "romacs-effect-names", gnu = "romacs:display", effect = Reads,
fn effect_names(cx; optional scope: Value) { cx.effect_names(scope) }}

// EFFECT's properties, each (KEYWORD KIND . NAMES).
defsubr! { name = "romacs-effect-schema", gnu = "romacs:display", effect = Reads,
fn effect_schema(cx, effect: Value) { cx.effect_schema(effect) }}
