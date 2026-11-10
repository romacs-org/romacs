use romacs_display_protocol::types::VideoId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender};

use crate::mailbox::PresentationFrameQueue;
use crate::sampling::GpuVideoContext;
use crate::system::VideoWake;
use crate::{
    FrameTiming, VideoColorimetry, VideoCommand, VideoCommandError, VideoCompositorImport,
    VideoFrameFormat, VideoGeometry, VideoInitError, VideoSessionState,
};

/// Decoder output with all information needed to replay the exact frame.
pub(crate) struct DecodedFrame<F> {
    pub(crate) lease: F,
    pub(crate) decode_residency: crate::VideoDecodeResidency,
    pub(crate) timing: FrameTiming,
    pub(crate) geometry: VideoGeometry,
    pub(crate) format: VideoFrameFormat,
    pub(crate) colorimetry: VideoColorimetry,
    /// Identity of the decoder-output representation that produced this
    /// frame. This is deliberately independent of [`crate::PlaybackEpoch`]:
    /// seeks change media position, while output generations change the
    /// native surface contract used by the GPU importer.
    pub(crate) output_generation: DecoderOutputGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct DecoderOutputGeneration(u32);

impl DecoderOutputGeneration {
    pub(crate) const INITIAL: Self = Self(0);

    pub(crate) fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("decoder output generation space exhausted"),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecoderOutputRejection {
    pub(crate) generation: DecoderOutputGeneration,
    pub(crate) format: VideoFrameFormat,
    pub(crate) reason: String,
}

/// Event emitted by a platform decoder adapter.
pub(crate) enum BackendEvent<F> {
    Opened {
        id: VideoId,
        width: u32,
        height: u32,
        initial_state: VideoSessionState,
    },
    /// Codec implementation selected by a backend that can expose it without
    /// leaking a native handle. Backends that cannot prove this leave the
    /// session identity absent rather than guessing from output residency.
    DecoderSelected {
        id: VideoId,
        decoder: crate::VideoDecoderIdentity,
    },
    Frame {
        id: VideoId,
        frame: DecodedFrame<F>,
    },
    /// Frames coalesced from the native bounded presentation queue before the compositor
    /// serviced it. This aggregates instead of turning diagnostic accounting
    /// into an unbounded per-frame control queue.
    FramesReplaced {
        id: VideoId,
        count: u64,
    },
    /// Decoder output caps moved once to a lower, explicitly constrained tier.
    OutputReconfigured {
        id: VideoId,
        generation: DecoderOutputGeneration,
    },
    StateChanged {
        id: VideoId,
        state: VideoSessionState,
    },
    /// Native loop handling consumed one replay permission.
    Looped {
        id: VideoId,
        remaining: crate::LoopMode,
    },
    Ended {
        id: VideoId,
    },
    Failed {
        id: VideoId,
        error: VideoCommandError,
    },
}

impl<F> BackendEvent<F> {
    pub(crate) const fn id(&self) -> VideoId {
        match self {
            Self::Opened { id, .. }
            | Self::DecoderSelected { id, .. }
            | Self::Frame { id, .. }
            | Self::StateChanged { id, .. }
            | Self::Looped { id, .. }
            | Self::Ended { id }
            | Self::Failed { id, .. } => *id,
            Self::FramesReplaced { id, .. } | Self::OutputReconfigured { id, .. } => *id,
        }
    }
}

pub(crate) trait DecoderBackend {
    type Frame;

    fn command(&mut self, command: VideoCommand) -> Result<(), VideoCommandError>;

    /// Advance native playback without blocking and return newly available
    /// events. Pull-based adapters use the presentation target to select the
    /// frame the compositor is about to show; push-based adapters only drain
    /// their bounded event bridge.
    fn service(&mut self, request: &crate::VideoServiceRequest) -> Vec<BackendEvent<Self::Frame>>;

    /// Ask the decoder to replace an output representation that the GPU
    /// importer rejected.  Most backends have no in-place fallback; the
    /// explicit result prevents an expected native-format rejection from
    /// being confused with a terminal decoder error.
    fn reconfigure_after_import_failure(
        &mut self,
        _id: VideoId,
        _rejection: &DecoderOutputRejection,
    ) -> Result<DecoderReconfiguration, String> {
        Ok(DecoderReconfiguration::Unsupported)
    }

