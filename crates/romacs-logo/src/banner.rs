//! Banners: a block of cells over a list's first lines that the render
//! thread moves by itself (lisp/ui/romacs-banner.el), in a pattern:
//!
//!   wave     a braille sine travelling across
//!   rain     braille drops falling, each column its own speed
//!   sparkle  cells lighting and fading at random
//!   sweep    a band of block shades moving along
//!
//! A frame is a pure function of its pattern, its size, its epoch (the
//! seed of what is random in it) and the seconds of motion.  Each lit
//! cell names one of the caller's colour stops by index, never a colour
//! of its own: shades come from how many dots or how dense a block a
//! cell has, so every theme's faces give the banner its colours.
use std::f64::consts::TAU;

/// One cell: its character and, unless it is empty, the index of the
/// colour stop it is drawn in.
pub type Cell = (char, Option<usize>);

/// The most colour stops a banner takes.
pub const MAX_STOPS: usize = 4;

/// How a banner moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Pattern {
    Wave,
    Rain,
    Sparkle,
    Sweep,
}

impl Pattern {
    pub const ALL: [Self; 4] = [Self::Wave, Self::Rain, Self::Sparkle, Self::Sweep];

    /// The pattern named NAME, as Lisp names it.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Wave => "wave",
            Self::Rain => "rain",
            Self::Sparkle => "sparkle",
            Self::Sweep => "sweep",
        }
    }
}

/// Whether a banner of COLS by ROWS cells in STOPS colours can be drawn:
/// 1..=400 columns, 1..=200 rows, 1..=4 stops.
pub fn valid(cols: usize, rows: usize, stops: usize) -> bool {
    (1..=400).contains(&cols) && (1..=200).contains(&rows) && (1..=MAX_STOPS).contains(&stops)
}

/// Dots of a braille cell (2 wide, 4 high), drawille's layout: bit of
/// the dot at column X and row Y.
const DOTS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
const BLANK: char = '\u{2800}';

fn braille(bits: u8) -> char {
    char::from_u32(0x2800 + u32::from(bits)).unwrap_or(BLANK)
}

/// A well-mixed 64 bits of the words given (splitmix64 over each).
fn hash(words: &[u64]) -> u64 {
    words.iter().fold(0x9e37_79b9_7f4a_7c15, |h, &w| {
        let mut z = (h ^ w).wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    })
}

/// HASH as a fraction in [0, 1).
fn unit(hash: u64) -> f64 {
    (hash >> 11) as f64 / (1u64 << 53) as f64
}

/// COLS by ROWS cells of PATTERN in STOPS colours, seeded by EPOCH,
/// after MOVED seconds; none when `valid` refuses the size.
pub fn frame(pattern: Pattern, cols: usize, rows: usize, stops: usize, epoch: u64, moved: f64) -> Vec<Cell> {
    if !valid(cols, rows, stops) {
        return vec![];
    }
    let moved = if moved.is_finite() { moved.max(0.0) } else { 0.0 };
    match pattern {
        Pattern::Wave => wave(cols, rows, stops, moved),
        Pattern::Rain => rain(cols, rows, stops, epoch, moved),
        Pattern::Sparkle => sparkle(cols, rows, stops, epoch, moved),
        Pattern::Sweep => sweep(cols, rows, stops, moved),
    }
}

/// Braille cells from a predicate on dots (X across, Y down), each lit
/// cell in the stop STOP gives its column and row.
fn dotted(cols: usize, rows: usize, lit: impl Fn(usize, usize) -> bool, stop: impl Fn(usize, usize) -> usize) -> Vec<Cell> {
    let mut cells = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        for col in 0..cols {
            let mut bits = 0u8;
            for (x, column) in DOTS.iter().enumerate() {
                for (y, bit) in column.iter().enumerate() {
                    if lit(col * 2 + x, row * 4 + y) {
                        bits |= bit;
                    }
                }
            }
            cells.push(if bits == 0 { (BLANK, None) } else { (braille(bits), Some(stop(col, row))) });
        }
    }
    cells
}

