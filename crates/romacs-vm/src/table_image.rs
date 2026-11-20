//! The restored image's entries of a table (`super::Table`), kept beside its
//! colours: the image is dense and never moves, so its entries are the indices
//! below `len`, and the remembered ones a bitmap.

/// The restored image's entries of a table: those below `len`, never freed
/// or moved; of them, the remembered, by bit. Empty in a process that
/// restored none, and in a builder.
#[derive(Clone, Default, romacs_snapshot::Snapshot)]
pub(super) struct ImageSet {
    pub(super) len: usize,
    pub(super) remembered: Vec<u64>,
}

impl ImageSet {
    /// Whether image entry I (below `len`) is remembered.
    #[inline]
    pub(super) fn remembered(&self, i: usize) -> bool {
        self.remembered[i / 64] >> (i % 64) & 1 != 0
    }
    /// Whether entry I is of the image and untouched since the restore.
    #[inline]
    pub(super) fn permanent(&self, i: usize) -> bool {
        i < self.len && !self.remembered(i)
    }
    /// Remember image entry I (below `len`).
    #[inline]
    pub(super) fn remember(&mut self, i: usize) {
        self.remembered[i / 64] |= 1 << (i % 64);
    }
    /// The permanent ones of the eight entries from BASE (a multiple of 8),
    /// each as its byte's low bit, as a colour word holds `IN_USE`.
    #[inline]
    pub(super) fn permanent_bytes(&self, base: usize) -> u64 {
        if base >= self.len {
            return 0;
        }
        let remembered = self.remembered[base / 64].to_le_bytes()[base % 64 / 8];
        let within = if self.len - base >= 8 { 0xFF } else { (1u8 << (self.len - base)) - 1 };
        spread(!remembered & within)
    }
}

/// By a byte's value, its eight bits, each as the low bit of its own byte.
const SPREAD: [u64; 256] = {
    let mut table = [0u64; 256];
    let mut bits = 0;
    while bits < 256 {
        let mut k = 0;
        while k < 8 {
            if bits >> k & 1 != 0 { table[bits] |= 1 << (8 * k); }
            k += 1;
        }
        bits += 1;
    }
    table
};

/// The eight bits of BITS, each as the low bit of its own byte.
#[inline]
fn spread(bits: u8) -> u64 {
    SPREAD[usize::from(bits)]
}