    /// Earliest time at which the platform adapter needs another service
    /// pass even when it has not published a cross-thread wake event.
    ///
    /// Most decoders push frame availability and therefore return `None`.
    /// Pull-based native APIs use this seam to participate in the common
    /// render scheduler instead of installing an independent display clock.
    fn next_service_deadline(&self, _now: Instant) -> Option<Instant> {
        None
    }

    /// Snapshot a decoder-owned native surface pool, when this backend owns
    /// one. The default makes the absence of a pool explicit without forcing
    /// fake or push-only decoders to manufacture telemetry.
    fn surface_pool_diagnostics(&self) -> Option<crate::VideoSurfacePoolDiagnostics> {
        None
    }

    /// Reset observation-only counters at an acknowledged benchmark boundary.
    fn begin_measurement_epoch(&mut self) {}
}

pub(crate) struct BackendPublisher<F> {
    events: Sender<BackendEvent<F>>,
    latest_frames: Arc<Mutex<HashMap<VideoId, PublishedFrameQueue<F>>>>,
    measurement_epoch: Arc<AtomicU64>,
    wake: VideoWake,
}

impl<F> Clone for BackendPublisher<F> {
    fn clone(&self) -> Self {
        Self {
            events: self.events.clone(),
            latest_frames: Arc::clone(&self.latest_frames),
            measurement_epoch: Arc::clone(&self.measurement_epoch),
            wake: self.wake.clone(),
        }
    }
}

pub(crate) struct BackendInbox<F> {
    events: Receiver<BackendEvent<F>>,
    latest_frames: Arc<Mutex<HashMap<VideoId, PublishedFrameQueue<F>>>>,
    measurement_epoch: Arc<AtomicU64>,
}

/// Observation generation captured before a decoder begins pulling a frame.
///
/// Unlike playback epochs, this has no media meaning. It makes an acknowledged
/// benchmark boundary reject work that began before the boundary but reached
/// the cross-thread publisher afterward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BackendMeasurementEpoch(u64);

struct PublishedFrameQueue<F> {
    frames: PresentationFrameQueue<DecodedFrame<F>>,
    replaced: u64,
}

pub(crate) fn backend_bridge<F>(wake: VideoWake) -> (BackendPublisher<F>, BackendInbox<F>) {
    let (events, incoming) = crossbeam_channel::unbounded();
    let latest_frames = Arc::new(Mutex::new(HashMap::new()));
    let measurement_epoch = Arc::new(AtomicU64::new(0));
    (
        BackendPublisher {
            events,
            latest_frames: Arc::clone(&latest_frames),
            measurement_epoch: Arc::clone(&measurement_epoch),
            wake,
        },
        BackendInbox {
            events: incoming,
            latest_frames,
            measurement_epoch,
        },
    )
}

impl<F> BackendPublisher<F> {
    pub(crate) fn event(&self, event: BackendEvent<F>) {
        if self.events.send(event).is_ok() {
            self.wake.notify();
        }
    }

    pub(crate) fn measurement_epoch(&self) -> BackendMeasurementEpoch {
        BackendMeasurementEpoch(self.measurement_epoch.load(Ordering::Acquire))
    }

    pub(crate) fn frame(
        &self,
        epoch: BackendMeasurementEpoch,
        id: VideoId,
        frame: DecodedFrame<F>,
    ) {
        let mut latest = lock_unpoisoned(&self.latest_frames);
        if epoch.0 != self.measurement_epoch.load(Ordering::Acquire) {
            return;
        }
        let published = latest.entry(id).or_insert_with(|| PublishedFrameQueue {
            frames: PresentationFrameQueue::default(),
            replaced: 0,
        });
        if published.frames.publish(frame).is_some() {
            published.replaced = published.replaced.saturating_add(1);
        }
        drop(latest);
        self.wake.notify();
    }
}

impl<F> BackendInbox<F> {
    pub(crate) fn drain(&self) -> Vec<BackendEvent<F>> {
        let mut events: Vec<_> = self.events.try_iter().collect();
        let frames = std::mem::take(&mut *lock_unpoisoned(&self.latest_frames));
        for (id, mut published) in frames {
            while let Some(frame) = published.frames.take() {
                events.push(BackendEvent::Frame { id, frame });
            }
            if published.replaced != 0 {
                events.push(BackendEvent::FramesReplaced {
                    id,
                    count: published.replaced,
                });
            }
        }
        events
    }

