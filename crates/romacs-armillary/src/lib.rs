//! The ASCII armillary sphere: three tori around a small sphere, drawn as
//! text.  Ported from thanosapollo.org ascii-home.js (used with the author's permission): its
//! mesh `V()`, star field and renderer `B()`, so a frame here is the
//! site's frame for the same view, byte for byte.  The arithmetic keeps the
//! script's order of operations and its `Float32Array`s (the mesh and the
//! depth buffer), which is what makes the frames identical.
//!
//! `romacs--armillary-frame` (romacs-vm's prims/armillary.rs) draws it for
//! Lisp (rlisp/romacs-armillary.el); the render thread turns it in the
//! *Romacs* landing, and the GUI host while a first start prepares its
//! world.
use std::f64::consts::PI;
use std::sync::OnceLock;

/// Brightness, dark to light.  Index 0 is empty space.
pub const RAMP: &[u8] = b" .,:;irsXA253hMHGS#9B&@";

const TAU: f64 = PI * 2.0;
/// The view's eye distance: a point at depth z is scaled by 1/(EYE - z).
const EYE: f64 = 5.5;
const LIGHT: [f64; 3] = [-0.45, 0.65, 0.61];

/// What a frame shows: COLS by ROWS cells, the sphere turned by PITCH,
/// YAW and ROLL (radians), ZOOM (clamped to 0.75..1.3), CELL_ASPECT a
/// cell's width over its height, and STARS whether the field behind it
/// shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub cols: usize,
    pub rows: usize,
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
    pub zoom: f64,
    pub cell_aspect: f64,
    pub stars: bool,
}

impl View {
    /// The script's bounds: 1..=200 columns, 1..=100 rows, finite angles
    /// and zoom, and a cell aspect in (0, 2].
    pub fn valid(&self) -> bool {
        (1..=200).contains(&self.cols) && (1..=100).contains(&self.rows)
            && [self.pitch, self.yaw, self.roll, self.zoom, self.cell_aspect].iter().all(|v| v.is_finite())
            && self.cell_aspect > 0.0 && self.cell_aspect <= 2.0
    }
}

/// Each ring: radius, tube radius, and the angles A, B and C `orient`
/// turns it by.
const RINGS: [[f64; 5]; 3] = [[1.6, 0.065, 0.85, 0.2, 0.0], [1.34, 0.075, -0.65, 1.05, 0.35], [1.08, 0.065, 1.25, -0.5, -0.3]];

/// The script's `W`: (X, Y, Z) turned by B about the y axis, then by A
/// about the x axis, then by C about the z axis.
fn orient(x: f64, y: f64, z: f64, [a, b, c]: [f64; 3]) -> [f64; 3] {
    let p = b.cos() * x + b.sin() * z;
    let m = -b.sin() * x + b.cos() * z;
    let d = a.cos() * y - a.sin() * m;
    let w = a.sin() * y + a.cos() * m;
    [c.cos() * p - c.sin() * d, c.sin() * p + c.cos() * d, w]
}

/// The mesh, built once: per sample a point then its unit normal, six
/// `f32`s.  The rings' 400 by 20 samples come first, then the core sphere
/// (radius 0.43) at 65 latitudes by 128 longitudes.
pub fn mesh() -> &'static [f32] {
    static MESH: OnceLock<Vec<f32>> = OnceLock::new();
    MESH.get_or_init(|| {
        let mut out = Vec::with_capacity((3 * 400 * 20 + 65 * 128) * 6);
        for [radius, tube, a, b, c] in RINGS {
            for i in 0..400 {
                let u = f64::from(i) / 400.0 * TAU;
                let (cu, su) = (u.cos(), u.sin());
                for j in 0..20 {
                    let v = f64::from(j) / 20.0 * TAU;
                    let (cv, sv) = (v.cos(), v.sin());
                    let r = radius + tube * cv;
                    out.extend(orient(r * cu, r * su, tube * sv, [a, b, c]).map(|x| x as f32));
                    out.extend(orient(cv * cu, cv * su, sv, [a, b, c]).map(|x| x as f32));
                }
            }
        }
        for i in 0..=64 {
            let theta = f64::from(i) / 64.0 * PI;
            for j in 0..128 {
                let phi = f64::from(j) / 128.0 * TAU;
                let (x, y, z) = (theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
                out.extend([x * 0.43, y * 0.43, z * 0.43, x, y, z].map(|x| x as f32));
            }
        }
        out
    })
}

/// The 96 stars on a golden-angle spiral behind the sphere.
fn stars() -> &'static [[f64; 3]] {
    static STARS: OnceLock<Vec<[f64; 3]>> = OnceLock::new();
    STARS.get_or_init(|| (0..96u32).map(|s| {
        let angle = f64::from(s) * 2.399963229728653;
        let radius = 1.8 + f64::from(s % 23) * 0.17;
        [angle.cos() * radius, angle.sin() * radius, -3.0 - f64::from(s % 7) * 0.1]
    }).collect())
}

