//! A payload's fields: little-endian integers, and byte strings with a
//! 32-bit length before them. No padding, no self-description: a class's
//! version says which fields a kind has.

use crate::Error;

/// Appends fields to a payload.
#[derive(Default)]
pub struct Writer(pub Vec<u8>);

macro_rules! put {
    ($($name:ident: $type:ty),*) => {$(
        #[must_use]
        pub fn $name(mut self, value: $type) -> Self {
            self.0.extend_from_slice(&value.to_le_bytes());
            self
        }
    )*};
}

impl Writer {
    put!(u8: u8, u16: u16, u32: u32, u64: u64, i64: i64);

    /// # Panics
    /// When VALUE is 4 GiB or more, which no record may carry.
    #[must_use]
    pub fn bytes(mut self, value: &[u8]) -> Self {
        let len = u32::try_from(value.len()).expect("field under 4 GiB");
        self.0.extend_from_slice(&len.to_le_bytes());
        self.0.extend_from_slice(value);
        self
    }

    #[must_use]
    pub fn str(self, value: &str) -> Self { self.bytes(value.as_bytes()) }

    #[must_use]
    pub fn finish(self) -> Vec<u8> { self.0 }
}

/// Reads fields from a payload, in the order they were written.
pub struct Reader<'a>(pub &'a [u8]);

macro_rules! take {
    ($($name:ident: $type:ty),*) => {$(
        /// # Errors
        /// [`Error::Truncated`] when the payload ends first.
        pub fn $name(&mut self) -> Result<$type, Error> {
            let (head, rest) = self.0.split_first_chunk().ok_or(Error::Truncated)?;
            self.0 = rest;
            Ok(<$type>::from_le_bytes(*head))
        }
    )*};
}

impl<'a> Reader<'a> {
    take!(u8: u8, u16: u16, u32: u32, u64: u64, i64: i64);

    /// # Errors
    /// [`Error::Truncated`] when the payload ends inside the field.
    pub fn bytes(&mut self) -> Result<&'a [u8], Error> {
        let len = self.u32()? as usize;
        let (head, rest) = self.0.split_at_checked(len).ok_or(Error::Truncated)?;
        self.0 = rest;
        Ok(head)
    }

    /// # Errors
    /// As [`Self::bytes`], and [`Error::Utf8`].
    pub fn str(&mut self) -> Result<&'a str, Error> { std::str::from_utf8(self.bytes()?).map_err(|_| Error::Utf8) }

    /// Whether every field was read. A newer version's extra fields are
    /// simply left: a reader takes what its version names.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_read_back_in_order() {
        let payload = Writer::default().u8(1).u16(2).u32(3).u64(4).i64(-5).bytes(b"\xff\x00").str("λ").finish();
        let mut reader = Reader(&payload);
        assert_eq!((reader.u8(), reader.u16(), reader.u32(), reader.u64(), reader.i64()), (Ok(1), Ok(2), Ok(3), Ok(4), Ok(-5)));
        assert_eq!(reader.bytes(), Ok(&b"\xff\x00"[..]));
        assert_eq!(reader.str(), Ok("λ"));
        assert!(reader.is_empty());
    }

    #[test]
    fn a_short_payload_is_an_error_not_a_panic() {
        let payload = Writer::default().str("hello").finish();
        for cut in 0..payload.len() {
            assert_eq!(Reader(&payload[..cut]).str(), Err(Error::Truncated), "cut at {cut}");
        }
        assert_eq!(Reader(&Writer::default().bytes(b"\xff").finish()).str(), Err(Error::Utf8));
        assert_eq!(Reader(&[]).u64(), Err(Error::Truncated));
    }
}
