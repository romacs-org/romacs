//! The compiled pattern's bytecode: opcodes and their two-byte operands.
//!
//! GNU: `re_opcode_t` and the `STORE_NUMBER` / `EXTRACT_NUMBER` macros in
//! regex-emacs.c.  The opcode numbers are GNU's, so bytecode dumps of the
//! two engines compare directly.

/// Bytecode opcodes for the compiled regex pattern.
///
/// **Strict GNU parity**: the numeric values here mirror
/// `enum re_opcode_t` in GNU `src/regex-emacs.c:202-337` exactly.
/// A compiled pattern emitted by our compiler is byte-compatible with
/// the same pattern emitted by GNU's compiler — every opcode occupies
/// the same numeric slot, so bytecode dumps can be compared directly
/// during debugging and future external tools can read either
/// without a translation layer.
///
/// The one-byte form we emit via `<op> as u8` is the same as GNU's
/// `BUF_COMPILED[pc++]` byte. **Do not reorder without updating the
/// GNU reference above.**
///
/// Each opcode may be followed by argument bytes in the bytecode buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::FromRepr, strum::VariantArray)]
#[repr(u8)]
pub enum RegexOp {
    /// No operation (padding/alignment). GNU `no_op` = 0.
    NoOp = 0,

    /// Succeed immediately — no more backtracking. GNU `succeed` = 1.
    Succeed = 1,

    /// Match N exact bytes.  Followed by one byte N, then N literal
    /// bytes. GNU `exactn` = 2.
    Exactn = 2,

    /// Match any character (except newline in some modes).
    /// GNU `anychar` = 3.
    AnyChar = 3,

    /// Match character in bitmap set. Same byte layout as GNU
    /// `charset` = 4:
    /// - 1 byte: bitmap length (low 7 bits), high bit = has range table
    /// - N bytes: bitmap (bit per character, low-bit-first)
    /// - Optional range table for multibyte characters
    Charset = 4,

    /// Match character NOT in bitmap set.  Same format as `Charset`.
    /// GNU `charset_not` = 5.
    CharsetNot = 5,

    /// Start remembering text for group N.  Followed by 1 byte: group
    /// number. GNU `start_memory` = 6.
    StartMemory = 6,

    /// Stop remembering text for group N.  Followed by 1 byte: group
    /// number. GNU `stop_memory` = 7.
    StopMemory = 7,

    /// Match duplicate of group N (backreference \N).  Followed by
    /// 1 byte: group number. GNU `duplicate` = 8.
    Duplicate = 8,

    /// Fail unless at beginning of line (^). GNU `begline` = 9.
    BegLine = 9,

    /// Fail unless at end of line ($). GNU `endline` = 10.
    EndLine = 10,

    /// Succeed at beginning of buffer/string. `` \` ``.
    /// GNU `begbuf` = 11.
    BegBuf = 11,

    /// Succeed at end of buffer/string. `\'`.
    /// GNU `endbuf` = 12.
    EndBuf = 12,

    /// Unconditional jump.  Followed by 2-byte signed offset.
    /// GNU `jump` = 13.
    Jump = 13,

    /// Push failure point, then continue.  Followed by 2-byte signed
    /// offset. GNU `on_failure_jump` = 14.
    OnFailureJump = 14,

    /// Like `OnFailureJump` but doesn't restore string position on
    /// failure. GNU `on_failure_keep_string_jump` = 15.
    OnFailureKeepStringJump = 15,

    /// Like `OnFailureJump` but detects infinite empty-match loops.
    /// GNU `on_failure_jump_loop` = 16.
    OnFailureJumpLoop = 16,

    /// Like `OnFailureJumpLoop` but for non-greedy operators.
    /// GNU `on_failure_jump_nastyloop` = 17.
    OnFailureJumpNastyloop = 17,

    /// Smart jump for greedy `*` and `+`.  Analyzes loop to optimize.
    /// GNU `on_failure_jump_smart` = 18.
    OnFailureJumpSmart = 18,

    /// Match N times then jump on failure.  Followed by 2-byte offset
    /// + 2-byte count. GNU `succeed_n` = 19.
    SucceedN = 19,

    /// Jump N times then fail.  Followed by 2-byte offset + 2-byte
    /// count. GNU `jump_n` = 20.
    JumpN = 20,

    /// Set counter at offset.  Followed by 2-byte offset + 2-byte
    /// value. GNU `set_number_at` = 21.
    SetNumberAt = 21,

    /// Succeed at word beginning (syntax-table aware).  `\<`.
    /// GNU `wordbeg` = 22.
    WordBeg = 22,

    /// Succeed at word end (syntax-table aware).  `\>`.
    /// GNU `wordend` = 23.
    WordEnd = 23,

    /// Succeed at word boundary (syntax-table aware).  `\b`.
    /// GNU `wordbound` = 24.
    WordBound = 24,

    /// Succeed at non-word boundary (syntax-table aware).  `\B`.
    /// GNU `notwordbound` = 25.
    NotWordBound = 25,

    /// Succeed at symbol beginning (syntax-table aware).  `\_<`.
    /// GNU `symbeg` = 26.
    SymBeg = 26,

    /// Succeed at symbol end (syntax-table aware).  `\_>`.
    /// GNU `symend` = 27.
    SymEnd = 27,

