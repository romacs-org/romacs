use crate::engine::compile::RegexCompileError;
use crate::engine::dfa;
use crate::engine::linear;
use crate::engine::opcode::{RegexOp, store_number, store_number_u16};
use crate::engine::pattern::{CompiledPattern, RewindView};
use crate::engine::scan::FoldedScans;
use crate::engine::search::StartAnchor;
use rustc_hash::FxHashMap;

impl CompiledPattern {
    seam! {
        // Not a `Default`: an empty pattern is only a compiler starting point.
        #[allow(clippy::new_without_default)]
        fn new() -> Self {
            Self {
                buffer: Vec::with_capacity(256),
                re_nsub: 0,
                fastmap: [false; 256],
                fastmap_translated: [false; 256],
                fastmap_accurate: false,
                sparse_ascii_fastmap: std::cell::OnceCell::new(),
                folded_scans: FoldedScans::default(),
                posix: false,
                multibyte: true,
                target_multibyte: true,
                can_be_null: false,
                uses_syntax: false,
                used_syntax: false,
                translate: None,
                multibyte_charsets: FxHashMap::default(),
                charset_class_bits: FxHashMap::default(),
                has_nongreedy_optional: false,
                pike_eligible: false,
                linear: linear::Linear::Ineligible,
                buffer_sealed: false,
                rewind: RewindView::Buffer,
                prefilter: std::cell::OnceCell::new(),
                start_anchor: StartAnchor::None,
                dfa: dfa::DfaCell::default(),
            }
        }
    }

    /// Splice `bytes` into the bytecode at `at`, keeping the
    /// opcode-position-keyed side tables consistent.
    ///
    /// `multibyte_charsets` and `charset_class_bits` are keyed by the byte
    /// position of each `Charset`/`CharsetNot` opcode.  GNU stores the
    /// charset range table inline in the bytecode, so moving the opcode also
    /// moves its range table; romacs keeps the range table in a side map, so
    /// every byte inserted *before* an already-emitted charset opcode must
    /// re-key that opcode's entry by the inserted byte count.  Failing to do
    /// so orphans the range table and the charset silently matches no
    /// non-ASCII character (the alternation/quantifier splices that insert an
    /// `on_failure_jump` ahead of a `[é]` are exactly this case).
    pub(crate) fn splice_bytecode(&mut self, at: usize, bytes: &[u8]) {
        let count = bytes.len();
        self.buffer.splice(at..at, bytes.iter().copied());
        if count != 0 {
            shift_charset_keys(&mut self.multibyte_charsets, at, count as isize);
            shift_charset_keys(&mut self.charset_class_bits, at, count as isize);
        }
    }

    /// Move the charset side-table keys for opcodes that lived in
    /// `[from..from_end)` to start at `to` instead.  Used by the non-greedy
    /// `*?`/`+?` quantifier compilers, which `truncate` the body bytes and
    /// re-`extend` them at a new offset.
    pub(crate) fn relocate_charset_keys(&mut self, from: usize, from_end: usize, to: usize) {
        relocate_charset_keys(&mut self.multibyte_charsets, from, from_end, to);
        relocate_charset_keys(&mut self.charset_class_bits, from, from_end, to);
    }

    /// Drop charset side-table keys for opcodes at or beyond `at`, mirroring a
    /// `buffer.truncate(at)`.
    pub(crate) fn truncate_charset_keys(&mut self, at: usize) {
        self.multibyte_charsets.retain(|&pos, _| pos < at);
        self.charset_class_bits.retain(|&pos, _| pos < at);
    }

    /// Copy the charset side-table entries for opcodes in `[from..from_end)`
    /// to the same relative positions starting at `to`.  Used by the greedy
    /// `P+` → `PP*` body duplication, which copies already-emitted body
    /// bytes to the end of the buffer.
    pub(crate) fn clone_charset_keys(&mut self, from: usize, from_end: usize, to: usize) {
        clone_charset_keys(&mut self.multibyte_charsets, from, from_end, to);
        clone_charset_keys(&mut self.charset_class_bits, from, from_end, to);
    }
}

