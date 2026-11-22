//! One `equal` walk's state (`hash.rs`'s `equal_at`): the pairs it has met
//! past GNU's depth limit, and its quit poll. Only `equal` called as itself
//! polls (`Walk::polled`); a hash lookup, `member` and every other caller walk
//! unpolled, so none of them stops halfway whatever scope it runs in.
use crate::Vm;
use rustc_hash::FxHashMap as HashMap;

/// Steps of a spine or vector between two asks of a polled walk.
pub(super) const WALK_CHUNK: usize = 4096;

#[derive(Default)]
pub(super) struct Walk {
    /// The pairs met past GNU's depth limit, made when first needed.
    pub(super) seen: Option<HashMap<u64, Vec<u64>>>,
    polled: bool,
    /// Steps since the walk last asked, over all its spines and vectors: a
    /// structure of many short ones (2048 vectors of 2048) asks too.
    steps: usize,
}

impl Walk {
    pub(super) fn polled() -> Self {
        Self { polled: true, ..Self::default() }
    }

    /// Whether the walk stops here, N more steps taken: polled, another
    /// `WALK_CHUNK` steps taken since it last asked, and the quit probe due.
    #[inline]
    pub(super) fn stopped(&mut self, n: usize, vm: &Vm) -> bool {
        if !self.polled { return false; }
        self.steps += n;
        if self.steps < WALK_CHUNK { return false; }
        self.steps = 0;
        probe_due(vm)
    }

    /// [`Self::stopped`] at a spine's end, STEPS long: the steps past its last
    /// whole chunk, which the spine's own asks did not count, count toward
    /// the walk's next ask.
    #[inline]
    pub(super) fn spine_ended(&mut self, steps: usize, vm: &Vm) -> bool {
        self.stopped(steps % WALK_CHUNK, vm)
    }
}

/// `Vm::own_stop_due`, kept out of the walk's loops.
#[cold]
#[inline(never)]
fn probe_due(vm: &Vm) -> bool {
    vm.own_stop_due()
}
