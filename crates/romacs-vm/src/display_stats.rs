//! What redisplay did, counted (`romacs--display-stats`): the cycles the
//! evaluator ran, the instructions and conses spent inside them, the
//! windows a snapshot reused or rebuilt, the bands it expanded, what this
//! evaluator asked of the layout host besides a paint's own layout, and
//! the frames it submitted. Plain counters, always on: a cycle pays two
//! reads of the thread's instruction counter for them.
//!
//! A cycle is what the host brackets with `Vm::redisplay_begin` and
//! `Vm::redisplay_done`: the frame's freeze, its comparison with the frame
//! on screen, and the layout, submission and layout application of a frame
//! that changed. Only the cycles, their instructions and their conses are
//! the bracket's; every other count is taken where the work is done, in a
//! cycle or outside one (a snapshot a test takes, a layout
//! `window-text-pixel-size` asks). A cycle that fails keeps its bracket
//! open, and the next begin starts over. A cycle begun inside another
//! (Lisp a redisplay runs reading input) takes the bracket: it is counted,
//! and the outer one is not.
use crate::{Result, Value, Vm};

/// The counts `romacs--display-stats` reports, in order.
const NAMES: [&str; COUNTS] = ["cycles", "instructions", "conses", "windows-visited", "windows-reused", "windows-rebuilt",
    "band-expansions", "band-evals", "layouts", "measures", "frames-submitted", "check-mismatches"];
const COUNTS: usize = 12;
const INSTRUCTIONS: usize = 1;

/// Redisplay's own counts since the VM started, the cycle under way, and
/// every count as the last reset read it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DisplayStats {
    cycles: u64,
    instructions: u64,
    conses: u64,
    /// Windows a snapshot answered from the last one (`Vm::reuse_shown`).
    pub(crate) reused: u64,
    /// Windows a snapshot set out to build in full: one whose build fails
    /// counts.
    pub(crate) rebuilt: u64,
    /// Mode lines, header lines and status bars expanded
    /// (`Vm::mode_line_band`): twice for a window whose reuse the bands'
    /// own Lisp undid, since its full build expands them again.
    pub(crate) bands: u64,
    /// Reused windows the reuse check found different (`Vm::check_reuse`).
    pub(crate) mismatches: u64,
    /// The thread's instruction counter and the conses made, at the begin
    /// of the cycle under way.
    open: Option<(Option<u64>, u64)>,
    base: [u64; COUNTS],
}
romacs_snapshot::snapshot_struct!(DisplayStats { cycles, instructions, conses, reused, rebuilt, bands, mismatches, open, base });

impl DisplayStats {
    fn begin(&mut self, conses: u64) {
        self.open = Some((crate::host::thread_instructions(), conses));
    }
    /// The cycle under way is over (`Vm::redisplay_done`); nothing without one.
    pub(crate) fn end(&mut self, conses: u64) {
        let Some((counter, made)) = self.open.take() else { return };
        self.cycles += 1;
        self.conses += conses.saturating_sub(made);
        if let (Some(from), Some(to)) = (counter, crate::host::thread_instructions()) {
            self.instructions += to.saturating_sub(from);
        }
    }
}

