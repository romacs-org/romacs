//! The fixed header and the records it frames.

use crate::Error;

/// Bytes of a header: sequence number, class, kind, payload length.
pub const HEADER: usize = 16;

/// The largest inline payload. Larger payloads go by [`crate::region`].
pub const MAX_PAYLOAD: u32 = 16 << 20;

/// One record. `seq` counts up from 1 per sender and direction; `class`
/// says whose `kind` it is ([`crate::hello`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub seq: u64,
    pub class: u16,
    pub kind: u16,
    pub payload: Vec<u8>,
}

impl Record {
    /// Append this record's wire form to OUT.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        encode(self.seq, self.class, self.kind, &self.payload, out);
    }
}

/// Append one record's wire form to OUT.
///
/// # Panics
/// When PAYLOAD is over [`MAX_PAYLOAD`]: the sender's error, never a peer's.
pub fn encode(seq: u64, class: u16, kind: u16, payload: &[u8], out: &mut Vec<u8>) {
    let len = u32::try_from(payload.len()).ok().filter(|len| *len <= MAX_PAYLOAD).expect("payload within MAX_PAYLOAD");
    out.reserve(HEADER + payload.len());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&class.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
}

/// Records out of a byte stream that arrives in pieces.
#[derive(Default)]
pub struct Decoder {
    bytes: Vec<u8>,
    start: usize,
}

impl Decoder {
    /// Add bytes the stream delivered.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.start == self.bytes.len() {
            self.bytes.clear();
            self.start = 0;
        }
        self.bytes.extend_from_slice(bytes);
    }

    /// The next whole record, or `None` until more bytes arrive.
    ///
    /// # Errors
    /// [`Error::Oversized`] when a header announces too long a payload;
    /// the stream is then unusable.
    pub fn pop(&mut self) -> Result<Option<Record>, Error> {
        let pending = &self.bytes[self.start..];
        let Some((header, rest)) = pending.split_first_chunk::<HEADER>() else {
            return Ok(None);
        };
        let mut fields = crate::Reader(header);
        let (seq, class, kind, len) = (fields.u64()?, fields.u16()?, fields.u16()?, fields.u32()?);
        if len > MAX_PAYLOAD {
            return Err(Error::Oversized(len));
        }
        let Some(payload) = rest.get(..len as usize) else {
            return Ok(None);
        };
        let record = Record { seq, class, kind, payload: payload.to_vec() };
        self.start += HEADER + len as usize;
        // Keep the buffer from growing with the stream's whole history.
        if self.start > 1 << 16 && self.start * 2 > self.bytes.len() {
            self.bytes.drain(..self.start);
            self.start = 0;
        }
        Ok(Some(record))
    }

    /// Bytes received that are not yet a whole record.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.bytes.len() - self.start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_survives_the_wire_in_any_split() {
        let records = [
            Record { seq: 1, class: 0, kind: 7, payload: vec![] },
            Record { seq: u64::MAX, class: 9, kind: u16::MAX, payload: b"payload".to_vec() },
        ];
        let mut wire = Vec::new();
        for record in &records {
            record.encode_into(&mut wire);
        }
        for split in 0..=wire.len() {
            let mut decoder = Decoder::default();
            let mut seen = Vec::new();
            for piece in [&wire[..split], &wire[split..]] {
                decoder.push(piece);
                while let Some(record) = decoder.pop().unwrap() {
                    seen.push(record);
                }
            }
            assert_eq!(seen, records, "split at {split}");
            assert_eq!(decoder.pending(), 0);
        }
    }

    #[test]
    fn the_header_is_sixteen_little_endian_bytes() {
        let mut wire = Vec::new();
        encode(0x0102_0304_0506_0708, 0x0a0b, 0x0c0d, b"x", &mut wire);
        assert_eq!(wire, [8, 7, 6, 5, 4, 3, 2, 1, 0x0b, 0x0a, 0x0d, 0x0c, 1, 0, 0, 0, b'x']);
    }

    #[test]
    fn an_oversized_length_is_refused_before_it_is_awaited() {
        let mut decoder = Decoder::default();
        let mut wire = vec![0; 12];
        wire.extend_from_slice(&(MAX_PAYLOAD + 1).to_le_bytes());
        decoder.push(&wire);
        assert_eq!(decoder.pop(), Err(Error::Oversized(MAX_PAYLOAD + 1)));
    }

    #[test]
    fn the_buffer_does_not_keep_what_was_read() {
        let mut decoder = Decoder::default();
        let mut wire = Vec::new();
        encode(1, 0, 0, &[0; 1000], &mut wire);
        for _ in 0..1000 {
            decoder.push(&wire);
            decoder.push(&wire[..5]);
            assert!(decoder.pop().unwrap().is_some());
            decoder.push(&wire[5..]);
            assert!(decoder.pop().unwrap().is_some());
        }
        assert!(decoder.bytes.capacity() < 1 << 20, "{}", decoder.bytes.capacity());
    }
}
