//! The CPU decoder chain the display, the GUI host and the attach helper
//! share: encoded bytes or a file to realized RGBA pixels, GNU's metadata
//! and masks.  No GPU.

use romacs_display_protocol::image::EncodedBytes;
use romacs_display_protocol::image_diagnostic::{ImageDiagnostic, ImageLoadIdentity};
use romacs_display_protocol::{
    ImageAnimationPolicy, ImageColorContext, ImageEmbeddedMetadata,
    ImageFrameIndex, ImageHeuristicMask, ImageLayoutExtent, ImageMaskKind, ImageMaskPolicy, ImageNativeExtent,
    ImageRasterExtent, ImageRealization, ImageReportedExtent, ImageRotation, ImageSequenceId,
    ImageSequenceRetirement, ImageSizeSpec, ResolvedImageGeometry,
};
use std::ops::ControlFlow;
use std::sync::atomic::Ordering;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;

use crate::image_bands::{
    BandFilling, BandSource, BandStep, BandedDecoder, DecodedBand, classify_alpha, classify_source_alpha, png_intrinsic_alpha,
};
use crate::image_sequence::{ImageSequenceCache, ImageSequenceResolution};

/// Maximum texture dimension (width or height)
pub const MAX_TEXTURE_SIZE: u32 = 4096;

/// Where a banded decode hands each band as it lands. Its answer decides
/// whether the attempt goes on: `Break` abandons it, and the caller then gets
/// the image decoded whole, never the prefix the bands reached.
pub type BandSink<'a> = &'a mut dyn FnMut(DecodedBand) -> ControlFlow<()>;

/// Clamp to the renderer's texture limit, preserving aspect ratio.
///
/// This is a GPU constraint only — GNU's `:max-width`/`:max-height` are applied
/// by `ImageSizeSpec::desired`, which knows the native size and so can keep the
/// aspect ratio against the right numbers.
pub fn constrain_dimensions(width: u32, height: u32) -> (u32, u32) {
    let mut width = width;
    let mut height = height;
    let width_limit = MAX_TEXTURE_SIZE;
    let height_limit = MAX_TEXTURE_SIZE;

    if width > width_limit {
        height = (f64::from(height) * f64::from(width_limit) / f64::from(width)) as u32;
        width = width_limit;
    }
    if height > height_limit {
        width = (f64::from(width) * f64::from(height_limit) / f64::from(height)) as u32;
        height = height_limit;
    }

    (width.max(1), height.max(1))
}

pub fn constrain_raster_extent(extent: ImageRasterExtent) -> ImageRasterExtent {
    let (width, height) = constrain_dimensions(extent.width(), extent.height());
    ImageRasterExtent::new(width, height)
}

/// The geometry a decode realizes to, raster clamped by the texture limit.
///
/// One function rather than a line in each caller, because two callers must
/// agree about it: `NativePixels::realize_bitmap` scales the whole image onto
/// this raster, and a banded decode builds the raster it writes into from it
/// (`image_bands::BandPlan::target`). A decode writing under a different raster
/// than the upload that finishes it would leave the texture holding rows from
/// two scales, which nothing downstream could detect.
pub fn realized_geometry(
    extent: ImageNativeExtent,
    size: ImageSizeSpec,
    realization: ImageRealization,
) -> ResolvedImageGeometry {
    let geometry = realization.resolve_geometry(size, extent, ImageRotation::None);
    geometry.with_raster(constrain_raster_extent(geometry.raster()))
}


/// What one attempt at a banded decode produced.
///
/// The two ways of not completing are separate arms because they mean
/// different things to a reader — one is a source that never had a row-wise
/// decoder, the other a decoder that stopped part-way — even though the caller
/// does the same thing with both.
pub enum BandedAttempt {
    /// Every row was decoded, into the raster the image is realized to.
    Complete(DecodedPixels),
    /// The source has no row-wise decoder at this size, or none for its format,
    /// or no band of it has anywhere to go: decode the whole image.
    NotBandable,
    /// A row-wise decode that could not finish. The bands it published are
    /// abandoned; decoding the whole image is the only way to end up with a
    /// whole one.
    Abandoned,
}

/// Pixels directly emitted by a decoder before an image spec is realized.
pub struct NativePixels {
    pub extent: ImageNativeExtent,
    pub rgba: Vec<u8>,
    pub embedded: ImageEmbeddedMetadata,
}

/// Pixels whose layout, GNU-reported, and GPU extents were resolved together.
pub struct DecodedPixels {
    pub geometry: ResolvedImageGeometry,
    pub rgba: Vec<u8>,
    pub mask: ImageMaskKind,
    pub embedded: ImageEmbeddedMetadata,
}

/// Whether an opt-in computed sequence owns frame selection for this load.
pub enum ComputedSequenceDecode {
    NotAnimated,
    Frame(DecodedPixels),
    /// The sequence exists, but its requested frame cannot be realized.
    Unavailable,
}

