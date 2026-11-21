//! What every match of a pattern contains, in ASCII, so that a short subject
//! lacking it fails before the matcher, the existence DFA and the candidate
//! scan.
//!
//! GNU has no counterpart: `fast_string_match` runs the matcher on every
//! file-name handler and `auto-mode-alist` entry.  The prefix literals are
//! the literal prefilter's ([`prefix_literal_set`]); the bytes are found by
//! reachability over the bytecode.

use crate::engine::opcode::{RegexOp, extract_number, opcode_len};
use crate::engine::pattern::CompiledPattern;
use crate::engine::scan::prefix_literal_set;
use smallvec::SmallVec;

/// Most bytes tried as required ones per pattern (each costs a walk of the
/// bytecode once, and a memchr per short search).
const MAX_REQUIRED_BYTES: usize = 4;

/// What every match contains.  A subject holding none of the prefixes, or
/// missing one of the bytes, cannot match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequiredLiterals {
    /// One of these starts every match (empty: no prefix set is proven).
    /// ASCII only, so they are the same bytes in unibyte and multibyte text.
    prefixes: Vec<Vec<u8>>,
    /// The first bytes of `prefixes`.
    first: [bool; 256],
    /// ASCII bytes every match consumes somewhere.
    bytes: SmallVec<[u8; MAX_REQUIRED_BYTES]>,
}

impl RequiredLiterals {
    /// Whether `text` (from a search's lowest candidate to its stop) holds
    /// what every match needs.
    pub(crate) fn admits(&self, text: &[u8]) -> bool {
        self.bytes.iter().all(|&byte| memchr::memchr(byte, text).is_some())
            && (self.prefixes.is_empty() || self.holds_a_prefix(text))
    }

    fn holds_a_prefix(&self, text: &[u8]) -> bool {
        text.iter().enumerate().any(|(at, &byte)| {
            self.first[byte as usize] && self.prefixes.iter().any(|prefix| text[at..].starts_with(prefix))
        })
    }
}

/// What every match of `pattern` contains, or `None` when nothing is proven
/// or the pattern is case-folded (its translation may map any byte onto a
/// literal's) or matches the empty string.
pub(crate) fn derive_required_literals(pattern: &CompiledPattern) -> Option<RequiredLiterals> {
    if pattern.translate.is_some() || pattern.can_be_null || pattern.buffer.is_empty() {
        return None;
    }
    let prefixes = ascii_prefixes(pattern);
    let mut first = [false; 256];
    for prefix in &prefixes {
        first[prefix[0] as usize] = true;
    }
    let bytes: SmallVec<[u8; MAX_REQUIRED_BYTES]> = literal_ascii_bytes(&pattern.buffer)
        .into_iter()
        .filter(|&byte| prefixes.is_empty() || !prefixes.iter().all(|prefix| prefix.contains(&byte)))
        .filter(|&byte| reaches_end_without(&pattern.buffer, byte) == Some(false))
        .take(MAX_REQUIRED_BYTES)
        .collect();
    (!prefixes.is_empty() || !bytes.is_empty()).then_some(RequiredLiterals { prefixes, first, bytes })
}

/// The prefix set, each literal cut at its first non-ASCII byte (a shorter
/// prefix of a required prefix is still required); empty when some literal
/// starts with a non-ASCII byte or no set is proven.
fn ascii_prefixes(pattern: &CompiledPattern) -> Vec<Vec<u8>> {
    let Some(literals) = prefix_literal_set(pattern) else {
        return Vec::new();
    };
    let cut: Vec<Vec<u8>> = literals
        .into_iter()
        .map(|literal| literal.into_iter().take_while(u8::is_ascii).collect::<Vec<u8>>())
        .collect();
    if cut.iter().any(Vec::is_empty) {
        return Vec::new();
    }
    cut
}

/// The distinct ASCII bytes of the pattern's literals, in bytecode order.  An
/// ASCII byte inside an `Exactn` is that character in either representation:
/// every byte of a multibyte or raw-byte sequence is at least 0x80.
fn literal_ascii_bytes(bytecode: &[u8]) -> Vec<u8> {
    let mut seen = [false; 128];
    let mut out = Vec::new();
    let mut pc = 0;
    while pc < bytecode.len() {
        let Some(len) = opcode_len(bytecode, pc) else {
            return Vec::new();
        };
        if bytecode[pc] == RegexOp::Exactn as u8 {
            for &byte in bytecode.get(pc + 2..pc + len).unwrap_or_default() {
                if byte.is_ascii() && !seen[byte as usize] {
                    seen[byte as usize] = true;
                    out.push(byte);
                }
            }
        }
        pc += len;
    }
    out
}

/// Whether some path from the pattern's start reaches a success without
/// passing a literal holding `byte`: every branch is taken both ways and every
/// counter is ignored, so the answer over-approximates the matcher's paths
/// (`Some(false)` proves `byte` required).  `None`: malformed bytecode.
fn reaches_end_without(bytecode: &[u8], byte: u8) -> Option<bool> {
    let mut seen = vec![false; bytecode.len()];
    let mut stack = vec![0usize];
    while let Some(pc) = stack.pop() {
        if pc >= bytecode.len() {
            return Some(true);
        }
        if std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        let op = RegexOp::from_byte(bytecode[pc])?;
        let len = opcode_len(bytecode, pc)?;
        let target = || -> Option<usize> {
            let offset = i64::from(extract_number(bytecode, pc + 1));
            usize::try_from(i64::try_from(pc).ok()? + 3 + offset).ok()
        };
        match op {
            RegexOp::Succeed | RegexOp::PosixEnd => return Some(true),
            RegexOp::Exactn => {
                if !bytecode.get(pc + 2..pc + len)?.contains(&byte) {
                    stack.push(pc + len);
                }
            }
            RegexOp::Jump => stack.push(target()?),
            RegexOp::OnFailureJump
            | RegexOp::OnFailureKeepStringJump
            | RegexOp::OnFailureJumpLoop
            | RegexOp::OnFailureJumpNastyloop
            | RegexOp::OnFailureJumpSmart
            | RegexOp::SucceedN
            | RegexOp::JumpN => {
                stack.push(pc + len);
                stack.push(target()?);
            }
            _ => stack.push(pc + len),
        }
    }
    Some(false)
}