/// Dots of the wave's length, and waves a second.
const WAVE: f64 = 16.0;
const WAVE_SPEED: f64 = 0.5;
/// Seconds a band of colour takes to cross its own width.
const BAND_DRIFT: f64 = 0.15;

fn wave(cols: usize, rows: usize, stops: usize, moved: f64) -> Vec<Cell> {
    let height = (rows * 4) as f64 - 1.0;
    let (centre, amplitude) = (height / 2.0, (height / 2.0 * 0.9).max(1.0).min(height / 2.0));
    let y_at = |x: usize| (centre + amplitude * (TAU * (x as f64 / WAVE - WAVE_SPEED * moved)).sin()).round() as usize;
    dotted(cols, rows, |x, y| y_at(x) == y, |col, _| band(col, cols, stops, moved))
}

/// The stop of column COL of COLS: STOPS bands across, drifting along.
fn band(col: usize, cols: usize, stops: usize, moved: f64) -> usize {
    let at = col as f64 / cols as f64 * stops as f64 + moved * BAND_DRIFT * stops as f64;
    (at.floor() as usize) % stops
}

/// Dots a second the slowest and the fastest drops fall.
const RAIN_SPEED: (f64, f64) = (6.0, 18.0);
const DROP: usize = 3;

fn rain(cols: usize, rows: usize, stops: usize, epoch: u64, moved: f64) -> Vec<Cell> {
    let height = rows * 4;
    // Each dot column's drop: its speed, where it starts, and how long
    // it waits above before it falls again.
    let drops: Vec<(f64, f64, f64)> = (0..cols * 2).map(|x| {
        let h = |salt: u64| unit(hash(&[epoch, x as u64, salt]));
        let period = (height + DROP) as f64 + h(2) * height as f64 * 1.5;
        (RAIN_SPEED.0 + h(0) * (RAIN_SPEED.1 - RAIN_SPEED.0), h(1) * period, period)
    }).collect();
    let head = |x: usize| {
        let (speed, start, period) = drops[x];
        ((start + speed * moved) % period).floor() as usize
    };
    dotted(cols, rows, |x, y| { let h = head(x); y <= h && h < y + DROP }, |col, _| {
        (hash(&[epoch, col as u64, 7]) % stops as u64) as usize
    })
}

/// Changes a second, the share of cells lit, and the fading glyphs a
/// lit cell goes through, brightest first.
const SPARKLE_RATE: f64 = 6.0;
const SPARKLE_SHARE: f64 = 0.12;
const SPARKLE_FADE: [char; 4] = ['⣿', '⢾', '⠶', '⠂'];

fn sparkle(cols: usize, rows: usize, stops: usize, epoch: u64, moved: f64) -> Vec<Cell> {
    let life = SPARKLE_FADE.len() as f64;
    (0..cols * rows).map(|cell| {
        let cell = cell as u64;
        let phase = moved * SPARKLE_RATE + unit(hash(&[epoch, cell, 1])) * life;
        let cycle = (phase / life).floor() as u64;
        let roll = hash(&[epoch, cell, cycle, 2]);
        if unit(roll) >= SPARKLE_SHARE {
            return (BLANK, None);
        }
        let age = ((phase % life).floor() as usize).min(SPARKLE_FADE.len() - 1);
        (SPARKLE_FADE[age], Some(((roll >> 32) % stops as u64) as usize))
    }).collect()
}

/// Cells a second the band moves, and its shades, faintest first.
const SWEEP_SPEED: f64 = 14.0;
const SHADES: [char; 4] = ['░', '▒', '▓', '█'];

