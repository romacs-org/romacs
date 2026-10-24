//! Unwired literal search kernel. Offsets are zero-based GNU internal bytes.
//! Inputs must be valid GNU text; search bounds are character boundaries.
mod cache;
mod hay;
mod literal;
#[cfg(test)]
mod model_tests;
mod rank;
pub use cache::{Cache, CacheCensus, KeyRef, Kind};
use hay::{Hay, One, Two};
use literal::Literal;

/// Search the full supplied region with call-local canonicalization, without
/// enumerating equivalences or copying the subject. Pattern conversion and KMP
/// take O(m + n) work and O(m) storage; offsets are logical subject bytes.
/// Only encountered pattern/subject characters are passed to `canon`.
pub fn translated_literal(
    pat: &[u8],
    pattern_multibyte: bool,
    h: &Haystack<'_>,
    backward: bool,
    canon: impl FnMut(u32) -> u32,
    st: &mut Stats,
) -> Option<(usize, usize)> {
    if h.s2.is_empty() {
        literal::translated(
            pat,
            pattern_multibyte,
            &One(h.s1),
            h.multibyte,
            backward,
            canon,
            st,
        )
    } else {
        literal::translated(
            pat,
            pattern_multibyte,
            &Two(h.s1, h.s2),
            h.multibyte,
            backward,
            canon,
            st,
        )
    }
}

/// A consistent canonicalization table and circular equivalence lists.
/// Every `eqv_next` cycle must return to its starting character.
pub trait CaseTable {
    fn canon(&self, c: u32) -> u32;
    fn eqv_next(&self, c: u32) -> u32;
}
/// Match-time environment; syntax operations are added with the regexp VM.
pub trait SyntaxAt {
    fn interrupted(&self) -> bool {
        false
    }
}
impl SyntaxAt for () {}
/// The accessible region. The storage seam may split a character.
pub struct Haystack<'a> {
    pub s1: &'a [u8],
    pub s2: &'a [u8],
    pub multibyte: bool,
    pub point: Option<usize>,
}
impl Haystack<'_> {
    pub fn len(&self) -> usize {
        self.s1.len() + self.s2.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Regs {
    pub start: Vec<isize>,
    pub end: Vec<isize>,
}
/// Deterministic work, accumulated across calls. Exact literals never run a VM.
/// `candidates` counts verifications; `verify_bytes` counts compared subject bytes.
/// `window_bytes` counts copied seam bytes, not scanned bytes.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Stats {
    pub compiles: u64,
    pub hits: u64,
    pub candidates: u64,
    pub steps: u64,
    pub pushes: u64,
    pub max_live: u64,
    pub verify_bytes: u64,
    pub window_bytes: u64,
    pub memo_engaged: u64,
    pub memo_inserts: u64,
    pub memo_pruned: u64,
}
#[derive(Debug)]
pub struct RegexError;
#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
    Overflow,
    Interrupted,
}
/// Owned compiled searcher. Neither bounds, point nor direction are retained.
pub struct Regex {
    literal: Literal,
    multibyte: bool,
}
impl Regex {
    /// Inline body plus all owned allocation-request bytes. Excludes allocator
    /// size-class rounding/metadata and any surrounding `Rc` control block.
    pub fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.literal.heap_bytes()
    }

    pub fn literal(pat: &[u8], pm: bool, tm: bool, case: Option<&dyn CaseTable>) -> Self {
        Self {
            literal: Literal::new(pat, pm, tm, case),
            multibyte: tm,
        }
    }
    pub fn nsub(&self) -> usize {
        0
    }
    /// Search inclusive candidate starts, with matches ending no later than stop.
    /// Failure leaves match registers untouched. Direction is the sign of range.
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        h: &Haystack,
        start: usize,
        range: isize,
        stop: usize,
        _env: &impl SyntaxAt,
        regs: &mut Regs,
        st: &mut Stats,
    ) -> Result<Option<usize>, Fail> {
        assert!(start <= h.len() && stop <= h.len());
        debug_assert_eq!(h.multibyte, self.multibyte);
        let end = start.saturating_add_signed(range).min(h.len());
        let (lo, hi) = if range < 0 {
            (end, start)
        } else {
            (start, end)
        };
        let found = if h.s2.is_empty() {
            self.run(&One(h.s1), lo, hi, stop, range < 0, st)
        } else {
            self.run(&Two(h.s1, h.s2), lo, hi, stop, range < 0, st)
        };
        if let Some((a, b)) = found {
            regs.start.clear();
            regs.start.push(a as isize);
            regs.end.clear();
            regs.end.push(b as isize);
        }
        Ok(found.map(|(a, _)| a))
    }
    fn run(
        &self,
        h: &impl Hay,
        lo: usize,
        hi: usize,
        stop: usize,
        back: bool,
        st: &mut Stats,
    ) -> Option<(usize, usize)> {
        debug_assert!(
            !self.multibyte
                || [lo, hi, stop]
                    .iter()
                    .all(|&p| p == h.len() || romacs_chars::char_head_p(h.byte(p)))
        );
        self.literal
            .search(h, lo, hi, stop, back, self.multibyte, st)
    }
    /// Return the matched byte length, not the ending offset.
    pub fn matches(
        &self,
        h: &Haystack,
        at: usize,
        stop: usize,
        env: &impl SyntaxAt,
        regs: &mut Regs,
        st: &mut Stats,
    ) -> Result<Option<usize>, Fail> {
        self.search(h, at, 0, stop, env, regs, st)
            .map(|p| p.map(|_| regs.end[0] as usize - at))
    }
}
