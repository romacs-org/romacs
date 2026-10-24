//! Dev-only, scaled strong-heap replay. Logical seconds, never physical time,
//! label memory samples. Timings are observations, not unit-test gates.
use romacs_mem::{
    AllocError, GcEvent, GcRecord, GcStats, Heap, HeapConfig, PvecType, Root, Trigger,
    raw::{ForeignFlags, ForeignHooks, ForeignKind, LegacyWeak, Marker, RawValue},
    spawn_isolate,
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub const LOGICAL_SECONDS: usize = 7200;
const STRINGS_PER_EVENT: usize = 16;
const RING: usize = 96;
const TEXT_CAPACITY: usize = 400_000;
const LEAK_BYTES: usize = 8192;
const KINDS: usize = 6;
/// Reclaim array order: cons, string, vector, symbol, float, legacy buffer.
pub const KIND_NAMES: [&str; KINDS] = [
    "cons",
    "string",
    "vector",
    "symbol",
    "float",
    "legacy_buffer",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
}
#[derive(Clone, Copy, Debug)]
pub enum Arm {
    Natural,
    Forced,
    Leak,
}

#[derive(Debug)]
pub struct Sample {
    pub logical_seconds: f64,
    pub record: GcRecord,
    /// Initialized page budget plus large spans; not resident RSS.
    pub committed_bytes: usize,
}
#[derive(Debug)]
pub struct Report {
    pub arm: Arm,
    pub samples: Vec<Sample>,
    pub stats: GcStats,
    pub epoch: u64,
    pub reclaimed: [usize; KINDS],
    pub payload_drops: usize,
    pub released_pages: usize,
    pub planted_bytes: usize,
    pub wall_ns: u128,
}

// Non-Lisp buffer storage, shared only with this isolate's mutator. No raw
// edges are hidden here; native vector slots own all graph references.
#[repr(C)]
struct Buffer {
    header: [u8; 16],
    text: Rc<RefCell<Vec<u8>>>,
    drops: Arc<AtomicUsize>,
}
impl Drop for Buffer {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}
// SAFETY: no Lisp edges, no allocation/callback/heap access during trace/drop.
// Mutator releases its text borrow before collection; bytes counts capacity.
struct BufferHooks;
// SAFETY: buffer() initializes Buffer fields after the header; there are no
// Lisp edges or weak entries. Destruction drops fields, never the cell itself.
unsafe impl ForeignHooks for BufferHooks {
    fn trace(&self, _: RawValue, _: &mut Marker<'_>) {}
    fn with_weak(&mut self, _: RawValue, _: &mut dyn FnMut(&mut dyn LegacyWeak)) {
        unreachable!()
    }
    fn finalizer_function(&self, _: RawValue) -> RawValue {
        unreachable!()
    }
    fn drop(&mut self, w: RawValue) {
        unsafe {
            std::ptr::drop_in_place((w.bits() & !7) as *mut Buffer);
        }
    }
    fn bytes(&self, w: RawValue) -> usize {
        unsafe { &*((w.bits() & !7) as *const Buffer) }
            .text
            .borrow()
            .capacity()
    }
    fn symbol_pinned(&self, _: RawValue) -> bool {
        false
    }
}

fn buffer(
    heap: &Heap,
    text: Rc<RefCell<Vec<u8>>>,
    drops: &Arc<AtomicUsize>,
) -> Result<Root, AllocError> {
    // SAFETY: adapter contains no edges or weak entries, and its RefCell is not
    // borrowed at this call or at any collection boundary.
    let value = unsafe {
        let w = heap.alloc_foreign(
            ForeignKind::Vectorlike,
            std::mem::size_of::<Buffer>(),
            ForeignFlags::NONE,
        )?;
        let ptr = (w.bits() & !7) as *mut Buffer;
        std::ptr::addr_of_mut!((*ptr).text).write(text);
        std::ptr::addr_of_mut!((*ptr).drops).write(drops.clone());
        w.value(heap)
    };
    let root = Root::new(heap, value, "replay-buffer");
    heap.account_external(value);
    Ok(root)
}

struct Mutator {
    chats: Vec<Root>,
    texts: Vec<Rc<RefCell<Vec<u8>>>>,
    ring: VecDeque<Root>,
    leak: Vec<Root>,
    allocated: [usize; KINDS],
    reclaimed: [usize; KINDS],
    drops: Arc<AtomicUsize>,
    serial: usize,
}
impl Mutator {
    fn new(heap: &Heap) -> Result<Self, AllocError> {
        let mut m = Self {
            chats: Vec::new(),
            texts: Vec::new(),
            ring: VecDeque::new(),
            leak: Vec::new(),
            allocated: [0; KINDS],
            reclaimed: [0; KINDS],
            drops: Arc::new(AtomicUsize::new(0)),
            serial: 0,
        };
        for _ in 0..3 {
            let text = Rc::new(RefCell::new(Vec::with_capacity(TEXT_CAPACITY)));
            m.chats.push(buffer(heap, text.clone(), &m.drops)?);
            m.texts.push(text);
            m.allocated[5] += 1;
        }
        Ok(m)
    }
    fn event(&mut self, heap: &Heap, event: usize, arm: Arm) -> Result<(), AllocError> {
        for _ in 0..STRINGS_PER_EVENT {
            self.serial += 1;
            // Alternating short ASCII / UTF-8; mean approximately 27 characters.
            let bytes = if self.serial.is_multiple_of(2) {
                "stream λ naïve 日本 emoji🙂 test".as_bytes()
            } else {
                b"stream tools callback test"
            };
            let string = heap.make_string(bytes, true)?;
            self.allocated[1] += 1;
            let mut list = heap.nil();
            for _ in 0..15 {
                list = heap.cons(string, list)?;
            }
            self.allocated[0] += 15;
            let vector = heap.make_vectorlike(
                PvecType::NormalVector,
                &[string, list, heap.nil(), heap.nil()],
                &[],
            )?;
            self.allocated[2] += 1;
            self.ring.push_back(Root::new(heap, vector, "replay-entry"));
            if self.ring.len() > RING {
                self.ring.pop_front();
            }
            if self.serial.is_multiple_of(46) {
                heap.make_symbol(string)?;
                self.allocated[3] += 1;
            }
            if self.serial.is_multiple_of(40) {
                heap.float(self.serial as f64)?;
                self.allocated[4] += 1;
            }
            let mut text = self.texts[self.serial % 3].borrow_mut();
            if text.len() + bytes.len() > TEXT_CAPACITY {
                text.clear();
            }
            text.extend_from_slice(bytes);
        }
        // Census shows high buffer-id churn with stable live count. These
        // temporary owners really die, rather than only overwriting a counter.
        if event.is_multiple_of(10) {
            let temp = buffer(heap, Rc::new(RefCell::new(vec![0; 2048])), &self.drops)?;
            self.allocated[5] += 1;
            drop(temp);
        }
        if matches!(arm, Arm::Leak) && event.is_multiple_of(30) {
            let value = heap.make_string(&vec![b'L'; LEAK_BYTES], false)?;
            self.leak.push(Root::new(heap, value, "planted-leak"));
            self.allocated[1] += 1;
        }
        self.special_churn_hook();
        Ok(())
    }
    // S5 seam: add weak-table removal, finalizer execution, marker unchaining
    // and RefCell undo-owner churn here, with separate NONZERO reclaim gates.
    // Strong S4 must not pretend Marker-tagged vectors implement unchaining.
    fn special_churn_hook(&mut self) {}

    fn collect(&mut self, heap: &mut Heap, event: usize, samples: &mut Vec<Sample>) {
        let cycle = heap.collect(&mut ());
        let live = [
            15 * self.ring.len(),
            self.ring.len() + self.leak.len(),
            self.ring.len(),
            0,
            0,
            self.chats.len(),
        ];
        let mut freed = 0;
        for (i, count) in live.iter().enumerate() {
            let dead = self.allocated[i] - count;
            freed += dead - self.reclaimed[i];
            self.reclaimed[i] = dead;
        }
        // Exact graph inventory, not a guessed kind split from an aggregate.
        assert_eq!(
            cycle.freed_objects, freed,
            "strong model disagrees with sweep"
        );
        assert_eq!(self.reclaimed[5], self.drops.load(Ordering::Relaxed));
        let stats = heap.allocation_stats();
        samples.push(Sample {
            logical_seconds: event as f64,
            record: cycle.record,
            committed_bytes: stats.committed_bytes,
        });
    }
}

/// Run all 7200 logical seconds without sleeping. Allocation scale is explicit
/// in README; this is a workload-shape gate, not a native editor/RSS benchmark.
pub fn replay(arm: Arm) -> Result<Report, AllocError> {
    spawn_isolate(
        HeapConfig {
            gc_cons_threshold: 16_000,
            gc_cons_percentage: 0.0,
            ..Default::default()
        },
        move |mut heap| {
            // SAFETY: this heap uses only the Buffer owner layout.
            unsafe {
                heap.set_foreign_hooks(Box::new(BufferHooks));
            }
            let start = Instant::now();
            let mut mutator = Mutator::new(&heap)?;
            let mut samples = Vec::new();
            heap.gc_event(true, Duration::ZERO);
            mutator.collect(&mut heap, 0, &mut samples);
            for event in 1..=LOGICAL_SECONDS {
                mutator.event(&heap, event, arm)?;
                if heap.gc_event(false, Duration::ZERO) == GcEvent::WantCycle {
                    mutator.collect(&mut heap, event, &mut samples);
                }
                if matches!(arm, Arm::Forced) && event.is_multiple_of(60) {
                    heap.gc_event(true, Duration::ZERO);
                    mutator.collect(&mut heap, event, &mut samples);
                }
                // Deterministic input wait: accumulate exactly the idle predicate's
                // half-threshold debt if ordinary churn has not already done so.
                if event.is_multiple_of(120) {
                    let missing = (heap.threshold() / 2).saturating_sub(heap.allocation_debt());
                    if missing > 0 {
                        heap.make_string(&vec![0; missing.saturating_sub(48)], false)?;
                        mutator.allocated[1] += 1;
                    }
                    if heap.gc_event(false, Duration::from_secs(1)) == GcEvent::WantCycle {
                        mutator.collect(&mut heap, event, &mut samples);
                    }
                }
            }
            // Read back the authoritative unbounded heap record, not only the
            // CycleReport copies observed at the caller. A lost event cannot pass.
            {
                let records = heap.gc_records();
                assert_eq!(records.len(), samples.len(), "heap dropped a GC record");
                for (sample, record) in samples.iter_mut().zip(records.iter()) {
                    assert_eq!(sample.record.seq, record.seq);
                    sample.record = *record;
                }
            }
            let stats = heap.gc_stats();
            let report = Report {
                arm,
                samples,
                stats,
                epoch: heap.epoch(),
                reclaimed: mutator.reclaimed,
                payload_drops: mutator.drops.load(Ordering::Relaxed),
                released_pages: heap.allocation_stats().released_pages,
                planted_bytes: mutator.leak.len() * LEAK_BYTES,
                wall_ns: start.elapsed().as_nanos(),
            };
            Ok(report)
        },
    )
    .join()
    .expect("replay isolate panicked")
}

pub fn median(values: &[usize]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    Some(if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] as f64 + sorted[n / 2] as f64) / 2.0
    } else {
        sorted[n / 2] as f64
    })
}
fn post_warmup(samples: &[Sample]) -> &[Sample] {
    &samples[samples.len() / 5..]
}
pub fn windows(samples: &[Sample], committed: bool) -> Option<[f64; 4]> {
    let samples = post_warmup(samples);
    if samples.len() < 4 {
        return None;
    }
    let mut out = [0.0; 4];
    for (i, value) in out.iter_mut().enumerate() {
        let window = &samples[i * samples.len() / 4..(i + 1) * samples.len() / 4];
        *value = median(
            &window
                .iter()
                .map(|s| {
                    if committed {
                        s.committed_bytes
                    } else {
                        s.record.accounted_after
                    }
                })
                .collect::<Vec<_>>(),
        )?;
    }
    Some(out)
}
fn stable(windows: Option<[f64; 4]>) -> bool {
    windows.is_some_and(|w| {
        let min = w.iter().copied().fold(f64::INFINITY, f64::min);
        let max = w.iter().copied().fold(0.0, f64::max);
        min > 0.0 && max <= min * 1.1
    })
}

