//! The attach helper's proofs (attachments.md §8, A0), on fixtures made
//! here: sizes under `--max-edge`, conversions to PNG, EXIF orientation,
//! metadata stripping that keeps the pixels, `max-image-size` refusal as
//! the display words it, the store's atomic and deduplicating writes,
//! `blobs` and `sweep`.
use image::ImageEncoder;
use romacs_attach::prepare::{Options, prepare};
use romacs_attach::{blobs, sweep};
use std::path::{Path, PathBuf};

/// A W×H picture with some structure, so a wrong decode shows.
fn picture(width: u32, height: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(width, height, |x, y| image::Rgba([(x * 7) as u8, (y * 5) as u8, ((x + y) * 3) as u8, 255]))
}

fn encoded(rgba: &image::RgbaImage, format: image::ImageFormat) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    match format {
        image::ImageFormat::Jpeg => image::DynamicImage::ImageRgba8(rgba.clone()).into_rgb8().write_to(&mut out, format).unwrap(),
        image::ImageFormat::WebP => {
            image::codecs::webp::WebPEncoder::new_lossless(&mut out)
                .write_image(rgba, rgba.width(), rgba.height(), image::ExtendedColorType::Rgba8)
                .unwrap();
        }
        _ => rgba.write_to(&mut out, format).unwrap(),
    }
    out.into_inner()
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn decoded(path: &Path) -> image::RgbaImage {
    image::open(path).unwrap().into_rgba8()
}

/// A TIFF IFD of ENTRIES (tag, type, count, value-or-offset) at OFFSET in
/// little-endian TIFF, with the next IFD's offset NEXT.
fn ifd(entries: &[(u16, u16, u32, u32)], next: u32) -> Vec<u8> {
    let mut out = (entries.len() as u16).to_le_bytes().to_vec();
    for &(tag, kind, count, value) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&next.to_le_bytes());
    out
}

/// EXIF (a TIFF stream) with ORIENTATION and a GPS IFD whose latitude
/// reference is "N", plus an ASCII description `SECRET-PLACE`.
fn exif(orientation: u16) -> Vec<u8> {
    let description = b"SECRET-PLACE\0";
    // Header 8, IFD0 at 8 with 3 entries (2 + 36 + 4 = 42 bytes) to 50,
    // the description at 50, the GPS IFD after it.
    let gps_at = 50 + description.len() as u32;
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend(ifd(&[(0x010e, 2, description.len() as u32, 50), (0x0112, 3, 1, u32::from(orientation)), (0x8825, 4, 1, gps_at)], 0));
    out.extend_from_slice(description);
    out.extend(ifd(&[(0x0001, 2, 2, u32::from_le_bytes(*b"N\0\0\0"))], 0));
    out
}

const XMP: &[u8] = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">SECRET-XMP</x:xmpmeta>";

