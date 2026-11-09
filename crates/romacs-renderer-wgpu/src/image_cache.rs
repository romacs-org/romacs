//! Async image loading and caching for wgpu renderer
//!
//! Provides non-blocking image loading:
//! - Dimension queries for pending-image placeholders
//! - Background decoding in thread pool
//! - GPU texture upload when ready
//! - LRU cache with memory limits

use romacs_display_protocol::image::EncodedBytes;
use romacs_display_protocol::image_diagnostic::{ImageDiagnostic, ImageLoadIdentity};
use romacs_display_protocol::{
    ImageAnimationPolicy, ImageCacheUsage, ImageColorContext,
    ImageFrameIndex, ImageId, ImageIntrinsicExtent, ImageLayoutExtent,
    ImageLoadAttempt, ImageLoadToken, ImageMaskPolicy,
    ImageRasterExtent, ImageRealization, ImageRotation, ImageSequenceId, ImageSizeSpec, ResolvedImageGeometry, RetainedImageSet,
};
#[cfg(test)]
use romacs_display_protocol::{ImageNativeExtent, ImageSequenceRetirement};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::num::{NonZeroU32, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use crate::image_bands::{DecodedBand, RasterBand, RowRange, TextureRows};
use crate::image_sequence::ImageSequenceCache;
use romacs_imaging::decode::*;
pub use romacs_imaging::decode::{
    ImageMetadata, SemanticImageDecoded, SemanticImageDecoder, SemanticImageRequest,
    SemanticImageSequenceReservation, SemanticImageSource,
};

#[cfg(target_os = "linux")]
use crate::external_buffer::DmaBufBuffer;

/// Maximum total cache memory in bytes (64MB)
const MAX_CACHE_MEMORY: usize = 64 * 1024 * 1024;

const MAX_IMAGE_DECODER_THREADS: usize = 4;

/// A deliberately small, non-empty pool of persistent image decoders.
///
/// Image requests are normally sparse and already queue through one shared
/// receiver.  Scaling this pool to every host CPU made each GUI reserve dozens
/// of idle thread stacks before it had seen an image.  GNU image decoding is
/// synchronous (`image.c` even declines WebP's multithreaded option), so four
/// asynchronous workers retain useful parallelism without making GUI startup
/// resources proportional to machine size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ImageDecoderPoolSize(NonZeroUsize);

impl ImageDecoderPoolSize {
    fn detected() -> Self {
        Self::from_available_parallelism(std::thread::available_parallelism().ok())
    }

    fn from_available_parallelism(available: Option<NonZeroUsize>) -> Self {
        let available = available.map_or(MAX_IMAGE_DECODER_THREADS, NonZeroUsize::get);
        Self(
            NonZeroUsize::new(available.min(MAX_IMAGE_DECODER_THREADS))
                .expect("the image decoder pool cap is nonzero"),
        )
    }

    const fn get(self) -> usize {
        self.0.get()
    }
}

/// Image loading state
#[derive(Debug, Clone)]
pub enum ImageState {
    /// Queued for loading
    Pending,
    /// Currently being decoded
    Decoding,
    /// Ready with texture
    Ready,
    /// Logically invalidated but retained by at least one presentation.
    Retiring,
    /// Failed to load
    Failed(String),
}

/// How much of a texture holds pixels, as a prefix of its rows.
///
/// A banded decode fills from the top: its bands are disjoint, in order, and
/// the first starts at row 0, so the rows that exist are always a *prefix* of
/// the texture — never a set, never a hole. That is what lets the draw side ask
/// one number how far down it may draw, and it is why `filled <= total` is an
/// invariant of this type rather than a check at each use: a value claiming more
/// rows than the texture has cannot be built, so "draw a row that was never
/// uploaded" cannot be said.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilledRows {
    filled: u32,
    total: NonZeroU32,
}

impl FilledRows {
    /// Every row of a texture that was written whole.
    #[must_use]
    pub fn complete(raster: ImageRasterExtent) -> Self {
        Self::partial(raster.height(), raster.height())
    }

    /// A texture whose rows are all still to be written.
    #[must_use]
    pub fn empty(raster: ImageRasterExtent) -> Self {
        Self::partial(0, raster.height())
    }

    fn partial(filled: u32, total: u32) -> Self {
        Self {
            filled,
            total: NonZeroU32::new(total).unwrap_or(NonZeroU32::MIN),
        }
    }

    /// The part of a `total`-row texture that holds pixels.
    #[must_use]
    pub const fn filled(&self) -> u32 {
        self.filled
    }

    /// How many rows the texture has.
    #[must_use]
    pub const fn total(&self) -> NonZeroU32 {
        self.total
    }

    /// Whether every row of the texture has been written.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.filled == self.total.get()
    }

    /// How far down the texture a quad may be drawn, as a texture coordinate.
    #[must_use]
    pub fn filled_fraction(&self) -> f32 {
        self.filled as f32 / self.total.get() as f32
    }

    /// This prefix extended by `rows`, which must start exactly where it ends.
    ///
    /// `None` for a band that would leave a hole or rewrite rows that are
    /// already there — a band from a superseded decode, or one that was applied
    /// twice. Holding the extended value is what the write of those rows is
    /// paired with, so a texture cannot come to claim rows nobody wrote.
    #[must_use]
    pub fn extend(self, rows: TextureRows) -> Option<Self> {
        (rows.start() == self.filled && rows.end() <= self.total.get()).then(|| Self {
            filled: rows.end(),
            ..self
        })
    }

    /// Trim a drawn span to the rows that hold pixels.
    ///
    /// `height` is how many pixels of the glyph are drawn and `v0..v1` the same
    /// span as texture rows, so the finish is affine and shortening one by a
    /// fraction shortens the other by the same fraction. `None` when nothing of
    /// the span is uploaded: the quad must be skipped rather than drawn against
    /// rows that were never written.
    #[must_use]
    pub(crate) fn clip_span(self, v0: f32, v1: f32, height: f32) -> Option<(f32, f32)> {
        if v1 <= v0 {
            return None;
        }
        let limit = self.filled_fraction();
        if v0 >= limit {
            return None;
        }
        if v1 <= limit {
            return Some((v1, height));
        }
        Some((limit, height * (limit - v0) / (v1 - v0)))
    }
}

