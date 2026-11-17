//! The safepoint-gap probe: how long the evaluator's thread ran between two
//! looks at the quit word, and which primitive was running then. A test
//! build's instrument (feature `safepoint-gap-probe`); without it every hook
//! here is empty and inlined away.
//!
//! A *safepoint* is a place that looks at the host's C-g and would act on it:
//! the engine's entry and periodic polls, the loader's poll between forms, a
//! native loop's poll ([`crate::Vm::native_poll`]) where it may quit, the
//! matcher's probe, a quittable wait's slice. Between two of them the thread
//! runs code a C-g cannot reach. The probe splits that span into segments,
//! each charged to the innermost primitive in flight (`enter`/`exit` around
//! every primitive body and collection), and when the span ends at a
//! safepoint longer than the threshold, appends one line to the file
//! `ROMACS_GAP_PROBE` names:
//!
//! `gap <wall ms> <cpu ms> <leaf> <leaf's wall ms> <frames>`
//!
//! where LEAF is the segment's primitive that took the most of the span (`-`
//! for the engine's own code), and FRAMES the innermost Lisp frames as LEAF
//! returned (or at the span's end). Thread CPU time is read at most once a
//! millisecond, so a CPU figure is good to about a millisecond; wall time is
//! read at every event. `ROMACS_GAP_PROBE_MS` sets the threshold (default 50).

#[cfg(feature = "safepoint-gap-probe")]
mod on {
    use std::cell::RefCell;
    use std::io::Write;
    use std::time::Duration;

    /// One primitive's share of the span since the last safepoint.
    struct Segment {
        leaf: &'static str,
        wall: Duration,
        frames: String,
    }

    struct Probe {
        /// Where to append rows; None turns the probe off.
        out: Option<std::path::PathBuf>,
        threshold: Duration,
        /// The last event (safepoint, entry or exit).
        last: std::time::Instant,
        /// The last safepoint, and the thread's CPU time near it.
        span_start: std::time::Instant,
        span_cpu: Duration,
        /// The last CPU read, and when.
        cpu_read: (std::time::Instant, Duration),
        stack: Vec<&'static str>,
        segments: Vec<Segment>,
    }

    fn thread_cpu() -> Duration {
        // HOST(rustix::query): the probe's own clock, in a test build only.
        let t = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
        Duration::new(u64::try_from(t.tv_sec).unwrap_or(0), u32::try_from(t.tv_nsec).unwrap_or(0))
    }

    impl Probe {
        fn new() -> Self {
            // HOST(std::env::var_os): the probe's output file, in a test build only.
            let out = std::env::var_os("ROMACS_GAP_PROBE").map(std::path::PathBuf::from);
            // HOST(std::env::var): the probe's threshold, in a test build only.
            let ms = std::env::var("ROMACS_GAP_PROBE_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(50);
            // HOST(Instant::now): the probe's own clock, in a test build only.
            let now = std::time::Instant::now();
            let cpu = thread_cpu();
            Self { out, threshold: Duration::from_millis(ms), last: now, span_start: now, span_cpu: cpu,
                   cpu_read: (now, cpu), stack: Vec::new(), segments: Vec::new() }
        }

        /// The time now, and the CPU time at most a millisecond old.
        fn now(&mut self) -> std::time::Instant {
            // HOST(Instant::now): the probe's own clock, in a test build only.
            let now = std::time::Instant::now();
            if now.duration_since(self.cpu_read.0) > Duration::from_millis(1) {
                self.cpu_read = (now, thread_cpu());
            }
            now
        }

        /// Charge the span since the last event to the innermost primitive.
        fn charge(&mut self, now: std::time::Instant, frames: impl FnOnce() -> String) {
            let wall = now.duration_since(self.last);
            self.last = now;
            let leaf = self.stack.last().copied().unwrap_or("-");
            match self.segments.iter_mut().find(|s| s.leaf == leaf) {
                Some(segment) => {
                    segment.wall += wall;
                    if wall * 4 >= self.threshold { segment.frames = frames(); }
                }
                None => {
                    let frames = if wall * 4 >= self.threshold { frames() } else { String::new() };
                    self.segments.push(Segment { leaf, wall, frames });
                }
            }
        }

        fn safepoint(&mut self, frames: impl FnOnce() -> String) {
            let now = self.now();
            let span = now.duration_since(self.span_start);
            if span >= self.threshold {
                self.charge(now, frames);
                let cpu = thread_cpu();
                self.cpu_read = (now, cpu);
                self.report(span, cpu.saturating_sub(self.span_cpu));
            } else {
                self.last = now;
            }
            self.segments.clear();
            self.span_start = now;
            self.span_cpu = self.cpu_read.1;
        }

        fn report(&self, wall: Duration, cpu: Duration) {
            let Some(path) = &self.out else { return };
            let Some(top) = self.segments.iter().max_by_key(|s| s.wall) else { return };
            let line = format!("gap {:.1} {:.1} {} {:.1} {}\n", wall.as_secs_f64() * 1e3, cpu.as_secs_f64() * 1e3,
                               top.leaf, top.wall.as_secs_f64() * 1e3, if top.frames.is_empty() { "-" } else { &top.frames });
            if let Ok(mut file) = crate::host::open_output(path, crate::host::OpenOptions::new().create(true).append(true)) {
                let _ = file.write_all(line.as_bytes());
            }
        }
    }