impl NativePixels {
    pub fn raster(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Self {
            extent: ImageNativeExtent::new(width, height),
            rgba,
            embedded: ImageEmbeddedMetadata::default(),
        }
    }

    pub fn from_raster_tuple((width, height, rgba): (u32, u32, Vec<u8>)) -> Self {
        Self::raster(width, height, rgba)
    }

    /// Resolve the spec's requested size against the decoded native size, then
    /// realize to texture pixels.
    ///
    /// This is where GNU's `compute_image_size` lands: the size cannot be known
    /// before decoding, so `:width`/`:height`/`:max-*` are applied here rather
    /// than as a bounding box handed to the decoder. With `AxisSize::Native` on
    /// both axes this reduces to `layout_dimension`, the previous behavior.
    pub fn realize_bitmap(
        self,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        realization: ImageRealization,
        mask_policy: ImageMaskPolicy,
    ) -> Option<DecodedPixels> {
        self.realize_bitmap_with_intrinsic_alpha(size, rotation, realization, mask_policy, false)
    }

    pub fn realize_bitmap_with_intrinsic_alpha(
        mut self,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        realization: ImageRealization,
        mask_policy: ImageMaskPolicy,
        intrinsic_alpha: bool,
    ) -> Option<DecodedPixels> {
        let source_mask = classify_source_alpha(&self.rgba, intrinsic_alpha);
        let mask = apply_mask_policy_with_source(
            &mut self.rgba,
            self.extent.dimensions(),
            mask_policy,
            source_mask,
        );
        let geometry = realized_geometry(self.extent, size, realization);
        let (raster_width, raster_height) = geometry.raster().dimensions();
        let (native_width, native_height) = self.extent.dimensions();
        let rgba = if geometry.raster() == ImageRasterExtent::new(native_width, native_height) {
            self.rgba
        } else {
            let source = image::RgbaImage::from_raw(native_width, native_height, self.rgba)?;
            image::imageops::resize(
                &source,
                raster_width,
                raster_height,
                image::imageops::FilterType::Lanczos3,
            )
            .into_raw()
        };
        // GNU rotates AFTER sizing, so `:width` sizes the upright image and the
        // turn then exchanges the axes (src/image.c:3169-3201). Quarter turns
        // are lossless, which is exactly why GNU only offers multiples of 90.
        let rgba = match rotation {
            ImageRotation::None => rgba,
            turn => {
                let source = image::RgbaImage::from_raw(raster_width, raster_height, rgba)?;
                let turned = match turn {
                    ImageRotation::Quarter => image::imageops::rotate90(&source),
                    ImageRotation::Half => image::imageops::rotate180(&source),
                    ImageRotation::ThreeQuarter => image::imageops::rotate270(&source),
                    ImageRotation::None => unreachable!("handled above"),
                };
                turned.into_raw()
            }
        };
        Some(DecodedPixels {
            geometry: geometry.oriented(rotation),
            rgba,
            mask,
            embedded: self.embedded,
        })
    }
}

pub fn most_frequent_corner_rgb(rgba: &[u8], (width, height): (u32, u32)) -> [u8; 3] {
    let pixel = |x: u32, y: u32| {
        let offset = ((y * width + x) * 4) as usize;
        [rgba[offset], rgba[offset + 1], rgba[offset + 2]]
    };
    let corners = [
        pixel(0, 0),
        pixel(width - 1, 0),
        pixel(width - 1, height - 1),
        pixel(0, height - 1),
    ];
    let mut best = corners[0];
    let mut best_count = 0;
    for candidate in corners {
        let count = corners
            .iter()
            .filter(|corner| **corner == candidate)
            .count();
        if count > best_count {
            best = candidate;
            best_count = count;
        }
    }
    best
}

pub fn rgb16_to_rgb8(rgb: [u16; 3]) -> [u8; 3] {
    rgb.map(|component| ((u32::from(component) + 128) / 257) as u8)
}

/// Apply GNU's image-mask policy at the decoder boundary.
///
/// This runs before bitmap scaling so mask identity describes the decoded
/// source, rather than alpha values introduced by interpolation. The renderer
/// may store each variant in RGBA, but storage must not collapse clipping masks
/// and continuous alpha into one semantic state.
pub fn apply_mask_policy(
    rgba: &mut [u8],
    dimensions: (u32, u32),
    policy: ImageMaskPolicy,
) -> ImageMaskKind {
    let source_mask = classify_alpha(rgba);
    apply_mask_policy_with_source(rgba, dimensions, policy, source_mask)
}