/// Gate only deterministic invariants. Pause/share figures remain informational.
pub fn classify(report: &Report) -> Verdict {
    let samples = &report.samples;
    let mut sum = 0_u128;
    let mut max = 0;
    let mut overshoot = 0;
    let mut triggers = [0; 3];
    for (i, s) in samples.iter().enumerate() {
        if s.record.seq != i as u64 + 1 {
            return Verdict::Fail;
        }
        sum += s.record.gc_elapsed_ns as u128;
        max = max.max(s.record.gc_elapsed_ns);
        overshoot = overshoot.max(s.record.overshoot);
        triggers[match s.record.trigger {
            Trigger::Threshold => 0,
            Trigger::Explicit => 1,
            Trigger::Idle => 2,
        }] += 1;
    }
    let valid = samples.len() >= 200
        && triggers[0] >= 200
        && triggers[1] > 0
        && triggers[2] > 0
        && report.reclaimed.iter().all(|c| *c > 0)
        && report.payload_drops == report.reclaimed[5]
        && report.released_pages > 0
        && samples.len() as u64 == report.epoch
        && report.stats.count == report.epoch
        && report.stats.sum_ns == sum
        && report.stats.max_ns == max
        && report.stats.max_overshoot == overshoot
        && overshoot > 0
        && stable(windows(samples, false))
        && stable(windows(samples, true));
    if valid { Verdict::Pass } else { Verdict::Fail }
}