fn sweep(cols: usize, rows: usize, stops: usize, moved: f64) -> Vec<Cell> {
    let half = (cols as f64 / 8.0).max(3.0);
    let lap = cols as f64 + 2.0 * half;
    // A quarter of the way in at rest, so the still frame shows the band.
    let centre = (moved * SWEEP_SPEED + half + cols as f64 / 4.0) % lap - half;
    (0..rows).flat_map(|_| 0..cols).map(|col| {
        // 0 at the band's edges and beyond, 1 at its centre.
        let light = (1.0 - (col as f64 + 0.5 - centre).abs() / half).max(0.0);
        if light == 0.0 {
            return (' ', None);
        }
        let shade = ((light * SHADES.len() as f64).floor() as usize).min(SHADES.len() - 1);
        let stop = ((light * stops as f64).floor() as usize).min(stops - 1);
        (SHADES[shade], Some(stop))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: [(usize, usize); 4] = [(1, 1), (7, 1), (40, 1), (23, 3)];

    #[test]
    fn every_pattern_fills_its_block() {
        for pattern in Pattern::ALL {
            for (cols, rows) in SIZES {
                for stops in 1..=MAX_STOPS {
                    for moved in [0.0, 0.37, 5.0, 1e6] {
                        assert_eq!(frame(pattern, cols, rows, stops, 3, moved).len(), cols * rows, "{pattern:?} {cols}x{rows}");
                    }
                }
            }
        }
    }

    #[test]
    fn colours_come_only_from_the_stops() {
        for pattern in Pattern::ALL {
            for stops in 1..=MAX_STOPS {
                for moved in [0.0, 0.2, 1.3, 7.7] {
                    for (ch, stop) in frame(pattern, 40, 2, stops, 9, moved) {
                        assert!(stop.is_none_or(|s| s < stops), "{pattern:?} stop {stop:?} of {stops}");
                        assert_eq!(stop.is_none(), ch == BLANK || ch == ' ', "{pattern:?}: an empty cell has no colour");
                    }
                }
            }
        }
    }

    #[test]
    fn a_frame_is_the_same_for_the_same_epoch_and_time() {
        for pattern in Pattern::ALL {
            assert_eq!(frame(pattern, 30, 2, 3, 5, 1.25), frame(pattern, 30, 2, 3, 5, 1.25));
        }
    }

    #[test]
    fn every_pattern_moves() {
        for pattern in Pattern::ALL {
            let frames: Vec<_> = [0.0, 0.1, 0.2, 0.3].iter().map(|&t| frame(pattern, 40, 1, 2, 1, t)).collect();
            assert!(frames.windows(2).all(|w| w[0] != w[1]), "{pattern:?} stands still");
        }
    }

    #[test]
    fn the_epoch_seeds_what_is_random() {
        for pattern in [Pattern::Rain, Pattern::Sparkle] {
            assert_ne!(frame(pattern, 60, 2, 2, 1, 0.5), frame(pattern, 60, 2, 2, 2, 0.5), "{pattern:?}");
        }
    }

    #[test]
    fn sparkle_lights_some_cells_not_all() {
        let lit = frame(Pattern::Sparkle, 100, 4, 2, 4, 2.0).iter().filter(|c| c.1.is_some()).count();
        assert!((10..120).contains(&lit), "{lit} of 400 lit");
    }

    #[test]
    fn sweep_is_brightest_at_its_band() {
        let cells = frame(Pattern::Sweep, 80, 1, 3, 0, 2.0);
        let brightest = cells.iter().filter(|c| c.0 == '█').count();
        assert!(brightest > 0 && brightest < 10, "{brightest}");
        assert!(cells.iter().filter(|c| c.0 == '█').all(|c| c.1 == Some(2)), "the band's heart in the last stop");
    }

    #[test]
    fn refused_sizes_draw_nothing() {
        for (cols, rows, stops) in [(0, 1, 2), (1, 0, 2), (401, 1, 2), (1, 201, 2), (10, 1, 0), (10, 1, 5)] {
            assert!(frame(Pattern::Wave, cols, rows, stops, 0, 0.0).is_empty());
        }
    }

    #[test]
    fn patterns_round_trip_their_names() {
        for pattern in Pattern::ALL {
            assert_eq!(Pattern::named(pattern.name()), Some(pattern));
        }
        assert_eq!(Pattern::named("thread"), None, "the thread is romacs-working's, not a banner's");
    }
}