pub fn apply_mask_policy_with_source(
    rgba: &mut [u8],
    dimensions: (u32, u32),
    policy: ImageMaskPolicy,
    source_mask: ImageMaskKind,
) -> ImageMaskKind {
    match policy {
        ImageMaskPolicy::Preserve => source_mask,
        ImageMaskPolicy::Suppress => {
            let mask = source_mask;
            if mask.has_clipping_mask() {
                for alpha in rgba.iter_mut().skip(3).step_by(4) {
                    *alpha = 255;
                }
                ImageMaskKind::None
            } else {
                // GNU's `:mask nil` clears only `img->mask`; it does not
                // discard continuous alpha represented by the image itself.
                mask
            }
        }
        ImageMaskPolicy::Heuristic(heuristic) => {
            let background = match heuristic {
                ImageHeuristicMask::FourCorners => most_frequent_corner_rgb(rgba, dimensions),
                ImageHeuristicMask::Rgb16(rgb) => rgb16_to_rgb8(rgb),
            };
            for pixel in rgba.chunks_exact_mut(4) {
                pixel[3] = if pixel[..3] == background { 0 } else { 255 };
            }
            ImageMaskKind::Clipping
        }
    }
}

/// Facts published with a completed decoded image realization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageMetadata {
    pub layout: ImageLayoutExtent,
    pub reported: ImageReportedExtent,
    /// GNU's four-corner background guess, encoded as 0x00RRGGBB.
    pub background: u32,
    /// Whether GNU's four-corner mask heuristic classifies the background as transparent.
    pub background_transparent: bool,
    /// The decoded alpha representation, independent of its RGBA storage.
    pub mask: ImageMaskKind,
    /// Decoder-owned metadata returned by GNU's `image-metadata`.
    pub embedded: ImageEmbeddedMetadata,
}

/// CPU-only semantic decode source. No GPU or render-command capability.
pub enum SemanticImageSource {
    File(String),
    Data {
        data: EncodedBytes,
        resources: crate::svg::SvgResourceContext,
    },
}

/// Immutable inputs required for a complete semantic decode, not a header probe.
pub struct SemanticImageRequest {
    pub source: SemanticImageSource,
    pub size: ImageSizeSpec,
    pub rotation: ImageRotation,
    pub realization: ImageRealization,
    pub colors: ImageColorContext,
    pub mask: ImageMaskPolicy,
    pub animation: ImageAnimationPolicy,
    pub frame: ImageFrameIndex,
    pub sequence: ImageSequenceId,
    pub limit: romacs_display_protocol::ImageSizeLimit,
    pub identity: ImageLoadIdentity,
}

/// Fully decoded immutable pixels, reusable for residency without re-reading
/// sources or re-evaluating SVG resources after semantic admission.
#[derive(Debug)]
pub struct SemanticImageDecoded {
    pub metadata: ImageMetadata,
    pub geometry: ResolvedImageGeometry,
    pub data: Vec<u8>,
}

impl SemanticImageDecoded {
    pub fn metadata(&self) -> &ImageMetadata {
        &self.metadata
    }
    pub fn size_bytes(&self) -> usize {
        self.data.len()
    }
}

/// Decoder-owned sequence state for synchronous semantic queries. Its lifetime
/// and retirement are independent of renderer residency and GPU reset.
pub struct SemanticImageDecoder {
    pub sequences: Arc<ImageSequenceCache>,
    pub sequence_bytes: std::sync::atomic::AtomicU64,
}

impl Default for SemanticImageDecoder {
    fn default() -> Self {
        Self {
            sequences: Arc::new(ImageSequenceCache::new()),
            sequence_bytes: Default::default(),
        }
    }
}

/// Reservation made before submission, so retirement fences even queued jobs.
pub struct SemanticImageSequenceReservation {
    pub _reservation: crate::image_sequence::ImageSequenceReservation,
}

impl SemanticImageDecoder {
    pub fn reserve_sequence(&self, sequence: ImageSequenceId) -> SemanticImageSequenceReservation {
        SemanticImageSequenceReservation {
            _reservation: self.sequences.reserve(sequence),
        }
    }

    pub fn retire(&self, retirement: ImageSequenceRetirement) {
        self.sequences.retire(retirement);
        self.sequences.publish_size_snapshot(&self.sequence_bytes);
    }

    pub fn cached_size_bytes(&self) -> usize {
        usize::try_from(self.sequence_bytes.load(Ordering::Acquire)).unwrap_or(usize::MAX)
    }

    /// Fully decode and realize pixels using the production decoder chain.
    /// Admission and decoding use the same byte snapshot. Metadata is computed
    /// only after the whole decode finishes; bands and headers are not answers.
    pub fn decode(&self, request: SemanticImageRequest) -> Result<ImageMetadata, ImageDiagnostic> {
        self.decode_pixels(request).map(|decoded| decoded.metadata)
    }

