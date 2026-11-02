// The *Romacs* landing's mark as a lit object: the R and its crescent
// extruded into a slab with rounded edges, ray marched through the distance
// field in iChannel0 (landing_field.wgsl), lit in the Ember Lattice manner
// (a warm key light, a gold rim, a cool fill) and turning slowly.  On open it
// assembles: braille dots, on the lattice of the text cells the page is
// laid out in, rain into the mark's shape, swell into a solid, and the solid
// gains depth and turns.  A band of light crosses it once a cycle and the
// dots of that lattice, etched in its face, glint as the band passes.
//
// Ideas, not code: extruded and rounded distance fields after Inigo
// Quilez's distance-function notes (iquilezles.org/articles/distfunctions),
// the glyph rain after Rezmason's matrix (MIT), a glow after Paper Shaders'
// glow border (Apache-2.0).
//
// Uniforms (the scene node's, in order): u0 yaw and u1 pitch in degrees, u2
// the light cycle's phase in [0, 1), u3 how far the mark has assembled in
// [0, 1], u4 and u5 the braille dot pitch in logical pixels, u6 the margin
// around the mark in logical pixels.  Rust prepends FIELD_HALF and
// MARK_HALF.  The output is straight alpha.

const FIELD_RANGE: f32 = 0.35;
const DEPTH: f32 = 0.34;
const ROUND: f32 = 0.05;
const CAMERA: f32 = 7.0;
const STEPS: i32 = 48;

const RED: vec3<f32> = vec3<f32>(1.0, 0.113, 0.1);
const BLUE: vec3<f32> = vec3<f32>(0.028, 0.43, 1.0);
const GOLD: vec3<f32> = vec3<f32>(0.66, 0.254, 0.014);
const BRIGHT: vec3<f32> = vec3<f32>(0.905, 0.797, 0.546);
const KEY: vec3<f32> = vec3<f32>(1.0, 0.86, 0.62);
const FILL: vec3<f32> = vec3<f32>(0.25, 0.36, 0.62);
const EMBER: vec3<f32> = vec3<f32>(1.0, 0.16, 0.05);

struct Field {
    d: f32,
    n: vec2<f32>,
    m: f32,
}

fn field(p: vec2<f32>) -> Field {
    let uv = vec2<f32>(p.x / FIELD_HALF.x, -p.y / FIELD_HALF.y) * 0.5 + 0.5;
    let t = textureSampleLevel(iChannel0, iChannel0Sampler, uv, 0.0);
    let n = t.gb * 2.0 - 1.0;
    return Field((t.r - 0.5) * 2.0 * FIELD_RANGE, n / max(length(n), 1e-4), t.a);
}

// Object to view: yaw about y, then pitch about x.
fn turn(v: vec3<f32>, yaw: f32, pitch: f32) -> vec3<f32> {
    let a = vec3<f32>(cos(yaw) * v.x + sin(yaw) * v.z, v.y, -sin(yaw) * v.x + cos(yaw) * v.z);
    return vec3<f32>(a.x, cos(pitch) * a.y - sin(pitch) * a.z, sin(pitch) * a.y + cos(pitch) * a.z);
}

fn unturn(v: vec3<f32>, yaw: f32, pitch: f32) -> vec3<f32> {
    let a = vec3<f32>(v.x, cos(pitch) * v.y + sin(pitch) * v.z, -sin(pitch) * v.y + cos(pitch) * v.z);
    return vec3<f32>(cos(yaw) * a.x - sin(yaw) * a.z, a.y, sin(yaw) * a.x + cos(yaw) * a.z);
}

// The slab of half depth h whose edges are rounded by r: the field eroded by
// r, extruded, then grown by r.
fn body(p: vec3<f32>, h: f32, r: f32) -> f32 {
    let w = vec2<f32>(field(p.xy).d + r, abs(p.z) - h + r);
    return min(max(w.x, w.y), 0.0) + length(max(w, vec2<f32>(0.0))) - r;
}

