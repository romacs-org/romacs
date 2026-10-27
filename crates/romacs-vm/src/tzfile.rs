//! TZif zone files as glibc's tzfile.c uses them: the transition table,
//! the POSIX footer for times after it, and `__tzfile_default', which lends
//! a `posixrules' file's transitions to a TZ string that names a DST zone
//! but gives no rule.
use crate::posix_tz::{Local, PosixTz};

#[derive(Clone, Copy, Debug)]
struct Type { offset: i64, dst: bool, name: usize, standard: bool, universal: bool }

#[derive(Clone, Debug)]
pub(crate) struct TzFile {
    transitions: Vec<i64>,
    indices: Vec<usize>,
    types: Vec<Type>,
    names: Vec<u8>,
    footer: Option<String>,
}

fn be(bytes: &[u8], at: usize, width: usize) -> Option<i64> {
    let b = bytes.get(at..at + width)?;
    Some(if width == 8 { i64::from_be_bytes(b.try_into().ok()?) } else { i64::from(i32::from_be_bytes(b.try_into().ok()?)) })
}

impl TzFile {
    /// The version 2+ body when present, else version 1. None for a file
    /// this reader does not take: not TZif, truncated, or with leap seconds
    /// (whose timescale it would misread).
    pub(crate) fn parse(bytes: &[u8]) -> Option<TzFile> {
        // isutcnt isstdcnt leapcnt timecnt typecnt charcnt
        let counts = |at: usize| -> Option<[usize; 6]> {
            if bytes.get(at..at + 4)? != b"TZif" { return None; }
            let mut out = [0; 6];
            for (i, n) in out.iter_mut().enumerate() { *n = be(bytes, at + 20 + i * 4, 4)? as u32 as usize; }
            Some(out)
        };
        let size = |c: [usize; 6], width: usize| -> Option<usize> {
            c[3].checked_mul(width + 1)?.checked_add(c[4].checked_mul(6)?)?.checked_add(c[5])?
                .checked_add(c[2].checked_mul(width + 4)?)?.checked_add(c[1])?.checked_add(c[0])
        };
        let first = counts(0)?;
        let (start, width, c) = if matches!(bytes.get(4), Some(b'2' | b'3' | b'4')) {
            let second = 44usize.checked_add(size(first, 4)?)?;
            (second + 44, 8, counts(second)?)
        } else {
            (44, 4, first)
        };
        let [isut, isstd, leaps, times, types, chars] = c;
        if leaps != 0 || types == 0 { return None; }
        let end = start.checked_add(size(c, width)?)?;
        bytes.get(..end)?;
        let transitions = (0..times).map(|i| be(bytes, start + i * width, width)).collect::<Option<Vec<_>>>()?;
        let indices: Vec<usize> = bytes[start + times * width..start + times * (width + 1)].iter().map(|&b| usize::from(b)).collect();
        if indices.iter().any(|&i| i >= types) { return None; }
        let info = start + times * (width + 1);
        let names = bytes[info + types * 6..info + types * 6 + chars].to_vec();
        let flags = info + types * 6 + chars + leaps * (width + 4);
        let types = (0..types).map(|i| Type {
            offset: be(bytes, info + i * 6, 4).unwrap(),
            dst: bytes[info + i * 6 + 4] != 0,
            name: usize::from(bytes[info + i * 6 + 5]),
            standard: i < isstd && bytes[flags + i] != 0,
            universal: i < isut && bytes[flags + isstd + i] != 0,
        }).collect();
        let footer = (width == 8).then(|| std::str::from_utf8(&bytes[end..]).ok().map(|s| s.trim_matches('\n').to_string())).flatten()
            .filter(|s| !s.is_empty());
        Some(TzFile { transitions, indices, types, names, footer })
    }

    fn local(&self, t: &Type) -> Option<Local> {
        let tail = self.names.get(t.name..)?;
        let name = &tail[..tail.iter().position(|&b| b == 0)?];
        Some(Local { name: String::from_utf8_lossy(name).into_owned(), offset: t.offset, dst: t.dst })
    }

    /// __tzfile_compute: the zone at UTC time T. Before the first transition
    /// it is the first standard type; from the last one on, the footer rule.
    pub(crate) fn at(&self, t: i64) -> Option<Local> {
        if self.transitions.is_empty() || t < self.transitions[0] {
            let first = self.types.iter().find(|ty| !ty.dst).unwrap_or(&self.types[0]);
            return self.local(first);
        }
        if t >= *self.transitions.last().unwrap() && let Some(footer) = &self.footer {
            return Some(PosixTz::parse(footer).at(t));
        }
        let i = self.transitions.partition_point(|&x| x <= t) - 1;
        self.local(&self.types[self.indices[i]])
    }

    /// The `rule_dstoff' a read of this file leaves: __tzfile_read sets it
    /// only for a file without transitions, so otherwise it is RETAINED.
    pub(crate) fn rule_dstoff(&self, retained: i64) -> i64 {
        if self.transitions.is_empty() { self.types[0].offset } else { retained }
    }

    /// __tzfile_default: this rule file's transitions, re-offset to STD and
    /// DST's offsets and named by them; None when it has fewer than two
    /// types. RULE_DST is glibc's process-wide rule_dstoff, which only a
    /// file without transitions (or the previous default) set; the rule
    /// standard offset is that of the latest standard-time transition.
    pub(crate) fn with_default(&self, std: &Local, dst: &Local, rule_dst: i64) -> Option<TzFile> {
        if self.types.len() < 2 { return None; }
        let rule_std = if self.transitions.is_empty() { self.types[0].offset } else {
            self.indices.iter().rev().map(|&i| self.types[i]).find(|ty| !ty.dst).map_or(0, |ty| ty.offset)
        };
        let rule_dst = self.rule_dstoff(rule_dst);
        let mut was_dst = false;
        let mut transitions = self.transitions.clone();
        let mut indices = self.indices.clone();
        for (time, index) in transitions.iter_mut().zip(indices.iter_mut()) {
            let ty = self.types[*index];
            *time += if ty.universal { 0 }
                else if was_dst && !ty.standard { dst.offset - rule_dst } else { std.offset - rule_std };
            was_dst = ty.dst;
            *index = usize::from(ty.dst);
        }
        let mut names = std.name.as_bytes().to_vec();
        names.push(0);
        let dst_at = names.len();
        names.extend_from_slice(dst.name.as_bytes());
        names.push(0);
        let types = vec![
            Type { offset: std.offset, dst: false, name: 0, standard: false, universal: false },
            Type { offset: dst.offset, dst: true, name: dst_at, standard: false, universal: false },
        ];
        Some(TzFile { transitions, indices, types, names, footer: self.footer.clone() })
    }
}