    pub fn decode_pixels(
        &self,
        request: SemanticImageRequest,
    ) -> Result<SemanticImageDecoded, ImageDiagnostic> {
        let SemanticImageRequest {
            source,
            size,
            rotation,
            realization,
            colors,
            mask,
            animation,
            frame,
            sequence,
            limit,
            identity,
        } = request;
        let (failure, data, resources) = match source {
            SemanticImageSource::File(path) => {
                let failure = DecodeFailureSource::File { path: path.clone() };
                let data = std::fs::read(&path)
                    .map(EncodedBytes::new)
                    .map_err(|_| failure.diagnostic(&identity))?;
                (failure, data, crate::svg::SvgResourceContext::BaseUri(path))
            }
            SemanticImageSource::Data { data, resources } => (
                DecodeFailureSource::Bytes { data: data.clone() },
                data,
                resources,
            ),
        };
        if crate::image_probe::admit(crate::image_probe::ImageProbeSource::Data(&data), limit)
            .is_err()
        {
            return Err(ImageDiagnostic::InvalidSize);
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            Decode::decode_data(
                data,
                size,
                rotation,
                colors,
                realization,
                mask,
                animation,
                frame,
                resources,
                &self.sequences,
                sequence,
                None,
            )
        }));
        self.sequences.publish_size_snapshot(&self.sequence_bytes);
        match result {
            Ok(Some(pixels)) => Ok(SemanticImageDecoded {
                metadata: Decode::metadata_from_rgba(
                    pixels.geometry,
                    &pixels.rgba,
                    pixels.mask,
                    pixels.embedded,
                ),
                geometry: pixels.geometry,
                data: pixels.rgba,
            }),
            Ok(None) | Err(_) => Err(failure.diagnostic(&identity)),
        }
    }
}


/// What a decode job can still say about a source once the decode chain has
/// collapsed every reason to `None`.
///
/// The chain that answers `Option` — banded attempt, whole-image decode, XPM,
/// XBM, SVG — loses its reason at every fallback: a banded attempt that cannot
/// finish hands the same bytes to the whole-image path, which hands them to
/// the next format. GNU does not have this problem because each of its loaders
/// checks its own signature and reports its own decoder's message, so the
/// reason is recovered here from the two facts that survive: which source was
/// asked for, and what its bytes turned out to be.
pub enum DecodeFailureSource {
    /// A `:file` source, named by the path a load command was given.
    File { path: String },
    /// A `:data` source, whose bytes are already in hand.
    Bytes { data: EncodedBytes },
    /// Raw pixels handed over by a caller, with no encoded format to name and
    /// no GNU loader behind them.
    Raw,
}

impl DecodeFailureSource {
    pub fn diagnostic(&self, identity: &ImageLoadIdentity) -> ImageDiagnostic {
        // A source that arrived without a specification has no declared type
        // and no printed spec, so there is no GNU sentence that is true of it.
        if identity.is_unspecified() {
            return ImageDiagnostic::NotDrawable;
        }
        match self {
            // GNU asks `emacs_open` first and only then checks the signature
            // (`src/image.c:8282-8303`), so "the file is not there" and "the
            // file is not a PNG" are different sentences. Re-reading on the
            // failure path is what recovers that distinction: it happens once,
            // only when the decode has already failed.
            Self::File { path } => match std::fs::read(path) {
                Err(_) => identity.not_found(),
                Ok(bytes) => Self::from_bytes(identity, &bytes),
            },
            Self::Bytes { data } => Self::from_bytes(identity, data.as_slice()),
            Self::Raw => ImageDiagnostic::NotDrawable,
        }
    }

    pub fn from_bytes(identity: &ImageLoadIdentity, data: &[u8]) -> ImageDiagnostic {
        if image::guess_format(data).is_err() {
            return identity.wrong_format();
        }
        // The format was recognised and the decode failed inside it, which is
        // GNU's `PNG error: %s` / `Error reading JPEG image ...` arm.
        let format = identity.format().clone();
        match image::load_from_memory(data) {
            // libpng's `png_read_data` reports a short read as `Read error`,
            // and a stream that ends early is exactly what the `image` crate
            // calls `UnexpectedEof`. Anything else keeps the decoder's own
            // words, which is what GNU passes through verbatim
            // (`image_error ("PNG error: %s", ...)`, `src/image.c:8184`).
            Err(image::ImageError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                ImageDiagnostic::FormatError {
                    format,
                    detail: "Read error".to_owned(),
                }
            }
            Err(error) => ImageDiagnostic::FormatError {
                format,
                detail: error.to_string(),
            },
            // The bytes decoded; something after the decoder refused them.
            Ok(_) => ImageDiagnostic::NotDrawable,
        }
    }
}


/// The CPU decoder chain: every format, banded or whole, to realized pixels.
pub struct Decode;

