//! gnulib diffseq.h `compareseq` as editfns.c configures it for
//! `replace-region-contents`: Myers' O(ND) difference with Eggert's
//! TOO_EXPENSIVE and big-snake heuristics, over character codes. The
//! partition choices are GNU's, so the edit script, and with it which
//! markers and properties survive a replacement, is GNU's.
use std::time::Instant;

/// diffseq.h SNAKE_LIMIT: snakes longer than this are "big".
const SNAKE_LIMIT: isize = 20;

/// The characters each sequence loses (A) and gains (B), or None when the
/// comparison passed DEADLINE.
pub(crate) fn compare(a: &[u32], b: &[u32], too_expensive: isize, deadline: Option<Instant>) -> Option<(Vec<bool>, Vec<bool>)> {
    let diags = a.len() + b.len() + 3;
    let mut fd = Vec::new();
    let mut bd = Vec::new();
    // editfns.c falls back to a plain replacement when this cannot be had.
    fd.try_reserve_exact(diags).ok()?;
    bd.try_reserve_exact(diags).ok()?;
    fd.resize(diags, 0);
    bd.resize(diags, 0);
    let mut cx = Context {
        a, b, fd, bd, offset: b.len() as isize + 1, too_expensive, deadline,
        quit: 0, aborted: false,
        deletions: vec![false; a.len()], insertions: vec![false; b.len()],
    };
    if cx.compareseq(0, a.len() as isize, 0, b.len() as isize, false) { return None; }
    Some((cx.deletions, cx.insertions))
}

/// The change runs of an edit script, as editfns.c walks them: from the
/// end backwards, each (A-START, A-END, B-START, B-END), zero-based.
pub(crate) fn runs(deletions: &[bool], insertions: &[bool]) -> Vec<(usize, usize, usize, usize)> {
    let (mut i, mut j) = (deletions.len() as isize, insertions.len() as isize);
    let set = |bits: &[bool], k: isize| k > 0 && bits[k as usize - 1];
    let mut out = Vec::new();
    while i >= 0 || j >= 0 {
        if set(deletions, i) || set(insertions, j) {
            let (end_a, end_b) = (i, j);
            while set(deletions, i) { i -= 1; }
            while set(insertions, j) { j -= 1; }
            out.push((i as usize, end_a as usize, j as usize, end_b as usize));
        }
        i -= 1;
        j -= 1;
    }
    out
}

struct Context<'a> {
    a: &'a [u32],
    b: &'a [u32],
    fd: Vec<isize>,
    bd: Vec<isize>,
    /// Index of diagonal 0 in FD and BD.
    offset: isize,
    too_expensive: isize,
    deadline: Option<Instant>,
    /// buffer_chars_equal's quit counter: the deadline is read when it wraps.
    quit: u16,
    aborted: bool,
    deletions: Vec<bool>,
    insertions: Vec<bool>,
}

struct Partition { xmid: isize, ymid: isize, lo_minimal: bool, hi_minimal: bool }

