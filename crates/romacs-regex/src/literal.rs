use crate::{
    CaseTable, Stats,
    hay::{Hay, One, decode, prev},
    rank::RANK,
};
use memchr::{memchr, memchr2, memchr2_iter, memchr3, memmem, memrchr, memrchr2, memrchr3};
use romacs_chars as chars;

#[allow(
    clippy::large_enum_variant,
    reason = "Cached plans are Rc-shared; inline finders avoid two extra allocations per program."
)]
enum Mode {
    Bytes {
        f: memmem::Finder<'static>,
        r: memmem::FinderRev<'static>,
    },
    AsciiFold {
        lo: Box<[u8]>,
        up: Box<[u8]>,
        anchor: usize,
        check: usize,
        fold: [u8; 128],
        failure: Box<[usize]>,
    },
    CharFold {
        canon: Box<[u32]>,
        failure: Box<[usize]>,
        map: Vec<(u32, u32)>,
        leads: Vec<u8>,
    },
}
pub(crate) struct Literal {
    mode: Mode,
    maxlen: usize,
}
impl Literal {
    /// Allocation-request bytes owned by the plan, excluding its inline body.
    /// memchr's owned finders each hold a separate boxed needle (not a shared
    /// slice); the sparse fold vectors retain capacity after deduplication.
    pub(crate) fn heap_bytes(&self) -> usize {
        match &self.mode {
            Mode::Bytes { f, r } => f.needle().len() + r.needle().len(),
            Mode::AsciiFold {
                lo, up, failure, ..
            } => lo.len() + up.len() + std::mem::size_of_val(&**failure),
            Mode::CharFold {
                canon,
                failure,
                map,
                leads,
            } => {
                std::mem::size_of_val(&**canon)
                    + std::mem::size_of_val(&**failure)
                    + map.capacity() * std::mem::size_of::<(u32, u32)>()
                    + leads.capacity()
            }
        }
    }