/// Cached image with GPU texture
pub struct CachedImage {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub bind_group: wgpu::BindGroup,
    /// Uploaded texture dimensions in physical device pixels.
    pub raster: ImageRasterExtent,
    /// How much of `raster` has been written.
    ///
    /// Everything, for an image that was decoded whole. A banded decode leaves
    /// this short of `raster` while its rows are still arriving, and every
    /// consumer that draws the image reads it from here — there is no second
    /// copy of "how far has it got" to fall out of step with the texture.
    pub filled: FilledRows,
    pub metadata: Option<ImageMetadata>,
    /// Memory size in bytes
    pub memory_size: usize,
    /// Monotonic access stamp for LRU eviction; refreshed by `get` (a `Cell`
    /// so draw-path lookups stay `&self`).
    last_access: Cell<u64>,
}

/// Decoded image data waiting for GPU upload
struct DecodedImage {
    load: ImageLoadToken,
    geometry: ResolvedImageGeometry,
    data: Vec<u8>, // RGBA
    metadata: ImageMetadata,
}


enum WorkerDecodeOutcome {
    /// One band of a decode that is still running. Intermediate, not terminal:
    /// it consumes no load generation, and the `Ready` that ends the decode
    /// supersedes it.
    Band {
        load: ImageLoadToken,
        decoded: DecodedBand,
    },
    Ready(DecodedImage),
    Failed {
        load: ImageLoadToken,
        diagnostic: ImageDiagnostic,
    },
}

impl WorkerDecodeOutcome {
    fn load(&self) -> ImageLoadToken {
        match self {
            Self::Band { load, .. } => *load,
            Self::Ready(decoded) => decoded.load,
            Self::Failed { load, .. } => *load,
        }
    }
}

/// A renderer image-cache lifecycle event. Callers must handle eviction as
/// well as terminal decode results so external catalogs cannot retain stale
/// residency state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageCacheEvent {
    /// One band of an image whose decode is still running.
    ///
    /// Intermediate by construction: the `Ready` for the same load carries the
    /// whole image, so a consumer that drew this band ends up correct. Only the
    /// rows are published — they are what says how far the decode has come, and
    /// the pixels have already gone into the texture.
    Band {
        load: ImageLoadToken,
        rows: RowRange,
    },
    Ready {
        load: ImageLoadToken,
        metadata: ImageMetadata,
    },
    Failed {
        load: ImageLoadToken,
        error: ImageDiagnostic,
    },
    Evicted {
        image: ImageId,
    },
}

#[derive(Default)]
struct ImageLoadLifecycle {
    next_attempt: u64,
    active: HashMap<ImageId, ImageLoadAttempt>,
}

impl ImageLoadLifecycle {
    fn generated_token(&mut self, image: ImageId) -> ImageLoadToken {
        self.next_attempt = self
            .next_attempt
            .checked_add(1)
            .expect("image load attempt exhausted");
        let attempt =
            ImageLoadAttempt::new(self.next_attempt).expect("checked nonzero image load attempt");
        ImageLoadToken::new(image, attempt)
    }

    #[cfg(test)]
    fn begin_generated(&mut self, image: ImageId) -> ImageLoadToken {
        let load = self.generated_token(image);
        self.begin(load)
    }

    fn begin(&mut self, load: ImageLoadToken) -> ImageLoadToken {
        self.next_attempt = self.next_attempt.max(load.attempt().get());
        self.active.insert(load.image(), load.attempt());
        load
    }

    fn accept(&mut self, load: ImageLoadToken) -> bool {
        if !self.is_current(load) {
            return false;
        }
        self.active.remove(&load.image());
        true
    }

    /// Whether `load` is still the attempt allowed to publish for its image,
    /// without consuming it.
    ///
    /// Intermediate publications — bands — check this and leave the attempt
    /// alone; only the terminal outcome consumes it.
    fn is_current(&self, load: ImageLoadToken) -> bool {
        self.active.get(&load.image()) == Some(&load.attempt())
    }

    /// Accept `outcome` if its attempt is still the current one.
    ///
    /// A band and a terminal outcome differ here: a band is progress from a
    /// decode that is still running, so it is checked and left alone, while
    /// only the terminal outcome consumes the attempt — once, so a superseded
    /// decode can never publish Ready after its replacement has begun.
    fn take_current(&mut self, outcome: WorkerDecodeOutcome) -> Option<WorkerDecodeOutcome> {
        if matches!(outcome, WorkerDecodeOutcome::Band { .. }) {
            return self.is_current(outcome.load()).then_some(outcome);
        }
        self.accept(outcome.load()).then_some(outcome)
    }

    fn free(&mut self, image: ImageId) {
        self.active.remove(&image);
    }

    fn clear(&mut self) {
        self.active.clear();
    }
}

/// Separates logical cache invalidation from presentation-safe GPU release.
#[derive(Default)]
struct ImageResidencyLifecycle {
    retiring: HashSet<ImageId>,
}

impl ImageResidencyLifecycle {
    fn request_retirement(&mut self, image: ImageId) {
        self.retiring.insert(image);
    }

    fn cancel_retirement(&mut self, image: ImageId) {
        self.retiring.remove(&image);
    }

    fn take_releasable(&mut self, retained: &RetainedImageSet) -> Vec<ImageId> {
        let mut releasable = self
            .retiring
            .iter()
            .copied()
            .filter(|image| !retained.contains(*image))
            .collect::<Vec<_>>();
        releasable.sort_unstable();
        for image in &releasable {
            self.retiring.remove(image);
        }
        releasable
    }

    fn clear(&mut self) {
        self.retiring.clear();
    }
}

