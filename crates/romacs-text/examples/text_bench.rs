//! Diagnostic kernels, never whole-editor row claims. Assertions run after timing.
use romacs_chars::{BytePos, CharPos};
use romacs_text::Text;
use std::{
    hint::black_box,
    io::Write,
    process::{Command, Stdio},
    time::Instant,
};
fn all(t: &Text) -> Vec<u8> {
    let mut v = vec![];
    t.copy_to(BytePos(0), BytePos(t.len_bytes()), &mut v);
    v
}
fn sha(b: &[u8]) -> String {
    let mut p = Command::new("/usr/bin/sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin.take().unwrap().write_all(b).unwrap();
    let o = p.wait_with_output().unwrap();
    assert!(o.status.success());
    String::from_utf8(o.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}
fn lcg(r: &mut usize, n: usize) -> usize {
    *r = (*r * 1103515245 + 12345) % n;
    *r
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let row = args.get(1).map(String::as_str).unwrap_or("all");
    let samples = args
        .get(2)
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(11);
    let x = std::fs::read(std::env::var("CORE_BENCH_XDISP").expect("CORE_BENCH_XDISP")).unwrap();
    assert!(x.is_ascii());
    let rows = [
        "insert-end-1e6",
        "insert-random-1e5",
        "markers-1e3-edits",
        "utf8-decode-xdisp",
        "position-xdisp",
        "early-edit-query",
        "charpos-mb",
    ];
    for name in rows {
        if row != "all" && row != name {
            continue;
        }
        let mut expected = vec![];
        if name == "insert-random-1e5" {
            expected = vec![b'x'; 1000000];
            let mut r = 7;
            for _ in 0..100000 {
                let p = lcg(&mut r, 1000000);
                expected.insert(p, b'y');
            }
        }
        let mut times = vec![];
        let mut answer = String::new();
        let mut moved = 0usize;
        for sample in 0..=samples {
            let mut t = if name == "insert-random-1e5" {
                Text::from_bytes(&vec![b'x'; 1000000], true)
            } else if name == "markers-1e3-edits" {
                Text::from_bytes(&vec![b'x'; 200000], true)
            } else if name == "position-xdisp" {
                Text::from_bytes(&x, true)
            } else if name == "early-edit-query" {
                Text::from_bytes(&"aé\n".repeat(25000000).into_bytes(), true)
            } else if name == "charpos-mb" {
                Text::from_bytes(&"漢😀a\n".repeat(111112).into_bytes(), true)
            } else {
                Text::new(true)
            };
            let markers: Vec<_> = if name == "markers-1e3-edits" {
                (1..=1000)
                    .map(|i| t.marker_new(BytePos(200 * i - 1), false))
                    .collect()
            } else {
                vec![]
            };
            let mut out = vec![];
            let mut sum = 0usize;
            let mut r = 7;
            let mut gap = t.len_bytes();
            moved = 0;
            let start = Instant::now();
            match name {
                "insert-end-1e6" => {
                    for _ in 0..1000000 {
                        t.insert(BytePos(t.len_bytes()), black_box(b"ab"), false);
                    }
                }
                "insert-random-1e5" => {
                    for _ in 0..100000 {
                        let p = lcg(&mut r, 1000000);
                        moved += gap.abs_diff(p) + 1;
                        t.insert(BytePos(p), b"yz", false);
                        t.delete(BytePos(p + 1), BytePos(p + 2));
                        gap = p + 1;
                    }
                }
                "markers-1e3-edits" => {
                    for i in 0..20000 {
                        t.insert(BytePos(7 * i), b"q", false);
                    }
                }
                "utf8-decode-xdisp" => {
                    t = Text::from_bytes(black_box(&x), true);
                    t.copy_to(BytePos(0), BytePos(t.len_bytes()), &mut out);
                }
                "position-xdisp" => {
                    for _ in 0..2000 {
                        let p = lcg(&mut r, x.len());
                        sum += 1 + t.newlines_before(BytePos(p));
                    }
                }
                "early-edit-query" => {
                    for _ in 0..10000 {
                        t.insert(BytePos(1024), b"a", false);
                        let c = lcg(&mut r, t.len_chars());
                        sum ^= t.byte_of_char(CharPos(c)).0;
                    }
                }
                "charpos-mb" => {
                    for _ in 0..100000 {
                        let c = lcg(&mut r, t.len_chars());
                        let b = t.byte_of_char(CharPos(c));
                        assert_eq!(black_box(t.char_of_byte(b)), CharPos(c));
                        sum ^= b.0;
                    }
                }
                _ => unreachable!(),
            }
            let elapsed = start.elapsed().as_secs_f64() * 1000.;
            if sample > 0 {
                times.push(elapsed);
            }
            black_box(sum);
            match name {
                "insert-end-1e6" => {
                    let got = all(&t);
                    assert_eq!(got, b"ab".repeat(1000000));
                    answer = sha(&got);
                }
                "insert-random-1e5" => {
                    let got = all(&t);
                    assert_eq!(got, expected);
                    assert_eq!(got.iter().filter(|&&b| b == b'y').count(), 100000);
                    answer = sha(&got);
                }
                "markers-1e3-edits" => {
                    for (i, m) in markers.iter().enumerate() {
                        let initial = 200 * (i + 1);
                        let shift = (initial.saturating_sub(2) / 6 + 1).min(20000);
                        let pos = t.marker_pos(*m).unwrap();
                        assert_eq!(pos.byte.0 + 1, initial + shift);
                        assert_eq!(pos.char.0, pos.byte.0);
                    }
                    answer = format!("markers={} bytes={}", markers.len(), t.len_bytes());
                }
                "utf8-decode-xdisp" => {
                    assert_eq!(out, x);
                    answer = sha(&out);
                }
                "position-xdisp" => {
                    let mut r = 7;
                    let expected: usize = (0..2000)
                        .map(|_| {
                            let p = lcg(&mut r, x.len());
                            1 + x[..p].iter().filter(|&&b| b == 10).count()
                        })
                        .sum();
                    assert_eq!(sum, expected);
                    answer = sum.to_string();
                }
                "early-edit-query" => {
                    assert_eq!(t.len_bytes(), 100010000);
                    for c in [0, 1023, 1024, 10000, t.len_chars()] {
                        let b = t.byte_of_char(CharPos(c));
                        assert_eq!(t.char_of_byte(b), CharPos(c));
                    }
                    answer = format!("checksum={sum}");
                }
                _ => answer = format!("checksum={sum}"),
            }
            t.check_invariants();
        }
        times.sort_by(f64::total_cmp);
        let median = times[times.len() / 2];
        println!(
            "k-{name}: {median:.6} ms [{:.6}, {:.6}] n={samples} answer={answer} moved={moved} GB/s={:.3}",
            times[0],
            times[times.len() - 1],
            moved as f64 / median / 1e6
        );
    }
}