impl Decode {
    /// Decode image file with size constraints
    #[allow(clippy::too_many_arguments)]
    pub fn decode_file(
        path: &str,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        colors: ImageColorContext,
        realization: ImageRealization,
        mask: ImageMaskPolicy,
        animation: ImageAnimationPolicy,
        frame: ImageFrameIndex,
        sequence_cache: &ImageSequenceCache,
        sequence: ImageSequenceId,
        sink: Option<BandSink<'_>>,
    ) -> Option<DecodedPixels> {
        // The bytes the decode will read, owned here and handed to the decode
        // chain by handle: `std::fs::read` gives the buffer, `EncodedBytes`
        // moves it, and the fallbacks below still have the handle they need.
        // The one clone is the atomic a decode job pays for the attempt to
        // hold the bytes while this copy keeps them for the fallbacks.
        let encoded = std::fs::read(path).ok().map(EncodedBytes::new);
        if let Some(pixels) = encoded.as_ref().and_then(|data| {
            Self::decode_raster_data(
                data.clone(),
                frame,
                sequence_cache,
                sequence,
                size,
                rotation,
                realization,
                mask,
                sink,
            )
        }) {
            return Some(pixels);
        }
        // Computed animation, file-backed arm: same gate and same sequence
        // machinery as the data arm, with the file's own resource context
        // so relative references resolve as the static path does.
        if animation.is_enabled()
            && let Some(data) = encoded.as_ref()
        {
            match Self::decode_computed_sequence_data(
                data.clone(),
                frame,
                sequence_cache,
                sequence,
                size,
                rotation,
                realization,
                colors,
                mask,
                &crate::svg::SvgResourceContext::BaseUri(path.to_owned()),
                animation,
            ) {
                ComputedSequenceDecode::Frame(pixels) => return Some(pixels),
                ComputedSequenceDecode::Unavailable => return None,
                ComputedSequenceDecode::NotAnimated => {}
            }
        }
        if !frame.is_first() {
            // GNU's static SVG loader ignores :index. Other still formats keep
            // their existing index validation; only a valid SVG may fall back.
            return Self::decode_svg_data(
                encoded.as_ref()?,
                size,
                rotation,
                realization,
                colors,
                mask,
                crate::svg::SvgResourceContext::BaseUri(path.to_owned()),
            );
        }
        // Fallbacks for the byte-oriented formats `image` cannot read. GNU
        // paints an XPM pixel whose color key has no resolvable color with the
        // frame's foreground pixel (src/image.c:6518-6538) — the frame
        // foreground this decode carries — and XBM takes both face colors.
        let fg = colors.foreground().rgba8();
        let bg = colors.background_rgba8();
        if let Some(result) =
            crate::xpm::decode_xpm_file(Path::new(path), colors.frame_foreground().rgb8())
        {
            return NativePixels::from_raster_tuple(result).realize_bitmap(
                size,
                rotation,
                realization,
                mask,
            );
        }
        if let Some(result) = crate::xbm::decode_xbm_file(Path::new(path), fg, bg) {
            return NativePixels::from_raster_tuple(result).realize_bitmap(
                size,
                rotation,
                realization,
                mask,
            );
        }
        // Fallback: try SVG via the shared vector backend.
        let data = encoded?;
        Self::decode_svg_data(
            &data,
            size,
            rotation,
            realization,
            colors,
            mask,
            crate::svg::SvgResourceContext::BaseUri(path.to_owned()),
        )
    }

    /// Decode image data with size constraints
    #[allow(clippy::too_many_arguments)]
    pub fn decode_data(
        data: EncodedBytes,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        colors: ImageColorContext,
        realization: ImageRealization,
        mask: ImageMaskPolicy,
        animation: ImageAnimationPolicy,
        frame: ImageFrameIndex,
        resources: crate::svg::SvgResourceContext,
        sequence_cache: &ImageSequenceCache,
        sequence: ImageSequenceId,
        sink: Option<BandSink<'_>>,
    ) -> Option<DecodedPixels> {
        if let Some(pixels) = Self::decode_raster_data(
            data.clone(),
            frame,
            sequence_cache,
            sequence,
            size,
            rotation,
            realization,
            mask,
            sink,
        ) {
            return Some(pixels);
        }
        // Computed animation: an SVG document under an enabled policy
        // materializes its frames on the sample grid, and `frame` selects
        // the slot exactly as `:index` selects a GIF frame.
        //
        // The policy gate is load-bearing, not an optimization: sequence
        // identity follows the resolve source, not the policy, so an entry
        // a previous `:animation` load warmed serves *animated* pixels on a
        // cache hit. A disabled request must take the static path below
        // unconditionally — cold or warm — or the GNU-compatible default
        // would depend on load order.
        if animation.is_enabled() {
            match Self::decode_computed_sequence_data(
                data.clone(),
                frame,
                sequence_cache,
                sequence,
                size,
                rotation,
                realization,
                colors,
                mask,
                &resources,
                animation,
            ) {
                ComputedSequenceDecode::Frame(pixels) => return Some(pixels),
                ComputedSequenceDecode::Unavailable => return None,
                ComputedSequenceDecode::NotAnimated => {}
            }
        }
        if !frame.is_first() {
            return Self::decode_svg_data(
                &data,
                size,
                rotation,
                realization,
                colors,
                mask,
                resources,
            );
        }
        // Fallbacks for the byte-oriented formats `image` cannot read; see the
        // file arm for why XPM takes the frame foreground.
        let fg = colors.foreground().rgba8();
        let bg = colors.background_rgba8();
        if let Some(result) = crate::xpm::decode_xpm_data(&data, colors.frame_foreground().rgb8()) {
            return NativePixels::from_raster_tuple(result).realize_bitmap(
                size,
                rotation,
                realization,
                mask,
            );
        }
        if let Some(result) = crate::xbm::decode_xbm_data(&data, fg, bg) {
            return NativePixels::from_raster_tuple(result).realize_bitmap(
                size,
                rotation,
                realization,
                mask,
            );
        }
        // Fallback: try SVG via the shared vector backend.
        Self::decode_svg_data(&data, size, rotation, realization, colors, mask, resources)
    }

