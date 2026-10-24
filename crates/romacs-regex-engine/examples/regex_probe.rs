//! The engine as a process, for the live-GNU differential oracle
//! (`tools/regex_oracle.py`) and the benchmark (`tools/regex_bench.py`).
//!
//! No Lisp, no host: the standard syntax table ([`DefaultSyntaxLookup`]) and
//! the standard case table, which is what GNU's `string-match` uses under
//! `(with-syntax-table (standard-syntax-table) ...)` in a fresh `emacs -Q`.
//!
//! Modes:
//!
//! - `probe`: one case per stdin line, fields tab-separated and hex-encoded
//!   (`pattern subject start case-fold posix`), the subject UTF-8 (multibyte).
//!   Prints one line per case: `nil`, `overflow`, `invalid`, or the match data in
//!   CHARACTER positions, trailing unset groups dropped (GNU `(match-data t)`
//!   on a string).
//! - `bench PATTERN FILE ITERS ALL`: one search workload (see [`bench`]).
//!   Timing is taken from outside, by `perf stat` on the whole process, with
//!   an `ITERS = 0` run of the same workload subtracted
//!   (`tools/regex_bench.py`).  No GC exists on this side: the crate has no
//!   Lisp heap.
use romacs_regex_engine::{
    CompiledPattern, DefaultSyntaxLookup, MatchRegisters, re_search, regex_compile,
    take_matcher_overflow,
};
use std::io::{BufRead, Write};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex field"))
        .collect()
}

fn char_index(text: &[u8], byte: i64) -> i64 {
    if byte < 0 {
        return byte;
    }
    let byte = byte as usize;
    text[..byte].iter().filter(|&&b| (b & 0xC0) != 0x80).count() as i64
}

fn byte_index(text: &[u8], chars: usize) -> usize {
    let mut seen = 0;
    for (i, &b) in text.iter().enumerate() {
        if (b & 0xC0) != 0x80 {
            if seen == chars {
                return i;
            }
            seen += 1;
        }
    }
    text.len()
}

fn match_data(text: &[u8], registers: &MatchRegisters) -> Vec<i64> {
    let mut data: Vec<i64> = registers
        .start
        .iter()
        .zip(&registers.end)
        .flat_map(|(&start, &end)| [char_index(text, start), char_index(text, end)])
        .collect();
    while data.len() > 2 && data[data.len() - 1] < 0 && data[data.len() - 2] < 0 {
        data.truncate(data.len() - 2);
    }
    data
}

fn search(pattern: &CompiledPattern, text: &[u8], start: usize) -> Option<(usize, MatchRegisters)> {
    re_search(
        pattern,
        text,
        start,
        (text.len() - start) as isize,
        &DefaultSyntaxLookup,
        0,
    )
}

fn probe() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.expect("stdin");
        let fields: Vec<&str> = line.split('\t').collect();
        let pattern = String::from_utf8(unhex(fields[0])).expect("UTF-8 pattern");
        let subject = unhex(fields[1]);
        let start: usize = fields[2].parse().expect("start");
        let fold = fields[3] == "1";
        let posix = fields[4] == "1";
        let reply = match regex_compile(&pattern, posix, fold) {
            Err(_) => "invalid".to_string(),
            Ok(compiled) => {
                let _ = take_matcher_overflow();
                let found = search(&compiled, &subject, byte_index(&subject, start));
                if take_matcher_overflow() {
                    "overflow".to_string()
                } else {
                    match found {
                        None => "nil".to_string(),
                        Some((_, regs)) => {
                            let data = match_data(&subject, &regs);
                            format!(
                                "({})",
                                data.iter()
                                    .map(|v| if *v < 0 {
                                        "nil".to_string()
                                    } else {
                                        v.to_string()
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            )
                        }
                    }
                }
            }
        };
        writeln!(out, "{reply}").expect("stdout");
    }
}

/// `bench PATTERN FILE ITERS ALL`: search FILE's text for PATTERN, ITERS
/// times.  With ALL = 1 every match is found the way a font-lock loop does
/// (restart at the match end, one past it after an empty match); with 0 one
/// search runs.  Prints a checksum of the match positions so both sides of a
/// comparison can be checked to have done the same work.
fn bench(args: &[String]) {
    let pattern = args[0].as_str();
    let text = std::fs::read(&args[1]).expect("FILE");
    let iters: usize = args[2].parse().expect("ITERS");
    let all = args[3] == "1";
    let compiled = regex_compile(pattern, false, false).expect("pattern compiles");
    let mut checksum: u64 = 0;
    let mut matches: u64 = 0;
    for _ in 0..iters {
        let mut start = 0usize;
        // Byte to character positions, incrementally: matches only advance.
        let (mut at_byte, mut at_char) = (0usize, 0u64);
        let mut chars_to = |byte: usize| {
            if byte < at_byte {
                (at_byte, at_char) = (0, 0);
            }
            at_char += text[at_byte..byte]
                .iter()
                .filter(|&&b| (b & 0xC0) != 0x80)
                .count() as u64;
            at_byte = byte;
            at_char
        };
        loop {
            let _ = take_matcher_overflow();
            let found = search(&compiled, &text, start);
            if take_matcher_overflow() {
                checksum = checksum.wrapping_add(1 << 40);
                break;
            }
            let Some((_, regs)) = found else { break };
            matches += 1;
            let s = chars_to(regs.start[0] as usize);
            let e = chars_to(regs.end[0] as usize);
            checksum = checksum
                .wrapping_mul(31)
                .wrapping_add(s.wrapping_mul(7).wrapping_add(e));
            if !all {
                break;
            }
            let (sb, eb) = (regs.start[0] as usize, regs.end[0] as usize);
            start = if eb > sb { eb } else { eb + 1 };
            if start > text.len() {
                break;
            }
        }
    }
    println!("matches={matches} checksum={checksum}");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("probe") => probe(),
        Some("bench") => bench(&args[1..]),
        _ => {
            eprintln!("usage: regex_probe probe | bench PATTERN FILE ITERS ALL");
            std::process::exit(2);
        }
    }
}
