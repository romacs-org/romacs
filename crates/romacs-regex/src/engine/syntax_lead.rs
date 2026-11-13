//! The syntax-aware first-character set: candidates for a pattern that can
//! start with `\sC`.
//!
//! GNU's `analyze_first` gives up on a leading `\sC` ("this match depends
//! on text properties"), and `re_search_2` then enters the matcher at every
//! position.  The class of a character is the table's wherever no
//! `syntax-table` property applies, and the host says where that is
//! ([`SyntaxLookup::plain_syntax_until`]), so there a search passes the
//! characters no path can start with, as a fastmap would.

use crate::engine::opcode::{RegexOp, extract_number, extract_number_u16, opcode_len};
use crate::engine::pattern::CompiledPattern;
use crate::engine::syntax::SyntaxLookup;

/// What a match can start with: one of `bytes`, a character whose syntax
/// class is in `classes` (bit `1 << class`), or any non-ASCII character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxLead {
    bytes: [bool; 128],
    classes: u16,
}

/// The answers for the ASCII bytes one search has asked about.
pub(crate) struct LeadMemo([Option<bool>; 128]);
impl LeadMemo {
    pub(crate) fn new() -> Self {
        Self([None; 128])
    }
}

impl SyntaxLead {
    #[cfg(test)]
    pub(crate) fn parts(&self) -> (Vec<u8>, u16) {
        ((0..128u8).filter(|&byte| self.bytes[usize::from(byte)]).collect(), self.classes)
    }

    /// Whether a match can start with `byte`, where the table alone gives
    /// its class.  A byte inside a character starts none.
    #[inline]
    pub(crate) fn admits(&self, byte: u8, multibyte: bool, memo: &mut LeadMemo, syntax: &dyn SyntaxLookup) -> bool {
        if byte >= 0x80 {
            return !multibyte || (byte & 0xC0) != 0x80;
        }
        *memo.0[usize::from(byte)].get_or_insert_with(|| {
            self.bytes[usize::from(byte)] || self.classes >> (syntax.char_syntax(char::from(byte)) as u16) & 1 != 0
        })
    }
}

/// The opcodes that consume a character, reachable from `starts` before
/// anything is consumed, every zero-width test taken as holding.  `None`
/// when a path succeeds first, or meets an opcode whose first character is
/// not one character (a back reference).
fn consumers(bytecode: &[u8], starts: &[usize]) -> Option<Vec<usize>> {
    let mut visited = vec![false; bytecode.len()];
    let mut stack = starts.to_vec();
    let mut found = Vec::new();
    while let Some(pc) = stack.pop() {
        if pc >= bytecode.len() {
            return None;
        }
        if std::mem::replace(&mut visited[pc], true) {
            continue;
        }
        let op = RegexOp::from_byte(bytecode[pc])?;
        let next = pc + opcode_len(bytecode, pc)?;
        let target = |at: usize| usize::try_from(i64::try_from(at).ok()? + 2 + i64::from(extract_number(bytecode, at))).ok();
        match op {
            RegexOp::Succeed | RegexOp::PosixEnd | RegexOp::Duplicate => return None,
            RegexOp::Exactn
            | RegexOp::AnyChar
            | RegexOp::Charset
            | RegexOp::CharsetNot
            | RegexOp::SyntaxSpec
            | RegexOp::NotSyntaxSpec
            | RegexOp::SyntaxSpecSet
            | RegexOp::CategorySpec
            | RegexOp::NotCategorySpec => found.push(pc),
            RegexOp::NoOp
            | RegexOp::StartMemory
            | RegexOp::StopMemory
            | RegexOp::BegLine
            | RegexOp::EndLine
            | RegexOp::BegBuf
            | RegexOp::EndBuf
            | RegexOp::WordBound
            | RegexOp::NotWordBound
            | RegexOp::WordBeg
            | RegexOp::WordEnd
            | RegexOp::SymBeg
            | RegexOp::SymEnd
            | RegexOp::AtDot
            | RegexOp::SetNumberAt => stack.push(next),
            RegexOp::Jump => stack.push(target(pc + 1)?),
            // Either edge, whatever the counter holds.
            RegexOp::OnFailureJump
            | RegexOp::OnFailureKeepStringJump
            | RegexOp::OnFailureJumpLoop
            | RegexOp::OnFailureJumpNastyloop
            | RegexOp::OnFailureJumpSmart
            | RegexOp::SucceedN
            | RegexOp::JumpN => {
                stack.push(next);
                stack.push(target(pc + 1)?);
            }
        }
    }
    Some(found)
}

/// The first-character set of `pattern`, when a syntax class is part of it
/// and it is not everything.  The fastmap serves a pattern without one.
///
/// Sound because it over-approximates: every zero-width test is taken as
/// holding and both edges of every branch and counter are followed.  A
/// case-folding pattern has none: its bytes are translated ones.
pub(crate) fn syntax_lead(pattern: &CompiledPattern) -> Option<SyntaxLead> {
    if pattern.translate.is_some() {
        return None;
    }
    let bytecode = &pattern.buffer[..];
    let mut lead = SyntaxLead { bytes: [false; 128], classes: 0 };
    for pc in consumers(bytecode, &[0])? {
        match RegexOp::from_byte(bytecode[pc])? {
            RegexOp::Exactn => {
                if let Some(slot) = lead.bytes.get_mut(usize::from(*bytecode.get(pc + 2)?)) {
                    *slot = true;
                }
            }
            RegexOp::Charset
                if !pattern.multibyte_charsets.contains_key(&pc)
                    && !pattern.charset_class_bits.contains_key(&pc) =>
            {
                let bitmap = bytecode.get(pc + 2..pc + 2 + usize::from(bytecode[pc + 1] & 0x7F))?;
                for (byte, slot) in lead.bytes.iter_mut().enumerate() {
                    *slot |= bitmap.get(byte / 8).is_some_and(|bits| bits >> (byte % 8) & 1 != 0);
                }
            }
            RegexOp::SyntaxSpec => lead.classes |= 1u16.checked_shl(u32::from(*bytecode.get(pc + 1)?))?,
            RegexOp::NotSyntaxSpec => lead.classes |= !1u16.checked_shl(u32::from(*bytecode.get(pc + 1)?))?,
            RegexOp::SyntaxSpecSet => lead.classes |= extract_number_u16(bytecode, pc + 1),
            _ => return None,
        }
    }
    (lead.classes != 0 && lead.classes != u16::MAX).then_some(lead)
}