impl Vm {
    /// A redisplay cycle begins on the evaluator; `redisplay_done` ends it.
    pub fn redisplay_begin(&mut self) {
        let conses = self.allocation.consed.conses;
        self.display_state.stats.begin(conses);
    }
    /// Every count since the VM started, in NAMES' order. The layout host's
    /// answers and the frames submitted are this thread's host calls
    /// (`crate::host_calls`), whoever made them: a paint lays its frame out
    /// with the engine itself, so `layout.lay_out` counts what redisplay's
    /// questions and Lisp's (`window-text-pixel-size`) asked besides.
    fn display_counts(&self) -> [u64; COUNTS] {
        let stats = &self.display_state.stats;
        let calls = romacs_display_protocol::call_count::of;
        [stats.cycles, stats.instructions, stats.conses, stats.reused + stats.rebuilt, stats.reused, stats.rebuilt,
         stats.bands, self.display_state.mode_line_evals,
         calls("layout.lay_out"), calls("layout.measure"), calls("sender.submit_scene"), stats.mismatches]
    }
    /// `romacs--display-stats`: ((NAME . COUNT)...) since the last reset,
    /// and with RESET a reset. `instructions` is nil where the thread has
    /// no instruction counter.
    pub(crate) fn display_stats(&mut self, reset: bool) -> Result<Value> {
        let now = self.display_counts();
        let base = self.display_state.stats.base;
        if reset { self.display_state.stats.base = now; }
        let counted = crate::host::thread_instructions().is_some();
        let mut list = Value::NIL;
        for i in (0..COUNTS).rev() {
            // A restored image's thread starts its host calls over: those
            // three read 0 until they pass the image's base, or a reset.
            let count = if i == INSTRUCTIONS && !counted { Value::NIL }
                else { Value::integer(i64::try_from(now[i].saturating_sub(base[i])).map_err(|e| e.to_string())?)? };
            let name = self.intern(NAMES[i]);
            let row = self.alloc_cons(name, count);
            list = self.alloc_cons(row, list);
        }
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(vm: &mut Vm, name: &str) -> String {
        vm.eval_source(&format!("(cdr (assq '{name} (romacs--display-stats)))")).unwrap().display().unwrap()
    }

    #[test]
    fn a_bracketed_cycle_counts_what_ran_inside_it_and_a_reset_starts_over() {
        let mut vm = Vm::new();
        assert_eq!(count(&mut vm, "cycles"), "0");
        vm.redisplay_begin();
        vm.eval_source("(cons 1 (cons 2 nil))").unwrap();
        vm.redisplay_done(std::time::Duration::ZERO);
        // Outside a cycle nothing is counted.
        vm.eval_source("(make-list 100 nil)").unwrap();
        assert_eq!(count(&mut vm, "cycles"), "1");
        let conses: u64 = count(&mut vm, "conses").parse().unwrap();
        assert!((2..100).contains(&conses), "the cycle's conses, not the session's: {conses}");
        if crate::host::thread_instructions().is_some() {
            let instructions: u64 = count(&mut vm, "instructions").parse().unwrap();
            assert!(instructions > 0, "the cycle's instructions");
        } else {
            assert_eq!(count(&mut vm, "instructions"), "nil");
        }
        // An end with no begin is no cycle.
        vm.redisplay_done(std::time::Duration::ZERO);
        assert_eq!(count(&mut vm, "cycles"), "1");
        assert_eq!(vm.eval_source("(cdr (assq 'cycles (romacs--display-stats t)))").unwrap().display().unwrap(), "1");
        assert_eq!((count(&mut vm, "cycles"), count(&mut vm, "conses")), ("0".into(), "0".into()));
    }

    /// What the counters cost a cycle: the two reads of the instruction
    /// counter, of which the part between them is counted as the cycle's.
    #[test]
    fn an_empty_cycle_costs_two_counter_reads() {
        let mut vm = Vm::new();
        let Some(before) = crate::host::thread_instructions() else { return };
        vm.redisplay_begin();
        vm.redisplay_done(std::time::Duration::ZERO);
        let whole = crate::host::thread_instructions().unwrap() - before;
        let counted: u64 = count(&mut vm, "instructions").parse().unwrap();
        eprintln!("an empty cycle: {whole} instructions in all (one read of the counter more than a cycle pays), {counted} of them counted as its own");
        assert!(counted < whole && whole < 20_000, "{counted} counted of {whole}");
    }

    #[test]
    fn a_snapshot_counts_its_windows_and_bands() {
        let mut vm = Vm::new();
        vm.set_frame_geometry(800, 280, 20, 10).unwrap();
        vm.eval_source("(setq mode-line-format \"band\")").unwrap();
        vm.frame_snapshot(800, 280).unwrap();
        let rebuilt: u64 = count(&mut vm, "windows-rebuilt").parse().unwrap();
        assert!(rebuilt >= 1, "the first snapshot builds its windows");
        vm.eval_source("(romacs--display-stats t)").unwrap();
        vm.frame_snapshot(800, 280).unwrap();
        let read = |vm: &mut Vm, name| count(vm, name).parse::<u64>().unwrap();
        let (visited, reused, rebuilt) = (read(&mut vm, "windows-visited"), read(&mut vm, "windows-reused"), read(&mut vm, "windows-rebuilt"));
        assert_eq!((visited, rebuilt), (reused, 0), "nothing changed: every window is reused");
        assert!(reused >= 1 && read(&mut vm, "band-expansions") >= 1, "a reused window expands its band again");
    }
}
