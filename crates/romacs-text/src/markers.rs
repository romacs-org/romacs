use super::{BytePos, CharPos, Edit, Text, TextPos};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkerId {
    slot: u32,
    generation: u32,
}
#[derive(Clone)]
struct Slot {
    generation: u32,
    dense: u32,
}
#[derive(Clone, Default)]
pub(super) struct Markers {
    byte: Vec<usize>,
    chars: Vec<usize>,
    after: Vec<usize>,
    owner: Vec<u32>,
    slots: Vec<Slot>,
    free: Vec<u32>,
}
// Vec-backed text positions fit nonnegative i64; their difference cannot overflow.
// Signed masks allow the generic-target loop to vectorize.
#[inline(never)]
fn insert_adjust(byte: &mut [usize], chars: &mut [usize], after: &[usize], e: Edit, bm: bool) {
    let f = e.at.byte.0;
    for ((b, c), after) in byte.iter_mut().zip(chars).zip(after) {
        let gt = ((f as i64).wrapping_sub(*b as i64) >> 63) as usize;
        let eq = 0usize.wrapping_sub((*b == f) as usize);
        let active = 0usize.wrapping_sub(((*after | bm as usize) != 0) as usize);
        let mask = gt | (eq & active);
        *b = b.wrapping_add(mask & e.ins.0);
        *c = c.wrapping_add(mask & e.ins.1);
    }
}
impl Markers {
    pub(super) fn reindex(&mut self, text: &Text) {
        for (byte, chars) in self.byte.iter_mut().zip(&mut self.chars) {
            let at = text.floor_char_boundary(BytePos(*byte));
            *byte = at.0;
            *chars = text.char_of_byte(at).0;
        }
    }

    pub(super) fn len(&self) -> usize {
        self.byte.len()
    }
    fn lookup(&self, m: MarkerId) -> Option<usize> {
        let s = self.slots.get(m.slot as usize)?;
        (s.generation == m.generation && s.dense != u32::MAX).then_some(s.dense as usize)
    }
    pub(super) fn new_marker(&mut self, b: usize, c: usize, after: bool) -> MarkerId {
        let dense = u32::try_from(self.len()).expect("marker capacity exhausted");
        let slot = if let Some(s) = self.free.pop() {
            s
        } else {
            let s = u32::try_from(self.slots.len()).expect("marker slots exhausted");
            self.slots.push(Slot {
                generation: 0,
                dense: u32::MAX,
            });
            s
        };
        self.slots[slot as usize].dense = dense;
        self.byte.push(b);
        self.chars.push(c);
        self.after.push(after as usize);
        self.owner.push(slot);
        MarkerId {
            slot,
            generation: self.slots[slot as usize].generation,
        }
    }
    pub(super) fn pos(&self, m: MarkerId) -> Option<TextPos> {
        let i = self.lookup(m)?;
        Some(TextPos {
            byte: BytePos(self.byte[i]),
            char: CharPos(self.chars[i]),
        })
    }
    pub(super) fn set(&mut self, m: MarkerId, b: usize, c: usize) -> bool {
        let Some(i) = self.lookup(m) else {
            return false;
        };
        self.byte[i] = b;
        self.chars[i] = c;
        true
    }
    pub(super) fn set_type(&mut self, m: MarkerId, after: bool) -> bool {
        let Some(i) = self.lookup(m) else {
            return false;
        };
        self.after[i] = after as usize;
        true
    }
    pub(super) fn free(&mut self, m: MarkerId) -> bool {
        let Some(i) = self.lookup(m) else {
            return false;
        };
        self.byte.swap_remove(i);
        self.chars.swap_remove(i);
        self.after.swap_remove(i);
        self.owner.swap_remove(i);
        if let Some(&owner) = self.owner.get(i) {
            self.slots[owner as usize].dense = i as u32;
        }
        let s = &mut self.slots[m.slot as usize];
        s.dense = u32::MAX;
        // Retire an exhausted generation instead of allowing stale identifiers to alias.
        if let Some(g) = s.generation.checked_add(1) {
            s.generation = g;
            self.free.push(m.slot);
        }
        true
    }
    #[inline]
    pub(super) fn adjust(&mut self, e: Edit, bm: bool, replace: bool) {
        if !self.byte.is_empty() {
            self.adjust_live(e, bm, replace);
        }
    }
    #[inline(never)]
    fn adjust_live(&mut self, e: Edit, bm: bool, replace: bool) {
        let f = e.at.byte.0;
        let fc = e.at.char.0;
        let end = f + e.del.0;
        if e.del.0 == 0 {
            insert_adjust(&mut self.byte, &mut self.chars, &self.after, e, bm);
        } else {
            for i in 0..self.byte.len() {
                let b = self.byte[i];
                let shift = if replace { b >= end } else { b > end };
                if shift {
                    self.byte[i] = b - e.del.0 + e.ins.0;
                    self.chars[i] = self.chars[i] - e.del.1 + e.ins.1;
                } else if b > f {
                    self.byte[i] = f;
                    self.chars[i] = fc;
                }
            }
        }
    }
    pub(super) fn check(&self, t: &Text) {
        assert_eq!(self.byte.len(), self.chars.len());
        assert_eq!(self.byte.len(), self.after.len());
        assert_eq!(self.byte.len(), self.owner.len());
        for (i, &b) in self.byte.iter().enumerate() {
            assert_eq!(t.char_of_byte(BytePos(b)).0, self.chars[i]);
            assert_eq!(self.slots[self.owner[i] as usize].dense as usize, i);
        }
    }
}
