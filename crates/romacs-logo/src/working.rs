//! The working thread: one row of braille cells, a thread whose motion
//! says how a piece of work goes, drawn frame after frame by the render
//! thread while the evaluator may be busy (lisp/ui/romacs-working.el).
//!
//! Idle, the thread lies slack and still; running, a wave travels along
//! it, and a bright bead rides it, a step for each piece of progress the
//! work reports (its beats); waiting on its user, it is
//! taut and pulses; failed, it is cut.  A frame is a pure function of the
//! work's state, its beats, the thread's colour and the seconds of motion.
use crate::Cell;

/// How the work goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    Idle,
    Running,
    Waiting,
    Failed,
}

/// Dots of a braille cell (2 wide, 4 high), drawille's layout: bit of
/// the dot at column X and row Y.
const DOTS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
/// Dots of the wave's length, and waves a second.
const WAVE: f64 = 12.0;
const SPEED: f64 = 0.45;
/// Cells the bead steps for each beat.
const STEP: usize = 1;

/// COLS cells of the thread for WORK in COLOUR (RGB), BEATS of progress
/// reported, after MOVED seconds; its bead, the wave's own dots where it
/// is, in BEAD (the text's colour, the most contrast the theme has).
pub fn thread(cols: usize, work: Work, beats: u64, colour: u32, bead_colour: u32, moved: f64) -> Vec<Cell> {
    let tau = std::f64::consts::TAU;
    let (amplitude, centre) = match work {
        Work::Running => (1.4, 1.5),
        Work::Idle | Work::Failed => (0.0, 2.0),
        Work::Waiting => (0.0, 1.0),
    };
    let bead = (beats as usize).wrapping_mul(STEP) % cols.max(1);
    let pulse = 0.55 + 0.45 * (0.5 + 0.5 * (tau * moved).sin());
    (0..cols).map(|cell| {
        // Failed: cut in its middle, when it has room for a cut (as
        // romacs-working-still cuts its still frame).
        let gap = (cols / 6).max(1);
        if work == Work::Failed && cols >= 3 && (cols / 2 - gap..cols / 2).contains(&cell) {
            return (' ', None);
        }
        let mut bits = 0u8;
        for (x, column) in DOTS.iter().enumerate() {
            let dot = (cell * 2 + x) as f64;
            let y = centre + amplitude * (tau * (dot / WAVE - SPEED * moved)).sin();
            bits |= column[(y.round() as usize).min(3)];
            // Taut: a solid bar three dots thick, as its still frame.
            if work == Work::Waiting { bits |= column[0] | column[1] | column[2]; }
        }
        if work == Work::Running && cell == bead {
            return (char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' '), Some(bead_colour));
        }
        let shade = match work {
            Work::Running => 1.0,
            Work::Waiting => pulse,
            Work::Idle => 0.6,
            Work::Failed => 1.0,
        };
        (char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' '), Some(scale(colour, shade)))
    }).collect()
}

/// COLOUR's channels times SHADE, toward white past 1.
fn scale(colour: u32, shade: f64) -> u32 {
    let channel = |shift: u32| {
        let c = f64::from((colour >> shift) & 0xff);
        let v = if shade <= 1.0 { c * shade } else { c + (255.0 - c) * (shade - 1.0) };
        (v.round().clamp(0.0, 255.0) as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_moves_and_idle_does_not() {
        assert_ne!(thread(12, Work::Running, 0, 0xd48a20, 0xffffff, 0.0), thread(12, Work::Running, 0, 0xd48a20, 0xffffff, 0.3));
        assert_eq!(thread(12, Work::Idle, 0, 0xd48a20, 0xffffff, 0.0), thread(12, Work::Idle, 0, 0xd48a20, 0xffffff, 5.0));
    }

    #[test]
    fn waiting_pulses_in_place() {
        let (a, b) = (thread(8, Work::Waiting, 0, 0xd48a20, 0xffffff, 0.0), thread(8, Work::Waiting, 0, 0xd48a20, 0xffffff, 0.25));
        assert!(a.iter().all(|c| c.0 == '⠿'), "taut and solid, as its still frame");
        assert_eq!(a.iter().map(|c| c.0).collect::<Vec<_>>(), b.iter().map(|c| c.0).collect::<Vec<_>>());
        assert_ne!(a[0].1, b[0].1);
    }

    #[test]
    fn progress_steps_the_bead_and_time_alone_does_not() {
        let bright = |cells: Vec<Cell>| cells.iter().position(|c| c.1 == Some(0xffffff));
        let at = bright(thread(12, Work::Running, 3, 0x808080, 0xffffff, 0.0));
        assert_eq!(at, Some(3));
        // The bead is the wave's own dots, so the wave reads through it,
        // and a one-cell thread still moves.
        assert_ne!(thread(12, Work::Running, 3, 0x808080, 0xffffff, 0.0)[3].0, '⣿');
        assert_ne!(thread(1, Work::Running, 0, 0x808080, 0xffffff, 0.0), thread(1, Work::Running, 0, 0x808080, 0xffffff, 0.6));
        assert_eq!(bright(thread(12, Work::Running, 3, 0x808080, 0xffffff, 2.5)), at, "the bead waits for progress");
        assert_eq!(bright(thread(12, Work::Running, 4, 0x808080, 0xffffff, 0.0)), Some(4));
        assert_eq!(bright(thread(2, Work::Running, 1, 0x808080, 0xffffff, 0.0)), Some(1), "a narrow thread's bead moves too");
    }

    #[test]
    fn narrow_threads_are_drawn() {
        for cols in 1..4 {
            assert!(thread(cols, Work::Failed, 0, 0x808080, 0xffffff, 0.0).iter().any(|c| c.0 != ' '), "{cols} cells");
        }
    }

    #[test]
    fn failed_is_cut() {
        let cells = thread(12, Work::Failed, 0, 0xff0000, 0xffffff, 0.0);
        assert!(cells.iter().any(|c| c.0 == ' '));
        assert!(cells.iter().any(|c| c.0 != ' '));
    }

    #[test]
    fn every_cell_is_a_braille_dot_pattern() {
        for work in [Work::Idle, Work::Running, Work::Waiting, Work::Failed] {
            for cell in thread(10, work, 0, 0x808080, 0xffffff, 1.7) {
                assert!(cell.0 == ' ' || ('\u{2800}'..='\u{28ff}').contains(&cell.0));
            }
        }
    }
}
