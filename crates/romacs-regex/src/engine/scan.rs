//! How a search finds its next candidate start beyond the plain fastmap:
//! the multi-literal prefilter, the sparse ASCII fastmap, and case-folded
//! scans.
//!
//! GNU: the fastmap skip loop of `re_search_2` in regex-emacs.c, which
//! these extend.

use crate::engine::opcode::{RegexOp, extract_number};
use crate::engine::pattern::{CompiledPattern, LiteralPrefilter};
use crate::engine::translate::{AsciiPreimage, CaseTranslation, ascii_translated_leading_code};
use regex_automata::MatchKind;
use regex_automata::util::prefilter::Prefilter as RaPrefilter;
use smallvec::SmallVec;

/// Cap on the number of distinct required literals handed to the prefilter.
/// Big keyword alternations (`el-defs` has ~40 `def*` heads) must fit; beyond
/// this, bail to the fastmap (the needle set stops being a useful filter).
const PREFILTER_MAX_LITERALS: usize = 128;
/// Cap on a single literal's length.  Reaching it finalizes the required
/// prefix early — a SHORTER required prefix is still a sound required literal
/// (every match on that path still starts with it), so this only trades
/// specificity for a bounded needle, never correctness.
const PREFILTER_MAX_LITERAL_LEN: usize = 64;
/// Total opcode-visit budget for the extraction walk; guarantees termination
/// on pathological bytecode (returns `None`, i.e. no prefilter — always safe).
const PREFILTER_BUDGET: usize = 16_384;

/// Build a SOUND multi-literal prefilter from the compiled bytecode, or
/// `None` when no required-literal set can be proven (then `re_search` keeps
/// the byte fastmap — always correct).
///
/// SOUNDNESS is the only correctness requirement: the backtracker verifies
/// every candidate the prefilter surfaces, so a prefilter is correct as long
/// as **every match contains one of the returned literals at
/// `offset` bytes from the match start**.  A prefilter that skips *past* a
/// real match (an unsound literal) is a bug caught by the differential fuzzer.
///
/// The extraction enumerates the required *offset-0* prefix set of the
/// pattern (`collect_prefix_literals`): a run of leading `Exactn`s (with
/// zero-width assertions / group markers skipped) forms a forced prefix, and
/// a leading alternation whose every arm begins with an `Exactn` splits that
/// prefix across the arms (`(\(catch\|throw\|require\)` → `["(catch",
/// "(throw", "(require"]`).  If ANY reachable path can begin a match without a
/// literal byte at offset 0 (a leading `\w` / charset / `.` / nullable arm),
/// extraction bails — no prefilter.  Offset is always `0`: a required literal
/// at a fixed byte offset k>0 would need a fixed-byte-width non-literal prefix,
/// which is fragile under multibyte, so it is deliberately not attempted
/// (conservative = sound).
///
/// A case-folded pattern stores its literals translated; its needles are
/// every ASCII spelling of each literal's ASCII prefix
/// ([`fold_prefix_literals`]).  That is sound only while no non-ASCII
/// character translates into ASCII (`AsciiPreimage::AsciiOnly`): a case-canon
/// char-table that does not meet it at build time gets no prefilter, and
/// `re_search` checks it again before using one.  Patterns whose
/// only required literals are single bytes are skipped too — the fastmap's
/// `memchr` already covers those, so a prefilter would add cost without
/// narrowing the candidate set.
pub(crate) fn build_literal_prefilter(pattern: &CompiledPattern) -> Option<LiteralPrefilter> {
    let fold = match pattern.translate.as_ref() {
        None => None,
        Some(table) => match table.ascii_preimage() {
            AsciiPreimage::AsciiOnly => Some(table),
            // Any character may fold into a needle byte.
            AsciiPreimage::Unknown => return None,
        },
    };
    // Nullable patterns match the empty string, so no non-empty literal is
    // required.  `re_search` also disables the fastmap skip for these
    // (GNU regex-emacs.c:3483) — keep the same gate.
    if pattern.can_be_null {
        return None;
    }
    if pattern.buffer.is_empty() {
        return None;
    }

    let mut literals: Vec<Vec<u8>> = Vec::new();
    let mut budget = PREFILTER_BUDGET;
    let mut visited: Vec<usize> = Vec::new();
    // A `None` here means some match path has no proven offset-0 literal —
    // extraction is unsound to use, so no prefilter at all.
    collect_prefix_literals(
        pattern,
        0,
        Vec::new(),
        &mut literals,
        &mut budget,
        &mut visited,
    )?;

    if literals.is_empty() {
        return None;
    }
    let mut literals = match fold {
        None => literals,
        Some(table) => fold_prefix_literals(table, &literals)?,
    };
    // Deduplicate (nested alternations can re-derive the same prefix).
    literals.sort();
    literals.dedup();
    // If every required literal is a single byte, the fastmap's memchr scan
    // already skips exactly these positions — a prefilter buys nothing.
    if literals.iter().all(|l| l.len() <= 1) {
        return None;
    }

    // `regex-automata` auto-selects memchr / memmem / Teddy / Aho-Corasick.
    let pf = RaPrefilter::new(MatchKind::LeftmostFirst, &literals)?;
    // Only adopt it when the crate deems the scan actually fast (otherwise the
    // fastmap path is at least as good).
    if !pf.is_fast() {
        return None;
    }
    if fold.is_some() {
        tracing::debug!(
            target: "romacs::regex",
            needles = literals.len(),
            "case-folded literal prefilter"
        );
    }
    Some(LiteralPrefilter { pf, offset: 0 })
}