    /// Decode a raster source while preserving multi-frame semantics.
    ///
    /// `DynamicImage` intentionally represents one still image and therefore
    /// drops both frame selection and animation metadata. Route animated
    /// formats through `AnimationDecoder` first, then use the still-image path
    /// only for frame zero.
    fn decode_raster_data(
        data: EncodedBytes,
        frame: ImageFrameIndex,
        sequence_cache: &ImageSequenceCache,
        sequence: ImageSequenceId,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        realization: ImageRealization,
        mask_policy: ImageMaskPolicy,
        sink: Option<BandSink<'_>>,
    ) -> Option<DecodedPixels> {
        match sequence_cache.resolve(sequence, &data, frame) {
            ImageSequenceResolution::Frame(frame) => {
                let (width, height) = frame.dimensions();
                let (rgba, embedded) = frame.into_parts();
                NativePixels {
                    extent: ImageNativeExtent::new(width, height),
                    rgba,
                    embedded,
                }
                .realize_bitmap(size, rotation, realization, mask_policy)
            }
            ImageSequenceResolution::MissingFrame => None,
            ImageSequenceResolution::NotAnimated => {
                Self::decode_still_image(data, size, rotation, realization, mask_policy, sink)
            }
        }
    }

    /// Decode one frame of a computed animation through the sequence cache.
    ///
    /// The SVG sampler answers "not animated" for every document the
    /// policy does not materialize (disabled policy, no plan, no loop), so
    /// this arm falling through is the ordinary static path, and the
    /// fallback ladder still ends at GNU's single-frame behavior.
    fn decode_computed_sequence_data(
        data: EncodedBytes,
        frame: ImageFrameIndex,
        sequence_cache: &ImageSequenceCache,
        sequence: ImageSequenceId,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        realization: ImageRealization,
        colors: ImageColorContext,
        mask: ImageMaskPolicy,
        resources: &crate::svg::SvgResourceContext,
        policy: ImageAnimationPolicy,
    ) -> ComputedSequenceDecode {
        match sequence_cache.resolve_svg(sequence, &data, frame, colors, resources, policy) {
            ImageSequenceResolution::Frame(frame) => {
                let (width, height) = frame.dimensions();
                let (rgba, embedded) = frame.into_parts();
                NativePixels {
                    extent: ImageNativeExtent::new(width, height),
                    rgba,
                    embedded,
                }
                .realize_bitmap(size, rotation, realization, mask)
                .map(ComputedSequenceDecode::Frame)
                .unwrap_or(ComputedSequenceDecode::Unavailable)
            }
            ImageSequenceResolution::MissingFrame => ComputedSequenceDecode::Unavailable,
            ImageSequenceResolution::NotAnimated => ComputedSequenceDecode::NotAnimated,
        }
    }