impl Context<'_> {
    fn early_abort(&self) -> bool {
        self.deadline.is_some_and(|d| d < Instant::now())
    }

    /// buffer_chars_equal: an abort found here unwinds the whole comparison,
    /// as GNU's longjmp does.
    fn equal(&mut self, x: isize, y: isize) -> bool {
        self.quit = self.quit.wrapping_add(1);
        if self.quit == 0 && self.early_abort() { self.aborted = true; }
        !self.aborted && self.a[x as usize] == self.b[y as usize]
    }

    fn f(&mut self, d: isize) -> &mut isize { &mut self.fd[(d + self.offset) as usize] }
    fn b(&mut self, d: isize) -> &mut isize { &mut self.bd[(d + self.offset) as usize] }

    /// diffseq.h `diag`: the midpoint of the shortest edit script for
    /// [XOFF, XLIM) against [YOFF, YLIM), with the heuristics GNU enables.
    fn diag(&mut self, xoff: isize, xlim: isize, yoff: isize, ylim: isize, find_minimal: bool) -> Partition {
        let (dmin, dmax) = (xoff - ylim, xlim - yoff);
        let (fmid, bmid) = (xoff - yoff, xlim - ylim);
        let (mut fmin, mut fmax, mut bmin, mut bmax) = (fmid, fmid, bmid, bmid);
        let odd = (fmid - bmid) & 1 != 0;
        *self.f(fmid) = xoff;
        *self.b(bmid) = xlim;
        let mut c = 1;
        loop {
            let mut big_snake = false;
            if fmin > dmin { fmin -= 1; *self.f(fmin - 1) = -1; } else { fmin += 1; }
            if fmax < dmax { fmax += 1; *self.f(fmax + 1) = -1; } else { fmax -= 1; }
            let mut d = fmax;
            while d >= fmin {
                let (tlo, thi) = (*self.f(d - 1), *self.f(d + 1));
                let x0 = if tlo < thi { thi } else { tlo + 1 };
                let (mut x, mut y) = (x0, x0 - d);
                while x < xlim && y < ylim && self.equal(x, y) { x += 1; y += 1; }
                big_snake |= x - x0 > SNAKE_LIMIT;
                *self.f(d) = x;
                if odd && bmin <= d && d <= bmax && *self.b(d) <= x {
                    return Partition { xmid: x, ymid: y, lo_minimal: true, hi_minimal: true };
                }
                d -= 2;
            }
            if bmin > dmin { bmin -= 1; *self.b(bmin - 1) = isize::MAX; } else { bmin += 1; }
            if bmax < dmax { bmax += 1; *self.b(bmax + 1) = isize::MAX; } else { bmax -= 1; }
            let mut d = bmax;
            while d >= bmin {
                let (tlo, thi) = (*self.b(d - 1), *self.b(d + 1));
                let x0 = if tlo < thi { tlo } else { thi - 1 };
                let (mut x, mut y) = (x0, x0 - d);
                while xoff < x && yoff < y && self.equal(x - 1, y - 1) { x -= 1; y -= 1; }
                big_snake |= x0 - x > SNAKE_LIMIT;
                *self.b(d) = x;
                if !odd && fmin <= d && d <= fmax && x <= *self.f(d) {
                    return Partition { xmid: x, ymid: y, lo_minimal: true, hi_minimal: true };
                }
                d -= 2;
            }
            if self.aborted {
                return Partition { xmid: xoff, ymid: yoff, lo_minimal: true, hi_minimal: true };
            }
            if !find_minimal {
                if 200 < c && big_snake {
                    if let Some(part) = self.forward_snake(xoff, xlim, yoff, ylim, fmin, fmax, fmid, c) { return part; }
                    if let Some(part) = self.backward_snake(xoff, xlim, yoff, ylim, bmin, bmax, bmid, c) { return part; }
                }
                if c >= self.too_expensive {
                    return self.halfway(xoff, xlim, yoff, ylim, (fmin, fmax), (bmin, bmax));
                }
            }
            c += 1;
        }
    }

    /// The forward diagonal with the most progress, ending in a big snake.
    #[allow(clippy::too_many_arguments)]
    fn forward_snake(&mut self, xoff: isize, xlim: isize, yoff: isize, ylim: isize, fmin: isize, fmax: isize, fmid: isize, c: isize) -> Option<Partition> {
        let (mut best, mut part) = (0, None);
        let mut d = fmax;
        while d >= fmin {
            let dd = d - fmid;
            let x = *self.f(d);
            let y = x - d;
            let v = (x - xoff) * 2 - dd;
            if v > 12 * (c + dd.abs()) && v > best
                && xoff + SNAKE_LIMIT <= x && x < xlim && yoff + SNAKE_LIMIT <= y && y < ylim
            {
                let mut k = 1;
                while self.equal(x - k, y - k) {
                    if k == SNAKE_LIMIT {
                        best = v;
                        part = Some(Partition { xmid: x, ymid: y, lo_minimal: true, hi_minimal: false });
                        break;
                    }
                    k += 1;
                }
            }
            d -= 2;
        }
        part
    }

    /// The backward diagonal with the most progress, starting a big snake.
    #[allow(clippy::too_many_arguments)]
    fn backward_snake(&mut self, xoff: isize, xlim: isize, yoff: isize, ylim: isize, bmin: isize, bmax: isize, bmid: isize, c: isize) -> Option<Partition> {
        let (mut best, mut part) = (0, None);
        let mut d = bmax;
        while d >= bmin {
            let dd = d - bmid;
            let x = *self.b(d);
            let y = x - d;
            let v = (xlim - x) * 2 + dd;
            if v > 12 * (c + dd.abs()) && v > best
                && xoff < x && x <= xlim - SNAKE_LIMIT && yoff < y && y <= ylim - SNAKE_LIMIT
            {
                let mut k = 0;
                while self.equal(x + k, y + k) {
                    if k == SNAKE_LIMIT - 1 {
                        best = v;
                        part = Some(Partition { xmid: x, ymid: y, lo_minimal: false, hi_minimal: true });
                        break;
                    }
                    k += 1;
                }
            }
            d -= 2;
        }
        part
    }

    /// Past TOO_EXPENSIVE: halfway between the best forward and backward
    /// diagonals so far.
    fn halfway(&mut self, xoff: isize, xlim: isize, yoff: isize, ylim: isize, (fmin, fmax): (isize, isize), (bmin, bmax): (isize, isize)) -> Partition {
        let (mut fxybest, mut fxbest) = (-1, 0);
        let mut d = fmax;
        while d >= fmin {
            let mut x = (*self.f(d)).min(xlim);
            let mut y = x - d;
            if ylim < y { x = ylim + d; y = ylim; }
            if fxybest < x + y { fxybest = x + y; fxbest = x; }
            d -= 2;
        }
        let (mut bxybest, mut bxbest) = (isize::MAX, 0);
        let mut d = bmax;
        while d >= bmin {
            let mut x = xoff.max(*self.b(d));
            let mut y = x - d;
            if y < yoff { x = yoff + d; y = yoff; }
            if x + y < bxybest { bxybest = x + y; bxbest = x; }
            d -= 2;
        }
        if (xlim + ylim) - bxybest < fxybest - (xoff + yoff) {
            Partition { xmid: fxbest, ymid: fxybest - fxbest, lo_minimal: true, hi_minimal: false }
        } else {
            Partition { xmid: bxbest, ymid: bxybest - bxbest, lo_minimal: false, hi_minimal: true }
        }
    }

    /// diffseq.h `compareseq`: true when the deadline ended it early. It
    /// recurses into the smaller half only, so its depth stays logarithmic.
    fn compareseq(&mut self, mut xoff: isize, mut xlim: isize, mut yoff: isize, mut ylim: isize, mut find_minimal: bool) -> bool {
        loop {
            while xoff < xlim && yoff < ylim && self.equal(xoff, yoff) { xoff += 1; yoff += 1; }
            while xoff < xlim && yoff < ylim && self.equal(xlim - 1, ylim - 1) { xlim -= 1; ylim -= 1; }
            if self.aborted { return true; }
            if xoff == xlim {
                for y in yoff..ylim {
                    self.insertions[y as usize] = true;
                    if self.early_abort() { return true; }
                }
                return false;
            }
            if yoff == ylim {
                for x in xoff..xlim {
                    self.deletions[x as usize] = true;
                    if self.early_abort() { return true; }
                }
                return false;
            }
            let part = self.diag(xoff, xlim, yoff, ylim, find_minimal);
            if self.aborted { return true; }
            let low = (xoff, part.xmid, yoff, part.ymid, part.lo_minimal);
            let high = (part.xmid, xlim, part.ymid, ylim, part.hi_minimal);
            let (first, second) = if (xlim + ylim) - (part.xmid + part.ymid) < (part.xmid + part.ymid) - (xoff + yoff) {
                (high, low)
            } else {
                (low, high)
            };
            if self.compareseq(first.0, first.1, first.2, first.3, first.4) { return true; }
            (xoff, xlim, yoff, ylim, find_minimal) = second;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(a: &str, b: &str) -> Vec<(usize, usize, usize, usize)> {
        let (a, b): (Vec<u32>, Vec<u32>) = (a.chars().map(u32::from).collect(), b.chars().map(u32::from).collect());
        let (deletions, insertions) = compare(&a, &b, 1_000_000, None).unwrap();
        runs(&deletions, &insertions)
    }

    #[test]
    fn runs_cover_exactly_the_differences_walking_backwards() {
        assert_eq!(script("abc", "abc"), vec![]);
        assert_eq!(script("abc", "abXc"), vec![(2, 2, 2, 3)]);
        assert_eq!(script("abXc", "abc"), vec![(2, 3, 2, 2)]);
        assert_eq!(script("abc", "aXbYc"), vec![(2, 2, 3, 4), (1, 1, 1, 2)]);
        assert_eq!(script("abc", ""), vec![(0, 3, 0, 0)]);
    }

    #[test]
    fn a_passed_deadline_abandons_the_comparison() {
        let a: Vec<u32> = (0..200_000).map(|i| i % 7).collect();
        let b: Vec<u32> = (0..200_000).map(|i| i % 11).collect();
        assert!(compare(&a, &b, 1_000_000, Some(Instant::now())).is_none());
    }
}
