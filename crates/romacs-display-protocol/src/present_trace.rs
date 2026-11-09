//! Opt-in per-presentation stage evidence, not a GPU/display fence.
//!
//! `ROMACS_PRESENT_TRACE=/absolute/path.jsonl` enables an append-only Linux
//! trace. Local stages use CLOCK_MONOTONIC; publish also samples CLOCK_REALTIME
//! immediately afterwards for mapping Lisp float-time offline. Wayland native
//! feedback retains its advertised clock ID; consumers must reject mismatched
//! domains. X11/non-Wayland captures contain local stages only.
//! Repeated draws of one sealed revision intentionally repeat its identity.
//! Input receive records preserve the platform timestamp/domain and carry an
//! `input_sequence`. Command scopes carry the first and last consumed sequences
//! as `input_sequence` and `input_sequence_end`; these are endpoints, not proof
//! that every intervening input was consumed. Unknown presentation IDs are zero.
//! `submit` marks the root glyph pass's queue submission, or the retained
//! static composition's submission, not the frame's final GPU submission; a
//! rebuilt retained static scene records it twice.
//!
//! Enabling the trace also requests Wayland presentation feedback for every
//! frame, which keeps the render thread waking while feedback is pending. A
//! traced run therefore measures a slightly busier renderer than an untraced one.
//!
//! Hot threads sample clocks and try_send bounded-queue records; optional detail
//! strings are allocated only while tracing. Nobody
//! waits on the queue, so a send never wakes another thread. One writer owns
//! the file and drains the queue every 10 ms into buffered JSONL (no fsync).
//! The queue is bounded; dropped records are reported separately. Abrupt
//! process exit can lose a tail, and so can a daemon's normal exit, which
//! is not drained at teardown. Call [`init`]
//! early so the writer thread is not started under a hot-path lock, and
//! [`shutdown`] after GUI/evaluator teardown for a normal drain.

use crate::PresentationId;
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use serde::Serialize;
use std::{
    ffi::OsString,
    io::{self, BufWriter, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

const CAPACITY: usize = 4096;
const DRAIN_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    RedisplayStart,
    RedisplayEnd,
    ResolveLayoutStart,
    ResolveLayoutEnd,
    FontifyStart,
    FontifyEnd,
    ModeLineStart,
    ModeLineEnd,
    ResolveStart,
    ResolveEnd,
    CoverageRetry,
    CommandStart,
    CommandEnd,
    InputReceived,
    FlattenStart,
    FlattenEnd,
    MaterializeStart,
    MaterializeEnd,
    AcquireStart,
    AcquireEnd,
    TessellateStart,
    TessellateEnd,
    AtlasUploadStart,
    AtlasUploadEnd,
    NonRootStart,
    NonRootEnd,
    GpuDone,
    SubmitStart,
    SubmitEnd,
    PresentCallStart,
    PresentCallEnd,
    Replaced,
    FontMissStart,
    FontMissEnd,
    FallbackStart,
    FallbackEnd,
    FontLoadStart,
    FontLoadEnd,
    ShapeStart,
    ShapeEnd,
    LayoutQueryStart,
    LayoutQueryEnd,

    Publish,
    RenderStart,
    Submit,
    Present,
    Discarded,
    Presented,
}

#[derive(Clone, Debug, Serialize)]
struct Record {
    stage: Stage,
    frame: u64,
    presentation: u64,
    clock_id: u32,
    ns: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    realtime_ns: Option<u64>,
    /// First consumed input sequence (or this received input's sequence).
    #[serde(skip_serializing_if = "Option::is_none")]
    input_sequence: Option<u64>,
    /// Last consumed input sequence; omitted for a single received input.
    #[serde(skip_serializing_if = "Option::is_none")]
    input_sequence_end: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    draw_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasons: Option<String>,
}

struct Recorder {
    events: Sender<Record>,
    dropped: Arc<AtomicU64>,
    stop: Sender<()>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

static RECORDER: OnceLock<Option<Recorder>> = OnceLock::new();

fn parse_path(value: Option<OsString>) -> Result<Option<PathBuf>, &'static str> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("ROMACS_PRESENT_TRACE requires an absolute path");
    }
    Ok(Some(path))
}

