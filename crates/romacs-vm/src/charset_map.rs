//! GNU charset.c's definition-time MAP reader (control flag zero).
//! Decoder/encoder realization is deliberately separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub from: u32,
    pub to: u32,
    pub character: u32,
}

/// Port read_hex's consumed-byte lookahead, including comment handling and
/// rejection of a final number without a terminating byte.
pub(crate) fn read_entries(bytes: &[u8], min: u32, max: u32) -> Vec<Entry> {
    fn get(bytes: &[u8], at: &mut usize) -> Option<u8> {
        let c = bytes.get(*at).copied();
        *at += usize::from(c.is_some());
        c
    }
    fn hex(bytes: &[u8], at: &mut usize, lookahead: Option<u8>, overflow: &mut bool) -> (u32, Option<u8>) {
        let mut c = lookahead.or_else(|| get(bytes, at));
        loop {
            if c == Some(b'#') {
                while c.is_some() && c != Some(b'\n') { c = get(bytes, at); }
            } else if c == Some(b'0') {
                c = get(bytes, at);
                if c.is_none() || c == Some(b'x') { break; }
            }
            if c.is_none() { break; }
            c = get(bytes, at);
        }
        let mut number = 0u32;
        if c.is_some() {
            loop {
                c = get(bytes, at);
                let Some(digit) = c.and_then(|b| (b as char).to_digit(16)) else { break };
                *overflow |= number > (u32::MAX >> 4);
                number = number.wrapping_mul(16).wrapping_add(digit);
            }
        }
        (number, c)
    }
    let mut entries = Vec::new();
    let mut at = 0;
    let mut lookahead = None;
    loop {
        let mut overflow = false;
        let (from, terminator) = hex(bytes, &mut at, lookahead, &mut overflow);
        let Some(terminator) = terminator else { break };
        let (to, next) = if terminator == b'-' {
            let (to, end) = hex(bytes, &mut at, None, &mut overflow);
            if end.is_none() { break; }
            (to, end)
        } else { (from, None) };
        let (character, end) = hex(bytes, &mut at, next, &mut overflow);
        if end.is_none() { break; }
        lookahead = end;
        if !overflow && from >= min && to <= max && from <= to && character <= 0x3fffff {
            entries.push(Entry { from, to, character });
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_latin_two_entries_preserve_code_positions() {
        let entries = read_entries(include_bytes!("../../../etc/charsets/8859-2.map"), 0, 255);
        let at = |code| entries.iter().find(|e| e.from <= code && code <= e.to).map(|e| e.character + code - e.from);
        assert_eq!((at(0), at(160), at(161), at(255)), (Some(0), Some(160), Some(260), Some(729)));
    }
    #[test]
    fn file_reader_preserves_gnu_lookahead_and_filters_ranges() {
        let entries = read_entries(b"# ignored 0x00 0x00\n0x41 0x100\n0x42-0x44 0x200\n0x46 0x100\n0x100 0x123\n0x41 0x100000000\n0x48 0x300", 0, 255);
        assert_eq!(entries, [Entry { from: 65, to: 65, character: 256 }, Entry { from: 66, to: 68, character: 512 }, Entry { from: 70, to: 70, character: 256 }]);
    }
}