    pub fn new(pat: &[u8], pm: bool, tm: bool, case: Option<&dyn CaseTable>) -> Self {
        let cs = pattern_chars(pat, pm, tm);
        let mut bytes = Vec::new();
        for &c in &cs {
            if tm {
                let mut b = [0; 5];
                let n = chars::char_string(c, &mut b);
                bytes.extend_from_slice(&b[..n]);
            } else {
                bytes.push(c as u8);
            }
        }
        let exact = || Self {
            maxlen: bytes.len(),
            mode: Mode::Bytes {
                f: memmem::Finder::new(&bytes).into_owned(),
                r: memmem::FinderRev::new(&bytes).into_owned(),
            },
        };
        let Some(case) = case else {
            return exact();
        };
        let mut cycles = Vec::new();
        for &c in &cs {
            let mut cycle = vec![c];
            let mut v = case.eqv_next(c);
            while v != c {
                cycle.push(v);
                v = case.eqv_next(v);
            }
            cycles.push(cycle);
        }
        if cycles.iter().all(|v| v.len() == 1) {
            return exact();
        }
        if cycles
            .iter()
            .all(|v| v.len() <= 2 && v.iter().all(|&c| c < 128))
        {
            let lo: Box<[_]> = cycles
                .iter()
                .map(|v| *v.iter().min().unwrap() as u8)
                .collect();
            let up: Box<[_]> = cycles
                .iter()
                .map(|v| *v.iter().max().unwrap() as u8)
                .collect();
            let anchor = (0..lo.len())
                .min_by_key(|&i| RANK[lo[i] as usize].max(RANK[up[i] as usize]))
                .unwrap();
            let check = (0..lo.len())
                .filter(|&i| lo[i] != lo[anchor] || up[i] != up[anchor])
                .min_by_key(|&i| RANK[lo[i] as usize].max(RANK[up[i] as usize]))
                .unwrap_or(anchor);
            let mut fold = std::array::from_fn(|i| i as u8);
            for (&a, &b) in lo.iter().zip(&up) {
                fold[b as usize] = a;
            }
            return Self {
                maxlen: lo.len(),
                mode: Mode::AsciiFold {
                    fold,
                    failure: prefix(&lo),
                    lo,
                    up,
                    anchor,
                    check,
                },
            };
        }
        let mut map = Vec::new();
        let mut leads = Vec::new();
        let mut maxlen = 0;
        for (i, cycle) in cycles.iter().enumerate() {
            maxlen += cycle
                .iter()
                .map(|&c| if tm { chars::char_bytes(c) } else { 1 })
                .max()
                .unwrap();
            for &c in cycle {
                map.push((c, case.canon(c)));
                if i == 0 {
                    let mut b = [0; 5];
                    chars::char_string(c, &mut b);
                    let lead = if tm { b[0] } else { c as u8 };
                    if !leads.contains(&lead) {
                        leads.push(lead);
                    }
                }
            }
        }
        // A canonical target need not translate to itself after an installed
        // table mutation. Unlisted subject characters are treated as identity,
        // so retain each possible target's actual translation as well.
        let targets: Vec<_> = map
            .iter()
            .map(|&(_, canonical)| (canonical, case.canon(canonical)))
            .collect();
        map.extend(targets);
        map.sort_unstable();
        map.dedup_by_key(|entry| entry.0);
        let canon: Box<[_]> = cs.iter().map(|&c| case.canon(c)).collect();
        Self {
            maxlen,
            mode: Mode::CharFold {
                failure: prefix(&canon),
                canon,
                map,
                leads,
            },
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        h: &impl Hay,
        lo: usize,
        hi: usize,
        stop: usize,
        back: bool,
        mb: bool,
        st: &mut Stats,
    ) -> Option<(usize, usize)> {
        if lo > stop {
            return None;
        }
        if self.maxlen == 0 {
            let p = if back { hi.min(stop) } else { lo };
            return Some((p, p));
        }
        let (a, b) = h.slices();
        let seam = a.len();
        let segment = |s: &[u8], base: usize, st: &mut Stats| {
            if base > stop || base > hi || lo > base + s.len() {
                return None;
            }
            let end = s.len().min(stop - base);
            let first = lo.saturating_sub(base);
            if first > end {
                return None;
            }
            self.scan(
                &s[..end],
                first,
                hi.saturating_sub(base).min(end),
                0,
                back,
                mb,
                st,
            )
            .map(|(x, y)| (x + base, y + base))
        };
        let cross = |st: &mut Stats| {
            if b.is_empty()
                || self.maxlen < 2
                || seam > stop
                || hi < seam.saturating_sub(self.maxlen - 1)
                || lo >= seam
            {
                return None;
            }
            let left = seam.saturating_sub(self.maxlen - 1).max(lo);
            let right = (seam + self.maxlen - 1).min(stop).min(h.len());
            let high = prev(h, seam, mb).min(hi);
            if left >= right || high < left {
                return None;
            }
            let mut window = Vec::with_capacity(right - left);
            window.extend_from_slice(&a[left..]);
            window.extend_from_slice(&b[..right - seam]);
            st.window_bytes += window.len() as u64;
            self.scan(&window, 0, high - left, seam - left + 1, back, mb, st)
                .map(|(x, y)| (x + left, y + left))
        };
        if back {
            segment(b, seam, st)
                .or_else(|| cross(st))
                .or_else(|| segment(a, 0, st))
        } else {
            segment(a, 0, st)
                .or_else(|| cross(st))
                .or_else(|| segment(b, seam, st))
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn scan(
        &self,
        s: &[u8],
        mut lo: usize,
        mut hi: usize,
        min_end: usize,
        back: bool,
        mb: bool,
        st: &mut Stats,
    ) -> Option<(usize, usize)> {
        if lo > hi || lo > s.len() {
            return None;
        }
        match &self.mode {
            Mode::Bytes { f, r } => {
                let n = f.needle().len();
                if n > s.len() {
                    return None;
                }
                lo = lo.max(min_end.saturating_sub(n));
                hi = hi.min(s.len() - n);
                if lo > hi {
                    return None;
                }
                let p = if back {
                    r.rfind(&s[lo..hi + n])
                } else {
                    f.find(&s[lo..hi + n])
                }? + lo;
                st.candidates += 1;
                Some((p, p + n))
            }
            Mode::AsciiFold {
                lo: pat,
                up,
                anchor,
                check,
                fold,
                failure,
            } => {
                let n = pat.len();
                if n > s.len() {
                    return None;
                }
                lo = lo.max(min_end.saturating_sub(n));
                hi = hi.min(s.len() - n);
                if lo > hi {
                    return None;
                }
                // A packed pair rejects common isolated anchor bytes before full verification.
                let mut positions =
                    memchr2_iter(pat[*anchor], up[*anchor], &s[lo + anchor..hi + anchor + 1]);
                let before = st.verify_bytes;
                while let Some(offset) = if back {
                    positions.next_back()
                } else {
                    positions.next()
                } {
                    let p = offset + lo;
                    let c = s[p + check];
                    if c != pat[*check] && c != up[*check] {
                        continue;
                    }
                    st.candidates += 1;
                    let yes = pat.iter().zip(up).zip(&s[p..p + n]).all(|((&a, &b), &c)| {
                        st.verify_bytes += 1;
                        c == a || c == b
                    });
                    if yes {
                        return Some((p, p + n));
                    }
                    // Retain the packed-pair fast path, but never spend quadratic work
                    // verifying periodic near matches. KMP restarts within this scan's bounds.
                    if st.verify_bytes - before > s.len() as u64 {
                        let input = (lo..hi + n).map(|p| {
                            let c = s[p];
                            (p, p + 1, fold.get(c as usize).copied().unwrap_or(c))
                        });
                        return linear(pat, failure, input, hi, min_end, back, st);
                    }
                }
                None
            }
            Mode::CharFold {
                canon,
                failure,
                map,
                leads,
            } => {
                let first = lo;
                let last = hi;
                let before = st.verify_bytes;
                while lo <= hi && lo < s.len() {
                    let p = anchor(leads, &s[lo..=hi.min(s.len() - 1)], back)? + lo;
                    if !mb || chars::char_head_p(s[p]) {
                        st.candidates += 1;
                        let mut end = p;
                        let mut yes = true;
                        for &want in canon {
                            if end == s.len() {
                                yes = false;
                                break;
                            }
                            let n = if mb {
                                chars::bytes_by_char_head(s[end])
                            } else {
                                1
                            };
                            if end + n > s.len() {
                                yes = false;
                                break;
                            }
                            let (c, n) = decode(&One(s), end, mb);
                            st.verify_bytes += n as u64;
                            if canonical(map, c) != want {
                                yes = false;
                                break;
                            }
                            end += n;
                        }
                        if yes && end >= min_end {
                            return Some((p, end));
                        }
                    }
                    if st.verify_bytes - before > s.len() as u64 {
                        let mut pos = first;
                        let limit = last.saturating_add(self.maxlen).min(s.len());
                        let input = std::iter::from_fn(|| {
                            while pos < limit && mb && !chars::char_head_p(s[pos]) {
                                pos += 1;
                            }
                            if pos >= limit {
                                return None;
                            }
                            let width = if mb {
                                chars::bytes_by_char_head(s[pos])
                            } else {
                                1
                            };
                            if width > limit - pos {
                                return None;
                            }
                            let start = pos;
                            let (c, n) = decode(&One(s), pos, mb);
                            pos += n;
                            Some((start, pos, canonical(map, c)))
                        });
                        return linear(canon, failure, input, last, min_end, back, st);
                    }
                    if back {
                        if p == 0 {
                            return None;
                        }
                        hi = p - 1;
                    } else {
                        lo = p + 1;
                    }
                }
                None
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_census_counts_both_needles_and_fold_allocation_capacities() {
        let exact = Literal::new(&[b'a'; 17], false, false, None);
        assert_eq!(exact.heap_bytes(), 34); // forward AND reverse owned needles
        struct Fold;
        impl CaseTable for Fold {
            fn canon(&self, c: u32) -> u32 {
                match c {
                    65 => 97,
                    0x100..=0x102 => 0x100,
                    _ => c,
                }
            }
            fn eqv_next(&self, c: u32) -> u32 {
                match c {
                    65 => 97,
                    97 => 65,
                    0x100 => 0x101,
                    0x101 => 0x102,
                    0x102 => 0x100,
                    _ => c,
                }
            }
        }
        let ascii = Literal::new(&[b'a'; 17], false, false, Some(&Fold));
        assert_eq!(ascii.heap_bytes(), 34 + 17 * std::mem::size_of::<usize>());
        let wide = Literal::new(&[0xc4, 0x80].repeat(17), true, true, Some(&Fold));
        let Mode::CharFold {
            canon,
            failure,
            map,
            leads,
        } = &wide.mode
        else {
            panic!("expected sparse fold")
        };
        assert!(
            map.capacity() > map.len(),
            "dedup must leave spare storage in this regression"
        );
        assert_eq!(
            wide.heap_bytes(),
            canon.len() * 4
                + failure.len() * std::mem::size_of::<usize>()
                + map.capacity() * 8
                + leads.capacity()
        );
    }
}

fn pattern_chars(pat: &[u8], pm: bool, tm: bool) -> Vec<u32> {
    let convert = |c| {
        if pm == tm {
            c
        } else if tm {
            chars::unibyte_to_char(c as u8)
        } else {
            c & 255
        }
    };
    if pm {
        chars::chars(pat).map(convert).collect()
    } else {
        pat.iter().map(|&c| convert(c as u32)).collect()
    }
}

pub(crate) fn translated(
    pat: &[u8],
    pm: bool,
    h: &impl Hay,
    mb: bool,
    back: bool,
    mut canon: impl FnMut(u32) -> u32,
    st: &mut Stats,
) -> Option<(usize, usize)> {
    if pat.is_empty() {
        let at = if back { h.len() } else { 0 };
        return Some((at, at));
    }
    let mut pat = pattern_chars(pat, pm, mb);
    for c in &mut pat {
        *c = canon(*c);
    }
    let failure = prefix(&pat);
    let mut pos = 0;
    let input = std::iter::from_fn(|| {
        if pos == h.len() {
            return None;
        }
        let start = pos;
        let (c, n) = decode(h, pos, mb);
        pos += n;
        Some((start, pos, canon(c)))
    });
    linear(&pat, &failure, input, h.len(), 0, back, st)
}

fn canonical(map: &[(u32, u32)], c: u32) -> u32 {
    map.binary_search_by_key(&c, |&(v, _)| v)
        .map_or(c, |i| map[i].1)
}

fn prefix<T: Eq>(pat: &[T]) -> Box<[usize]> {
    let mut failure = vec![0; pat.len()];
    let mut matched = 0;
    for i in 1..pat.len() {
        while matched > 0 && pat[i] != pat[matched] {
            matched = failure[matched - 1];
        }
        if pat[i] == pat[matched] {
            matched += 1;
        }
        failure[i] = matched;
    }
    failure.into_boxed_slice()
}

// KMP compares each decoded subject character at most twice amortized. The ring
// retains byte starts without copying or canonicalizing the subject. A forward
// pass also supplies the last eligible start for a backward search.
fn linear<T: Eq>(
    pat: &[T],
    failure: &[usize],
    input: impl Iterator<Item = (usize, usize, T)>,
    hi: usize,
    min_end: usize,
    back: bool,
    st: &mut Stats,
) -> Option<(usize, usize)> {
    let mut starts = vec![0; pat.len()];
    let mut matched = 0;
    let mut found = None;
    for (i, (start, end, c)) in input.enumerate() {
        starts[i % pat.len()] = start;
        loop {
            st.verify_bytes += (end - start) as u64;
            if pat[matched] == c {
                matched += 1;
                break;
            }
            if matched == 0 {
                break;
            }
            matched = failure[matched - 1];
        }
        if matched == pat.len() {
            let start = starts[(i + 1 - pat.len()) % pat.len()];
            if start <= hi && end >= min_end {
                st.candidates += 1;
                found = Some((start, end));
                if !back {
                    return found;
                }
            }
            matched = failure[matched - 1];
        }
    }
    found
}

fn anchor(leads: &[u8], s: &[u8], back: bool) -> Option<usize> {
    match (leads, back) {
        ([a], false) => memchr(*a, s),
        ([a], true) => memrchr(*a, s),
        ([a, b], false) => memchr2(*a, *b, s),
        ([a, b], true) => memrchr2(*a, *b, s),
        ([a, b, c], false) => memchr3(*a, *b, *c, s),
        ([a, b, c], true) => memrchr3(*a, *b, *c, s),
        (_, false) => s.iter().position(|b| leads.contains(b)),
        (_, true) => s.iter().rposition(|b| leads.contains(b)),
    }
}
