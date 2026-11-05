//! Metadata out of an image without re-encoding it (attachments.md §2.3):
//! EXIF (location included), XMP and text go; the pixels' bytes stay as
//! they are.  Each function answers None for bytes that are not the format
//! they claim, which the caller then refuses rather than passes on.

/// BYTES of FORMAT (a Lisp image type) without their metadata, or None when
/// the format has none to strip here or the bytes are malformed.
#[must_use]
pub fn strip(format: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    match format {
        "jpeg" => jpeg(bytes),
        "png" => png(bytes),
        "webp" => webp(bytes),
        "gif" => gif(bytes),
        _ => None,
    }
}

fn be16(bytes: &[u8], at: usize) -> Option<usize> {
    Some(usize::from(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?)))
}

fn be32(bytes: &[u8], at: usize) -> Option<usize> {
    usize::try_from(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?)).ok()
}

fn le32(bytes: &[u8], at: usize) -> Option<usize> {
    usize::try_from(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?)).ok()
}

/// JPEG without its APP1 (EXIF, XMP) and APP13 (IPTC) segments; the scan
/// from the first SOS on is copied as it is.
fn jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut out = bytes[..2].to_vec();
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xff {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        match marker {
            // Fill bytes before a marker.
            0xff => at += 1,
            // The entropy-coded data and the rest: copied whole.
            0xda | 0xd9 => {
                out.extend_from_slice(&bytes[at..]);
                return Some(out);
            }
            0x01 | 0xd0..=0xd7 => {
                out.extend_from_slice(&bytes[at..at + 2]);
                at += 2;
            }
            _ => {
                let end = at + 2 + be16(bytes, at + 2)?;
                let segment = bytes.get(at..end)?;
                if marker != 0xe1 && marker != 0xed {
                    out.extend_from_slice(segment);
                }
                at = end;
            }
        }
    }
}

/// PNG without its `eXIf`, `tEXt`, `iTXt` and `zTXt` chunks.
fn png(bytes: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return None;
    }
    let mut out = SIGNATURE.to_vec();
    let mut at = SIGNATURE.len();
    while at < bytes.len() {
        let end = at + 12 + be32(bytes, at)?;
        let chunk = bytes.get(at..end)?;
        if !matches!(&chunk[4..8], b"eXIf" | b"tEXt" | b"iTXt" | b"zTXt") {
            out.extend_from_slice(chunk);
        }
        at = end;
    }
    Some(out)
}

/// WebP without its `EXIF` and `XMP ` chunks, the VP8X flags that announce
/// them cleared and the RIFF size made again.
fn webp(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    let mut chunks = Vec::new();
    let mut at = 12;
    let end = (8 + le32(bytes, 4)?).min(bytes.len());
    while at + 8 <= end {
        let size = le32(bytes, at + 4)?;
        let next = at + 8 + size + (size & 1);
        let chunk = bytes.get(at..next.min(end))?;
        match &chunk[..4] {
            b"EXIF" | b"XMP " => {}
            b"VP8X" => {
                let mut chunk = chunk.to_vec();
                *chunk.get_mut(8)? &= !(0x08 | 0x04);
                chunks.extend_from_slice(&chunk);
            }
            _ => chunks.extend_from_slice(chunk),
        }
        at = next;
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&u32::try_from(4 + chunks.len()).ok()?.to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&chunks);
    Some(out)
}

/// The data sub-blocks from AT: where they end (past the terminator) and
/// the first one.
fn sub_blocks(bytes: &[u8], mut at: usize) -> Option<(usize, &[u8])> {
    let mut first: Option<&[u8]> = None;
    loop {
        let n = usize::from(*bytes.get(at)?);
        at += 1;
        if n == 0 {
            return Some((at, first.unwrap_or_default()));
        }
        let block = bytes.get(at..at + n)?;
        first.get_or_insert(block);
        at += n;
    }
}

/// GIF without its comments and application extensions other than
/// NETSCAPE2.0's looping.
fn gif(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(b"GIF8") {
        return None;
    }
    let table = |flags: u8| if flags & 0x80 != 0 { 3 << ((flags & 7) + 1) } else { 0 };
    let mut at = 13 + table(*bytes.get(10)?);
    let mut out = bytes.get(..at)?.to_vec();
    loop {
        match *bytes.get(at)? {
            0x3b => {
                out.push(0x3b);
                return Some(out);
            }
            0x2c => {
                let data = at + 10 + table(*bytes.get(at + 9)?);
                // The LZW minimum code size, then the image's sub-blocks.
                let (end, _) = sub_blocks(bytes, data + 1)?;
                out.extend_from_slice(bytes.get(at..end)?);
                at = end;
            }
            0x21 => {
                let label = *bytes.get(at + 1)?;
                let (end, first) = sub_blocks(bytes, at + 2)?;
                let keep = match label {
                    0xfe => false,
                    0xff => first == b"NETSCAPE2.0",
                    _ => true,
                };
                if keep {
                    out.extend_from_slice(bytes.get(at..end)?);
                }
                at = end;
            }
            _ => return None,
        }
    }
}
