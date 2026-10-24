pub(crate) trait Hay {
    fn slices(&self) -> (&[u8], &[u8]);
    fn len(&self) -> usize {
        let (a, b) = self.slices();
        a.len() + b.len()
    }
    fn byte(&self, p: usize) -> u8;
}
pub(crate) struct One<'a>(pub &'a [u8]);
pub(crate) struct Two<'a>(pub &'a [u8], pub &'a [u8]);
impl Hay for One<'_> {
    fn slices(&self) -> (&[u8], &[u8]) {
        (self.0, &[])
    }
    #[inline]
    fn byte(&self, p: usize) -> u8 {
        self.0[p]
    }
}
impl Hay for Two<'_> {
    fn slices(&self) -> (&[u8], &[u8]) {
        (self.0, self.1)
    }
    #[inline]
    fn byte(&self, p: usize) -> u8 {
        if p < self.0.len() {
            self.0[p]
        } else {
            self.1[p - self.0.len()]
        }
    }
}
/// Decoding and reverse stepping share the same logical-byte accessor.
pub(crate) fn decode(h: &impl Hay, p: usize, mb: bool) -> (u32, usize) {
    if !mb {
        return (h.byte(p) as u32, 1);
    }
    let n = romacs_chars::bytes_by_char_head(h.byte(p));
    let mut b = [0; 5];
    for (i, v) in b[..n].iter_mut().enumerate() {
        *v = h.byte(p + i);
    }
    romacs_chars::string_char(&b[..n])
}
pub(crate) fn prev(h: &impl Hay, mut p: usize, mb: bool) -> usize {
    p -= 1;
    while mb && !romacs_chars::char_head_p(h.byte(p)) {
        p -= 1;
    }
    p
}
