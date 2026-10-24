use romacs_regex::*;
use std::{hint::black_box, time::Instant};
#[path = "../bench/org_corpus.rs"]
mod org_corpus;

struct Fold;
impl CaseTable for Fold {
    fn canon(&self, c: u32) -> u32 {
        if (65..=90).contains(&c) { c + 32 } else { c }
    }
    fn eqv_next(&self, c: u32) -> u32 {
        match c {
            65..=90 => c + 32,
            97..=122 => c - 32,
            _ => c,
        }
    }
}
fn main() {
    let row = std::env::args().nth(1).expect("row");
    if row == "k-cache-org" {
        bench_org_cache();
        return;
    }
    if row == "k-cache-churn" {
        let mut c = Cache::new();
        let mut st = Stats::default();
        for _ in 0..100 {
            for i in 0..129 {
                c.get(
                    KeyRef {
                        kind: Kind::Literal,
                        pattern: &[i],
                        pattern_multibyte: false,
                        target_multibyte: false,
                        posix: false,
                        whitespace: None,
                        case: None,
                    },
                    || Ok(Regex::literal(&[i], false, false, None)),
                    &mut st,
                )
                .unwrap();
            }
        }
        println!("{row} compiles={} hits={}", st.compiles, st.hits);
        return;
    }
    let text = std::fs::read(std::env::var("CORE_BENCH_XDISP").expect("CORE_BENCH_XDISP")).unwrap();
    let split = text.len() / 2;
    let h = Haystack {
        s1: &text[..split],
        s2: &text[split..],
        multibyte: true,
        point: None,
    };
    let back = row == "k-literal-back-xdisp";
    let miss = row == "k-literal-miss";
    assert!(back || miss || row == "k-literal-xdisp");
    let re = Regex::literal(
        if miss {
            b"zzqqxx_no_hit!"
        } else {
            b"glyph_row"
        },
        false,
        true,
        Some(&Fold),
    );
    let mut regs = Regs::default();
    let mut st = Stats::default();
    let mut answer = 0;
    let start = Instant::now();
    for _ in 0..if miss { 1 } else { 20 } {
        let mut p = if back { h.len() } else { 0 };
        loop {
            let range = if back {
                -(p as isize)
            } else {
                (h.len() - p) as isize
            };
            if re
                .search(
                    black_box(&h),
                    p,
                    range,
                    if back { p } else { h.len() },
                    &(),
                    &mut regs,
                    &mut st,
                )
                .unwrap()
                .is_none()
            {
                break;
            }
            answer += 1;
            p = if back {
                regs.start[0] as usize
            } else {
                regs.end[0] as usize
            };
            if back && p == 0 {
                break;
            }
        }
    }
    println!(
        "{row} ms={:.6} answer={answer} candidates={} verify_bytes={} window_bytes={}",
        start.elapsed().as_secs_f64() * 1000.0,
        st.candidates,
        st.verify_bytes,
        st.window_bytes
    );
    assert_eq!(answer, if miss { 0 } else { 10_040 });
}

fn bench_org_cache() {
    let patterns = org_corpus::patterns();
    let start = Instant::now();
    for _ in 0..2_000 {
        for pattern in &patterns {
            black_box(Regex::literal(black_box(pattern), false, false, None));
        }
    }
    let compile_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut cache = Cache::new();
    let mut stats = Stats::default();
    let start = Instant::now();
    for _ in 0..2_000 {
        for pattern in &patterns {
            black_box(
                cache
                    .get(
                        KeyRef {
                            kind: Kind::Regexp,
                            pattern: black_box(pattern),
                            pattern_multibyte: false,
                            target_multibyte: false,
                            posix: false,
                            whitespace: None,
                            case: None,
                        },
                        || Ok(Regex::literal(pattern, false, false, None)),
                        &mut stats,
                    )
                    .unwrap(),
            );
        }
    }
    println!(
        "k-cache-org compile_ms={compile_ms:.6} cache_ms={:.6} compiles={} hits={}",
        start.elapsed().as_secs_f64() * 1000.0,
        stats.compiles,
        stats.hits,
    );
}
