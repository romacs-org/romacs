//! One range's sweep of a table (`super::RawTable::sweep_range`): the
//! entries in use that no mark reached are freed, the rest made old, eight
//! colours at a time, and the restored image's untouched entries 64 at a time.
use super::{unreached, ImageSet, FREE, IN_USE, LOW_BITS, OLD};

/// A sweep of one range of a table (`RawTable::sweep_range`): its colours
/// and entries, the range's first index, and what it has counted so far.
pub(super) struct Sweep<'a, T, F> {
    pub(super) colors: &'a mut [u8],
    pub(super) entries: &'a mut [T],
    pub(super) base: usize,
    pub(super) sticky: u8,
    pub(super) old_too: u64,
    pub(super) dead: u8,
    pub(super) quarantine: bool,
    pub(super) destroy: &'a mut F,
    pub(super) freed: usize,
    pub(super) last: usize,
}

impl<T, F: FnMut(usize, &mut T)> Sweep<'_, T, F> {
    /// Sweep the eight entries (fewer at TO) from AT, which lie below the
    /// image's end when IMAGE, the image's entries being the set's: the next
    /// entry to sweep.
    #[inline]
    pub(super) fn step<const IMAGE: bool>(&mut self, at: usize, to: usize, image: &ImageSet) -> usize {
        let (colors, base) = (&mut *self.colors, self.base);
        // The image's untouched entries, 64 at a time: a block whose
        // remembered word is empty and whose colours are all OLD | IN_USE
        // (as the capture left them; a store would have remembered one)
        // keeps every entry and writes nothing.
        if IMAGE && (base + at).is_multiple_of(64) && at + 64 <= to && image.remembered[(base + at) / 64] == 0
            && colors[at..at + 64].chunks_exact(8).all(|word| u64::from_le_bytes(word.try_into().expect("eight colours")) == LOW_BITS * u64::from(OLD | IN_USE))
        {
            self.last = base + at + 64;
            return at + 64;
        }
        let end = (at + 8).min(to);
        // Eight entries with none dead, by far the commonest case when the
        // heap is mostly live: make the ones in use old in one store, or in
        // none when they are so already, as the old are to a minor.
        if let Ok(bytes) = <[u8; 8]>::try_from(&colors[at..end]) {
            let word = u64::from_le_bytes(bytes);
            let in_use = word & LOW_BITS;
            let permanent = if IMAGE { image.permanent_bytes(base + at) } else { 0 };
            let reached = word >> 7 | (word >> 4 & self.old_too) | permanent;
            if in_use & !reached == 0 {
                // Each byte in use becomes OLD | IN_USE; the rest stay. An
                // untouched image entry is so already: nothing is written
                // where a word holds only those.
                let clean = (word & !(in_use * 0xFF)) | (in_use * u64::from(OLD | IN_USE));
                if clean != word { colors[at..end].copy_from_slice(&clean.to_le_bytes()); }
                if clean != 0 {
                    self.last = base + end - clean.leading_zeros() as usize / 8;
                }
                return end;
            }
        }
        for (i, (color, entry)) in (base + at..).zip(colors[at..end].iter_mut().zip(&mut self.entries[at..end])) {
            if unreached(*color, self.sticky) && !(IMAGE && image.permanent(i)) {
                (self.destroy)(i, entry);
                *color = self.dead;
                self.freed += 1;
                if self.quarantine { self.last = i + 1; }
            } else if *color != FREE {
                if *color & IN_USE != 0 && *color != OLD | IN_USE { *color = OLD | IN_USE; }
                self.last = i + 1;
            }
        }
        end
    }
}
