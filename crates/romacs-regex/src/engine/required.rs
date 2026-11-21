//! The ASCII bytes every match of a pattern consumes, so that a short search
//! whose text lacks one fails before the candidate scan, the existence DFA
//! and the matcher: TRAMP's handler pattern needs a `:`, which an ordinary
//! file name has not, and its miss otherwise runs the backtracker.
//!
//! GNU has no counterpart: `fast_string_match` runs the matcher on every
//! file-name handler and `auto-mode-alist` entry.  A byte is required when no
//! path from the pattern's start reaches a success without a literal holding
//! it (reachability over the bytecode).

use crate::engine::opcode::{RegexOp, extract_number, opcode_len};
use crate::engine::pattern::{CompiledPattern, PREFILTER_MIN_BUILD_SPAN};
use smallvec::SmallVec;
use std::cell::{Cell, OnceCell};

/// Most bytes kept per pattern (each costs a walk of the bytecode once, and a
/// memchr per short search).
const MAX_REQUIRED_BYTES: usize = 4;

/// Where a pattern stands in finding its required bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequiredState {
    /// Not yet searched over a short span.
    Unseen,
    /// One short search done: the second derives the bytes, so a pattern
    /// searched once pays nothing for them.
    SeenOnce,
    /// Nothing to test: no byte is required, or the pattern is case-folded (a
    /// translation may map any byte onto a literal's) or matches the empty
    /// string.  Every later search reads this one state and goes on.
    Absent,
    /// The bytes are derived and tested on each short search.
    Bytes,
}

/// A compiled pattern's required bytes and how far they are known.
#[derive(Clone, Debug)]
pub(crate) struct RequiredBytes {
    state: Cell<RequiredState>,
    bytes: OnceCell<SmallVec<[u8; MAX_REQUIRED_BYTES]>>,
}

impl Default for RequiredBytes {
    fn default() -> Self {
        Self { state: Cell::new(RequiredState::Unseen), bytes: OnceCell::new() }
    }
}

impl RequiredBytes {
    /// Whether a search should ask [`rejects`]: one load, false for good once
    /// the pattern has nothing to test.
    #[inline]
    pub(crate) fn may_reject(&self) -> bool {
        self.state.get() != RequiredState::Absent
    }

    #[cfg(test)]
    pub(crate) fn state(&self) -> RequiredState {
        self.state.get()
    }
}

/// Whether a search of `pattern` from `start` over `range` (`re_search`'s
/// arguments) provably fails: its span, from the lowest candidate to the stop
/// every match ends by (the range's end forward, `start` backward), is shorter
/// than the prefilter's build span and lacks a required byte.  Out of line, so
/// a search that has nothing to test pays only [`RequiredBytes::may_reject`].
#[inline(never)]
pub(crate) fn rejects(pattern: &CompiledPattern, text: &[u8], start: usize, range: isize) -> bool {
    let required = &pattern.required;
    if pattern.translate.is_some() || pattern.can_be_null || pattern.buffer.is_empty() {
        required.state.set(RequiredState::Absent);
        return false;
    }
    let (lowest, stop) = if range < 0 {
        (start.saturating_sub(range.unsigned_abs()), start)
    } else {
        (start, start.saturating_add(range.unsigned_abs()))
    };
    let stop = stop.min(text.len());
    if lowest >= stop || stop - lowest >= PREFILTER_MIN_BUILD_SPAN {
        return false;
    }
    match required.state.get() {
        RequiredState::Unseen => {
            required.state.set(RequiredState::SeenOnce);
            return false;
        }
        RequiredState::SeenOnce => {
            let bytes = derive_required_bytes(pattern);
            required.state.set(if bytes.is_empty() { RequiredState::Absent } else { RequiredState::Bytes });
            let _ = required.bytes.set(bytes);
        }
        RequiredState::Absent | RequiredState::Bytes => {}
    }
    let span = &text[lowest..stop];
    required
        .bytes
        .get()
        .is_some_and(|bytes| bytes.iter().any(|&byte| memchr::memchr(byte, span).is_none()))
}

/// The ASCII bytes every match of `pattern` consumes (at most
/// [`MAX_REQUIRED_BYTES`]); empty for a case-folded or nullable pattern.
pub(crate) fn derive_required_bytes(pattern: &CompiledPattern) -> SmallVec<[u8; MAX_REQUIRED_BYTES]> {
    if pattern.translate.is_some() || pattern.can_be_null || pattern.buffer.is_empty() {
        return SmallVec::new();
    }
    literal_ascii_bytes(&pattern.buffer)
        .into_iter()
        .filter(|&byte| reaches_end_without(&pattern.buffer, byte) == Some(false))
        .take(MAX_REQUIRED_BYTES)
        .collect()
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