/// Caps on the ASCII case variants of one literal, tried in turn until the
/// whole needle set fits [`FOLDED_VARIANTS_MAX`].  A literal is cut where its
/// variants would pass the cap.
const FOLDED_VARIANT_CAPS: [usize; 4] = [16, 8, 4, 2];
/// Cap on a case-folded needle set: Teddy stays fast to about 64 needles
/// (regex-automata's teddy.rs), and aho-corasick's packed searcher takes at
/// most 128.
const FOLDED_VARIANTS_MAX: usize = 64;

/// Replace each required prefix literal of a case-folded pattern by every
/// byte string a match can start with.
///
/// Sound: the pattern stores its literals translated, and the matcher
/// accepts a text character `c` against a stored ASCII character `x` iff
/// `translate(c) == x` (`match_exactn_char_at` and the `Exactn` run).  Under
/// `AsciiPreimage::AsciiOnly` every such `c` is an ASCII byte, one byte of
/// text; a unibyte text's bytes from 0x80 up are characters from 0x80 up (or
/// raw bytes, never translated), so they never match an ASCII `x` either.
/// So a match starting at `p` has, at `p`, one of the returned spellings of
/// its literal's ASCII prefix.  Cutting a literal short -- at its first
/// non-ASCII byte, or where its variants would pass the cap -- keeps it a
/// required prefix.  `None` (no prefilter) when a literal starts with a
/// non-ASCII byte, or holds an ASCII byte no character translates to.
#[cold]
#[inline(never)]
pub(crate) fn fold_prefix_literals(
    table: &CaseTranslation,
    literals: &[Vec<u8>],
) -> Option<Vec<Vec<u8>>> {
    let mut preimage: [SmallVec<[u8; 2]>; 0x80] = std::array::from_fn(|_| SmallVec::new());
    for y in 0..0x80u8 {
        let x = table.translate(u32::from(y));
        if x < 0x80 {
            preimage[x as usize].push(y);
        }
    }
    'caps: for cap in FOLDED_VARIANT_CAPS {
        let mut out: Vec<Vec<u8>> = Vec::new();
        for literal in literals {
            let mut variants: Vec<Vec<u8>> = vec![Vec::with_capacity(literal.len())];
            for &x in literal {
                if x >= 0x80 {
                    break;
                }
                let spellings = &preimage[x as usize];
                if spellings.is_empty() {
                    // Nothing translates to `x`: the literal cannot match.
                    return None;
                }
                if variants.len() * spellings.len() > cap {
                    break;
                }
                variants = variants
                    .iter()
                    .flat_map(|prefix| {
                        spellings.iter().map(move |&y| {
                            let mut spelled = prefix.clone();
                            spelled.push(y);
                            spelled
                        })
                    })
                    .collect();
            }
            if variants[0].is_empty() {
                // A leading non-ASCII character: no ASCII prefix to require.
                return None;
            }
            out.extend(variants);
            if out.len() > FOLDED_VARIANTS_MAX {
                continue 'caps;
            }
        }
        return Some(out);
    }
    None
}

/// Finalize a branch's accumulated required prefix.  An EMPTY prefix means the
/// path can begin a match without any offset-0 literal, so the whole pattern
/// is un-prefilterable (return `None`); a non-empty prefix is a sound required
/// literal for every match on this path.
fn finalize_prefix(prefix: Vec<u8>, out: &mut Vec<Vec<u8>>) -> Option<()> {
    if prefix.is_empty() {
        return None;
    }
    out.push(prefix);
    Some(())
}

