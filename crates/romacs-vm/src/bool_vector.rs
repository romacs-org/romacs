//! Packed, pointer-free storage; unused high bits are always zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoolVector {
    pub(crate) len: usize,
    pub(crate) bytes: Vec<u8>,
}
romacs_snapshot::snapshot_struct!(BoolVector { len, bytes });
impl BoolVector {
    pub(crate) fn new(len: usize, init: bool) -> Result<Self, String> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len.div_ceil(8)).map_err(|_| "Memory exhausted--use M-x save-some-buffers then exit and restart Emacs")?;
        bytes.resize(len.div_ceil(8), if init { 255 } else { 0 });
        let mut out = Self { len, bytes };
        out.mask_padding();
        Ok(out)
    }
    pub(crate) fn mask_padding(&mut self) {
        if self.len % 8 != 0 {
            if let Some(last) = self.bytes.last_mut() { *last &= (1 << (self.len % 8)) - 1; }
        }
    }
    pub(crate) fn get(&self, index: usize) -> Option<bool> {
        (index < self.len).then(|| self.bytes[index / 8] & (1 << (index % 8)) != 0)
    }
    pub(crate) fn set(&mut self, index: usize, value: bool) {
        let mask = 1 << (index % 8);
        if value { self.bytes[index / 8] |= mask; } else { self.bytes[index / 8] &= !mask; }
    }
    pub(crate) fn printed(&self) -> String {
        let mut out = format!("#&{}\"", self.len);
        for &byte in &self.bytes {
            match byte {
                b'"' => out.push_str("\\\""),
                b'\\' => out.push_str("\\\\"),
                0..=127 => out.push(char::from(byte)),
                _ => out.push_str(&format!("\\{:03o}", byte)),
            }
        }
        out.push('"');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_printing_matches_pinned_gnu_bytes() {
        let mut bits = BoolVector::new(3, false).unwrap();
        bits.set(0, true);
        bits.set(2, true);
        assert_eq!(bits.printed(), "#&3\"\u{5}\"");
        assert_eq!(BoolVector::new(9, true).unwrap().printed(), "#&9\"\\377\u{1}\"");
    }
    #[test]
    fn packed_boundaries_and_padding() {
        for len in [0, 1, 7, 8, 9, 63, 64, 65] {
            let mut bits = BoolVector::new(len, true).unwrap();
            assert_eq!(bits.bytes.len(), len.div_ceil(8));
            assert_eq!(bits.bytes.iter().map(|b| b.count_ones() as usize).sum::<usize>(), len);
            assert_eq!(bits.get(len), None);
            for i in 0..len { bits.set(i, i % 2 == 0); }
            for i in 0..len { assert_eq!(bits.get(i), Some(i % 2 == 0)); }
        }
    }
}