fn hash(n: vec3<f32>) -> f32 {
    return fract(sin(dot(n, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

fn ease_out(x: f32) -> f32 {
    let y = 1.0 - clamp(x, 0.0, 1.0);
    return 1.0 - y * y * y;
}

// The band of light: its strength at object point p in this phase.
fn band(p: vec2<f32>, phase: f32) -> f32 {
    let along = dot(p, vec2<f32>(0.8, 0.6));
    let at = mix(-2.0, 2.0, phase / 0.42);
    return exp(-pow((along - at) / 0.14, 2.0)) * step(phase, 0.42);
}

// How much of a braille dot of RADIUS logical pixels at q's lattice point
// covers q, which is in lattice units of PITCH.
fn dot_cover(q: vec2<f32>, pitch: vec2<f32>, radius: f32, scale: f32) -> f32 {
    let off = (fract(q) - 0.5) * pitch;
    return clamp((radius - length(off)) * scale + 0.5, 0.0, 1.0);
}

struct Lit {
    colour: vec3<f32>,
    face: f32,
}

fn shade(p: vec3<f32>, h: f32, r: f32, eye: vec3<f32>, yaw: f32, pitch: f32, phase: f32,
         flash: f32, lattice: vec2<f32>, pitch_px: vec2<f32>, k: f32, scale: f32) -> Lit {
    let f = field(p.xy);
    let w = vec2<f32>(f.d + r, abs(p.z) - h + r);
    var g = vec2<f32>(0.0, 1.0);
    if w.x > 0.0 && w.y > 0.0 {
        g = normalize(w);
    } else if w.x > w.y {
        g = vec2<f32>(1.0, 0.0);
    }
    let side = select(-1.0, 1.0, p.z >= 0.0);
    let n = turn(normalize(vec3<f32>(f.n * g.x, side * g.y)), yaw, pitch);
    let albedo = mix(RED, BLUE, f.m) * 0.62;
    let key = normalize(vec3<f32>(-0.45, 0.6, 0.66));
    let rim = normalize(vec3<f32>(0.75, 0.35, -0.55));
    let fill = normalize(vec3<f32>(-0.6, -0.5, 0.45));
    let back = clamp((p.z + h) / max(2.0 * h, 1e-3), 0.0, 1.0);
    let ndv = max(dot(n, eye), 0.0);
    let spec = pow(max(dot(n, normalize(key + eye)), 0.0), 90.0);
    let fresnel = 0.04 + 0.96 * pow(1.0 - ndv, 5.0);
    // A studio around it: dark floor, a warm soft box above, a gold kicker
    // behind on the right.  Its reflection is the gloss.
    let mirrored = reflect(-eye, n);
    let studio = mix(vec3<f32>(0.012, 0.01, 0.009), KEY * 0.75, smoothstep(0.05, 0.85, mirrored.y))
        + GOLD * 1.6 * smoothstep(0.35, 0.95, dot(mirrored, rim));
    let face = g.y * step(0.0, p.z);
    let sheen = mix(0.8, 1.12, clamp(p.y * 0.5 + 0.5, 0.0, 1.0));
    var c = albedo * (0.06 + 0.7 * max(dot(n, key), 0.0)) * mix(0.25, 1.0, back) * mix(1.0, sheen, face)
        + albedo * FILL * max(dot(n, fill), 0.0) * 0.4
        + studio * (0.08 + fresnel)
        + KEY * spec * 1.1
        + GOLD * pow(1.0 - ndv, 4.0) * (0.2 + 1.6 * max(dot(n, rim), 0.0));
    let light = band(p.xy, phase);
    // The face keeps the lattice it assembled on: its dots are pressed in
    // and catch the band of light.
    let etched = dot_cover(lattice, pitch_px, 0.22 * min(pitch_px.x, pitch_px.y), scale) * face;
    c = c * (1.0 - 0.1 * etched) + BRIGHT * light * (0.12 + 1.4 * spec + 0.9 * fresnel + 0.6 * etched);
    c += BRIGHT * flash * (0.06 + 1.8 * fresnel);
    return Lit(c, face);
}

fn tone(c: vec3<f32>) -> vec3<f32> {
    return clamp(c * (2.51 * c + 0.03) / (c * (2.43 * c + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn over(top: vec4<f32>, under: vec4<f32>) -> vec4<f32> {
    return top + under * (1.0 - top.a);
}

fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
    let scale = max(u.iResolution.z, 1e-3);
    let size = u.iResolution.xy / scale;
    let q = vec2<f32>(fragCoord.x, u.iResolution.y - fragCoord.y) / scale;
    let k = (size.y - 2.0 * u_u6()) * 0.5;
    let s = vec2<f32>(q.x - size.x * 0.5, size.y * 0.5 - q.y) / k;
    let pixel = 1.0 / (k * scale);
    let pitch_px = max(vec2<f32>(u_u4(), u_u5()), vec2<f32>(1.0));
    let assembled = clamp(u_u3(), 0.0, 1.0);
    let phase = fract(u_u2());
    let shape = smoothstep(0.62, 1.0, assembled);
    let yaw = radians(u_u0()) * shape;
    let pitch = radians(u_u1()) * shape;
    let h = 0.5 * DEPTH * shape + 0.004;
    let r = min(ROUND, h);
    let flash = exp(-pow((assembled - 0.74) / 0.07, 2.0));

    let eye_ray = normalize(vec3<f32>(s, -CAMERA));
    let origin = unturn(vec3<f32>(0.0, 0.0, CAMERA), yaw, pitch);
    let ray = unturn(eye_ray, yaw, pitch);

    // The glow: the field where the ray crosses the face's plane.
    let front = origin + ray * ((h - origin.z) / ray.z);
    let near = field(front.xy);
    let reach = max(near.d, 0.0);
    let breath = 0.82 + 0.18 * cos(6.2831853 * phase * 2.0);
    let glow = (exp(-reach / 0.045) * 0.32 + exp(-reach / 0.11) * 0.14)
        * (1.0 - smoothstep(0.12, 0.27, reach)) * breath * smoothstep(0.45, 1.0, assembled);
    var out = vec4<f32>(tone(mix(EMBER, BLUE * 0.7, near.m)) * glow, glow);

    // Braille dots falling into the mark, column by column.
    let dots_on = 1.0 - smoothstep(0.80, 0.95, assembled);
    if dots_on > 0.0 {
        let col = floor(q.x / pitch_px.x);
        let landed = clamp((assembled - hash(vec3<f32>(col, 1.0, 7.0)) * 0.36) / 0.3, 0.0, 1.0);
        let fall = (1.0 - ease_out(landed)) * size.y * 0.8;
        // The mark's lattice waits as a ghost of the braille the page
        // showed, until its column lands.
        let still = (floor(q / pitch_px) + 0.5) * pitch_px;
        let ghost = field(vec2<f32>(still.x - size.x * 0.5, size.y * 0.5 - still.y) / k);
        if ghost.d < 0.0 && landed < 1.0 {
            let faint = dot_cover(q / pitch_px, pitch_px, 0.3 * min(pitch_px.x, pitch_px.y), scale)
                * 0.22 * (1.0 - landed) * dots_on;
            out = over(vec4<f32>(tone(mix(RED, BLUE, ghost.m)) * faint, faint), out);
        }
        let lattice = vec2<f32>(q.x, q.y + fall) / pitch_px;
        let home = (floor(lattice) + 0.5) * pitch_px;
        let at = field(vec2<f32>(home.x - size.x * 0.5, size.y * 0.5 - home.y) / k);
        if at.d < 0.0 {
            let swell = smoothstep(0.55, 0.85, assembled);
            let radius = mix(0.3, 0.82, swell) * min(pitch_px.x, pitch_px.y);
            let flicker = select(1.0, step(0.3, hash(vec3<f32>(floor(lattice), floor(assembled * 40.0)))), landed < 1.0);
            let cover = dot_cover(lattice, pitch_px, radius, scale) * flicker * dots_on;
            let hot = 1.0 - landed;
            let colour = tone(mix(RED, BLUE, at.m) * (0.9 + 1.4 * hot) + GOLD * hot * 1.5);
            out = over(vec4<f32>(colour * cover, cover), out);
        }
    }

    // The solid: march the slab within its bounds.
    let solid = smoothstep(0.58, 0.86, assembled);
    if solid > 0.0 {
        let bound = vec3<f32>(MARK_HALF.x + 0.06, MARK_HALF.y + 0.06, h + 0.01);
        let safe = select(ray, vec3<f32>(1e-6), abs(ray) < vec3<f32>(1e-6));
        let t_a = (-bound - origin) / safe;
        let t_b = (bound - origin) / safe;
        let t_near = max(max(min(t_a.x, t_b.x), min(t_a.y, t_b.y)), min(t_a.z, t_b.z));
        let t_far = min(min(max(t_a.x, t_b.x), max(t_a.y, t_b.y)), max(t_a.z, t_b.z));
        // Across the slab the ray moves this far in its plane; the field is
        // a distance, so a ray that starts farther from the mark misses it.
        let across = 2.0 * h * length(ray.xy) / max(abs(ray.z), 1e-3);
        if t_far > max(t_near, 0.0) && near.d < across + 2.0 * pixel {
            var t = max(t_near, 0.0);
            var closest = 1e9;
            var closest_t = t;
            var hit = false;
            for (var i = 0; i < STEPS; i++) {
                let d = body(origin + ray * t, h, r);
                if d < closest {
                    closest = d;
                    closest_t = t;
                }
                if d < 0.25 * pixel {
                    hit = true;
                    break;
                }
                t += max(d * 0.9, 0.3 * pixel);
                if t > t_far { break; }
            }
            let cover = select(clamp(1.0 - closest / (1.5 * pixel), 0.0, 1.0), 1.0, hit) * solid;
            if cover > 0.0 {
                let p = origin + ray * select(closest_t, t, hit);
                let screen = vec2<f32>(p.x * k + size.x * 0.5, size.y * 0.5 - p.y * k);
                let lit = shade(p, h, r, -eye_ray, yaw, pitch, phase, flash, screen / pitch_px,
                                pitch_px, k, scale);
                out = over(vec4<f32>(tone(lit.colour) * cover, cover), out);
            }
        }
    }
    return vec4<f32>(out.rgb / max(out.a, 1e-4), out.a);
}
