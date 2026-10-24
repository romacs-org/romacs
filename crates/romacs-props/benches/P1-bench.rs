//! Standalone executable; built by P1-bench-build.sh, not Cargo's bench runner.
//! P1–P5 retain propdesign/crate/src/main.rs's property workloads and LCG.
#![forbid(unsafe_code)]
use romacs_chars::CharPos as P;
use romacs_props::{Op, PropSets, Props, SetId};
use std::{
    fs::{File, OpenOptions},
    hint::black_box,
    io::{BufRead, BufReader, Write},
    time::Instant,
};

struct Control {
    command: File,
    ack: BufReader<File>,
}
impl Control {
    fn open() -> Option<Self> {
        let command = std::env::var_os("P1_PERF_CONTROL")?;
        let ack = std::env::var_os("P1_PERF_ACK").expect("missing perf ack FIFO");
        Some(Self {
            command: OpenOptions::new().write(true).open(command).unwrap(),
            ack: BufReader::new(File::open(ack).unwrap()),
        })
    }
    fn send(&mut self, command: &str) {
        writeln!(self.command, "{command}").unwrap();
        self.command.flush().unwrap();
        let mut reply = String::new();
        assert!(self.ack.read_line(&mut reply).unwrap() > 0, "perf ack EOF");
        assert_eq!(
            reply.trim_matches(|c: char| c.is_whitespace() || c == '\0'),
            "ack",
            "unexpected perf ack"
        );
    }
}
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, m: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % m
    }
}
fn mix(h: &mut u64, v: u64) {
    *h = (*h ^ v).wrapping_mul(1099511628211);
}
fn face_base(s: &mut PropSets<u64>) -> Props {
    let mut p = Props::filled(400000, SetId::EMPTY);
    for i in 0..100000u64 {
        p.apply(
            s,
            P(4 * i as usize),
            P(4 * i as usize + 2),
            Op::Add(&[(2, 100 + i % 5)]),
        );
    }
    p
}
fn digest(p: &Props, s: &PropSets<u64>, h: &mut u64) -> usize {
    mix(h, p.len() as u64);
    let mut n = 0;
    for (a, b, id) in p.runs(P(0), P(p.len())) {
        mix(h, a.0 as u64);
        mix(h, b.0 as u64);
        let pairs = s.pairs(id);
        mix(h, pairs.len() as u64);
        for &(key, value) in pairs {
            mix(h, key);
            mix(h, value);
        }
        n += 1;
    }
    n
}
fn main() {
    let row = std::env::args().nth(1).expect("row P1..P5");
    assert!(matches!(row.as_str(), "P1" | "P2" | "P3" | "P4" | "P5"));
    let mut s = PropSets::<u64>::new();
    let faces: Vec<_> = (0..5).map(|f| s.intern(&[(2, 100 + f), (7, 1)])).collect();
    let base = if matches!(row.as_str(), "P3" | "P4" | "P5") {
        Some(face_base(&mut s))
    } else {
        None
    };
    let mut control = Control::open();
    // FIFO ACK and setup excluded from wall clock. Instructions also include
    // Instant timestamps and the disable command/ACK transport, not setup/digest.
    if let Some(c) = &mut control {
        c.send("enable");
    }
    let start = Instant::now();
    let mut checksum = 14695981039346656037u64;
    let mut answer = 0usize;
    let mut ops = 0usize;
    let mut result = None;
    match row.as_str() {
        "P1" => {
            let mut p = Props::new();
            for i in 0..200000 {
                p.insert(P(p.len()), 5, faces[i % 5]);
            }
            ops = 200000;
            answer = black_box(p.runs(P(0), P(p.len())).count());
            result = Some(p);
        }
        "P2" => {
            let mut p = Props::filled(400000, SetId::EMPTY);
            for i in 0..100000u64 {
                p.apply(
                    &mut s,
                    P(4 * i as usize),
                    P(4 * i as usize + 2),
                    Op::Add(&[(2, 100 + i % 5)]),
                );
            }
            ops = 100000;
            answer = black_box(p.runs(P(0), P(p.len())).count());
            result = Some(p);
        }
        "P3" => {
            let mut p = base.as_ref().unwrap().clone();
            let mut k = 0u64;
            let mut a = 0;
            while a < 400000 {
                let e = (a + 1500).min(400000);
                p.apply(&mut s, P(a), P(e), Op::Remove(&[2]));
                ops += 1;
                let mut q = a;
                while q + 3 < e {
                    k += 1;
                    p.apply(&mut s, P(q), P(q + 2), Op::Add(&[(2, 100 + k % 5)]));
                    ops += 1;
                    q += 5;
                }
                a = e;
            }
            answer = black_box(p.runs(P(0), P(p.len())).count());
            result = Some(p);
        }
        "P4" => {
            let base = base.as_ref().unwrap();
            let mut r = Lcg(7);
            for _ in 0..1000000 {
                let (_, _, id) = base.run_at(P(r.next(400000)));
                if s.get(id, 2).is_some() {
                    answer += 1;
                }
            }
            ops = 1000000;
        }
        "P5" => {
            let base = base.as_ref().unwrap();
            for _ in 0..10 {
                let mut q = P(0);
                loop {
                    ops += 1;
                    match base.next_single(q, P(base.len()), |id| s.get(id, 2).unwrap_or(0)) {
                        Some(x) => {
                            answer += 1;
                            q = x;
                        }
                        None => break,
                    }
                }
            }
        }
        _ => unreachable!(),
    }
    black_box((answer, checksum, &result));
    let mut elapsed_ns = start.elapsed().as_nanos();
    if let Some(c) = &mut control {
        c.send("disable");
    }
    let p = result.as_ref().or(base.as_ref()).unwrap();
    // Replay queries outside both measured scopes to authenticate all outputs,
    // not merely a hit count; fresh process per row prevents replay warming.
    if row == "P4" {
        let mut r = Lcg(7);
        for _ in 0..1000000 {
            let (a, b, id) = p.run_at(P(r.next(400000)));
            mix(&mut checksum, a.0 as u64);
            mix(&mut checksum, b.0 as u64);
            mix(&mut checksum, s.get(id, 2).unwrap_or(0));
        }
    } else if row == "P5" {
        for _ in 0..10 {
            let mut q = P(0);
            while let Some(x) = p.next_single(q, P(p.len()), |id| s.get(id, 2).unwrap_or(0)) {
                mix(&mut checksum, x.0 as u64);
                q = x;
            }
        }
    }
    let runs = digest(p, &s, &mut checksum);
    mix(&mut checksum, answer as u64);
    #[cfg(p1_candidate)]
    let bytes = p.allocated_bytes().to_string();
    #[cfg(not(p1_candidate))]
    let bytes = p.p1_allocated_bytes().to_string();
    // Original closures also drop their local Props before returning. Preserve
    // that cost with a second measured span, after untimed digest/memory reads.
    if result.is_some() {
        if let Some(c) = &mut control {
            c.send("enable");
        }
        let teardown = Instant::now();
        drop(result);
        elapsed_ns += teardown.elapsed().as_nanos();
        if let Some(c) = &mut control {
            c.send("disable");
        }
    }
    println!(
        "{{\"row\":\"{row}\",\"ops\":{ops},\"elapsed_ns\":{elapsed_ns},\"answer\":{answer},\"checksum\":\"{checksum:016x}\",\"runs\":{runs},\"allocated_bytes\":{bytes}}}"
    );
}
