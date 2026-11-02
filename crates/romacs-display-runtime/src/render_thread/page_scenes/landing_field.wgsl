// The Romacs mark's distance field, rendered once into a shader surface the
// lit mark (landing_mark.wgsl) samples as iChannel0.  Rust prepends the
// mark's outlines as SEGMENTS (object space: the mark's height spans y -1 to
// 1, y up; the R's segments first, R_SEGMENTS of them, then the crescent's)
// and FIELD_HALF, the half extent the texture covers.
//
// Each texel: r the signed distance to the nearest outline (negative inside,
// FIELD_RANGE maps to 0 and 1), gb the outward normal there, a 1 inside or
// nearest the crescent and 0 for the R.  The surface's colour encoding
// round-trips: what this writes is what a sampler reads back.

const FIELD_RANGE: f32 = 0.35;

fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
    let p = (fragCoord / u.iResolution.xy * 2.0 - 1.0) * FIELD_HALF;
    var best = 1e9;
    var nearest = p;
    var edge = vec2<f32>(1.0, 0.0);
    var crescent = false;
    var in_r = false;
    var in_crescent = false;
    for (var i = 0u; i < SEGMENT_COUNT; i++) {
        let s = SEGMENTS[i];
        let a = s.xy;
        let ab = s.zw - s.xy;
        let h = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-12), 0.0, 1.0);
        let q = a + ab * h;
        let d = dot(p - q, p - q);
        if d < best {
            best = d;
            nearest = q;
            edge = ab;
            crescent = i >= R_SEGMENTS;
        }
        // Even-odd: a ray to +x crosses this segment.
        if (s.y > p.y) != (s.w > p.y) && p.x < s.x + (p.y - s.y) / (s.w - s.y) * ab.x {
            if i < R_SEGMENTS { in_r = !in_r; } else { in_crescent = !in_crescent; }
        }
    }
    let inside = in_r || in_crescent;
    let distance = sqrt(best);
    var normal = p - nearest;
    if distance < 1e-6 { normal = vec2<f32>(edge.y, -edge.x); }
    normal = normalize(normal) * select(1.0, -1.0, inside);
    let signed = select(distance, -distance, inside);
    let material = select(select(0.0, 1.0, crescent), 1.0, in_crescent);
    return vec4<f32>(clamp(0.5 + signed / (2.0 * FIELD_RANGE), 0.0, 1.0), normal * 0.5 + 0.5,
                     select(material, 0.0, in_r));
}