fn jpeg_with_metadata(rgba: &image::RgbaImage, orientation: u16) -> Vec<u8> {
    let jpeg = encoded(rgba, image::ImageFormat::Jpeg);
    let segment = |marker: u8, payload: &[u8]| {
        let mut s = vec![0xff, marker];
        s.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        s.extend_from_slice(payload);
        s
    };
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend(exif(orientation));
    let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    xmp.extend_from_slice(XMP);
    let mut out = jpeg[..2].to_vec();
    out.extend(segment(0xe1, &app1));
    out.extend(segment(0xe1, &xmp));
    out.extend(segment(0xed, b"Photoshop 3.0\0SECRET-IPTC"));
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    let crc = crc32(&body);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xedb8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

fn png_with_metadata(rgba: &image::RgbaImage) -> Vec<u8> {
    let png = encoded(rgba, image::ImageFormat::Png);
    // After the signature and IHDR (8 + 25 bytes).
    let mut out = png[..33].to_vec();
    out.extend(png_chunk(b"eXIf", &exif(1)));
    out.extend(png_chunk(b"tEXt", b"Comment\0SECRET-TEXT"));
    out.extend(png_chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0SECRET-ITXT"));
    out.extend_from_slice(&png[33..]);
    out
}

fn webp_with_metadata(rgba: &image::RgbaImage) -> Vec<u8> {
    let webp = encoded(rgba, image::ImageFormat::WebP);
    let vp8l = &webp[12..];
    let riff_chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut c = kind.to_vec();
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(data);
        if data.len() % 2 == 1 {
            c.push(0);
        }
        c
    };
    let (w, h) = (rgba.width() - 1, rgba.height() - 1);
    let mut vp8x = vec![0x08 | 0x04, 0, 0, 0];
    vp8x.extend_from_slice(&w.to_le_bytes()[..3]);
    vp8x.extend_from_slice(&h.to_le_bytes()[..3]);
    let mut body = b"WEBP".to_vec();
    body.extend(riff_chunk(b"VP8X", &vp8x));
    body.extend_from_slice(vp8l);
    body.extend(riff_chunk(b"EXIF", &exif(1)));
    body.extend(riff_chunk(b"XMP ", XMP));
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

fn gif_with_metadata(rgba: &image::RgbaImage) -> Vec<u8> {
    let gif = encoded(rgba, image::ImageFormat::Gif);
    let table = |flags: u8| if flags & 0x80 != 0 { 3 << ((flags & 7) + 1) } else { 0 };
    let at = 13 + table(gif[10]);
    let mut out = gif[..at].to_vec();
    out.extend_from_slice(&[0x21, 0xfe, 13]);
    out.extend_from_slice(b"SECRET-COMMEN");
    out.push(0);
    out.extend_from_slice(&[0x21, 0xff, 11]);
    out.extend_from_slice(b"XMP DataXMP");
    out.push(10);
    out.extend_from_slice(b"SECRET-APP");
    out.push(0);
    out.extend_from_slice(&gif[at..]);
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn max_edge_fits_the_longest_side() {
    let dir = tempfile::tempdir().unwrap();
    let src = write(dir.path(), "wide.png", &encoded(&picture(300, 200), image::ImageFormat::Png));
    let out = prepare(&src, &dir.path().join("store"), &Options { max_edge: Some(100), ..Default::default() }).unwrap();
    let (width, height) = out.size.unwrap();
    assert_eq!(width, 100);
    assert!((66..=67).contains(&height), "{height}");
    assert_eq!(decoded(&out.stored.path).dimensions(), (width, height));
    assert_eq!(out.mime, Some("image/png"));
    // Within the edge: the file itself, unchanged.
    let out = prepare(&src, &dir.path().join("store"), &Options { max_edge: Some(400), ..Default::default() }).unwrap();
    assert_eq!(std::fs::read(&out.stored.path).unwrap(), std::fs::read(&src).unwrap());
}

#[test]
fn other_formats_and_a_still_animation_become_png() {
    let dir = tempfile::tempdir().unwrap();
    let options = Options { formats: vec!["png".into(), "jpeg".into()], still: true, ..Default::default() };
    let rgba = picture(30, 20);
    let mut frames = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut frames);
        encoder.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
        for shade in [0u8, 200] {
            let frame = image::RgbaImage::from_pixel(30, 20, image::Rgba([shade, 0, 0, 255]));
            encoder.encode_frame(image::Frame::new(frame)).unwrap();
        }
    }
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="30"><rect width="60" height="30" fill="#08f"/></svg>"##;
    for (name, bytes, size) in [
        ("a.tiff", encoded(&rgba, image::ImageFormat::Tiff), (30, 20)),
        ("a.bmp", encoded(&rgba, image::ImageFormat::Bmp), (30, 20)),
        ("a.svg", svg.to_vec(), (60, 30)),
        ("anim.gif", frames.clone(), (30, 20)),
    ] {
        let src = write(dir.path(), name, &bytes);
        let out = prepare(&src, &dir.path().join("store"), &options).unwrap();
        assert_eq!((out.mime, out.size), (Some("image/png"), Some(size)), "{name}");
        assert!(out.stored.path.extension().is_some_and(|e| e == "png"), "{name}");
        let pixels = decoded(&out.stored.path);
        assert_eq!(pixels.dimensions(), size, "{name}");
        if name == "anim.gif" {
            assert_eq!(pixels.get_pixel(0, 0)[0], 0, "the first frame");
        }
    }
    // Not still: an animation the rule takes goes as it is.
    let src = write(dir.path(), "anim2.gif", &frames);
    let out = prepare(&src, &dir.path().join("store"), &Options::default()).unwrap();
    assert_eq!(std::fs::read(&out.stored.path).unwrap(), frames);
}

#[test]
fn exif_orientation_six_is_applied() {
    let dir = tempfile::tempdir().unwrap();
    let src = write(dir.path(), "turned.jpg", &jpeg_with_metadata(&picture(40, 20), 6));
    let out = prepare(&src, &dir.path().join("store"), &Options::default()).unwrap();
    assert_eq!(out.size, Some((20, 40)));
    assert_eq!(out.mime, Some("image/jpeg"));
    let bytes = std::fs::read(&out.stored.path).unwrap();
    assert!(!contains(&bytes, b"Exif\0\0") && !contains(&bytes, b"SECRET"), "re-encoded without metadata");
    assert_eq!(decoded(&out.stored.path).dimensions(), (20, 40));
}

#[test]
fn stripping_drops_metadata_and_keeps_the_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let rgba = picture(24, 16);
    for (name, bytes) in [
        ("m.jpg", jpeg_with_metadata(&rgba, 1)),
        ("m.png", png_with_metadata(&rgba)),
        ("m.webp", webp_with_metadata(&rgba)),
        ("m.gif", gif_with_metadata(&rgba)),
    ] {
        assert!(contains(&bytes, b"SECRET"), "{name}: the fixture carries metadata");
        let src = write(dir.path(), name, &bytes);
        let out = prepare(&src, &dir.path().join("store"), &Options { strip: true, ..Default::default() }).unwrap();
        let stripped = std::fs::read(&out.stored.path).unwrap();
        assert!(!contains(&stripped, b"SECRET"), "{name}: metadata left");
        assert!(!contains(&stripped, b"Exif\0\0") && !contains(&stripped, b"eXIf") && !contains(&stripped, b"XMP "), "{name}");
        assert_eq!(decoded(&out.stored.path), decoded(&src), "{name}: the same pixels");
        assert!(stripped.len() < bytes.len(), "{name}");
    }
}