/// Walk the bytecode from `pc`, accumulating the forced literal `prefix`
/// (offset 0), branching at alternation / quantifier `on_failure_jump`s.  On
/// every terminal (a non-literal consuming op, end-of-pattern, `succeed`, a
/// jump, or a loop back-edge) the accumulated prefix is finalized as one
/// required literal.  Returns `None` (bail the whole prefilter) if any path
/// finalizes with an empty prefix, if the budget/literal caps are exceeded, or
/// if the bytecode is malformed.
fn collect_prefix_literals(
    buf: &CompiledPattern,
    mut pc: usize,
    mut prefix: Vec<u8>,
    out: &mut Vec<Vec<u8>>,
    budget: &mut usize,
    visited: &mut Vec<usize>,
) -> Option<()> {
    let bytecode = &buf.buffer;
    loop {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        if out.len() > PREFILTER_MAX_LITERALS {
            return None;
        }
        // End of pattern: a match may end here.  The prefix is required iff
        // non-empty (empty => a nullable/empty-arm path => unsound).
        if pc >= bytecode.len() {
            return finalize_prefix(prefix, out);
        }
        // Loop back-edge already on this path: stop and finalize.  The
        // accumulated prefix (>= one body iteration's literal) is still
        // required on the "took the loop" path; the exit branch (enumerated at
        // the loop's on_failure_jump) covers the "skipped it" path.
        if visited.contains(&pc) {
            return finalize_prefix(prefix, out);
        }
        visited.push(pc);

        let op = RegexOp::from_byte(bytecode[pc])?;
        match op {
            RegexOp::Exactn => {
                let count = *bytecode.get(pc + 1)? as usize;
                if count == 0 || pc + 2 + count > bytecode.len() {
                    return None;
                }
                prefix.extend_from_slice(&bytecode[pc + 2..pc + 2 + count]);
                if prefix.len() >= PREFILTER_MAX_LITERAL_LEN {
                    // Sound early cut: a shorter required prefix still holds.
                    return finalize_prefix(prefix, out);
                }
                pc += 2 + count;
            }

            // Zero-width assertions and bookkeeping: do not consume input or
            // shift the offset — walk through them, offset unchanged.
            RegexOp::BegLine
            | RegexOp::EndLine
            | RegexOp::BegBuf
            | RegexOp::EndBuf
            | RegexOp::AtDot
            | RegexOp::WordBound
            | RegexOp::NotWordBound
            | RegexOp::WordBeg
            | RegexOp::WordEnd
            | RegexOp::SymBeg
            | RegexOp::SymEnd
            | RegexOp::NoOp => {
                pc += 1;
            }

            RegexOp::StartMemory | RegexOp::StopMemory => {
                pc += 2;
            }

            // Alternation / quantifier split: BOTH the fallthrough arm and the
            // jump target can begin a match, and every match takes exactly one
            // path, so the UNION of both arms' required literals is sound.
            RegexOp::OnFailureJump
            | RegexOp::OnFailureKeepStringJump
            | RegexOp::OnFailureJumpLoop
            | RegexOp::OnFailureJumpNastyloop
            | RegexOp::OnFailureJumpSmart => {
                if pc + 2 >= bytecode.len() {
                    return None;
                }
                let offset = extract_number(bytecode, pc + 1) as i64;
                let target = ((pc as i64) + 3 + offset) as usize;
                let fallthrough = pc + 3;
                let mut v1 = visited.clone();
                collect_prefix_literals(buf, fallthrough, prefix.clone(), out, budget, &mut v1)?;
                let mut v2 = visited.clone();
                return collect_prefix_literals(buf, target, prefix, out, budget, &mut v2);
            }

            // Every other op either consumes NON-literal input (AnyChar,
            // Charset(Not), Syntax/Category specs, Duplicate) or is a control
            // terminal for this analysis (Succeed, Jump, SucceedN, JumpN,
            // SetNumberAt).  In all cases the required prefix so far is
            // finalized; a non-literal consuming op after a non-empty prefix is
            // fine (the prefix is still required), and reaching one with an
            // empty prefix means no offset-0 literal is required => bail.
            _ => return finalize_prefix(prefix, out),
        }
    }
}

/// A compact, allocation-free candidate set for the memchr skip loop.
#[derive(Clone, Copy, Debug)]
pub enum SparseAsciiFastmap {
    One(u8),
    Two(u8, u8),
    Three(u8, u8, u8),
}

/// Representation of the searched text (`CompiledPattern::target_multibyte`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRepr {
    Unibyte,
    Multibyte,
}

impl TextRepr {
    #[inline]
    pub(crate) fn of(target_multibyte: bool) -> Self {
        if target_multibyte {
            Self::Multibyte
        } else {
            Self::Unibyte
        }
    }
}

/// How a case-folded search finds its next fastmap candidate.
#[derive(Clone, Debug)]
pub enum FoldedScan {
    /// The candidates are exactly the positions holding one of 1-3 ASCII
    /// bytes: memchr finds them.
    Sparse(SparseAsciiFastmap),
    /// The candidates are exactly the positions `p` with `accept[text[p]]`.
    Table(Box<[bool; 256]>),
    /// Candidacy needs a decoded character's translated leading code, or the
    /// translation is a char-table: keep the per-character loop.
    PerChar,
}