fn recorder() -> Option<&'static Recorder> {
    RECORDER
        .get_or_init(|| {
            let path = match parse_path(std::env::var_os("ROMACS_PRESENT_TRACE")) {
                Ok(path) => path?,
                Err(error) => {
                    tracing::warn!(error, "presentation trace disabled");
                    return None;
                }
            };
            if !cfg!(target_os = "linux") {
                tracing::warn!("presentation trace requires Linux clocks");
                return None;
            }
            let (events, incoming) = bounded(CAPACITY);
            let (stop, stopped) = bounded(1);
            let dropped = Arc::new(AtomicU64::new(0));
            let writer_dropped = dropped.clone();
            let writer = match std::thread::Builder::new()
                .name("romacs-present-trace".into())
                .spawn(move || {
                    let result = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .and_then(|file| {
                            write_records(BufWriter::new(file), incoming, stopped, &writer_dropped)
                        });
                    if let Err(error) = result {
                        tracing::warn!(%error, "cannot write presentation trace");
                    }
                }) {
                Ok(writer) => writer,
                Err(error) => {
                    tracing::warn!(%error, "cannot start presentation trace writer");
                    return None;
                }
            };
            Some(Recorder {
                events,
                dropped,
                stop,
                writer: Mutex::new(Some(writer)),
            })
        })
        .as_ref()
}

/// Environment configuration is cached once. Unset: no clocks or allocation.
#[inline]
pub fn enabled() -> bool {
    recorder().is_some()
}

/// Read the configuration and start the writer, if enabled, before any hot
/// path records a stage.
pub fn init() {
    let _ = recorder();
}

#[cfg(target_os = "linux")]
fn clock_ns(clock: libc::clockid_t) -> Option<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes to this valid, initialized timespec.
    if unsafe { libc::clock_gettime(clock, &mut time) } != 0 {
        return None;
    }
    u64::try_from(time.tv_sec)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(u64::try_from(time.tv_nsec).ok()?)
}

fn sample(stage: Stage) -> Option<(u32, u64, Option<u64>)> {
    #[cfg(target_os = "linux")]
    {
        let ns = clock_ns(libc::CLOCK_MONOTONIC)?;
        let realtime = if stage == Stage::Publish {
            Some(clock_ns(libc::CLOCK_REALTIME)?)
        } else {
            None
        };
        Some((libc::CLOCK_MONOTONIC as u32, ns, realtime))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = stage;
        None
    }
}