#[test]
fn a_gif_keeps_its_looping() {
    let dir = tempfile::tempdir().unwrap();
    let mut frames = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut frames);
        encoder.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
        encoder.encode_frame(image::Frame::new(picture(8, 8))).unwrap();
        encoder.encode_frame(image::Frame::new(picture(8, 8))).unwrap();
    }
    assert!(contains(&frames, b"NETSCAPE2.0"));
    let src = write(dir.path(), "loop.gif", &frames);
    let out = prepare(&src, &dir.path().join("store"), &Options { strip: true, ..Default::default() }).unwrap();
    assert!(contains(&std::fs::read(&out.stored.path).unwrap(), b"NETSCAPE2.0"));
}

#[test]
fn max_image_size_refuses_as_the_display_does() {
    let dir = tempfile::tempdir().unwrap();
    let src = write(dir.path(), "big.png", &encoded(&picture(300, 200), image::ImageFormat::Png));
    let refused = prepare(&src, &dir.path().join("store"), &Options { max_image_size: Some((299, 1000)), ..Default::default() });
    assert_eq!(refused, Err(romacs_display_protocol::image_diagnostic::ImageDiagnostic::InvalidSize.message()));
    assert!(prepare(&src, &dir.path().join("store"), &Options { max_image_size: Some((300, 200)), ..Default::default() }).is_ok());
}

#[test]
fn the_store_writes_atomically_and_keeps_one_of_identical_copies() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let src = write(dir.path(), "a.png", &encoded(&picture(10, 10), image::ImageFormat::Png));
    let copy = write(dir.path(), "copy-of-a.png", &std::fs::read(&src).unwrap());
    let first = prepare(&src, &store, &Options::default()).unwrap();
    let second = prepare(&copy, &store, &Options::default()).unwrap();
    assert_eq!(first.stored, second.stored);
    let names: Vec<_> = std::fs::read_dir(&store).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, [first.stored.path.file_name().unwrap().to_owned()], "one file, no temporary left");
    assert_eq!(first.stored.path.file_stem().unwrap().to_string_lossy(), first.stored.sha256);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&first.stored.path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(&store).unwrap().permissions().mode() & 0o777, 0o700);
}

