//! `prepare`: an attachment's outgoing copy (attachments.md §2.4).  An
//! image that already fits the consumer's rule goes as it is, its metadata
//! stripped when asked; one that does not (a format outside the rule, too
//! large, rotated by its EXIF orientation, an animation asked for still) is
//! decoded by the display's own decoder chain and encoded again.  Anything
//! else goes byte for byte.
use crate::store::{Stored, sha256_hex, store, store_copy};
use image::ImageEncoder;
use image::metadata::Orientation;
use romacs_display_protocol::image_diagnostic::{ImageDiagnostic, ImageDiagnosticSubject, ImageFormatName, ImageLoadIdentity};
use romacs_display_protocol::{
    AxisSize, ImageAnimationPolicy, ImageFrameIndex, ImageNativeExtent, ImageRealization, ImageRotation, ImageSequenceId,
    ImageSizeLimit, ImageSizeSpec,
};
use romacs_imaging::decode::{SemanticImageDecoder, SemanticImageRequest, SemanticImageSource};
use romacs_imaging::image_probe::{ImageProbeSource, probe_image_reported, probe_svg_reported, sniff_format};
use std::path::Path;

/// What a consumer's rule asks of an item's copy (`romacs-attach-tool
/// prepare`'s options).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The longest side an image may have.
    pub max_edge: Option<u32>,
    /// The bytes a PNG may have before it goes as a JPEG.
    pub max_bytes: Option<u64>,
    /// The image formats the consumer takes, first the one others become.
    pub formats: Vec<String>,
    /// An animation's first frame only.
    pub still: bool,
    /// No EXIF, XMP or text metadata in an image.
    pub strip: bool,
    /// Say whether a non-image is valid UTF-8.
    pub text: bool,
    /// GNU's `max-image-size' as pixels: a larger image is refused.
    pub max_image_size: Option<(u32, u32)>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_edge: None,
            max_bytes: None,
            formats: ["png", "jpeg", "gif", "webp"].map(String::from).to_vec(),
            still: false,
            strip: false,
            text: false,
            max_image_size: None,
        }
    }
}

/// An outgoing copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub stored: Stored,
    /// The image's MIME type; None for a non-image.
    pub mime: Option<&'static str>,
    pub size: Option<(u32, u32)>,
    /// With `text': whether a non-image is valid UTF-8.
    pub utf8: Option<bool>,
}

/// The formats `encode' writes, which are also those `strip' handles.
const ENCODABLE: [&str; 4] = ["png", "jpeg", "webp", "gif"];

/// The formats `sniff_format' knows by text it may find in a non-image (a
/// C header's `#define', a document quoting `<svg'): an image only if it
/// decodes.
const TEXTUAL: [&str; 4] = ["xbm", "xpm", "svg", "pbm"];

const RASTERS: [&str; 6] = ["png", "jpeg", "gif", "webp", "tiff", "bmp"];

fn mime(format: &str) -> &'static str {
    match format {
        "png" => "image/png",
        "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "xpm" => "image/x-xpixmap",
        "xbm" => "image/x-xbitmap",
        _ => "image/x-portable-anymap",
    }
}

fn extension(format: &str) -> &str {
    if format == "jpeg" { "jpg" } else { format }
}

/// The size in pixels BYTES of FORMAT give in their header, as the display
/// reports it at scale 1.
fn native_size(format: &str, bytes: &[u8]) -> Option<(u32, u32)> {
    let (size, rotation, realization) = (ImageSizeSpec::default(), ImageRotation::None, ImageRealization::new(1.0, 1.0, 1.0));
    match format {
        "svg" => probe_svg_reported(bytes, size, rotation, realization),
        f if RASTERS.contains(&f) => probe_image_reported(ImageProbeSource::Data(bytes), size, rotation, realization),
        _ => None,
    }
}

/// The EXIF orientation BYTES' decoder reads.
fn orientation(bytes: &[u8]) -> Orientation {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_decoder().ok())
        .and_then(|mut decoder| image::ImageDecoder::orientation(&mut decoder).ok())
        .unwrap_or(Orientation::NoTransforms)
}

/// Whether BYTES of FORMAT hold more than one frame.
fn animated(format: &str, bytes: &[u8]) -> bool {
    use image::AnimationDecoder;
    match format {
        "gif" => image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))
            .is_ok_and(|decoder| decoder.into_frames().take(2).count() > 1),
        "webp" => image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes)).is_ok_and(|d| d.has_animation()),
        _ => false,
    }
}

/// SRC's first frame as the display decodes it, fitted inside MAX_EDGE.
fn pixels(src: &Path, format: &str, max_edge: Option<u32>, limit: ImageSizeLimit) -> Result<image::RgbaImage, String> {
    let fit = || AxisSize::resolve(None, max_edge);
    let request = SemanticImageRequest {
        source: SemanticImageSource::File(src.to_string_lossy().into_owned()),
        size: ImageSizeSpec::new(fit(), fit()),
        rotation: ImageRotation::None,
        realization: ImageRealization::new(1.0, 1.0, 1.0),
        colors: Default::default(),
        mask: Default::default(),
        animation: ImageAnimationPolicy::disabled(),
        frame: ImageFrameIndex::new(0),
        sequence: ImageSequenceId::new(1).expect("nonzero"),
        limit,
        identity: ImageLoadIdentity::new(ImageFormatName::from_lisp_type(format),
                                         ImageDiagnosticSubject::File(src.to_string_lossy().into_owned())),
    };
    let decoded = SemanticImageDecoder::default().decode_pixels(request).map_err(|d| d.message())?;
    let (width, height) = decoded.geometry.raster().dimensions();
    image::RgbaImage::from_raw(width, height, decoded.data).ok_or_else(|| "decoded pixels do not fill their raster".into())
}

