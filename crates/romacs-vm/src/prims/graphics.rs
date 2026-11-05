//! GNU image.c's primitives over the image cache (graphics.rs,
//! prim/leaf/graphics.rs).
use super::*;
use crate::graphics::Dependency;
use crate::prim::leaf::graphics::IMAGE_TYPES;

fn invalid() -> Fault { Fault::Error("Invalid image specification".into()) }

defsubr! { name = "init-image-library", gnu = "image.c:13391", effect = Pure,
fn init_image_library(cx, kind: Value) {
    let name = if cx.kind(kind) == Kind::Symbol { cx.symbol_name(kind)? } else { return Ok(Value::NIL) };
    Ok(if IMAGE_TYPES.contains(&name.as_str()) { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "imagep", gnu = "image.c:13306", effect = Reads,
fn imagep(cx, spec: Value) {
    Ok(if cx.image_spec(spec)?.is_some() { Value::TRUE } else { Value::NIL })
}}

/// GNU `image_error': a load's message, logged to *Messages* without
/// showing it (vadd_to_log), then RESULT.
fn logged(cx: &mut Leaf<'_>, result: Value, error: Option<String>) -> Next {
    let Some(message) = error else { return Ok(Step::Done(result)) };
    let text = cx.make_string(message.into_bytes(), true);
    let format = cx.make_string(b"%s".to_vec(), false);
    let call = { let f = cx.intern("message"); let tail = cx.cons(text, Value::NIL); let tail = cx.cons(format, tail); cx.cons(f, tail) };
    let quiet = { let v = cx.intern("inhibit-message"); let binding = cx.cons(Value::TRUE, Value::NIL); let binding = cx.cons(v, binding); cx.cons(binding, Value::NIL) };
    let form = { let body = cx.cons(call, Value::NIL); let body = cx.cons(quiet, body); let head = cx.intern("let"); cx.cons(head, body) };
    Ok(Step::Call { f: cx.intern("eval"), args: vec![form], then: returned, saved: vec![result] })
}

fn returned(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next { Ok(Step::Done(saved[0])) }

defsubr! { name = "image-size", gnu = "image.c:1673", effect = Global,
fn image_size(cx, spec: Value; optional pixels: Value, frame: Value) -> Step {
    let parsed = cx.image_spec(spec)?.ok_or_else(invalid)?;
    let env = cx.image_env(frame)?;
    let (_, entry, error) = cx.image_lookup(&parsed, &env)?;
    let width = entry.facts.width + 2 * entry.margins.0;
    let height = entry.facts.height + 2 * entry.margins.1;
    let (w, h) = if pixels == Value::NIL {
        (cx.float(f64::from(width) / f64::from(env.column)), cx.float(f64::from(height) / f64::from(env.line)))
    } else {
        (Value::integer(i64::from(width)).unwrap(), Value::integer(i64::from(height)).unwrap())
    };
    let size = cx.cons(w, h);
    logged(cx, size, error)
}}

// The size in pixels `image-size' reports at scale 1 for the image in FILE
// (found as an image spec's `:file' is), read from its header, or from an
// SVG root's `viewBox', `width' and `height', without decoding it: (WIDTH .
// HEIGHT), or nil when no window system can read it so (a type its
// signature does not tell, XBM, XPM, PBM, an SVG without dimensions, a
// file not found).  It never waits for a decode (rlisp/romacs-attach.el
// sizes attachments with it).
defsubr! { name = "romacs-image-header-size", gnu = "romacs:images", effect = Reads,
fn romacs_image_header_size(cx, file: Value) { cx.image_header_size(file) }}

defsubr! { name = "image-mask-p", gnu = "image.c:1710", effect = Global,
fn image_mask_p(cx, spec: Value; optional frame: Value) -> Step {
    let parsed = cx.image_spec(spec)?.ok_or_else(invalid)?;
    let env = cx.image_env(frame)?;
    let (_, entry, error) = cx.image_lookup(&parsed, &env)?;
    logged(cx, if entry.facts.mask { Value::TRUE } else { Value::NIL }, error)
}}

defsubr! { name = "image-metadata", gnu = "image.c:1733", effect = Global,
fn image_metadata(cx, spec: Value; optional frame: Value) -> Step {
    let Some(parsed) = cx.image_spec(spec)? else { return Ok(Step::Done(Value::NIL)) };
    let env = cx.image_env(frame)?;
    let (_, entry, error) = cx.image_lookup(&parsed, &env)?;
    let metadata = &entry.facts.metadata;
    // GNU's `img->lisp_data': a GIF's `(count N delay D extension-data E)',
    // a WebP's `(delay D count N)'.
    let count = metadata.frame_count().map(|n| Value::integer(i64::from(n)).unwrap());
    let delay = metadata.frame_delay().and_then(|d| d.seconds()).map(|s| cx.float(s));
    let mut extension = Value::NIL;
    for (code, data) in entry.extension.iter().rev() {
        let data = cx.make_string(data.clone(), false);
        extension = cx.cons(data, extension);
        extension = cx.cons(Value::integer(i64::from(*code)).unwrap(), extension);
    }
    let order: &[&str] = if parsed.kind == "webp" { &["delay", "count"] } else { &["count", "delay", "extension-data"] };
    let mut list = Value::NIL;
    for name in order.iter().rev() {
        let value = match *name { "count" => count, "delay" => delay, _ => (extension != Value::NIL).then_some(extension) };
        if let Some(value) = value {
            list = cx.cons(value, list);
            let key = cx.intern(name);
            list = cx.cons(key, list);
        }
    }
    logged(cx, list, error)
}}

defsubr! { name = "image-flush", gnu = "image.c:2571", effect = Global,
fn image_flush(cx, spec: Value; optional frame: Value) {
    let parsed = cx.image_spec(spec)?.ok_or_else(invalid)?;
    // FRAME t flushes SPEC on every frame.
    let all = frame == Value::TRUE;
    let env = cx.image_env(if all { Value::NIL } else { frame })?;
    let (key, _, _) = cx.image_lookup_display(&parsed, &env)?;
    cx.image_uncache(|k, _| k.plist == key.plist && (all || k.frame == key.frame));
    Ok(Value::NIL)
}}

defsubr! { name = "clear-image-cache", gnu = "image.c:2467", effect = Global,
fn clear_image_cache(cx; optional filter: Value, animation_cache: Value) {
    // The animation decoder caches are the display host's own; until
    // multi-frame images are shown it keeps none for an image.
    if animation_cache != Value::NIL { return Ok(Value::NIL) }
    if filter == Value::NIL || cx.kind(filter) == Kind::Frame {
        let frame = cx.image_env(filter)?.frame;
        cx.image_uncache(|k, _| k.frame == frame);
    } else if filter == Value::TRUE {
        cx.image_uncache(|_, _| true);
    } else {
        let auto = filter == cx.intern("auto");
        let file = cx.string_view(filter).ok().map(|s| String::from_utf8_lossy(s.bytes()).into_owned());
        cx.image_uncache(|_, e| e.dependencies.iter().any(|d| match d {
            Dependency::Auto => auto,
            Dependency::File(f) => file.as_deref() == Some(f.as_str()),
        }));
    }
    Ok(Value::NIL)
}}

defsubr! { name = "image-cache-size", gnu = "image.c:13367", effect = Reads,
fn image_cache_size(cx) {
    Ok(Value::integer(cx.image_cache_bytes() as i64).unwrap())
}}

defsubr! { name = "image-transforms-p", gnu = "image.c:13335", effect = Reads,
fn image_transforms_p(cx; optional frame: Value) {
    let _ = frame;
    if !cx.window_system_frame_p() { return Ok(Value::NIL) }
    let (scale, rotate) = (cx.intern("scale"), cx.intern("rotate90"));
    let tail = cx.cons(rotate, Value::NIL);
    Ok(cx.cons(scale, tail))
}}

defsubr! { name = "lookup-image", gnu = "image.c:13315", effect = Global,
fn lookup_image(cx, spec: Value) -> Step {
    // GNU defines it only in GLYPH_DEBUG builds: the image's cache id.
    let parsed = cx.image_spec(spec)?.ok_or_else(invalid)?;
    let env = cx.image_env(Value::NIL)?;
    let (key, _, error) = cx.image_lookup(&parsed, &env)?;
    let id = Value::integer(cx.image_cache_id(&key) as i64).unwrap();
    logged(cx, id, error)
}}

defsubr! { name = "lookup-image-map", gnu = "xdisp.c:36552", effect = Reads,
fn lookup_image_map(cx, map: Value, x: Value, y: Value) {
    if map == Value::NIL { return Ok(Value::NIL) }
    let x = x.integer_value().ok_or(Fault::Wrong("fixnump", x))?.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    let y = y.integer_value().ok_or(Fault::Wrong("fixnump", y))?.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    // find_hot_spot: the first element whose AREA holds X, Y.
    let mut tail = map;
    for _ in 0..cx.list_limit() {
        let Some((element, rest)) = cx.pair(tail) else { break };
        if let Some((area, _)) = cx.pair(element) && on_hot_spot(cx, area, x, y)? { return Ok(element) }
        tail = rest;
    }
    Ok(Value::NIL)
}}

/// xdisp.c `on_hot_spot_p': `(rect . ((X0 . Y0) . (X1 . Y1)))',
/// `(circle . ((X0 . Y0) . R))' or `(poly . [X0 Y0 X1 Y1 ...])' holds X, Y.
fn on_hot_spot(cx: &mut Leaf<'_>, area: Value, x: i64, y: i64) -> std::result::Result<bool, Fault> {
    let Some((shape, body)) = cx.pair(area) else { return Ok(false) };
    let int = |v: Value| v.integer_value();
    if shape == cx.intern("rect") {
        let Some(((x0, y0), (x1, y1))) = cx.pair(body).and_then(|(a, b)| Some((cx.pair(a)?, cx.pair(b)?))) else { return Ok(false) };
        return Ok(int(x0).is_some_and(|v| x >= v) && int(y0).is_some_and(|v| y >= v)
            && int(x1).is_some_and(|v| x <= v) && int(y1).is_some_and(|v| y <= v));
    }
    if shape == cx.intern("circle") {
        let Some((center, radius)) = cx.pair(body) else { return Ok(false) };
        let Some((cx0, cy0)) = cx.pair(center) else { return Ok(false) };
        let Some(r) = cx.number_value(radius) else { return Ok(false) };
        let (Some(x0), Some(y0)) = (int(cx0), int(cy0)) else { return Ok(false) };
        let (dx, dy) = ((x0 - x) as f64, (y0 - y) as f64);
        return Ok(dx * dx + dy * dy <= r * r);
    }
    if shape == cx.intern("poly") {
        let Ok(points) = cx.vector_values(body) else { return Ok(false) };
        let n = points.len();
        if n < 6 || n % 2 == 1 { return Ok(false) }
        let (Some(mut x0), Some(mut y0)) = (int(points[n - 2]), int(points[n - 1])) else { return Ok(false) };
        let mut inside = false;
        for pair in points.chunks(2) {
            let (x1, y1) = (x0, y0);
            let (Some(px), Some(py)) = (int(pair[0]), int(pair[1])) else { return Ok(false) };
            (x0, y0) = (px, py);
            // Does this segment cross the line from (X, Y) to (X, infinity)?
            if x0 >= x { if x1 >= x { continue } } else if x1 < x { continue }
            if y > y0 && y > y1 { continue }
            if y < y0 + ((y1 - y0) * (x - x0)) / (x1 - x0) { inside = !inside; }
        }
        return Ok(inside);
    }
    Ok(false)
}