pub fn accounted_median(report: &Report) -> f64 {
    median(
        &post_warmup(&report.samples)
            .iter()
            .map(|s| s.record.accounted_after)
            .collect::<Vec<_>>(),
    )
    .unwrap_or(0.0)
}
pub fn forced_matches(normal: &Report, forced: &Report) -> bool {
    let n = accounted_median(normal);
    let f = accounted_median(forced);
    n > 0.0 && (f - n).abs() <= n * 0.05
}
/// OLS bytes against logical seconds at t >= 600; convert to MiB/logical hour.
pub fn slope(report: &Report) -> Option<f64> {
    let s: Vec<_> = report
        .samples
        .iter()
        .filter(|s| s.logical_seconds >= 600.0)
        .collect();
    if s.len() < 2 {
        return None;
    }
    let mean_t = s.iter().map(|s| s.logical_seconds).sum::<f64>() / s.len() as f64;
    let mean_b = s
        .iter()
        .map(|s| s.record.accounted_after as f64)
        .sum::<f64>()
        / s.len() as f64;
    let numerator = s
        .iter()
        .map(|s| (s.logical_seconds - mean_t) * (s.record.accounted_after as f64 - mean_b))
        .sum::<f64>();
    let denominator = s
        .iter()
        .map(|s| (s.logical_seconds - mean_t).powi(2))
        .sum::<f64>();
    (denominator > 0.0).then_some(numerator / denominator * 3600.0 / 1_048_576.0)
}
pub fn p99(report: &Report) -> Option<u64> {
    let mut ns: Vec<_> = report
        .samples
        .iter()
        .map(|s| s.record.gc_elapsed_ns)
        .collect();
    ns.sort_unstable();
    if ns.is_empty() {
        None
    } else {
        Some(ns[(99 * ns.len()).div_ceil(100) - 1])
    }
}
pub fn classify_suite() -> (Verdict, Verdict, Verdict) {
    let normal = replay(Arm::Natural).expect("normal allocation");
    let leak = replay(Arm::Leak).expect("leak allocation");
    let forced = replay(Arm::Forced).expect("forced allocation");
    assert!(forced_matches(&normal, &forced));
    assert!(leak.planted_bytes as f64 >= accounted_median(&normal) * 0.3);
    (classify(&normal), classify(&leak), classify(&forced))
}
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