/// Copy entries keyed in `[from..from_end)` to `pos - from + to`, keeping the
/// originals.
fn clone_charset_keys<V: Clone>(
    map: &mut FxHashMap<usize, V>,
    from: usize,
    from_end: usize,
    to: usize,
) {
    if map.is_empty() {
        return;
    }
    let cloned: Vec<(usize, V)> = map
        .iter()
        .filter(|&(&pos, _)| pos >= from && pos < from_end)
        .map(|(&pos, v)| (pos - from + to, v.clone()))
        .collect();
    for (pos, v) in cloned {
        map.insert(pos, v);
    }
}

/// Shift every key `>= at` in an opcode-position-keyed map by `delta` bytes.
fn shift_charset_keys<V>(map: &mut FxHashMap<usize, V>, at: usize, delta: isize) {
    if map.is_empty() || delta == 0 {
        return;
    }
    let moved: Vec<(usize, V)> = map
        .extract_if(|&pos, _| pos >= at)
        .map(|(pos, v)| ((pos as isize + delta) as usize, v))
        .collect();
    for (pos, v) in moved {
        map.insert(pos, v);
    }
}

/// Re-key opcode entries that lived in `[from..from_end)` so they start at `to`.
fn relocate_charset_keys<V>(
    map: &mut FxHashMap<usize, V>,
    from: usize,
    from_end: usize,
    to: usize,
) {
    if map.is_empty() {
        return;
    }
    let moved: Vec<(usize, V)> = map
        .extract_if(|&pos, _| pos >= from && pos < from_end)
        .map(|(pos, v)| (pos - from + to, v))
        .collect();
    // Drop any remaining keys at or beyond `from` that were not relocated
    // (they belonged to bytes that the caller truncated away).
    map.retain(|&pos, _| pos < from);
    for (pos, v) in moved {
        map.insert(pos, v);
    }
}

/// Compile an interval \{n,m\} into bytecode.
fn checked_i16_offset(offset: isize) -> Result<i16, RegexCompileError> {
    i16::try_from(offset).map_err(|_| RegexCompileError {
        message: "Regular expression too big".to_string(),
    })
}

fn checked_u16_counter(value: usize) -> Result<u16, RegexCompileError> {
    u16::try_from(value).map_err(|_| RegexCompileError {
        message: "Regular expression too big".to_string(),
    })
}

pub(crate) fn store_jump_at(
    buffer: &mut [u8],
    op_pos: usize,
    op: RegexOp,
    target: usize,
) -> Result<(), RegexCompileError> {
    buffer[op_pos] = op as u8;
    let offset = checked_i16_offset(target as isize - (op_pos + 3) as isize)?;
    store_number(buffer, op_pos + 1, offset);
    Ok(())
}

pub(crate) fn store_jump2_at(
    buffer: &mut [u8],
    op_pos: usize,
    op: RegexOp,
    target: usize,
    count: usize,
) -> Result<(), RegexCompileError> {
    store_jump_at(buffer, op_pos, op, target)?;
    store_number_u16(buffer, op_pos + 3, checked_u16_counter(count)?);
    Ok(())
}

pub(crate) fn insert_jump(
    buf: &mut CompiledPattern,
    at: usize,
    op: RegexOp,
    target: usize,
) -> Result<(), RegexCompileError> {
    buf.splice_bytecode(at, &[op as u8, 0, 0]);
    store_jump_at(&mut buf.buffer, at, op, target)
}

pub(crate) fn insert_jump2(
    buf: &mut CompiledPattern,
    at: usize,
    op: RegexOp,
    target: usize,
    count: usize,
) -> Result<(), RegexCompileError> {
    buf.splice_bytecode(at, &[op as u8, 0, 0, 0, 0]);
    store_jump2_at(&mut buf.buffer, at, op, target, count)
}

pub(crate) fn insert_set_number_at(
    buf: &mut CompiledPattern,
    at: usize,
    target_counter_offset: usize,
    value: usize,
) -> Result<(), RegexCompileError> {
    buf.splice_bytecode(at, &[RegexOp::SetNumberAt as u8, 0, 0, 0, 0]);
    let offset = checked_i16_offset(target_counter_offset as isize)?;
    store_number(&mut buf.buffer, at + 1, offset);
    store_number_u16(&mut buf.buffer, at + 3, checked_u16_counter(value)?);
    Ok(())
}