#[test]
fn a_non_image_goes_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let src = write(dir.path(), "notes.TXT", "héllo\n".as_bytes());
    let out = prepare(&src, &dir.path().join("store"), &Options { strip: true, text: true, ..Default::default() }).unwrap();
    assert_eq!((out.mime, out.size, out.utf8), (None, None, Some(true)));
    assert_eq!(std::fs::read(&out.stored.path).unwrap(), "héllo\n".as_bytes());
    assert!(out.stored.path.extension().is_some_and(|e| e == "txt"));
    let src = write(dir.path(), "blob.bin", &[0xff, 0xfe, 0x00]);
    assert_eq!(prepare(&src, &dir.path().join("store"), &Options { text: true, ..Default::default() }).unwrap().utf8, Some(false));
}

#[test]
fn blobs_reduces_every_image_block_of_a_line() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("attachments");
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let small = vec![7u8; 2048];
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let line = serde_json::json!({"type": "user", "message": {"content": [
        {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64(&big)}},
        {"type": "tool_result", "content": [{"type": "image", "mimeType": "image/jpeg", "data": b64(&small)}]},
        {"type": "text", "text": "the data key alone is kept", "data": "not an image"}]}}).to_string();
    let reduced = blobs::reduce_line(&line, &store).unwrap();
    assert!(reduced.len() < 1024, "{} bytes", reduced.len());
    let value: serde_json::Value = serde_json::from_str(&reduced).unwrap();
    let content = &value["message"]["content"];
    let reference = |v: &serde_json::Value| (v["romacs-file"].as_str().unwrap().to_owned(), v["bytes"].as_u64().unwrap());
    let (big_ref, big_bytes) = reference(&content[0]["source"]["data"]);
    let (small_ref, small_bytes) = reference(&content[1]["content"][0]["data"]);
    assert_eq!((big_bytes, small_bytes), (3_000_000, 2048));
    assert!(big_ref.starts_with("attachments/") && big_ref.ends_with(".png"));
    assert!(small_ref.ends_with(".jpg"));
    assert_eq!(std::fs::read(dir.path().join(&big_ref)).unwrap(), big);
    assert_eq!(std::fs::read(dir.path().join(&small_ref)).unwrap(), small);
    assert_eq!(content[2]["data"], "not an image");
    // A line without the keys, or not JSON, comes back as it is.
    for plain in [r#"{"type":"text","data":"x"}"#, "not json \"media_type\""] {
        assert_eq!(blobs::reduce_line(plain, &store).unwrap(), plain);
    }
}

#[test]
fn sweep_removes_dead_staging_and_evicts_least_recently_used() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let staging = root.join("staging");
    let live = std::process::id().to_string();
    // A pid above the kernel's maximum is never alive.
    for name in [live.as_str(), "4194999", "not-a-pid"] {
        std::fs::create_dir_all(staging.join(name)).unwrap();
        std::fs::write(staging.join(name).join("f"), b"x").unwrap();
    }
    let received = root.join("received");
    std::fs::create_dir_all(received.join(".partial")).unwrap();
    let now = std::time::SystemTime::now();
    for i in 0..10u64 {
        let path = received.join(format!("{i}.png"));
        std::fs::write(&path, vec![0u8; 100]).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(now - std::time::Duration::from_secs(1000 - i * 10)).unwrap();
    }
    // Under both bounds: nothing goes.
    assert_eq!(sweep::sweep(root, 2000, 20).unwrap(), sweep::Totals { bytes: 1000, files: 10 });
    // Over the byte budget: cut to 80% of it, oldest first.
    assert_eq!(sweep::sweep(root, 900, 20).unwrap(), sweep::Totals { bytes: 700, files: 7 });
    let mut left: Vec<_> = std::fs::read_dir(&received).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    left.sort();
    assert_eq!(left, [".partial", "3.png", "4.png", "5.png", "6.png", "7.png", "8.png", "9.png"]);
    // Over the file count: cut to 80% of it.
    assert_eq!(sweep::sweep(root, 10_000, 5).unwrap(), sweep::Totals { bytes: 400, files: 4 });
    let mut staged: Vec<_> = std::fs::read_dir(&staging).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    staged.sort();
    let mut expected = vec![live, "not-a-pid".to_owned()];
    expected.sort();
    assert_eq!(staged, expected);
}

