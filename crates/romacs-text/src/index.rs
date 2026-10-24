use super::*;
impl Text {
    pub(super) fn locate(&self, key: usize, field: fn(Sum) -> usize) -> (usize, Sum) {
        if key == field(self.total) {
            let i = self.chunks.len() - 1;
            let c = self.chunks[i].wide();
            return (
                i,
                Sum {
                    bytes: self.total.bytes - c.bytes,
                    chars: self.total.chars - c.chars,
                    nl: self.total.nl - c.nl,
                },
            );
        }
        self.chunks.locate(key, field)
    }
    #[inline]
    pub fn char_of_byte(&self, b: BytePos) -> CharPos {
        debug_assert!(self.is_char_boundary(b));
        if self.total.bytes == self.total.chars {
            return CharPos(b.0);
        }
        if b.0 == self.gap.start {
            return self.gap_char;
        }
        self.char_of_byte_scan(b)
    }
    #[inline(never)]
    fn char_of_byte_scan(&self, b: BytePos) -> CharPos {
        let (i, s) = self.locate(b.0, |s| s.bytes);
        if self.chunks[i].bytes == self.chunks[i].chars {
            return CharPos(s.chars + b.0 - s.bytes);
        }
        if b.0.abs_diff(self.gap.start) < b.0 - s.bytes {
            return if b.0 < self.gap.start {
                CharPos(self.gap_char.0 - self.range_sum(b.0, self.gap.start).chars)
            } else {
                CharPos(self.gap_char.0 + self.range_sum(self.gap.start, b.0).chars)
            };
        }
        CharPos(s.chars + self.range_sum(s.bytes, b.0).chars)
    }
    #[inline]
    pub fn byte_of_char(&self, c: CharPos) -> BytePos {
        assert!(c.0 <= self.len_chars());
        if self.total.bytes == self.total.chars {
            return BytePos(c.0);
        }
        if c == self.gap_char {
            return BytePos(self.gap.start);
        }
        self.byte_of_char_scan(c)
    }
    #[inline(never)]
    fn byte_of_char_scan(&self, c: CharPos) -> BytePos {
        let (i, s) = self.locate(c.0, |s| s.chars);
        if self.chunks[i].bytes == self.chunks[i].chars {
            return BytePos(s.bytes + c.0 - s.chars);
        }
        let distance = c.0.abs_diff(self.gap_char.0);
        if distance < c.0 - s.chars {
            let mut p = self.gap.start;
            let mut left = distance;
            while left > 0 {
                if c < self.gap_char {
                    p -= 1;
                } else {
                    p += 1;
                }
                scanned_bytes(1);
                if self.is_char_boundary(BytePos(p)) {
                    left -= 1;
                }
            }
            return BytePos(p);
        }
        let mut left = c.0 - s.chars;
        let mut offset = s.bytes;
        let (a, b) = self.segments(
            BytePos(s.bytes),
            BytePos(s.bytes + self.chunks[i].bytes as usize),
        );
        for run in [a, b] {
            for block in run.chunks(8) {
                scanned_bytes(block.len());
                let n = head_count(block);
                if n <= left {
                    left -= n;
                    offset += block.len();
                } else {
                    for &v in block {
                        if char_head_p(v) {
                            if left == 0 {
                                return BytePos(offset);
                            }
                            left -= 1;
                        }
                        offset += 1;
                    }
                }
            }
        }
        BytePos(offset)
    }
    pub fn newlines_before(&self, b: BytePos) -> usize {
        assert!(b.0 <= self.len_bytes());
        let (_, s) = self.locate(b.0, |s| s.bytes);
        {
            let (a, b) = self.segments(BytePos(s.bytes), b);
            scanned_bytes(a.len() + b.len());
            s.nl + memchr::memchr_iter(b'\n', a).count() + memchr::memchr_iter(b'\n', b).count()
        }
    }
    pub fn count_newlines(&self, from: BytePos, to: BytePos) -> usize {
        assert!(from <= to);
        self.newlines_before(to) - self.newlines_before(from)
    }
    /// Search `[from, limit)` forwards for positive `count`, or `[limit, from)`
    /// backwards for negative `count`. A successful forward search returns just
    /// after the newline; a successful backward search returns the newline itself.
    /// A shortage returns `limit`; zero count returns `from`. The second result
    /// is always the unsigned number found, including on shortage.
    ///
    /// This is the brief's adapted API, not GNU C `find_newline`: GNU returns
    /// after the newline in both directions and signs the backward count.
    /// Owners calling that primitive must convert coordinates and count signs.
    #[inline]
    pub fn find_newline(&self, from: BytePos, limit: BytePos, count: isize) -> (BytePos, usize) {
        if count == 1 && from < limit && limit.0 <= self.len_bytes() {
            let end = from.0.saturating_add(128).min(limit.0);
            let end = if from.0 < self.gap.start {
                end.min(self.gap.start)
            } else {
                end
            };
            let physical = from.0
                + if from.0 >= self.gap.start {
                    self.gap.len()
                } else {
                    0
                };
            let run = &self.buf[physical..physical + end - from.0];
            if let Some(at) = memchr::memchr(b'\n', run) {
                scanned_bytes(at + 1);
                return (BytePos(from.0 + at + 1), 1);
            }
            scanned_bytes(run.len());
            return self.find_newline_tail(BytePos(end), limit, count);
        }
        self.find_newline_tail(from, limit, count)
    }