/// Async image cache
pub struct ImageCache {
    /// Budget accounting events since the last drain (texture create/free).
    accounting: Vec<crate::media_budget::MediaAccounting>,
    /// Next image ID
    next_id: AtomicU32,
    /// Cached textures: id -> CachedImage
    textures: HashMap<ImageId, CachedImage>,
    /// Image states: id -> state
    states: HashMap<ImageId, ImageState>,
    /// Identifies the one decode request currently allowed to publish for each ID.
    loads: ImageLoadLifecycle,
    residency: ImageResidencyLifecycle,
    /// Assets pinned by accepted or queued render presentations.
    retained_images: RetainedImageSet,
    /// Pending dimensions (before texture is ready)
    pending_dimensions: HashMap<ImageId, ImageLayoutExtent>,
    /// Channel to receive decoded images
    decoded_rx: mpsc::Receiver<WorkerDecodeOutcome>,
    /// Channel to send decode requests
    decode_tx: mpsc::Sender<DecodeRequest>,
    /// CPU decoder/compositor state shared across all frames of an animation.
    sequence_cache: Arc<ImageSequenceCache>,
    /// A test's hold on banded decodes (`band_hold`); elsewhere only the
    /// decoder threads carry the (empty) handle.
    #[cfg(any(test, feature = "test-gpu"))]
    band_hold: BandHoldHandle,
    /// Bind group layout for image textures
    bind_group_layout: wgpu::BindGroupLayout,
    /// Sampler for image textures
    sampler: wgpu::Sampler,
    /// Total cached memory
    total_memory: usize,
    /// Monotonic clock stamping `CachedImage::last_access` (LRU order).
    access_clock: Cell<u64>,
}

/// Pick the least-recently-used entry: the id with the smallest access stamp
/// (ties broken by smaller id for determinism).
fn lru_unpresented_victim(
    entries: impl Iterator<Item = (ImageId, u64)>,
    retained: &RetainedImageSet,
) -> Option<ImageId> {
    entries
        .filter(|(image, _)| !retained.contains(*image))
        .min_by_key(|&(id, stamp)| (stamp, id))
        .map(|(id, _)| id)
}

/// Request to decode an image
struct DecodeRequest {
    load: ImageLoadToken,
    source: ImageSource,
    size: ImageSizeSpec,
    rotation: ImageRotation,
    /// Semantic and device geometry resolved by evaluator/layout.
    realization: ImageRealization,
    /// Resolved face colors used by face-sensitive formats and cache identity.
    colors: ImageColorContext,
    mask: ImageMaskPolicy,
    animation: ImageAnimationPolicy,
    frame: ImageFrameIndex,
    /// What GNU calls this source when the decode fails.  It travels with the
    /// job because the failure is worded by the loader's own rules (`Not a PNG
    /// file: `%s'`), and the job is the last place that knows both the bytes
    /// and what they were asked to be.
    identity: ImageLoadIdentity,
}

/// What a failed decode of SOURCE names, captured before the decode
/// consumes it.
///
/// Both captures are cheap references to what the job already owns — a
/// path clone and a handle to the same bytes — because this runs on the
/// decode path for every job, not only the failing ones.
fn failure_source_of(source: &ImageSource) -> DecodeFailureSource {
    match source {
        ImageSource::File { path, .. } => DecodeFailureSource::File { path: path.clone() },
        ImageSource::Data { data, .. } => DecodeFailureSource::Bytes { data: data.clone() },
        ImageSource::RawArgb32 { .. } => DecodeFailureSource::Raw,
        #[cfg(test)]
        ImageSource::Panic => DecodeFailureSource::Raw,
    }
}

/// Image source
enum ImageSource {
    File {
        path: String,
        sequence: ImageSequenceId,
    },
    Data {
        data: EncodedBytes,
        resources: crate::svg::SvgResourceContext,
        sequence: ImageSequenceId,
    },
    #[cfg(test)]
    Panic,
    /// Raw ARGB32 pixel data (A,R,G,B byte order, 4 bytes per pixel)
    RawArgb32 {
        data: Vec<u8>,
        width: u32,
        height: u32,
        stride: u32,
    },
}

/// A test's hold on banded decodes: having published a band of the held
/// image that reaches the held fraction of its raster's rows, the decoder
/// waits until the hold is released.  A test that needs a frame drawn at
/// one point of a fill gets that point whatever the scheduler does, rather
/// than racing a decoder that may finish before it looks.  Only test
/// builds have it; elsewhere the handle is nothing and the hook does
/// nothing.
#[cfg(any(test, feature = "test-gpu"))]
#[derive(Default)]
pub struct BandHold {
    held: Mutex<Option<(ImageId, f32)>>,
    released: std::sync::Condvar,
}

#[cfg(any(test, feature = "test-gpu"))]
impl BandHold {
    /// Hold IMAGE's decode at its first band reaching FRACTION of its rows.
    pub fn hold(&self, image: ImageId, fraction: f32) {
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) = Some((image, fraction));
    }

    /// Let a held decode go on, and every later band with it.
    pub fn release(&self) {
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.released.notify_all();
    }

    fn after_band(&self, image: ImageId, reached: f32) {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        while held.is_some_and(|(id, fraction)| id == image && reached >= fraction) {
            held = self.released.wait(held).unwrap_or_else(|e| e.into_inner());
        }
    }
}

#[cfg(any(test, feature = "test-gpu"))]
type BandHoldHandle = Arc<BandHold>;
#[cfg(not(any(test, feature = "test-gpu")))]
type BandHoldHandle = ();

#[cfg(any(test, feature = "test-gpu"))]
fn hold_after_band(hold: &BandHoldHandle, image: ImageId, reached: f32) {
    hold.after_band(image, reached);
}
#[cfg(not(any(test, feature = "test-gpu")))]
fn hold_after_band(_: &BandHoldHandle, _: ImageId, _: f32) {}

impl ImageCache {
    /// The hold tests put on this cache's banded decodes ([`BandHold`]).
    #[cfg(any(test, feature = "test-gpu"))]
    pub fn band_hold(&self) -> &Arc<BandHold> {
        &self.band_hold
    }

