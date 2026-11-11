//! Scanners (rlisp/romacs-scanner.el): a table tabulated from a Lisp step
//! function, run over buffer text. Romacs's own; GNU has no equivalent.
use crate::prim::Fault;
use crate::Value;

/// A scanner's alphabet: the ASCII characters, and one more for every other
/// character.
const SYMBOLS: usize = 129;
/// The states a table can hold: an entry is a state and a mark in a byte.
const STATES: usize = 128;
/// An entry's mark: the character that took this transition is reported.
const MARK: u16 = 0x8000;
/// The high bit of each byte of a word.
const HIGH: u64 = 0x8080_8080_8080_8080;

/// A scanner's table by byte: for a state in the high byte and a byte of
/// text in the low, the next state in the high byte, and `MARK`.
type Bytes = [u16; STATES * 256];

/// ENTRIES (a state and a mark for each symbol of each state) as a table by
/// byte of text in GNU's internal form. In multibyte text a character past
/// ASCII is its lead byte, which takes the transition of the last symbol;
/// its other bytes change nothing. In unibyte text every byte is a
/// character.
fn by_byte(entries: &[u8], multibyte: bool) -> Box<Bytes> {
    let mut table: Box<Bytes> = vec![0; STATES * 256].into_boxed_slice().try_into().expect("the table's size");
    for (row, state) in entries.chunks_exact(SYMBOLS).zip(0u8..) {
        for byte in 0..=255u8 {
            let trailing = multibyte && (0x80..0xC0).contains(&byte);
            let entry = if trailing { state } else { row[usize::from(byte).min(SYMBOLS - 1)] };
            table[(usize::from(state) << 8) | usize::from(byte)] = (u16::from(entry & 127) << 8) | if entry & 128 != 0 { MARK } else { 0 };
        }
    }
    table
}

/// Run TABLE over BYTES from STATE, calling MARKED with the offset of each
/// byte whose transition is marked. The state it ends in.
fn run(table: &Bytes, state: usize, bytes: &[u8], mut marked: impl FnMut(usize)) -> usize {
    let mut state = state << 8;
    for (at, &byte) in bytes.iter().enumerate() {
        // A state is under 128, so the index is within the table.
        let entry = table[(state | usize::from(byte)) & (STATES * 256 - 1)];
        state = usize::from(entry & 0x7f00);
        if entry & MARK != 0 { marked(at); }
    }
    state >> 8
}

/// The characters BYTES hold: in multibyte text, the bytes that are not a
/// character's trailing bytes (10xxxxxx), counted eight at a time.
fn characters(bytes: &[u8], multibyte: bool) -> usize {
    if !multibyte { return bytes.len(); }
    let words = bytes.chunks_exact(8);
    let rest = words.remainder();
    let trailing: usize = words.map(|word| {
        let word = u64::from_le_bytes(word.try_into().expect("eight bytes"));
        // Bit 7 set and bit 6 clear: at most eight a word.
        usize::try_from((word & !(word << 1) & HIGH).count_ones()).expect("a count of bytes")
    }).sum();
    bytes.len() - trailing - rest.iter().filter(|&&byte| byte & 0xC0 == 0x80).count()
}

defsubr! { name = "romacs--scanner-run", gnu = "romacs:scanner", effect = Reads,
    fn romacs_scanner_run(cx, table: Value, state: Value, start: Value, end: Value, count: Value) {
        // TABLE: a vector of 129 entries a state, each the next state plus
        // 128 when the transition is marked. Returns (STATE . MARKED): the
        // positions of the marked characters from START to END, or how many
        // there are when COUNT is non-nil.
        let entries = cx.vector_values(table)?;
        let states = entries.len() / SYMBOLS;
        if states == 0 || states > STATES || entries.len() % SYMBOLS != 0 { return Err(Fault::Wrong("romacs-scanner-table-p", table)); }
        let mut packed = Vec::with_capacity(entries.len());
        for entry in &entries {
            let byte = entry.integer_value().and_then(|n| u8::try_from(n).ok()).filter(|&n| usize::from(n & 127) < states);
            packed.push(byte.ok_or(Fault::Wrong("romacs-scanner-table-p", table))?);
        }
        let from = state.integer_value().and_then(|n| usize::try_from(n).ok()).filter(|&n| n < states)
            .ok_or(Fault::Wrong("romacs-scanner-state-p", state))?;
        let first = start.integer_value().ok_or(Fault::Wrong("fixnump", start))?;
        let last = end.integer_value().ok_or(Fault::Wrong("fixnump", end))?;
        let only_count = count != Value::NIL;
        let scanned = cx.scan_region(first, last, |bytes, multibyte| {
            let table = by_byte(&packed, multibyte);
            let (mut positions, mut total) = (Vec::new(), 0i64);
            // Characters are counted only up to each mark, from the last.
            let (mut reached, mut position) = (0, first);
            let state = run(&table, from, bytes, |at| {
                total += 1;
                if !only_count {
                    position += i64::try_from(characters(&bytes[reached..at], multibyte)).expect("a buffer's size");
                    reached = at;
                    positions.push(position);
                }
            });
            (state, positions, total)
        })?;
        let Some((state, positions, total)) = scanned else {
            let data = cx.cons(end, Value::NIL);
            let data = cx.cons(start, data);
            return Err(Fault::Signal(cx.intern("args-out-of-range"), data));
        };
        let marked = if only_count {
            Value::integer(total).map_err(Fault::Error)?
        } else {
            let mut marks = Value::NIL;
            for &position in positions.iter().rev() {
                let position = Value::integer(position).map_err(Fault::Error)?;
                marks = cx.cons(position, marks);
            }
            marks
        };
        let state = i64::try_from(state).ok().and_then(Value::fixnum).expect("a state number is a fixnum");
        Ok(cx.cons(state, marked))
    }
}
