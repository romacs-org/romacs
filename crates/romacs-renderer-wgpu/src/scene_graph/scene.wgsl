// The scene pass. Vertex positions are target device pixels; colours are
// linear and premultiplied; the target is sRGB, so blending is in linear
// light. Every pipeline uses premultiplied-alpha blending.

struct Uniforms {
    screen_size: vec2<f32>,
    time: f32,
    content_alpha: f32,
    content_scale: f32,
    _pivot_padding: f32,
    content_pivot: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(1) @binding(0)
var t_source: texture_2d<f32>;
@group(1) @binding(1)
var s_source: sampler;

fn clip_position(p: vec2<f32>) -> vec4<f32> {
    let x = (p.x / uniforms.screen_size.x) * 2.0 - 1.0;
    let y = 1.0 - (p.y / uniforms.screen_size.y) * 2.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

// ─── Rect terms (RoundedRectVertex) ──────────────────────────────────

const TERM_BODY: i32 = 0;
const TERM_SHADOW: i32 = 1;
const TERM_GLOW: i32 = 2;
const TERM_SAMPLE: i32 = 3;

struct RectIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) rect_min: vec2<f32>,
    @location(3) rect_max: vec2<f32>,
    @location(4) params: vec2<f32>,
    @location(5) style: vec4<f32>,
    @location(6) color2: vec4<f32>,
}

struct RectOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) color2: vec4<f32>,
    @location(2) @interpolate(flat) rect: vec4<f32>,
    @location(3) @interpolate(flat) params: vec2<f32>,
    @location(4) @interpolate(flat) term: vec2<f32>,
    @location(5) local: vec2<f32>,
}

@vertex
fn vs_rect(in: RectIn) -> RectOut {
    var out: RectOut;
    out.position = clip_position(in.position);
    out.color = in.color;
    out.color2 = in.color2;
    out.rect = vec4<f32>(in.rect_min, in.rect_max);
    out.params = in.params;
    out.term = in.style.xy;
    out.local = in.style.zw;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn rect_distance(in: RectOut) -> f32 {
    let center = (in.rect.xy + in.rect.zw) * 0.5;
    let half_size = (in.rect.zw - in.rect.xy) * 0.5;
    return sd_rounded_box(in.local - center, half_size, in.params.y);
}

// Coverage of the inside of an edge at signed distance d, `aa` units wide.
fn inside(d: f32, aa: f32) -> f32 {
    return clamp(0.5 - d / aa, 0.0, 1.0);
}

// Abramowitz–Stegun 7.1.26-style closed form; |error| < 2e-4, plenty for
// an 8-bit shadow.
fn erf(x: f32) -> f32 {
    let a = 0.147;
    let x2 = x * x;
    let t = sqrt(1.0 - exp(-x2 * (1.2732395 + a * x2) / (1.0 + a * x2)));
    return sign(x) * t;
}

@fragment
fn fs_rect(in: RectOut) -> @location(0) vec4<f32> {
    let d = rect_distance(in);
    let aa = max(fwidth(d), 1e-4);
    let term = i32(round(in.term.x));
    if term == TERM_SHADOW {
        let sigma = in.term.y;
        var coverage: f32;
        if sigma * 2.0 < aa {
            coverage = inside(d, aa);
        } else {
            coverage = 0.5 - 0.5 * erf(d / (sigma * 1.4142135));
        }
        return in.color * coverage;
    }
    if term == TERM_GLOW {
        // Emitted light: full strength at the edge, a smooth falloff to
        // nothing at `radius`, outside the shape only.
        let t = clamp(d / max(in.term.y, 1e-4), 0.0, 1.0);
        let falloff = (1.0 - t) * (1.0 - t) * (1.0 - t * 0.5);
        return in.color * falloff * (1.0 - inside(d, aa));
    }
    // The body: the stroke band over the fill, then the item's opacity.
    let outer = inside(d, aa);
    let stroke = in.color2 * (outer - inside(d + in.params.x, aa));
    let fill = in.color * outer;
    return (stroke + fill * (1.0 - stroke.a)) * in.term.y;
}

// Bilinear filtering of a straight-alpha texture, premultiplying each texel
// before it is weighted: filtering first would pull the colour of
// transparent neighbours (usually black) into every soft edge.
fn sample_straight(uv: vec2<f32>) -> vec4<f32> {
    let size = textureDimensions(t_source);
    let last = vec2<i32>(size) - vec2<i32>(1);
    let p = uv * vec2<f32>(size) - 0.5;
    let f = fract(p);
    let i = vec2<i32>(floor(p));
    let a = premultiply(textureLoad(t_source, clamp(i, vec2<i32>(0), last), 0));
    let b = premultiply(textureLoad(t_source, clamp(i + vec2<i32>(1, 0), vec2<i32>(0), last), 0));
    let c = premultiply(textureLoad(t_source, clamp(i + vec2<i32>(0, 1), vec2<i32>(0), last), 0));
    let e = premultiply(textureLoad(t_source, clamp(i + vec2<i32>(1, 1), vec2<i32>(0), last), 0));
    return mix(mix(a, b, f.x), mix(c, e, f.x), f.y);
}

fn premultiply(texel: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(texel.rgb * texel.a, texel.a);
}

// A texture drawn into the rect, corners rounded by the same distance.
// `color2.x` is 1 for straight-alpha sources (images, shader surfaces) and
// 0 for premultiplied ones (scene targets).
@fragment
fn fs_sample(in: RectOut) -> @location(0) vec4<f32> {
    let d = rect_distance(in);
    let aa = max(fwidth(d), 1e-4);
    let uv = (in.local - in.rect.xy) / max(in.rect.zw - in.rect.xy, vec2<f32>(1e-4));
    var texel = textureSample(t_source, s_source, uv);
    if in.color2.x > 0.5 {
        texel = sample_straight(uv);
    }
    return texel * in.color.a * inside(d, aa);
}

// ─── Path triangles (RectVertex) ─────────────────────────────────────

struct FillIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
}

struct FillOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_fill(in: FillIn) -> FillOut {
    var out: FillOut;
    out.position = clip_position(in.position);
    out.color = in.color;
    return out;
}

@fragment
fn fs_fill(in: FillOut) -> @location(0) vec4<f32> {
    return in.color;
}

// ─── Glyphs (GlyphVertex) ────────────────────────────────────────────

struct GlyphIn {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct GlyphOut {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_glyph(in: GlyphIn) -> GlyphOut {
    var out: GlyphOut;
    out.position = clip_position(in.position);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    return out;
}

// Alpha-mask glyphs. Swash coverage is linear; blending it in linear light
// thins light-on-dark text, so coverage is lifted the way the root glyph
// shader's gamma round trip lifts it.
@fragment
fn fs_glyph_mask(in: GlyphOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(t_source, s_source, in.tex_coords).r;
    return in.color * pow(coverage, 1.0 / 1.43);
}

// Colour glyphs (emoji): the raster is straight-alpha RGBA in sRGB pages.
@fragment
fn fs_glyph_color(in: GlyphOut) -> @location(0) vec4<f32> {
    let texel = textureSample(t_source, s_source, in.tex_coords);
    return vec4<f32>(texel.rgb * texel.a, texel.a) * in.color.a;
}