/// The case-folded candidate scans of one pattern, one per text
/// representation: the multibyte scan is derived from `fastmap_translated`,
/// the unibyte one from `fastmap`.
#[derive(Clone, Debug, Default)]
pub(crate) struct FoldedScans {
    pub(crate) unibyte: std::cell::OnceCell<FoldedScan>,
    pub(crate) multibyte: std::cell::OnceCell<FoldedScan>,
}

/// Fold `TRANSLATE` into the fastmap, once per pattern and representation.
///
/// The result has EXACTLY the candidates of the per-character loops in
/// `re_search`, position for position:
///
/// - Unibyte text: `accept[b]` is, by construction, the loops' test
///   `fastmap[translate_byte(b)]`.
/// - Multibyte text: the loops test every ASCII byte, which is
///   `accept[b]`; skip continuation bytes; and test a lead byte through the
///   translated character's leading code.  Under `AsciiPreimage::AsciiOnly`
///   a non-ASCII character translates to a non-ASCII one, whose leading code
///   is at least 0xC0, and the gate below has established that
///   `fastmap_translated[0x80..]` is all false: no lead byte is a candidate
///   in either scan.  (Valid internal text only: an overlong form such as
///   `E0 81 81`, which no Lisp string or buffer holds, decodes to ASCII.)
/// - The end of the text is tried by both, when in range.
///
/// A case-canon char-table translation qualifies while it is `AsciiOnly`:
/// asking fills its ASCII byte-memo slots, which the table then reads (and
/// the matcher reads) as frozen from then on, exactly as the loops would
/// after meeting those bytes; the unibyte table fills the other slots the
/// same way.  A later char-table write can make it `Unknown`, so every
/// search checks `ascii_preimage` before using a scan built here.
#[cold]
#[inline(never)]
pub(crate) fn build_folded_scan(pattern: &CompiledPattern, table: &CaseTranslation) -> FoldedScan {
    let scan = fold_translation_into_fastmap(pattern, table);
    let multibyte = pattern.target_multibyte;
    match &scan {
        FoldedScan::Table(accept) => tracing::debug!(
            target: "romacs::regex",
            accepted_bytes = accept.iter().filter(|&&accepted| accepted).count(),
            multibyte,
            "case-folded candidate scan: table"
        ),
        _ => tracing::debug!(
            target: "romacs::regex",
            ?scan,
            multibyte,
            "case-folded candidate scan"
        ),
    }
    scan
}

/// The body of [`build_folded_scan`].
fn fold_translation_into_fastmap(pattern: &CompiledPattern, table: &CaseTranslation) -> FoldedScan {
    if table.ascii_preimage() != AsciiPreimage::AsciiOnly {
        return FoldedScan::PerChar;
    }
    let mut accept = [false; 256];
    match TextRepr::of(pattern.target_multibyte) {
        TextRepr::Multibyte => {
            // A non-ASCII leading code in the map needs the decoded character.
            if pattern.fastmap_translated[0x80..].contains(&true) {
                return FoldedScan::PerChar;
            }
            for byte in 0..0x80u8 {
                accept[byte as usize] =
                    pattern.fastmap_translated[ascii_translated_leading_code(table, byte)];
            }
        }
        TextRepr::Unibyte => {
            for byte in 0..=u8::MAX {
                accept[byte as usize] = pattern.fastmap[table.translate_byte(byte) as usize];
            }
        }
    }
    match sparse_ascii_fastmap(&accept) {
        Some(bytes) => FoldedScan::Sparse(bytes),
        None => FoldedScan::Table(Box::new(accept)),
    }
}

/// Derive a small (1–3 byte), pure ASCII set once per compiled fastmap.
/// Repeated short searches otherwise scan the same 256 entries on every call.
pub(crate) fn sparse_ascii_fastmap(fastmap: &[bool; 256]) -> Option<SparseAsciiFastmap> {
    let mut bytes = [0; 3];
    let mut len = 0;
    for (byte, &set) in fastmap.iter().enumerate() {
        if set {
            if byte >= 0x80 || len == bytes.len() {
                return None;
            }
            bytes[len] = byte as u8;
            len += 1;
        }
    }
    match len {
        1 => Some(SparseAsciiFastmap::One(bytes[0])),
        2 => Some(SparseAsciiFastmap::Two(bytes[0], bytes[1])),
        3 => Some(SparseAsciiFastmap::Three(bytes[0], bytes[1], bytes[2])),
        _ => None,
    }
}