#[test]
fn the_tool_prints_one_json_object() {
    let dir = tempfile::tempdir().unwrap();
    let src = write(dir.path(), "a.png", &encoded(&picture(300, 200), image::ImageFormat::Png));
    let tool = env!("CARGO_BIN_EXE_romacs-attach-tool");
    let run = |args: &[&str]| std::process::Command::new(tool).args(args).output().unwrap();
    let store = dir.path().join("store");
    let out = run(&["prepare", src.to_str().unwrap(), "--into", store.to_str().unwrap(), "--max-edge", "150", "--strip"]);
    assert!(out.status.success());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!((value["width"].as_u64(), value["mime"].as_str()), (Some(150), Some("image/png")));
    let out = run(&["prepare", src.to_str().unwrap(), "--into", store.to_str().unwrap(), "--max-image-size", "10,10"]);
    assert!(!out.status.success());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(value["error"].as_str().unwrap().contains("max-image-size"));
}

/// Text the signature sniff would take for an image (a C header's
/// `#define', prose quoting `<svg', a line starting `P1') goes byte for
/// byte when it does not decode as one.
#[test]
fn text_that_looks_like_an_image_goes_as_text() {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in [("config.h", "#define VERSION 3\nint main(void);\n"),
                         ("notes.md", "Embed it as <svg width=\"10\"> in the page.\n"),
                         ("plan.txt", "P1 is the first phase.\n")] {
        let src = write(dir.path(), name, text.as_bytes());
        let out = prepare(&src, &dir.path().join("store"), &Options { strip: true, text: true, ..Default::default() }).unwrap();
        assert_eq!((out.mime, out.utf8), (None, Some(true)), "{name}");
        assert_eq!(std::fs::read(&out.stored.path).unwrap(), text.as_bytes(), "{name}");
    }
}

/// A format the rule takes but no stripper reads (TIFF, BMP) is converted
/// when stripping; a conversion becomes the rule's first format.
#[test]
fn stripping_converts_what_it_cannot_strip_and_conversion_follows_the_rule() {
    let dir = tempfile::tempdir().unwrap();
    let src = write(dir.path(), "a.tiff", &encoded(&picture(12, 8), image::ImageFormat::Tiff));
    let formats = |f: &[&str]| f.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let out = prepare(&src, &dir.path().join("store"),
                      &Options { strip: true, formats: formats(&["png", "tiff"]), ..Default::default() }).unwrap();
    assert_eq!((out.mime, out.size), (Some("image/png"), Some((12, 8))));
    let out = prepare(&src, &dir.path().join("store"), &Options { formats: formats(&["webp", "png"]), ..Default::default() }).unwrap();
    assert_eq!(out.mime, Some("image/webp"));
    assert_eq!(decoded(&out.stored.path), decoded(&src));
}

/// A real XBM over `max-image-size' is refused, not sent as text.
#[test]
fn an_oversized_textual_image_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let xbm = "#define icon_width 2\n#define icon_height 1\nstatic unsigned char icon_bits[] = { 0x01 };\n";
    let src = write(dir.path(), "icon.xbm", xbm.as_bytes());
    let refused = prepare(&src, &dir.path().join("store"), &Options { max_image_size: Some((1, 1)), ..Default::default() });
    assert_eq!(refused, Err(romacs_display_protocol::image_diagnostic::ImageDiagnostic::InvalidSize.message()));
    let out = prepare(&src, &dir.path().join("store"), &Options::default()).unwrap();
    assert_eq!((out.mime, out.size), (Some("image/png"), Some((2, 1))));
}