/// RGBA encoded as FORMAT: a JPEG at quality 85 over white, the rest
/// lossless.
fn encode(rgba: &image::RgbaImage, format: &str) -> Result<Vec<u8>, String> {
    let (width, height) = rgba.dimensions();
    let mut out = Vec::new();
    let result = match format {
        "jpeg" => {
            let over_white = |p: &image::Rgba<u8>| {
                let a = u16::from(p[3]);
                image::Rgb([0, 1, 2].map(|i| ((u16::from(p[i]) * a + 255 * (255 - a)) / 255) as u8))
            };
            let rgb = image::RgbImage::from_fn(width, height, |x, y| over_white(rgba.get_pixel(x, y)));
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
                .write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
        }
        "webp" => image::codecs::webp::WebPEncoder::new_lossless(&mut out)
            .write_image(rgba, width, height, image::ExtendedColorType::Rgba8),
        "gif" => image::codecs::gif::GifEncoder::new(&mut out)
            .encode(rgba, width, height, image::ExtendedColorType::Rgba8),
        _ => image::codecs::png::PngEncoder::new(&mut out)
            .write_image(rgba, width, height, image::ExtendedColorType::Rgba8),
    };
    result.map(|()| out).map_err(|e| e.to_string())
}

/// The format a re-encoded image of FORMAT becomes under OPTIONS: a photo
/// stays a JPEG, the rest become PNG, when the rule takes them.
fn target(format: &str, options: &Options) -> Result<&'static str, String> {
    let encodable = |f: &str| ENCODABLE.into_iter().find(|&e| e == f);
    // A format the rule takes stays itself (a photo a JPEG, a screenshot a
    // PNG); any other becomes the rule's first.
    options.formats.iter().any(|f| f == format).then(|| encodable(format)).flatten()
        .or_else(|| options.formats.iter().find_map(|f| encodable(f)))
        .ok_or_else(|| "the rule takes no image format".to_owned())
}

/// SRC, a non-image whose contents are BYTES, copied byte for byte.
fn verbatim(src: &Path, dir: &Path, bytes: &[u8], options: &Options) -> Result<Prepared, String> {
    let ext = src.extension().map_or("bin".into(), |e| e.to_string_lossy().to_lowercase());
    let stored = store_copy(dir, src, sha256_hex(bytes), &ext).map_err(|e| e.to_string())?;
    Ok(Prepared { stored, mime: None, size: None, utf8: options.text.then(|| std::str::from_utf8(bytes).is_ok()) })
}

/// SRC's outgoing copy in DIR under OPTIONS.
pub fn prepare(src: &Path, dir: &Path, options: &Options) -> Result<Prepared, String> {
    let bytes = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let Some(format) = sniff_format(&bytes[..bytes.len().min(romacs_imaging::image_probe::HEAD_BYTES)]) else {
        return verbatim(src, dir, &bytes, options);
    };
    let native = native_size(format, &bytes);
    let limit = options.max_image_size.map_or(ImageSizeLimit::UNLIMITED, |(w, h)| {
        ImageSizeLimit::from_frame_ratio(1.0, Some(ImageNativeExtent::new(w, h)))
    });
    if let (Some((w, h)), Some((lw, lh))) = (native, options.max_image_size)
        && (w > lw || h > lh)
    {
        return Err(ImageDiagnostic::InvalidSize.message());
    }
    let turn = orientation(&bytes);
    let fits = native.is_some_and(|(w, h)| options.max_edge.is_none_or(|edge| w.max(h) <= edge));
    let taken = options.formats.iter().any(|f| f == format);
    let still = options.still && animated(format, &bytes);
    // Stripping keeps the bytes only where a stripper exists; another
    // format the rule takes is converted.
    let strippable = !options.strip || ENCODABLE.contains(&format);
    let as_is = taken && fits && strippable && turn == Orientation::NoTransforms && !still;
    let (mut out, mut out_format, mut size) = if as_is {
        let out = if options.strip { crate::strip::strip(format, &bytes).ok_or("malformed image")? } else { bytes };
        (out, format, native.expect("fits"))
    } else {
        let rgba = match pixels(src, format, options.max_edge, limit) {
            Ok(rgba) => rgba,
            // Refused for its size, it is an image: the refusal stands.
            Err(error) if TEXTUAL.contains(&format) && error != ImageDiagnostic::InvalidSize.message() => {
                return verbatim(src, dir, &bytes, options);
            }
            Err(error) => return Err(error),
        };
        let mut rgba = image::DynamicImage::ImageRgba8(rgba);
        rgba.apply_orientation(turn);
        let rgba = rgba.into_rgba8();
        let into = target(format, options)?;
        (encode(&rgba, into)?, into, rgba.dimensions())
    };
    // A PNG over the byte limit goes as a JPEG, when the rule takes one.
    if options.max_bytes.is_some_and(|max| out.len() as u64 > max)
        && out_format == "png"
        && options.formats.iter().any(|f| f == "jpeg")
    {
        let rgba = image::load_from_memory(&out).map_err(|e| e.to_string())?.into_rgba8();
        out = encode(&rgba, "jpeg")?;
        out_format = "jpeg";
        size = rgba.dimensions();
    }
    let unchanged = as_is && !options.strip && out_format == format;
    let stored = if unchanged { store_copy(dir, src, sha256_hex(&out), extension(out_format)) } else { store(dir, &out, extension(out_format)) }
        .map_err(|e| e.to_string())?;
    Ok(Prepared { stored, mime: Some(mime(out_format)), size: Some(size), utf8: None })
}
