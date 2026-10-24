//! GNU eval.c evaluation-depth storage and emergency reserve contract.
use super::*;

const MAX_DEPTH: &str = "max-lisp-eval-depth";
const RESERVE: &str = "lisp-eval-depth-reserve";

// A byte reserve for the largest compiler/condition paths, NOT a native depth
// limit. Unknown stack bounds fail closed rather than guessing a depth. It
// must stay well under the smallest thread a VM legitimately runs on:
// 256 KiB refused every compile on traversal.rs's 256 KiB dispatch thread.
// Reentry does not depend on it: execute_args/load/lazy compile grow a fresh
// segment below a 256-512 KiB red zone before running.
pub(super) const NATIVE_STACK_HEADROOM: usize = 128 * 1024;
#[inline(never)]
pub(super) fn native_stack_room() -> bool {
    stacker::remaining_stack().is_some_and(|bytes| bytes >= NATIVE_STACK_HEADROOM)
}

/// The current native stack pointer, read without a call so the threaded
/// handlers keep their sibling calls.
#[inline(always)]
pub(super) fn stack_pointer() -> usize {
    let sp: usize;
    #[cfg(target_arch = "x86_64")]
    unsafe { std::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack, preserves_flags)) };
    #[cfg(target_arch = "aarch64")]
    unsafe { std::arch::asm!("mov {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags)) };
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    { sp = stacker::remaining_stack().map_or(0, |r| r.saturating_add(NATIVE_STACK_HEADROOM)) };
    sp
}

/// The address below which fewer than NATIVE_STACK_HEADROOM bytes remain on
/// the current stack segment; usize::MAX (refuse) when the bound is unknown.
/// Valid for the segment it is computed on: each engine computes its own.
pub(super) fn native_stack_floor() -> usize {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    { stacker::remaining_stack().map_or(usize::MAX, |rem| stack_pointer().saturating_sub(rem).saturating_add(NATIVE_STACK_HEADROOM)) }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    { NATIVE_STACK_HEADROOM.saturating_mul(2) }
}

impl Vm {
    /// GNU eval.c:2604-2609: limits below 100 are raised, not merely read as 100.
    pub(super) fn eval_depth_limit(&mut self) -> usize {
        let old = self.forwarded.eval_limit();
        let limit = if (self.eval_depth as u128) > old.max(0) as u128 && old < 100 { 100 } else { old };
        if limit != old { self.forwarded.set_integer(MAX_DEPTH, limit); }
        usize::try_from(limit).unwrap_or(0)
    }

    /// Check an already incremented depth; entry/exit ownership stays with the engine.
    pub(super) fn check_eval_depth(&mut self) -> Result<()> {
        if self.eval_depth > self.eval_depth_limit() {
            Err("excessive-lisp-nesting".into())
        } else {
            Ok(())
        }
    }

    /// GNU eval.c:266-276. The returned integer is the unwind token: callers
    /// must restore it on every exit, in LIFO order. No Lisp call site exists
    /// yet for debugger/handler-bind; signal-hook-function borrows twenty.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn max_ensure_room(&mut self, room: i64) -> Option<i64> {
        let depth = i64::try_from(self.eval_depth).unwrap_or(i64::MAX);
        let sum = depth.checked_add(room).unwrap_or(i64::MAX);
        let old = self.forwarded.integer(MAX_DEPTH);
        let reserve = self.forwarded.integer(RESERVE);
        // Widen the difference: a negative user limit must not wrap before
        // the minimum with the (bounded) reserve has been taken.
        let diff = (i128::from(sum) - i128::from(old)).min(i128::from(reserve));
        if diff <= 0 {
            return None;
        }
        let diff = i64::try_from(diff).expect("positive difference bounded by reserve");
        self.forwarded.set_integer(MAX_DEPTH, old + diff);
        self.forwarded.set_integer(RESERVE, reserve - diff);
        Some(old)
    }

    /// GNU eval.c:255-260: return CURRENT max minus saved max to CURRENT
    /// reserve; do not restore a saved pair (Lisp may have changed either).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn restore_stack_limits(&mut self, old_depth: i64) {
        let current = self.forwarded.integer(MAX_DEPTH);
        let reserve = self.forwarded.integer(RESERVE);
        let restored = i128::from(reserve) + i128::from(current) - i128::from(old_depth);
        let restored = i64::try_from(restored).expect("restored eval reserve fits intmax_t");
        self.forwarded.set_integer(RESERVE, restored);
        self.forwarded.set_integer(MAX_DEPTH, old_depth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(vm: &mut Vm, name: &str) -> i64 {
        let slot = vm.slot(name);
        let fwd = vm.forwarding(slot).expect("forwarded depth integer");
        vm.forwarded_value(fwd).unwrap().integer_value().unwrap()
    }

    fn limits(vm: &mut Vm, max: i64, reserve: i64) {
        vm.forwarded.set_integer(MAX_DEPTH, max);
        vm.forwarded.set_integer(RESERVE, reserve);
    }

    #[test]
    fn forwarded_defaults_and_integer_contract() {
        let mut vm = Vm::new();
        for (name, expected) in [
            ("max-lisp-eval-depth", 1600),
            ("lisp-eval-depth-reserve", 200),
        ] {
            let slot = vm.slot(name);
            assert!(
                matches!(vm.forwarding(slot), Some(forwarded::Fwd::Int(_))),
                "DEPTH_FORWARDED: {name}"
            );
            assert_eq!(number(&mut vm, name), expected, "DEPTH_DEFAULT: {name}");
            let value = vm
                .eval_source_raw(&format!(
                    "(condition-case e (setq {name} 'bad) (wrong-type-argument (car e)))"
                ))
                .unwrap();
            assert_eq!(vm.display(value).unwrap(), "wrong-type-argument", "DEPTH_INTEGER_TYPE: {name}");
            let value = vm
                .eval_source_raw(&format!("(let (({name} 321)) {name})"))
                .unwrap();
            assert_eq!(value.integer_value(), Some(321), "DEPTH_LET_BINDS: {name}");
            assert_eq!(number(&mut vm, name), expected, "DEPTH_LET_RESTORES: {name}");
        }
        assert_eq!(vm.eval_depth, 0);
    }

    #[test]
    fn limit_clamps_and_checks_strict_boundary() {
        let mut vm = Vm::new();
        limits(&mut vm, -2, 200);
        vm.eval_depth = 99;
        assert_eq!(vm.eval_depth_limit(), 100, "DEPTH_CLAMP");
        assert_eq!(number(&mut vm, "max-lisp-eval-depth"), 100);
        vm.eval_depth = 100;
        assert!(vm.check_eval_depth().is_ok(), "DEPTH_BOUNDARY_INCLUSIVE");
        vm.eval_depth = 101;
        assert!(vm.check_eval_depth().is_err(), "DEPTH_BOUNDARY");
    }

    #[test]
    fn reserve_borrow_and_exact_restoration_arithmetic() {
        let mut vm = Vm::new();
        limits(&mut vm, 1600, 200);
        vm.eval_depth = 1590;
        let old = vm.max_ensure_room(50).expect("DEPTH_RESERVE_BORROW");
        assert_eq!(old, 1600, "DEPTH_RESERVE_TOKEN");
        assert_eq!(number(&mut vm, "max-lisp-eval-depth"), 1640, "DEPTH_RESERVE_RAISED");
        assert_eq!(number(&mut vm, "lisp-eval-depth-reserve"), 160, "DEPTH_RESERVE_TAKEN");
        // GNU restores from CURRENT values, not a snapshot of both globals.
        limits(&mut vm, 1660, 123);
        vm.restore_stack_limits(old);
        assert_eq!(
            number(&mut vm, "max-lisp-eval-depth"),
            1600,
            "DEPTH_RESTORE"
        );
        assert_eq!(number(&mut vm, "lisp-eval-depth-reserve"), 183, "DEPTH_RESTORE_RESERVE_FROM_CURRENT");
    }

    #[test]
    fn reserve_caps_noops_and_nested_tokens() {
        let mut vm = Vm::new();
        limits(&mut vm, 1600, 200);
        vm.eval_depth = 1500;
        assert_eq!(vm.max_ensure_room(50), None, "DEPTH_RESERVE_NOOP");
        vm.eval_depth = 1590;
        let outer = vm.max_ensure_room(50).expect("DEPTH_RESERVE_NESTED");
        let inner = vm.max_ensure_room(1000).unwrap();
        assert_eq!(number(&mut vm, "max-lisp-eval-depth"), 1800, "DEPTH_RESERVE_CAPPED");
        assert_eq!(number(&mut vm, "lisp-eval-depth-reserve"), 0, "DEPTH_RESERVE_EXHAUSTED");
        assert_eq!(vm.max_ensure_room(1000), None, "DEPTH_RESERVE_EMPTY_NOOP");
        vm.restore_stack_limits(inner);
        vm.restore_stack_limits(outer);
        assert_eq!(number(&mut vm, "max-lisp-eval-depth"), 1600, "DEPTH_NESTED_RESTORE_MAX");
        assert_eq!(number(&mut vm, "lisp-eval-depth-reserve"), 200, "DEPTH_NESTED_RESTORE_RESERVE");
        limits(&mut vm, 1600, -5);
        assert_eq!(vm.max_ensure_room(1000), None, "DEPTH_NEGATIVE_RESERVE_NOOP");
    }

    #[test]
    fn reserve_checked_sum_overflow_saturates_before_borrow() {
        let mut vm = Vm::new();
        limits(&mut vm, 1600, 200);
        vm.eval_depth = usize::MAX;
        let old = vm
            .max_ensure_room(i64::MAX)
            .expect("DEPTH_RESERVE_OVERFLOW");
        assert_eq!(number(&mut vm, "max-lisp-eval-depth"), 1800, "DEPTH_OVERFLOW_CAPPED");
        assert_eq!(number(&mut vm, "lisp-eval-depth-reserve"), 0, "DEPTH_OVERFLOW_TAKEN");
        vm.restore_stack_limits(old);
        assert_eq!(number(&mut vm, "max-lisp-eval-depth"), 1600);
        assert_eq!(number(&mut vm, "lisp-eval-depth-reserve"), 200);
    }
}