    pub(crate) fn remove_frame(&self, id: VideoId) {
        lock_unpoisoned(&self.latest_frames).remove(&id);
    }

    /// Advance the accepted observation generation and drop frames published
    /// before the boundary while retaining ordered lifecycle/failure events.
    /// Holding the frame-map lock across both operations closes the race with
    /// a producer publishing an old-generation frame.
    pub(crate) fn begin_measurement_epoch(&self) {
        let mut frames = lock_unpoisoned(&self.latest_frames);
        self.measurement_epoch.fetch_add(1, Ordering::AcqRel);
        frames.clear();
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
#[path = "tests/backend_bridge_test.rs"]
mod bridge_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecoderReconfiguration {
    Applied {
        generation: DecoderOutputGeneration,
    },
    /// The decoder has already moved past the rejected output generation.
    /// The caller should discard this stale frame without counting another
    /// transition or poisoning the live session.
    Superseded,
    Unsupported,
}

pub(crate) struct ImportedFrame<S> {
    pub(crate) sampled: S,
    pub(crate) completed_import: CompletedFrameImport,
}

/// What the importer actually did, including byte volume only where the
/// platform API makes it observable. The enum prevents impossible states such
/// as reporting CPU-upload bytes for a direct external surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletedFrameImport {
    BorrowedNativeSurface,
    CpuUpload { bytes: u64 },
}

impl CompletedFrameImport {
    pub(crate) const fn path(self) -> VideoCompositorImport {
        match self {
            Self::BorrowedNativeSurface => VideoCompositorImport::BorrowedNativeSurface,
            Self::CpuUpload { .. } => VideoCompositorImport::CpuUpload,
        }
    }
}

/// Result of attempting to import one decoded frame into compositor-owned
/// GPU state. Backpressure is an expected bounded-resource condition: the
/// caller drops this already-stale decoded frame and waits for the next one
/// instead of poisoning the playback session.
pub(crate) enum FrameImportOutcome<S> {
    Ready(ImportedFrame<S>),
    Backpressured,
    /// The representation is valid decoder output but unusable by the active
    /// GPU interop stack.  The common system keeps playback alive only when
    /// the decoder confirms that it installed a lower-tier representation.
    ReconfigureDecoder {
        rejection: DecoderOutputRejection,
    },
}

pub(crate) trait FrameImporter<F> {
    type Sampled;

    /// Classify compositor work before performing native import, mapping, copy,
    /// or upload work. The policy boundary uses this to reject forbidden
    /// fallback paths without causing their side effects first.
    fn compositor_import(&self, frame: &DecodedFrame<F>) -> VideoCompositorImport;

    fn import(
        &mut self,
        frame: DecodedFrame<F>,
    ) -> Result<FrameImportOutcome<Self::Sampled>, String>;

    /// Snapshot an importer-owned native surface pool, when present.
    fn surface_pool_diagnostics(&self) -> Option<crate::VideoSurfacePoolDiagnostics> {
        None
    }

    /// Reset observation-only counters without discarding reusable surfaces.
    fn begin_measurement_epoch(&mut self) {}
}

pub(crate) trait Platform {
    const BACKEND: crate::VideoDecodeBackend;
    type Frame;
    type Sampled;
    type Decoder: DecoderBackend<Frame = Self::Frame>;
    type Importer: FrameImporter<Self::Frame, Sampled = Self::Sampled>;
}

pub(crate) trait ProductionPlatform: Platform {
    fn create(
        gpu: GpuVideoContext,
        policy: crate::FrameImportPolicy,
        wake: VideoWake,
    ) -> Result<(Self::Decoder, Self::Importer), VideoInitError>;
}