    /// Create a new image cache
    pub fn new(device: &wgpu::Device) -> Self {
        // Create bind group layout for image textures
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Image Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Image Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        // Create channels for async decoding
        let (decode_tx, decode_rx) = mpsc::channel::<DecodeRequest>();
        let (decoded_tx, decoded_rx) = mpsc::channel::<WorkerDecodeOutcome>();
        let sequence_cache = Arc::new(ImageSequenceCache::new());
        let band_hold = BandHoldHandle::default();

        // Wrap receiver in Arc<Mutex> for sharing across threads
        let decode_rx = Arc::new(Mutex::new(decode_rx));

        let pool_size = ImageDecoderPoolSize::detected();
        tracing::info!("Starting {} image decoder threads", pool_size.get());
        for i in 0..pool_size.get() {
            let rx = Arc::clone(&decode_rx);
            let tx = decoded_tx.clone();
            let sequence_cache = Arc::clone(&sequence_cache);
            let band_hold = band_hold.clone();
            thread::spawn(move || {
                Self::decoder_thread_pooled(i, rx, tx, sequence_cache, band_hold);
            });
        }

        Self {
            next_id: AtomicU32::new(1),
            textures: HashMap::new(),
            states: HashMap::new(),
            loads: ImageLoadLifecycle::default(),
            residency: ImageResidencyLifecycle::default(),
            retained_images: RetainedImageSet::default(),
            pending_dimensions: HashMap::new(),
            decoded_rx,
            decode_tx,
            sequence_cache,
            #[cfg(any(test, feature = "test-gpu"))]
            band_hold,
            bind_group_layout,
            accounting: Vec::new(),
            sampler,
            total_memory: 0,
            access_clock: Cell::new(0),
        }
    }

    /// Advance the access clock and return a fresh stamp.
    fn next_access_stamp(&self) -> u64 {
        let stamp = self.access_clock.get() + 1;
        self.access_clock.set(stamp);
        stamp
    }

    fn begin_load(&mut self, load: ImageLoadToken) -> ImageLoadToken {
        let image = load.image();
        self.residency.cancel_retirement(image);
        if let Some(cached) = self.textures.remove(&image) {
            self.total_memory -= cached.memory_size;
            self.accounting
                .push(crate::media_budget::MediaAccounting::Freed {
                    media_type: crate::media_budget::MediaType::Image,
                    id: image.get(),
                });
        }
        self.pending_dimensions.remove(&image);
        self.loads.begin(load)
    }