impl Recorder {
    fn send(&self, record: Record) {
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
            self.events.try_send(record)
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn stamp_with(
    recorder: &Recorder,
    stage: Stage,
    frame: u64,
    presentation: PresentationId,
    clock: impl FnOnce(Stage) -> Option<(u32, u64, Option<u64>)>,
) -> Option<Record> {
    let Some((clock_id, ns, realtime_ns)) = clock(stage) else {
        recorder.dropped.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    Some(Record {
        stage,
        frame,
        presentation: presentation.get(),
        clock_id,
        ns,
        realtime_ns,
        input_sequence: None,
        input_sequence_end: None,
        draw_seq: current_draw(),
        detail: None,
        plan: (stage == Stage::RenderStart)
            .then(|| DRAW_PLAN.with(|p| p.borrow().as_ref().map(|p| p.0.clone())))
            .flatten(),
        reasons: (stage == Stage::RenderStart)
            .then(|| DRAW_PLAN.with(|p| p.borrow().as_ref().map(|p| p.1.clone())))
            .flatten(),
    })
}

fn record_to(
    recorder: Option<&Recorder>,
    stage: Stage,
    frame: u64,
    presentation: PresentationId,
    clock: impl FnOnce(Stage) -> Option<(u32, u64, Option<u64>)>,
) {
    let Some(recorder) = recorder else { return };
    if let Some(record) = stamp_with(recorder, stage, frame, presentation, clock) {
        recorder.send(record);
    }
}

#[inline]
pub fn record(stage: Stage, frame: u64, presentation: PresentationId) {
    record_to(recorder(), stage, frame, presentation, sample);
}

thread_local! {
    static DRAW: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static DRAW_IDENTITY: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
    static DRAW_PLAN: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}

pub fn current_draw() -> Option<u64> {
    DRAW.with(|draw| draw.get())
}
pub fn current_draw_identity() -> (u64, PresentationId) {
    let (frame, presentation) = DRAW_IDENTITY.with(|i| i.get());
    (frame, PresentationId::new(presentation))
}

/// The draw scope is thread-bound; callbacks carry the cookie explicitly.
#[must_use]
pub struct DrawScope {
    previous: Option<u64>,
    previous_identity: (u64, u64),
    previous_plan: Option<(String, String)>,
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl DrawScope {
    pub fn begin(frame: u64, presentation: PresentationId, plan: &str, reasons: String) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let next = enabled().then(|| NEXT.fetch_add(1, Ordering::Relaxed));
        Self {
            previous: DRAW.with(|draw| draw.replace(next)),
            previous_identity: DRAW_IDENTITY.with(|i| i.replace((frame, presentation.get()))),
            previous_plan: DRAW_PLAN.with(|p| p.replace(next.map(|_| (plan.to_owned(), reasons)))),
            _thread_bound: std::marker::PhantomData,
        }
    }
}
impl Drop for DrawScope {
    fn drop(&mut self) {
        DRAW.with(|draw| draw.set(self.previous));
        DRAW_IDENTITY.with(|i| i.set(self.previous_identity));
        DRAW_PLAN.with(|p| p.replace(self.previous_plan.take()));
    }
}

pub fn record_draw(stage: Stage, frame: u64, presentation: PresentationId, draw_seq: Option<u64>) {
    if let Some(recorder) = recorder()
        && let Some(mut record) = stamp_with(recorder, stage, frame, presentation, sample)
    {
        record.draw_seq = draw_seq;
        recorder.send(record);
    }
}

pub fn record_detail(stage: Stage, frame: u64, presentation: PresentationId, detail: &str) {
    if let Some(recorder) = recorder()
        && let Some(mut record) = stamp_with(recorder, stage, frame, presentation, sample)
    {
        record.detail = Some(detail.to_owned());
        recorder.send(record);
    }
}

/// Opt-in local diagnostic scope. Zero identities mean unknown, not a sealed
/// presentation. Dropping the scope records its end, including on unwind.
#[must_use]
pub struct Span {
    end: Option<Stage>,
    frame: u64,
    presentation: PresentationId,
    inputs: (Option<u64>, Option<u64>),
    detail: Option<String>,
}

impl Span {
    #[inline]
    pub fn new(start: Stage, end: Stage, frame: u64, presentation: Option<PresentationId>) -> Self {
        Self::with_inputs(start, end, frame, presentation, (None, None))
    }

    #[cfg(test)]
    pub(crate) fn command(frame: u64, first: Option<u64>, last: Option<u64>) -> Self {
        Self::with_inputs(
            Stage::CommandStart,
            Stage::CommandEnd,
            frame,
            None,
            (first, last),
        )
    }

    fn with_inputs(
        start: Stage,
        end: Stage,
        frame: u64,
        presentation: Option<PresentationId>,
        inputs: (Option<u64>, Option<u64>),
    ) -> Self {
        let span = Self {
            end: enabled().then_some(end),
            frame,
            presentation: presentation.unwrap_or_else(|| PresentationId::new(0)),
            inputs,
            detail: None,
        };
        if span.end.is_some() {
            span.record(start);
        }
        span
    }

    fn record(&self, stage: Stage) {
        if let Some(recorder) = recorder()
            && let Some(mut record) =
                stamp_with(recorder, stage, self.frame, self.presentation, sample)
        {
            record.detail = self.detail.clone();
            record.input_sequence = self.inputs.0;
            record.input_sequence_end = self.inputs.1;
            recorder.send(record);
        }
    }
}

impl Drop for Span {
    #[inline]
    fn drop(&mut self) {
        if let Some(end) = self.end {
            self.record(end);
        }
    }
}

/// Preserve the platform receive timestamp and its clock domain. This is not
/// sampled command time, and does not imply a presentation has been sealed.
pub(crate) fn input_received(frame: u64, sequence: u64, clock_id: u32, ns: u64) {
    if let Some(recorder) = recorder() {
        recorder.send(Record {
            stage: Stage::InputReceived,
            frame,
            presentation: 0,
            clock_id,
            ns,
            realtime_ns: None,
            input_sequence: Some(sequence),
            input_sequence_end: None,
            draw_seq: current_draw(),
            detail: None,
            plan: None,
            reasons: None,
        });
    }
}

/// A stage timestamp taken now and queued later by [`Stamp::emit`], so a
/// caller can sample inside a critical section and queue after leaving it.
#[must_use]
pub struct Stamp(Record);

/// Sample a stage without queueing it. `None` when the trace is disabled.
#[inline]
pub fn stamp(stage: Stage, frame: u64, presentation: PresentationId) -> Option<Stamp> {
    stamp_with(recorder()?, stage, frame, presentation, sample).map(Stamp)
}

impl Stamp {
    pub fn emit(self) {
        if let Some(recorder) = recorder() {
            recorder.send(self.0);
        }
    }
}

/// Preserve the compositor's native timestamp/domain, not callback receive time.
#[inline]
pub fn presented(frame: u64, presentation: PresentationId, clock_id: u32, ns: u64) {
    presented_draw(frame, presentation, clock_id, ns, None);
}

pub fn presented_draw(
    frame: u64,
    presentation: PresentationId,
    clock_id: u32,
    ns: u64,
    draw_seq: Option<u64>,
) {
    let Some(recorder) = recorder() else { return };
    recorder.send(Record {
        stage: Stage::Presented,
        frame,
        presentation: presentation.get(),
        clock_id,
        ns,
        realtime_ns: None,
        input_sequence: None,
        input_sequence_end: None,
        draw_seq,
        detail: None,
        plan: None,
        reasons: None,
    });
}

fn write_record(writer: &mut impl Write, record: &Record) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, record)?;
    writer.write_all(b"\n")
}

fn flush_records(writer: &mut impl Write, dropped: &AtomicU64) -> io::Result<()> {
    let count = dropped.swap(0, Ordering::Relaxed);
    if count != 0 {
        writeln!(writer, "{{\"stage\":\"dropped\",\"count\":{count}}}")?;
    }
    writer.flush()
}

fn write_records(
    mut writer: impl Write,
    incoming: Receiver<Record>,
    stopped: Receiver<()>,
    dropped: &AtomicU64,
) -> io::Result<()> {
    loop {
        // Sleep on the stop channel only: producers never have a waiting
        // receiver to wake, so their sends stay free of syscalls.
        let stop = !matches!(
            stopped.recv_timeout(DRAIN_INTERVAL),
            Err(crossbeam_channel::RecvTimeoutError::Timeout)
        );
        // A bounded drain per pass, even if a producer is still exiting.
        for _ in 0..CAPACITY {
            match incoming.try_recv() {
                Ok(record) => write_record(&mut writer, &record)?,
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    return flush_records(&mut writer, dropped);
                }
            }
        }
        flush_records(&mut writer, dropped)?;
        if stop {
            return Ok(());
        }
    }
}

/// Only after GUI/evaluator teardown; never wait for file I/O on a hot thread.
/// Does not initialize an otherwise unused recorder.
pub fn shutdown() {
    if let Some(Some(recorder)) = RECORDER.get() {
        let _ = recorder.stop.try_send(());
        if let Some(writer) = recorder.writer.lock().unwrap().take() {
            let _ = writer.join();
        }
    }
}

#[cfg(test)]
mod tests;