    /// Decode a still raster source, row-wise where the source allows it.
    ///
    /// The two arms of [`BandSource`] are the whole decision: a source with a
    /// row-wise decoder above the size threshold is decoded band by band,
    /// straight into the raster its texture holds, and everything else — every
    /// other format, every source below the threshold, and any row-wise decode
    /// that could not finish — is decoded whole and realized afterwards, which
    /// is what this code did for every source before banding existed.
    pub fn decode_still_image(
        data: EncodedBytes,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        realization: ImageRealization,
        mask_policy: ImageMaskPolicy,
        sink: Option<BandSink<'_>>,
    ) -> Option<DecodedPixels> {
        match Self::attempt_banded(
            data.clone(),
            size,
            realization,
            BandFilling::of(rotation, mask_policy),
            sink,
        ) {
            BandedAttempt::Complete(pixels) => Some(pixels),
            BandedAttempt::NotBandable | BandedAttempt::Abandoned => {
                let intrinsic_alpha = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
                    png::Decoder::new(std::io::Cursor::new(data.clone()))
                        .read_info()
                        .map(|reader| png_intrinsic_alpha(reader.info()))
                        .unwrap_or(false)
                } else {
                    false
                };
                Self::decode_whole(data)?.realize_bitmap_with_intrinsic_alpha(
                    size,
                    rotation,
                    realization,
                    mask_policy,
                    intrinsic_alpha,
                )
            }
        }
    }

    /// One attempt at a banded decode.
    ///
    /// The whole attempt resolves to one raster, from the *header's* extent
    /// through the same `realized_geometry` the finished upload resolves from
    /// the decoded one, and the source writes its rows into it as it reads
    /// them. Step 1 pins those two extents equal, so the raster is the raster
    /// the finished upload would write; if a header ever disagreed with its own
    /// pixels, the source's own row count would differ from the height the
    /// target was built for and the attempt is abandoned rather than published.
    pub fn attempt_banded(
        data: EncodedBytes,
        size: ImageSizeSpec,
        realization: ImageRealization,
        filling: BandFilling,
        mut sink: Option<BandSink<'_>>,
    ) -> BandedAttempt {
        // A band has a destination only where the texture can be built up from
        // the top: an unrotated realization whose mask policy leaves the pixels
        // alone. Where it has none — a turn lands a band's rows in the raster's
        // columns, and a mask that rewrites pixels needs all of them first —
        // there is nothing to write a band into as it arrives, and the only way
        // to hold the rows until the end is the native-size buffer this step
        // exists to remove. Such a source takes the whole-image path, which is
        // what it took before banding and what it still looks like on screen:
        // empty until the decode completes, then whole.
        if filling == BandFilling::Deferred {
            return BandedAttempt::NotBandable;
        }
        let mut source = match BandSource::open(data, size, realization) {
            BandSource::Banded(source) => source,
            BandSource::Whole => return BandedAttempt::NotBandable,
        };
        loop {
            match source.next_band() {
                BandStep::Band(band) => {
                    if let Some(sink) = sink.as_deref_mut()
                        && sink(band).is_break()
                    {
                        return BandedAttempt::Abandoned;
                    }
                }
                // A completed source is the only one that yields pixels, so a
                // prefix cannot escape as an image.
                BandStep::Done => {
                    let Some(pixels) = source.into_raster() else {
                        return BandedAttempt::Abandoned;
                    };
                    let geometry = realized_geometry(pixels.native(), size, realization);
                    debug_assert_eq!(
                        geometry.raster(),
                        pixels.raster(),
                        "a decode fills the raster its realization resolves"
                    );
                    let mask = pixels.mask();
                    return BandedAttempt::Complete(DecodedPixels {
                        geometry,
                        rgba: pixels.into_rgba(),
                        mask,
                        embedded: ImageEmbeddedMetadata::default(),
                    });
                }
                BandStep::Failed => return BandedAttempt::Abandoned,
            }
        }
    }

    /// The whole image in one decode: the path every source took before
    /// banding, and the one a banded attempt falls back to.
    pub fn decode_whole(data: EncodedBytes) -> Option<NativePixels> {
        Self::process_image(image::load_from_memory(&data).ok()?)
    }

    pub fn metadata_from_rgba(
        geometry: ResolvedImageGeometry,
        rgba: &[u8],
        mask_kind: ImageMaskKind,
        embedded: ImageEmbeddedMetadata,
    ) -> ImageMetadata {
        let (raster_width, raster_height) = geometry.raster().dimensions();
        let pixel = |x: u32, y: u32| {
            let offset = ((y * raster_width + x) * 4) as usize;
            [
                rgba[offset],
                rgba[offset + 1],
                rgba[offset + 2],
                rgba[offset + 3],
            ]
        };
        let corners = [
            pixel(0, 0),
            pixel(raster_width - 1, 0),
            pixel(raster_width - 1, raster_height - 1),
            pixel(0, raster_height - 1),
        ];
        let most_frequent = |values: [[u8; 4]; 4], key: fn([u8; 4]) -> u32| {
            let mut best = values[0];
            let mut best_count = 0;
            for candidate in values {
                let count = values
                    .iter()
                    .filter(|value| key(**value) == key(candidate))
                    .count();
                if count > best_count {
                    best = candidate;
                    best_count = count;
                }
            }
            best
        };
        let background = most_frequent(corners, |pixel| {
            (u32::from(pixel[0]) << 16) | (u32::from(pixel[1]) << 8) | u32::from(pixel[2])
        });
        let mask = most_frequent(corners, |pixel| u32::from(pixel[3] == 0));
        ImageMetadata {
            layout: geometry.layout(),
            reported: geometry.reported(),
            background: (u32::from(background[0]) << 16)
                | (u32::from(background[1]) << 8)
                | u32::from(background[2]),
            background_transparent: mask[3] == 0,
            mask: mask_kind,
            embedded,
        }
    }

    /// Decode SVG data through the platform SVG backend, returning RGBA pixels.
    pub fn decode_svg_data(
        data: &[u8],
        size: ImageSizeSpec,
        rotation: ImageRotation,
        realization: ImageRealization,
        colors: ImageColorContext,
        mask_policy: ImageMaskPolicy,
        resources: crate::svg::SvgResourceContext,
    ) -> Option<DecodedPixels> {
        let mut decoded = crate::svg::decode(data, size, rotation, realization, colors, resources)?;
        let mask = apply_mask_policy(
            &mut decoded.rgba,
            decoded.geometry.raster().dimensions(),
            mask_policy,
        );
        Some(DecodedPixels {
            geometry: decoded.geometry,
            rgba: decoded.rgba,
            mask,
            embedded: ImageEmbeddedMetadata::default(),
        })
    }

    /// Process decoded image: resize if needed, convert to RGBA
    /// Decode to NATIVE pixels. Sizing happens in `realize_bitmap`, which is
    /// the only place that knows both the native size and the requested one.
    pub fn process_image(img: image::DynamicImage) -> Option<NativePixels> {
        let rgba = img.to_rgba8();
        Some(NativePixels::raster(
            rgba.width(),
            rgba.height(),
            rgba.into_raw(),
        ))
    }
    pub fn convert_argb32_to_rgba(
        data: &[u8],
        width: u32,
        height: u32,
        stride: u32,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let bytes_per_pixel = 4u32;
        let expected_min_size = (height.saturating_sub(1)) * stride + width * bytes_per_pixel;
        if data.len() < expected_min_size as usize {
            tracing::warn!(
                "ARGB32 data too small: got {} bytes, expected at least {} for {}x{} with stride {}",
                data.len(),
                expected_min_size,
                width,
                height,
                stride
            );
            return None;
        }

        // Convert ARGB32 to RGBA
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            let row_start = (y * stride) as usize;
            for x in 0..width {
                let pixel_start = row_start + (x * bytes_per_pixel) as usize;
                let a = data[pixel_start];
                let r = data[pixel_start + 1];
                let g = data[pixel_start + 2];
                let b = data[pixel_start + 3];
                let idx = ((y * width + x) * 4) as usize;
                rgba[idx] = r;
                rgba[idx + 1] = g;
                rgba[idx + 2] = b;
                rgba[idx + 3] = a;
            }
        }

        // Apply size constraints if needed
        let (cw, ch) = constrain_dimensions(width, height);
        if cw != width || ch != height {
            // Need to resize - use image crate
            let img = image::RgbaImage::from_raw(width, height, rgba)?;
            let resized =
                image::imageops::resize(&img, cw, ch, image::imageops::FilterType::Lanczos3);
            Some((cw, ch, resized.into_raw()))
        } else {
            Some((width, height, rgba))
        }
    }

    /// Convert RGB24 raw pixel data to RGBA
    /// Input format: R,G,B byte order (3 bytes per pixel)
    /// Output format: R,G,B,A byte order (4 bytes per pixel, alpha=255)
    pub fn convert_rgb24_to_rgba(
        data: &[u8],
        width: u32,
        height: u32,
        stride: u32,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let bytes_per_pixel = 3u32;
        let expected_min_size = (height.saturating_sub(1)) * stride + width * bytes_per_pixel;
        if data.len() < expected_min_size as usize {
            tracing::warn!(
                "RGB24 data too small: got {} bytes, expected at least {} for {}x{} with stride {}",
                data.len(),
                expected_min_size,
                width,
                height,
                stride
            );
            return None;
        }

        // Convert RGB24 to RGBA (add alpha=255)
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            let row_start = (y * stride) as usize;
            for x in 0..width {
                let pixel_start = row_start + (x * bytes_per_pixel) as usize;
                let r = data[pixel_start];
                let g = data[pixel_start + 1];
                let b = data[pixel_start + 2];
                let idx = ((y * width + x) * 4) as usize;
                rgba[idx] = r;
                rgba[idx + 1] = g;
                rgba[idx + 2] = b;
                rgba[idx + 3] = 255;
            }
        }

        // Apply size constraints if needed
        let (cw, ch) = constrain_dimensions(width, height);
        if cw != width || ch != height {
            // Need to resize - use image crate
            let img = image::RgbaImage::from_raw(width, height, rgba)?;
            let resized =
                image::imageops::resize(&img, cw, ch, image::imageops::FilterType::Lanczos3);
            Some((cw, ch, resized.into_raw()))
        } else {
            Some((width, height, rgba))
        }
    }

}