/// VIEW's cells, row by row, each a byte of [`RAMP`] or a star's `+`.
/// VIEW must be [`View::valid`].
pub fn cells(view: &View) -> Vec<u8> {
    let (t, s) = (view.cols, view.rows);
    let (tf, sf, r) = (t as f64, s as f64, view.cell_aspect);
    let scale = (tf * r).min(sf) * 1.35 * 0.75f64.max(view.zoom.min(1.3));
    let cell = |x: f64, y: f64| -> Option<usize> {
        let (col, row) = (floor(tf / 2.0 + x / r), floor(sf / 2.0 - y));
        (col >= 0.0 && col < tf && row >= 0.0 && row < sf).then(|| row as usize * t + col as usize)
    };
    let mut depth = vec![f32::NEG_INFINITY; t * s];
    let mut out = vec![b' '; t * s];
    if view.stars {
        let turn = view.yaw * 0.13;
        let (k, f) = (turn.cos(), turn.sin());
        for (i, &[x, y, z]) in stars().iter().enumerate() {
            let p = 1.0 / (EYE - z);
            if let Some(at) = cell((x * k - y * f) * p * scale, (x * f + y * k) * p * scale) {
                out[at] = if i % 19 == 0 { b'+' } else { b'.' };
            }
        }
    }
    let (cy, sy, cp, sp, cr, sr) = (view.yaw.cos(), view.yaw.sin(), view.pitch.cos(), view.pitch.sin(), view.roll.cos(), view.roll.sin());
    let rot = [
        [cr * cy - sr * sp * sy, -sr * cp, cr * sy + sr * sp * cy],
        [sr * cy + cr * sp * sy, cr * cp, sr * sy - cr * sp * cy],
        [-cp * sy, sp, cp * cy],
    ];
    for sample in mesh().chunks_exact(6) {
        let [x, y, z] = turn(&rot, &sample[..3]);
        let p = 1.0 / (EYE - z);
        let Some(at) = cell(x * p * scale, y * p * scale) else { continue };
        if z <= f64::from(depth[at]) { continue; }
        depth[at] = z as f32;
        let [nx, ny, nz] = turn(&rot, &sample[3..]);
        let diffuse = 0f64.max(LIGHT[0] * nx + LIGHT[1] * ny + LIGHT[2] * nz);
        let light = 1f64.min(0.18 + diffuse * 0.7 + 0f64.max(nz).powf(12.0) * 0.2);
        out[at] = RAMP[1.max((light * (RAMP.len() - 1) as f64).floor() as usize)];
    }
    out
}

/// V turned by ROT, each coordinate summed in the script's order.  Plain
/// arithmetic, not `array::map`: this is the inner loop.
#[inline]
fn turn(rot: &[[f64; 3]; 3], v: &[f32]) -> [f64; 3] {
    let (a, b, c) = (f64::from(v[0]), f64::from(v[1]), f64::from(v[2]));
    [rot[0][0] * a + rot[0][1] * b + rot[0][2] * c,
     rot[1][0] * a + rot[1][1] * b + rot[1][2] * c,
     rot[2][0] * a + rot[2][1] * b + rot[2][2] * c]
}

/// `Math.floor` of X, finite and well inside i64, without a libm call.
#[inline]
fn floor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// VIEW's frame: its rows joined by newlines, as the script's `B()`.
pub fn frame(view: &View) -> String {
    text(&cells(view), view.cols)
}

/// CELLS as rows of COLS joined by newlines.
pub fn text(cells: &[u8], cols: usize) -> String {
    let rows: Vec<&[u8]> = cells.chunks_exact(cols).collect();
    String::from_utf8(rows.join(&b'\n')).expect("ASCII")
}

/// Which of SHADES (at least one) shades CELL shows: none for empty
/// space, the last for a star's `+', else its brightness's share of them.
pub fn shade_of(cell: u8, faces: usize) -> Option<usize> {
    let lit = RAMP.len() - 1;
    match cell {
        b' ' => None,
        b'+' => Some(faces - 1),
        _ => RAMP.iter().position(|&c| c == cell).map(|i| ((i - 1) * faces / lit).min(faces - 1)),
    }
}

/// The byte ranges of the frame text (rows of COLS CELLS joined by
/// newlines) that show one of SHADES shades, dark to light, each with that
/// shade's index.
pub fn shade_runs(cells: &[u8], cols: usize, faces: usize) -> Vec<(std::ops::Range<usize>, usize)> {
    let mut runs: Vec<(std::ops::Range<usize>, usize)> = vec![];
    if faces == 0 { return runs; }
    for (i, &cell) in cells.iter().enumerate() {
        let Some(face) = shade_of(cell, faces) else { continue };
        let at = i + i / cols;
        match runs.last_mut() {
            Some((range, last)) if *last == face && range.end == at => range.end = at + 1,
            _ => runs.push((at..at + 1, face)),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_has_every_sample() {
        assert_eq!(mesh().len(), (3 * 400 * 20 + 65 * 128) * 6);
    }

    #[test]
    fn frame_is_rows_of_columns() {
        let view = View { cols: 7, rows: 3, pitch: -0.25, yaw: 0.4, roll: -0.15, zoom: 1.0, cell_aspect: 0.5, stars: true };
        let frame = frame(&view);
        assert_eq!(frame.split('\n').map(str::len).collect::<Vec<_>>(), [7, 7, 7]);
    }

    #[test]
    fn runs_join_neighbours_of_one_shade_within_a_row() {
        // ".," are the two dimmest, "@" the brightest; rows of 3 cells.
        let runs = shade_runs(b".,@ +.", 3, 2);
        assert_eq!(runs, [(0..2, 0), (2..3, 1), (5..6, 1), (6..7, 0)]);
    }
}
