//! The artifact's byte encoding: LEB128 unsigned integers, zigzag signed
//! ones, and length-prefixed byte strings. Reading never trusts a length:
//! every read is bounded by the bytes left.
use crate::Result;

#[derive(Default)]
pub(super) struct Writer {
    pub(super) bytes: Vec<u8>,
}
impl Writer {
    pub(super) fn byte(&mut self, b: u8) {
        self.bytes.push(b);
    }
    pub(super) fn u(&mut self, mut n: u64) {
        loop {
            let low = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                self.bytes.push(low);
                return;
            }
            self.bytes.push(low | 0x80);
        }
    }
    pub(super) fn n(&mut self, n: usize) {
        self.u(n as u64);
    }
    pub(super) fn i(&mut self, n: i64) {
        self.u(((n << 1) ^ (n >> 63)) as u64);
    }
    pub(super) fn data(&mut self, b: &[u8]) {
        self.n(b.len());
        self.bytes.extend_from_slice(b);
    }
    pub(super) fn str(&mut self, s: &str) {
        self.data(s.as_bytes());
    }
    pub(super) fn raw(&mut self, b: &[u8]) {
        self.bytes.extend_from_slice(b);
    }
}

pub(super) struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Reader { data, at: 0 }
    }
    pub(super) fn done(&self) -> bool {
        self.at == self.data.len()
    }
    pub(super) fn byte(&mut self) -> Result<u8> {
        let b = *self.data.get(self.at).ok_or("compiled Lisp artifact truncated")?;
        self.at += 1;
        Ok(b)
    }
    pub(super) fn u(&mut self) -> Result<u64> {
        let mut n = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            n |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(n);
            }
        }
        Err("compiled Lisp artifact integer overflow".into())
    }
    pub(super) fn n(&mut self) -> Result<usize> {
        usize::try_from(self.u()?).map_err(|_| "compiled Lisp artifact integer overflow".into())
    }
    /// A count of items each at least one byte long: never more than the
    /// bytes left, so a corrupt count cannot ask for a huge allocation.
    pub(super) fn count(&mut self) -> Result<usize> {
        let n = self.n()?;
        if n > self.data.len() - self.at {
            return Err("compiled Lisp artifact count exceeds its data".into());
        }
        Ok(n)
    }
    pub(super) fn i(&mut self) -> Result<i64> {
        let u = self.u()?;
        Ok(((u >> 1) as i64) ^ -((u & 1) as i64))
    }
    pub(super) fn data(&mut self) -> Result<&'a [u8]> {
        let n = self.count()?;
        let out = &self.data[self.at..self.at + n];
        self.at += n;
        Ok(out)
    }
    pub(super) fn str(&mut self) -> Result<&'a str> {
        std::str::from_utf8(self.data()?).map_err(|_| "compiled Lisp artifact name is not UTF-8".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integers_and_strings_round_trip() {
        let mut w = Writer::default();
        for n in [0u64, 1, 127, 128, 300, u64::MAX] {
            w.u(n);
        }
        for n in [0i64, -1, 1, i64::MIN, i64::MAX] {
            w.i(n);
        }
        w.str("défaut");
        let mut r = Reader::new(&w.bytes);
        for n in [0u64, 1, 127, 128, 300, u64::MAX] {
            assert_eq!(r.u().unwrap(), n);
        }
        for n in [0i64, -1, 1, i64::MIN, i64::MAX] {
            assert_eq!(r.i().unwrap(), n);
        }
        assert_eq!(r.str().unwrap(), "défaut");
        assert!(r.done());
        assert!(Reader::new(&[0x80]).u().is_err());
        assert!(Reader::new(&[5, 1]).data().is_err());
    }
}
