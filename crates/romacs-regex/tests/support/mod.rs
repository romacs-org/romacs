use romacs_regex::*;
pub struct Fold;
impl CaseTable for Fold {
    fn canon(&self, c: u32) -> u32 {
        match c {
            65..=90 => c + 32,
            0x3a3 | 0x3c2 => 0x3c3,
            0x1c4 | 0x1c5 => 0x1c6,
            0x23a => 0x2c65,
            0x23e => 0x2c66,
            0x1e9e => 0xdf,
            0xc9 => 0xe9,
            0x391..=0x3a1 => c + 32,
            _ => c,
        }
    }
    fn eqv_next(&self, c: u32) -> u32 {
        match c {
            65..=90 => c + 32,
            97..=122 => c - 32,
            0x3a3 => 0x3c2,
            0x3c2 => 0x3c3,
            0x3c3 => 0x3a3,
            0x1c4 => 0x1c5,
            0x1c5 => 0x1c6,
            0x1c6 => 0x1c4,
            0x391..=0x3a1 => c + 32,
            0x3b1..=0x3c1 => c - 32,
            0x23a => 0x2c65,
            0x23e => 0x2c66,
            0x1e9e => 0xdf,
            0xc9 => 0xe9,
            0x2c65 => 0x23a,
            0x2c66 => 0x23e,
            0xdf => 0x1e9e,
            0xe9 => 0xc9,
            _ => c,
        }
    }
}
#[allow(dead_code, clippy::too_many_arguments)]
pub fn find(
    re: &Regex,
    text: &[u8],
    split: usize,
    mb: bool,
    start: usize,
    range: isize,
    stop: usize,
) -> Option<(usize, usize)> {
    let h = Haystack {
        s1: &text[..split],
        s2: &text[split..],
        multibyte: mb,
        point: Some(split),
    };
    let mut regs = Regs::default();
    re.search(
        &h,
        start,
        range,
        stop,
        &(),
        &mut regs,
        &mut Stats::default(),
    )
    .unwrap()
    .map(|p| (p, regs.end[0] as usize))
}