    thread_local! {
        static PROBE: RefCell<Probe> = RefCell::new(Probe::new());
    }

    fn frames(vm: Option<&crate::Vm>) -> String {
        let Some(vm) = vm else { return "(no-vm)".into() };
        let names: Vec<&str> = vm.backtrace.frames.iter().rev().take(4)
            .map(|f| f.function.index_if(7).and_then(|n| n.checked_sub(1)).and_then(|n| vm.symbol_names.get(n))
                 .map_or("?", String::as_str))
            .collect();
        if names.is_empty() { "(top)".into() } else { names.join("<") }
    }

    // Out of line: the engine dispatches by sibling calls, which a
    // guard inlined into its functions would turn into nested frames.
    #[inline(never)]
    pub(crate) fn safepoint(vm: Option<&crate::Vm>) {
        PROBE.with(|p| if let Ok(mut p) = p.try_borrow_mut() && p.out.is_some() { p.safepoint(|| frames(vm)) });
    }

    // Out of line: the engine dispatches by sibling calls, which a
    // guard inlined into its functions would turn into nested frames.
    #[inline(never)]
    pub(crate) fn enter(vm: &crate::Vm, leaf: impl FnOnce() -> &'static str) {
        PROBE.with(|p| if let Ok(mut p) = p.try_borrow_mut() && p.out.is_some() {
            let now = p.now();
            p.charge(now, || frames(Some(vm)));
            p.stack.push(leaf());
        });
    }

    // Out of line: the engine dispatches by sibling calls, which a
    // guard inlined into its functions would turn into nested frames.
    #[inline(never)]
    pub(crate) fn exit(vm: &crate::Vm) {
        PROBE.with(|p| if let Ok(mut p) = p.try_borrow_mut() && p.out.is_some() {
            let now = p.now();
            p.charge(now, || frames(Some(vm)));
            p.stack.pop();
        });
    }
}

#[cfg(feature = "safepoint-gap-probe")]
pub(crate) use on::{enter, exit, safepoint};

#[cfg(not(feature = "safepoint-gap-probe"))]
#[inline(always)]
pub(crate) fn safepoint(_vm: Option<&crate::Vm>) {}
#[cfg(not(feature = "safepoint-gap-probe"))]
#[inline(always)]
pub(crate) fn enter(_vm: &crate::Vm, _leaf: impl FnOnce() -> &'static str) {}
#[cfg(not(feature = "safepoint-gap-probe"))]
#[inline(always)]
pub(crate) fn exit(_vm: &crate::Vm) {}

/// QUIT, as a VM's signal: in the probe's build a batch VM's is held twice,
/// as a host shares it, so the matcher and waits poll as they do under a
/// host (`Vm::poll_quittable`, the regexp's probe) and the spans are a
/// host's.
#[inline(always)]
pub(crate) fn as_hosted(quit: std::sync::Arc<crate::quit::QuitSignal>) -> std::sync::Arc<crate::quit::QuitSignal> {
    #[cfg(feature = "safepoint-gap-probe")]
    std::mem::forget(quit.clone());
    quit
}

/// PRIMITIVE's body, BODY, run with the probe charging its time to it.
#[inline(always)]
pub(crate) fn within<T>(vm: &mut crate::Vm, primitive: impl FnOnce() -> &'static str, body: impl FnOnce(&mut crate::Vm) -> T) -> T {
    enter(vm, primitive);
    let out = body(vm);
    exit(vm);
    out
}