    #[inline(never)]
    fn find_newline_tail(&self, from: BytePos, limit: BytePos, count: isize) -> (BytePos, usize) {
        if count == 0 {
            return (from, 0);
        }
        let forward = count > 0;
        assert!(from.0 <= self.len_bytes() && limit.0 <= self.len_bytes());
        assert!(if forward {
            from <= limit
        } else {
            limit <= from
        });
        let want = count.unsigned_abs();
        let mut from = from;
        // Ordinary line motion rarely reaches a chunk boundary. Search a bounded
        // byte window before descending summaries; on a miss continue after it,
        // so long newline-free spans still skip through the tree without rescans.
        if want == 1 {
            let (lo, hi) = if forward {
                (from.0, from.0.saturating_add(128).min(limit.0))
            } else {
                (from.0.saturating_sub(128).max(limit.0), from.0)
            };
            let (a, b) = self.segments(BytePos(lo), BytePos(hi));
            let mut runs = [(lo, a), (lo + a.len(), b)];
            if !forward {
                runs.reverse();
            }
            for (base, run) in runs {
                let found = if forward {
                    memchr::memchr(b'\n', run)
                } else {
                    memchr::memrchr(b'\n', run)
                };
                if let Some(at) = found {
                    scanned_bytes(if forward { at + 1 } else { run.len() - at });
                    return (BytePos(base + at + usize::from(forward)), 1);
                }
                scanned_bytes(run.len());
            }
            from = BytePos(if forward { hi } else { lo });
        }
        let mut left = want;
        if from == limit {
            return (limit, 0);
        }
        let (mut i, prefix) = self.locate(if forward { from.0 } else { from.0 - 1 }, |s| s.bytes);
        let mut start = prefix.bytes;
        let mut pos = from.0;
        loop {
            let chunk = self.chunks[i].wide();
            let end = start + chunk.bytes;
            let lo = if forward { pos } else { limit.0.max(start) };
            let hi = if forward { limit.0.min(end) } else { pos };
            if lo == start && hi == end && left > chunk.nl {
                left -= chunk.nl;
            } else {
                let (a, b) = self.segments(BytePos(lo), BytePos(hi));
                let mut runs = [(lo, a), (lo + a.len(), b)];
                if !forward {
                    runs.reverse();
                }
                for (base, run) in runs {
                    let mut matches = memchr::memchr_iter(b'\n', run);
                    while let Some(at) = if forward {
                        matches.next()
                    } else {
                        matches.next_back()
                    } {
                        left -= 1;
                        if left == 0 {
                            scanned_bytes(if forward { at + 1 } else { run.len() - at });
                            return (BytePos(base + at + usize::from(forward)), want);
                        }
                    }
                    scanned_bytes(run.len());
                }
            }
            pos = if forward { hi } else { lo };
            if pos == limit.0 {
                return (limit, want - left);
            }
            if forward {
                i += 1;
                start = end;
            } else {
                i -= 1;
                start -= self.chunks[i].bytes as usize;
            }
        }
    }
    pub(super) fn pieces(&self, mut f: usize, z: usize) -> Vec<Chunk> {
        let mut out = vec![];
        while f < z {
            let mut e = (f + T).min(z);
            while !self.is_char_boundary(BytePos(e)) {
                e -= 1;
            }
            out.push(self.range_sum(f, e).into());
            f = e;
        }
        out
    }
}