    fn allocate_image_id(&self) -> ImageId {
        let raw = self
            .next_id
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_add(1)
            })
            .expect("image identity space exhausted");
        ImageId::new(raw)
    }

    /// Background decoder thread (pooled version)
    fn decoder_thread_pooled(
        thread_id: usize,
        rx: Arc<Mutex<mpsc::Receiver<DecodeRequest>>>,
        tx: mpsc::Sender<WorkerDecodeOutcome>,
        sequence_cache: Arc<ImageSequenceCache>,
        band_hold: BandHoldHandle,
    ) {
        tracing::debug!("Decoder thread {} started", thread_id);
        loop {
            // Lock, receive, unlock immediately to allow other threads to grab work
            let request = {
                let guard = rx.lock().unwrap_or_else(|e| e.into_inner());
                guard.recv()
            };

            match request {
                Ok(request) => {
                    tracing::debug!(
                        "Thread {} decoding image {}",
                        thread_id,
                        request.load.image()
                    );
                    let DecodeRequest {
                        load,
                        source,
                        size,
                        rotation,
                        realization,
                        colors,
                        mask,
                        animation,
                        frame,
                        identity,
                    } = request;
                    // A banded decode reports each band as it lands, which is
                    // the whole point of decoding that way: the display side
                    // can act on a band long before the decode finishes, and
                    // the render thread can write it into the texture. The sink
                    // is a borrow of this closure rather than a returned
                    // collection — a band is worth having before the next one
                    // exists — and it is optional, because a caller that only
                    // wants the image decodes the same pixels without any of
                    // this.
                    let mut publish_band = |decoded: DecodedBand| {
                        let placement = decoded.placed().placement();
                        let reached = placement.rows().end() as f32 / placement.raster().dimensions().1.max(1) as f32;
                        let _ = tx.send(WorkerDecodeOutcome::Band { load, decoded });
                        hold_after_band(&band_hold, load.image(), reached);
                        std::ops::ControlFlow::Continue(())
                    };
                    let sink: Option<romacs_imaging::decode::BandSink<'_>> = Some(&mut publish_band);
                    // The chain below answers `Option` and each arm collapses
                    // its own reason, so what the failure is worded *about* is
                    // captured before the bytes are handed over: the source
                    // itself, and whether reading it worked at all.
                    let failure_source = failure_source_of(&source);
                    let result = catch_unwind(AssertUnwindSafe(|| match source {
                        #[cfg(test)]
                        ImageSource::Panic => panic!("injected decoder panic"),
                        ImageSource::File { path, sequence } => Decode::decode_file(
                            &path,
                            size,
                            rotation,
                            colors,
                            realization,
                            mask,
                            animation,
                            frame,
                            &sequence_cache,
                            sequence,
                            sink,
                        ),
                        ImageSource::Data {
                            data,
                            resources,
                            sequence,
                        } => Decode::decode_data(
                            data,
                            size,
                            rotation,
                            colors,
                            realization,
                            mask,
                            animation,
                            frame,
                            resources,
                            &sequence_cache,
                            sequence,
                            sink,
                        ),
                        ImageSource::RawArgb32 {
                            data,
                            width,
                            height,
                            stride,
                        } => Decode::convert_argb32_to_rgba(&data, width, height, stride)
                            .map(NativePixels::from_raster_tuple)
                            .and_then(|pixels| {
                                pixels.realize_bitmap(size, rotation, realization, mask)
                            }),
                    }));

                    let outcome = match result {
                        Ok(Some(pixels)) => {
                            WorkerDecodeOutcome::Ready(Self::decoded_image(load, pixels))
                        }
                        Ok(None) => WorkerDecodeOutcome::Failed {
                            load,
                            diagnostic: failure_source.diagnostic(&identity),
                        },
                        Err(_) => {
                            tracing::warn!(
                                "Decoder thread {} recovered from a panic while decoding image {}",
                                thread_id,
                                load.image()
                            );
                            WorkerDecodeOutcome::Failed {
                                load,
                                diagnostic: failure_source.diagnostic(&identity),
                            }
                        }
                    };
                    let _ = tx.send(outcome);
                }
                Err(_) => {
                    // Channel closed, exit thread
                    tracing::debug!("Decoder thread {} exiting", thread_id);
                    break;
                }
            }
        }
    }

    #[cfg(test)]
    fn decode_data_with_metadata(
        data: &[u8],
        size: ImageSizeSpec,
        rotation: ImageRotation,
        fg_bg: (u32, u32),
    ) -> Option<DecodedImage> {
        Self::decode_data_with_metadata_at_scale(data, size, rotation, fg_bg, 1.0)
    }

    #[cfg(test)]
    fn decode_data_with_metadata_for_frame(
        data: &[u8],
        frame: ImageFrameIndex,
    ) -> Option<DecodedImage> {
        let pixels = Decode::decode_data(
            EncodedBytes::copy_of(data),
            ImageSizeSpec::default(),
            ImageRotation::None,
            ImageColorContext::default(),
            ImageRealization::default(),
            ImageMaskPolicy::Preserve,
            ImageAnimationPolicy::disabled(),
            frame,
            crate::svg::SvgResourceContext::Isolated,
            &ImageSequenceCache::new(),
            ImageSequenceId::new(1).expect("non-zero test sequence"),
            None,
        )?;
        Some(Self::decoded_image(
            ImageLoadToken::new(
                ImageId::new(0),
                ImageLoadAttempt::new(1).expect("test load attempt"),
            ),
            pixels,
        ))
    }

    #[cfg(test)]
    fn decode_data_with_metadata_at_scale(
        data: &[u8],
        size: ImageSizeSpec,
        rotation: ImageRotation,
        fg_bg: (u32, u32),
        raster_scale: f32,
    ) -> Option<DecodedImage> {
        Self::decode_data_with_metadata_at_realization(
            data,
            size,
            rotation,
            fg_bg,
            1.0,
            raster_scale,
        )
    }

    #[cfg(test)]
    fn decode_data_with_metadata_at_realization(
        data: &[u8],
        size: ImageSizeSpec,
        rotation: ImageRotation,
        fg_bg: (u32, u32),
        layout_scale: f32,
        device_scale: f32,
    ) -> Option<DecodedImage> {
        // Convenience path: layout already equals image-pixel space.
        Self::decode_data_with_metadata_at_full_realization(
            data,
            size,
            rotation,
            fg_bg,
            ImageRealization::with_device_scale(layout_scale, device_scale),
        )
    }

    #[cfg(test)]
    fn decode_data_with_metadata_at_full_realization(
        data: &[u8],
        size: ImageSizeSpec,
        rotation: ImageRotation,
        fg_bg: (u32, u32),
        realization: ImageRealization,
    ) -> Option<DecodedImage> {
        let pixels = Decode::decode_data(
            EncodedBytes::copy_of(data),
            size,
            rotation,
            ImageColorContext::from_pixels(fg_bg.0, fg_bg.1),
            realization,
            ImageMaskPolicy::Preserve,
            ImageAnimationPolicy::disabled(),
            ImageFrameIndex::default(),
            crate::svg::SvgResourceContext::Isolated,
            &ImageSequenceCache::new(),
            ImageSequenceId::new(1).expect("non-zero test sequence"),
            None,
        )?;
        Some(Self::decoded_image(
            ImageLoadToken::new(
                ImageId::new(0),
                ImageLoadAttempt::new(1).expect("test load attempt"),
            ),
            pixels,
        ))
    }

    fn decoded_image(load: ImageLoadToken, pixels: DecodedPixels) -> DecodedImage {
        let metadata =
            Decode::metadata_from_rgba(pixels.geometry, &pixels.rgba, pixels.mask, pixels.embedded);
        DecodedImage {
            load,
            geometry: pixels.geometry,
            data: pixels.rgba,
            metadata,
        }
    }

    /// Get bind group layout
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    /// Get sampler (for sharing with video cache)
    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    fn query_file_intrinsic_extent(path: &str) -> Option<ImageIntrinsicExtent> {
        if let Some(extent) = crate::image_probe::probe_intrinsic_extent(
            crate::image_probe::ImageProbeSource::File(path),
        ) {
            return Some(extent);
        }

        // Fallback: try SVG.
        let data = std::fs::read(path).ok()?;
        crate::svg::query_intrinsic_extent(&data)
    }

    #[cfg(test)]
    /// Query image data dimensions.
    ///
    /// Raster formats read only their header; SVG requires document parsing.
    pub(crate) fn query_data_dimensions(data: &[u8]) -> Option<ImageNativeExtent> {
        Self::query_data_intrinsic_extent(data).map(Self::unscaled_dimensions)
    }

    fn query_data_intrinsic_extent(data: &[u8]) -> Option<ImageIntrinsicExtent> {
        crate::image_probe::probe_intrinsic_extent(crate::image_probe::ImageProbeSource::Data(data))
            .or_else(|| crate::svg::query_intrinsic_extent(data))
    }

    #[cfg(test)]
    /// Preserve the pixel-query contract: return a bounding integer extent.
    /// Pending-image sizing must instead retain the fractional source extent.
    fn unscaled_dimensions(intrinsic: ImageIntrinsicExtent) -> ImageNativeExtent {
        let (width, height) = intrinsic.dimensions();
        ImageNativeExtent::new(width.ceil() as u32, height.ceil() as u32)
    }

    /// Load image from file (async)
    /// Returns image ID immediately, texture loads in background
    pub fn load_file(
        &mut self,
        path: &str,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        colors: ImageColorContext,
        raster_scale: f32,
    ) -> ImageId {
        let image = self.allocate_image_id();
        let load = self.loads.generated_token(image);
        self.load_file_with_id(
            load,
            path,
            size,
            rotation,
            ImageRealization::with_device_scale(1.0, raster_scale),
            colors,
            ImageMaskPolicy::default(),
            ImageAnimationPolicy::disabled(),
            ImageFrameIndex::default(),
            ImageSequenceId::new(u64::from(image.get()))
                .expect("allocated image identity is non-zero"),
            ImageLoadIdentity::unspecified(),
        );
        image
    }

    /// Load image from data with a pre-allocated ID (for threaded mode)
    ///
    /// The bytes arrive as the handle the request already holds, and are moved
    /// into the decode queue: the caller's buffer is the decode's buffer, which
    /// is what the catalog's own key to the same request points at too.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn load_data_with_id(
        &mut self,
        load: ImageLoadToken,
        data: EncodedBytes,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        realization: ImageRealization,
        colors: ImageColorContext,
        mask: ImageMaskPolicy,
        animation: ImageAnimationPolicy,
        frame: ImageFrameIndex,
        sequence: ImageSequenceId,
        resources: crate::svg::SvgResourceContext,
        identity: ImageLoadIdentity,
    ) {
        let load = self.begin_load(load);
        let image = load.image();
        // Query dimensions for the pending-image placeholder.
        if let Some(dims) = Self::query_data_intrinsic_extent(&data) {
            self.pending_dimensions.insert(
                image,
                realization.resolve_geometry(size, dims, rotation).layout(),
            );
        }

        // Queue for async decode
        self.states.insert(image, ImageState::Pending);
        let _ = self.decode_tx.send(DecodeRequest {
            load,
            source: ImageSource::Data {
                data,
                resources,
                sequence,
            },
            size,
            rotation,
            realization,
            colors,
            mask,
            animation,
            frame,
            identity,
        });
    }

    /// Load image from file with a pre-allocated ID (for threaded mode)
    /// This allows the calling code to allocate the ID before sending a command.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn load_file_with_id(
        &mut self,
        load: ImageLoadToken,
        path: &str,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        realization: ImageRealization,
        colors: ImageColorContext,
        mask: ImageMaskPolicy,
        animation: ImageAnimationPolicy,
        frame: ImageFrameIndex,
        sequence: ImageSequenceId,
        identity: ImageLoadIdentity,
    ) {
        let load = self.begin_load(load);
        let image = load.image();
        // Query dimensions for the pending-image placeholder.
        if let Some(dims) = Self::query_file_intrinsic_extent(path) {
            self.pending_dimensions.insert(
                image,
                realization.resolve_geometry(size, dims, rotation).layout(),
            );
        }

        // Queue for async decode
        self.states.insert(image, ImageState::Pending);
        let _ = self.decode_tx.send(DecodeRequest {
            load,
            source: ImageSource::File {
                path: path.to_string(),
                sequence,
            },
            size,
            rotation,
            realization,
            colors,
            mask,
            animation,
            frame,
            identity,
        });
    }

    pub(crate) fn memory_usage(&self) -> ImageCacheUsage {
        ImageCacheUsage::new(
            u64::try_from(self.total_memory).unwrap_or(u64::MAX),
            u64::try_from(self.sequence_cache.resident_bytes()).unwrap_or(u64::MAX),
        )
    }

    /// Load image from raw ARGB32 pixel data with a pre-allocated ID (for threaded mode)
    pub(crate) fn load_raw_argb32_with_id(
        &mut self,
        load: ImageLoadToken,
        data: &[u8],
        width: u32,
        height: u32,
        stride: u32,
    ) {
        let load = self.begin_load(load);
        let image = load.image();
        self.pending_dimensions
            .insert(image, ImageLayoutExtent::new(width, height));
        self.states.insert(image, ImageState::Pending);
        let _ = self.decode_tx.send(DecodeRequest {
            rotation: ImageRotation::None,
            load,
            source: ImageSource::RawArgb32 {
                data: data.to_vec(),
                width,
                height,
                stride,
            },
            size: ImageSizeSpec::default(),
            realization: ImageRealization::default(),
            colors: ImageColorContext::default(),
            mask: ImageMaskPolicy::default(),
            animation: ImageAnimationPolicy::disabled(),
            frame: ImageFrameIndex::default(),
            identity: ImageLoadIdentity::unspecified(),
        });
    }

    /// Import image from DMA-BUF (zero-copy if supported)
    #[cfg(target_os = "linux")]
    pub fn import_dmabuf(
        &mut self,
        dmabuf: DmaBufBuffer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> ImageId {
        let image = self.allocate_image_id();
        let (width, height) = dmabuf.dimensions();

        // Try zero-copy import
        if let Some(texture) = dmabuf.to_wgpu_texture(device, queue) {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("DMA-BUF Image Bind Group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

            let memory_size = (width * height * 4) as usize;
            self.total_memory += memory_size;
            self.accounting
                .push(crate::media_budget::MediaAccounting::Registered {
                    media_type: crate::media_budget::MediaType::Image,
                    id: image.get(),
                    size_bytes: memory_size,
                });

            self.textures.insert(
                image,
                CachedImage {
                    texture,
                    view,
                    bind_group,
                    raster: ImageRasterExtent::new(width, height),
                    // The imported buffer is the whole image already.
                    filled: FilledRows::complete(ImageRasterExtent::new(width, height)),
                    metadata: None,
                    memory_size,
                    last_access: Cell::new(self.next_access_stamp()),
                },
            );
            self.states.insert(image, ImageState::Ready);

            tracing::info!(
                "Imported DMA-BUF image {} ({}x{}) zero-copy",
                image,
                width,
                height
            );
        } else {
            self.states
                .insert(image, ImageState::Failed("DMA-BUF import failed".into()));
            tracing::warn!("DMA-BUF import failed for image {}", image);
        }

        image
    }

    /// Process pending decoded images (call each frame)
    pub(crate) fn process_pending(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Vec<ImageCacheEvent> {
        let mut events = Vec::new();
        // Drain decoded images from channel
        while let Ok(outcome) = self.decoded_rx.try_recv() {
            let Some(outcome) = self.loads.take_current(outcome) else {
                continue;
            };
            match outcome {
                WorkerDecodeOutcome::Band { load, decoded } => {
                    let (source, placed) = decoded.into_parts();
                    tracing::debug!(
                        "Image {} decoded rows {}..{} into texture rows {}..{}",
                        load.image(),
                        source.start(),
                        source.end(),
                        placed.placement().rows().start(),
                        placed.placement().rows().end(),
                    );
                    self.upload_band(device, queue, load, &placed);
                    events.push(ImageCacheEvent::Band { load, rows: source });
                }
                WorkerDecodeOutcome::Ready(decoded) => {
                    events.push(ImageCacheEvent::Ready {
                        load: decoded.load,
                        metadata: decoded.metadata.clone(),
                    });
                    self.upload_texture(device, queue, decoded);
                }
                WorkerDecodeOutcome::Failed { load, diagnostic } => {
                    // A failed decode keeps no texture: a band may already have
                    // created one, and it holds rows of an image that will never
                    // arrive. Leaving it would draw them forever.
                    self.release(load.image());
                    self.states
                        .insert(load.image(), ImageState::Failed(diagnostic.message()));
                    self.pending_dimensions.remove(&load.image());
                    events.push(ImageCacheEvent::Failed {
                        load,
                        error: diagnostic,
                    });
                }
            }
        }

        // Evict if over memory limit
        self.evict_if_needed(&mut events);
        events
    }

    /// The texture, view and bind group every image in this cache is drawn
    /// through.
    ///
    /// One constructor for the whole cache: a banded decode creates its texture
    /// here and the finished upload gets the same allocation, so the two cannot
    /// disagree about format, usage or size.
    fn create_image_texture(
        &self,
        device: &wgpu::Device,
        raster: ImageRasterExtent,
    ) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
        let (width, height) = raster.dimensions();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Image Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            // `COPY_SRC` costs nothing to carry and is what makes these
            // textures' contents observable: a band-filled texture ending
            // byte-for-byte as the whole-image path's is a claim only a readback
            // can settle.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Image Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        (texture, view, bind_group)
    }

    /// Write one band's pixels into the image's texture.
    ///
    /// The first band creates the texture, and the image is drawable from that
    /// moment on: the quad is step 1's and does not move, the texture only grows
    /// downward, and what has not arrived yet is simply not drawn
    /// (`FilledRows::clip_span`). Every later band appends to it.
    ///
    /// A band that does not continue the prefix is dropped rather than written.
    /// Rows written out of order, or twice, would leave the texture holding
    /// pixels from two different scalings and nothing downstream could tell; a
    /// band naming another raster belongs to other pixels entirely. Both are
    /// states this refuses to enter, and the whole-image upload at the end is
    /// what makes the refusal invisible.
    ///
    /// Writing in place, under a compositor that samples this texture, is a
    /// deliberate choice over staging into a second texture and swapping: a
    /// shadow would double the peak allocation for the largest texture this
    /// cache holds (64 MiB at the 4096 limit) while the decode is already
    /// holding the whole native image, and the swap would have to rebind a
    /// texture the frames already presented are sampling. The state a partial
    /// write describes is a prefix, which [`FilledRows`] expresses exactly; the
    /// state a swap would need — two textures at two fill levels, and a rule for
    /// when one may become the other — is the one that would have needed
    /// policing.
    fn upload_band(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        load: ImageLoadToken,
        placed: &RasterBand,
    ) {
        let image = load.image();
        let placement = placed.placement();
        let raster = placement.raster();
        if !self.textures.contains_key(&image) {
            let (texture, view, bind_group) = self.create_image_texture(device, raster);
            let memory_size = (raster.width() * raster.height() * 4) as usize;
            self.total_memory += memory_size;
            self.accounting
                .push(crate::media_budget::MediaAccounting::Registered {
                    media_type: crate::media_budget::MediaType::Image,
                    id: image.get(),
                    size_bytes: memory_size,
                });
            self.textures.insert(
                image,
                CachedImage {
                    texture,
                    view,
                    bind_group,
                    raster,
                    filled: FilledRows::empty(raster),
                    metadata: None,
                    memory_size,
                    last_access: Cell::new(self.next_access_stamp()),
                },
            );
        }
        let Some(cached) = self.textures.get_mut(&image) else {
            return;
        };
        if cached.raster != raster {
            tracing::warn!(
                "Image {} band for a {}x{} raster into a {}x{} texture",
                image,
                raster.width(),
                raster.height(),
                cached.raster.width(),
                cached.raster.height()
            );
            return;
        }
        let rows = placement.rows();
        let Some(filled) = cached.filled.extend(rows) else {
            tracing::debug!(
                "Image {} band for rows {}..{} does not continue {} written",
                image,
                rows.start(),
                rows.end(),
                cached.filled.filled()
            );
            return;
        };
        let (raster_width, raster_height) = raster.dimensions();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &cached.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: rows.start(),
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            placed.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(raster_width * 4),
                rows_per_image: Some(rows.len().get()),
            },
            wgpu::Extent3d {
                width: raster_width,
                height: rows.len().get(),
                depth_or_array_layers: 1,
            },
        );
        cached.filled = filled;
        tracing::debug!(
            "Image {} filled rows {}..{} of {} (raster {}x{})",
            image,
            rows.start(),
            rows.end(),
            raster_height,
            raster_width,
            raster_height
        );
    }

    /// Establish GPU residency from the complete CPU result, never a header or
    /// band. Synchronous upload consumes the renderer load epoch exactly once.
    pub fn upload_semantic_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        load: ImageLoadToken,
        decoded: &SemanticImageDecoded,
    ) -> ImageCacheEvent {
        self.begin_load(load);
        assert!(self.loads.accept(load));
        self.upload_image_data(
            device,
            queue,
            load,
            decoded.geometry,
            &decoded.data,
            decoded.metadata.clone(),
        );
        ImageCacheEvent::Ready {
            load,
            metadata: decoded.metadata.clone(),
        }
    }

    /// Upload decoded image to GPU texture
    fn upload_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        decoded: DecodedImage,
    ) {
        self.upload_image_data(
            device,
            queue,
            decoded.load,
            decoded.geometry,
            &decoded.data,
            decoded.metadata,
        );
    }

    fn upload_image_data(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        load: ImageLoadToken,
        geometry: ResolvedImageGeometry,
        data: &[u8],
        metadata: ImageMetadata,
    ) {
        let image = load.image();
        let raster = geometry.raster();
        let (raster_width, raster_height) = raster.dimensions();
        // A banded decode may have already built this image's texture, at
        // exactly this raster (`realized_geometry` is the one place either side
        // resolves it). Keep it: the write below covers every texel of it, so
        // the finished image is the whole-image path's bytes — written by the
        // same call, to the same shape of texture — and the bind group the
        // presented frames already sample never changes under them. A texture of
        // any other raster is not this image's and is released rather than
        // written into.
        let existing = self.textures.remove(&image);
        let (texture, view, bind_group, memory_size) = match existing {
            Some(cached) if cached.raster == raster => (
                cached.texture,
                cached.view,
                cached.bind_group,
                cached.memory_size,
            ),
            other => {
                if let Some(cached) = other {
                    self.total_memory -= cached.memory_size;
                    self.accounting
                        .push(crate::media_budget::MediaAccounting::Freed {
                            media_type: crate::media_budget::MediaType::Image,
                            id: image.get(),
                        });
                }
                let (texture, view, bind_group) = self.create_image_texture(device, raster);
                let memory_size = (raster_width * raster_height * 4) as usize;
                self.total_memory += memory_size;
                self.accounting
                    .push(crate::media_budget::MediaAccounting::Registered {
                        media_type: crate::media_budget::MediaType::Image,
                        id: image.get(),
                        size_bytes: memory_size,
                    });
                (texture, view, bind_group, memory_size)
            }
        };

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(raster_width * 4),
                rows_per_image: Some(raster_height),
            },
            wgpu::Extent3d {
                width: raster_width,
                height: raster_height,
                depth_or_array_layers: 1,
            },
        );

        let layout = metadata.layout;

        self.textures.insert(
            image,
            CachedImage {
                texture,
                view,
                bind_group,
                raster,
                // The write above covered the whole texture, whether or not a
                // banded decode had got part of the way through it.
                filled: FilledRows::complete(raster),
                metadata: Some(metadata),
                memory_size,
                last_access: Cell::new(self.next_access_stamp()),
            },
        );

        self.states.insert(image, ImageState::Ready);
        self.pending_dimensions.remove(&image);

        tracing::debug!(
            "Uploaded image {} (layout {}x{}, raster {}x{}, {}KB)",
            image,
            layout.width(),
            layout.height(),
            raster_width,
            raster_height,
            memory_size / 1024
        );
    }

    /// Evict least-recently-used textures until under the memory limit.
    fn evict_if_needed(&mut self, events: &mut Vec<ImageCacheEvent>) {
        while self.total_memory > MAX_CACHE_MEMORY && !self.textures.is_empty() {
            let victim = lru_unpresented_victim(
                self.textures
                    .iter()
                    // A half-filled texture belongs to a decode that is still
                    // running: the load owns it, and taking it away would leave
                    // the rows that follow with nowhere to go — the next band
                    // continues a prefix that no longer exists. It becomes
                    // evictable the moment the decode ends, one way or another.
                    .filter(|(_, cached)| cached.filled.is_complete())
                    .map(|(&id, cached)| (id, cached.last_access.get())),
                &self.retained_images,
            );
            if let Some(id) = victim
                && let Some(cached) = self.textures.remove(&id)
            {
                self.total_memory -= cached.memory_size;
                self.states.remove(&id);
                self.accounting
                    .push(crate::media_budget::MediaAccounting::Freed {
                        media_type: crate::media_budget::MediaType::Image,
                        id: id.get(),
                    });
                events.push(ImageCacheEvent::Evicted { image: id });
                tracing::debug!(
                    "Evicted image {} to free {}KB",
                    id,
                    cached.memory_size / 1024
                );
            } else {
                // The cache may temporarily exceed its budget while every
                // candidate is owned by a retained presentation.
                break;
            }
        }
    }

    /// Get cached image if ready. Refreshes the entry's LRU access stamp.
    pub fn get(&self, image: ImageId) -> Option<&CachedImage> {
        let cached = self.textures.get(&image)?;
        cached.last_access.set(self.next_access_stamp());
        Some(cached)
    }

    /// Get image dimensions (pending or loaded)
    pub(crate) fn get_dimensions(&self, image: ImageId) -> Option<ImageLayoutExtent> {
        // A texture with metadata is a finished decode, and its layout is the
        // answer. A texture without one is the prefix a banded decode has
        // uploaded — its rows say nothing about how big the image is — so the
        // layout step 1 resolved from the header is the answer until then.
        if let Some(layout) = self
            .textures
            .get(&image)
            .and_then(|cached| cached.metadata.as_ref())
            .map(|metadata| metadata.layout)
        {
            return Some(layout);
        }
        if let Some(layout) = self.pending_dimensions.get(&image) {
            return Some(*layout);
        }
        self.textures
            .get(&image)
            .map(|cached| ImageLayoutExtent::new(cached.raster.width(), cached.raster.height()))
    }

    /// Get image state
    pub fn get_state(&self, image: ImageId) -> Option<&ImageState> {
        self.states.get(&image)
    }

    /// Check if image is ready
    pub(crate) fn is_ready(&self, image: ImageId) -> bool {
        matches!(self.states.get(&image), Some(ImageState::Ready))
    }

    /// Whether async decode work still needs the render thread to poll its result channel.
    pub(crate) fn has_pending(&self) -> bool {
        self.states
            .values()
            .any(|state| matches!(state, ImageState::Pending | ImageState::Decoding))
    }

    /// Logically retire an image. Its texture remains drawable until no
    /// accepted or queued presentation references the identity.
    pub fn retire(&mut self, image: ImageId) {
        self.loads.free(image);
        self.pending_dimensions.remove(&image);
        if self.textures.contains_key(&image) {
            self.residency.request_retirement(image);
            self.states.insert(image, ImageState::Retiring);
        } else {
            self.states.remove(&image);
        }
    }

    /// Publish the complete presentation lifetime fence and release every
    /// retirement which has crossed it.
    pub fn synchronize_retained_images(&mut self, retained: RetainedImageSet) {
        self.retained_images = retained;
        for image in self.residency.take_releasable(&self.retained_images) {
            self.release(image);
        }
    }

    fn release(&mut self, image: ImageId) {
        if let Some(cached) = self.textures.remove(&image) {
            self.total_memory -= cached.memory_size;
            self.accounting
                .push(crate::media_budget::MediaAccounting::Freed {
                    media_type: crate::media_budget::MediaType::Image,
                    id: image.get(),
                });
        }
        self.states.remove(&image);
    }

    /// Drain budget accounting events accumulated since the last call.
    pub(crate) fn drain_accounting(&mut self) -> Vec<crate::media_budget::MediaAccounting> {
        std::mem::take(&mut self.accounting)
    }

    /// Clear entire cache
    pub fn clear(&mut self) {
        self.loads.clear();
        self.residency.clear();
        self.retained_images = RetainedImageSet::default();
        self.textures.clear();
        self.states.clear();
        self.pending_dimensions.clear();
        self.total_memory = 0;
    }
}

#[cfg(test)]
#[path = "image_cache/tests/image_cache_test.rs"]
mod tests;