    /// Match character with syntax class C.  Followed by 1 byte:
    /// syntax code.  `\sC`. GNU `syntaxspec` = 28.
    SyntaxSpec = 28,

    /// Match character without syntax class C.  Followed by 1 byte.
    /// `\SC`. GNU `notsyntaxspec` = 29.
    NotSyntaxSpec = 29,

    /// Succeed if at point.  `\=`. GNU `at_dot` = 30.
    AtDot = 30,

    /// Match character with category C.  Followed by 1 byte: category
    /// code.  `\cC`. GNU `categoryspec` = 31.
    CategorySpec = 31,

    /// Match character without category C.  Followed by 1 byte.
    /// `\CC`. GNU `notcategoryspec` = 32.
    NotCategorySpec = 32,

    /// Match one character whose syntax class is in a set.  Followed by a
    /// 2-byte little-endian bitmask (bit `1 << class`).  This is a
    /// romacs-only fusion op with NO GNU counterpart: the post-compile
    /// peephole `fuse_syntaxspec_alternations` collapses an alternation of
    /// single positive `\sC` / `\w` branches (e.g. `\w\|\s_`) into one op,
    /// replacing per-branch `on_failure_jump` backtracking and repeated
    /// `char_syntax` lookups with a single lookup + mask test.  Because a
    /// character has exactly one syntax class, the fused branches are
    /// mutually exclusive, so this is a semantics-preserving rewrite.
    SyntaxSpecSet = 33,

    /// Terminal op of a POSIX pattern: runs the longest-match bookkeeping
    /// that non-POSIX patterns skip via their trailing `Succeed`
    /// (regex-emacs.c:4272-4344 — the "fell off the end" path, reified as
    /// an opcode so a sealed matcher never needs the per-op end-of-buffer
    /// check).  romacs-only, like `SyntaxSpecSet`.
    PosixEnd = 34,
}

impl RegexOp {
    /// Convert a byte to an opcode.  Returns None for invalid bytes.
    #[inline]
    pub(crate) fn from_byte(b: u8) -> Option<Self> {
        Self::from_repr(b)
    }
}

// ---------------------------------------------------------------------------
// Bytecode helpers
// ---------------------------------------------------------------------------

/// Store a 2-byte signed offset at position in bytecode buffer.
pub(crate) fn store_number(buf: &mut [u8], pos: usize, number: i16) {
    let bytes = number.to_le_bytes();
    buf[pos] = bytes[0];
    buf[pos + 1] = bytes[1];
}

/// Read a 2-byte signed offset from bytecode buffer.
pub(crate) fn extract_number(buf: &[u8], pos: usize) -> i16 {
    i16::from_le_bytes([buf[pos], buf[pos + 1]])
}

/// Store a 2-byte UNSIGNED repeat counter at position in bytecode buffer. GNU
/// counters are `unsigned short` (0..=RE_DUP_MAX), unlike the signed jump
/// offsets stored by `store_number`.
pub(crate) fn store_number_u16(buf: &mut [u8], pos: usize, number: u16) {
    let bytes = number.to_le_bytes();
    buf[pos] = bytes[0];
    buf[pos + 1] = bytes[1];
}

/// Read a 2-byte UNSIGNED repeat counter from bytecode buffer.
pub(crate) fn extract_number_u16(buf: &[u8], pos: usize) -> u16 {
    u16::from_le_bytes([buf[pos], buf[pos + 1]])
}

/// Byte length of the opcode at `pc`, or `None` for a malformed buffer.
pub(crate) fn opcode_len(bytecode: &[u8], pc: usize) -> Option<usize> {
    let op = RegexOp::from_byte(*bytecode.get(pc)?)?;
    Some(match op {
        RegexOp::NoOp
        | RegexOp::Succeed
        | RegexOp::PosixEnd
        | RegexOp::AnyChar
        | RegexOp::BegLine
        | RegexOp::EndLine
        | RegexOp::BegBuf
        | RegexOp::EndBuf
        | RegexOp::AtDot
        | RegexOp::WordBound
        | RegexOp::NotWordBound
        | RegexOp::WordBeg
        | RegexOp::WordEnd
        | RegexOp::SymBeg
        | RegexOp::SymEnd => 1,
        RegexOp::Exactn => 2 + *bytecode.get(pc + 1)? as usize,
        RegexOp::Charset | RegexOp::CharsetNot => 2 + (*bytecode.get(pc + 1)? & 0x7F) as usize,
        RegexOp::StartMemory
        | RegexOp::StopMemory
        | RegexOp::Duplicate
        | RegexOp::SyntaxSpec
        | RegexOp::NotSyntaxSpec
        | RegexOp::CategorySpec
        | RegexOp::NotCategorySpec => 2,
        RegexOp::Jump
        | RegexOp::OnFailureJump
        | RegexOp::OnFailureKeepStringJump
        | RegexOp::OnFailureJumpLoop
        | RegexOp::OnFailureJumpNastyloop
        | RegexOp::OnFailureJumpSmart
        | RegexOp::SyntaxSpecSet => 3,
        RegexOp::SucceedN | RegexOp::JumpN | RegexOp::SetNumberAt => 5,
    })
}
