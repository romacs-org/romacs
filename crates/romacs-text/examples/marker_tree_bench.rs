//! P2 isolated marker edit benchmark. Optional perf-stat FIFO control excludes
//! construction and checksumming from instruction counts. Use
//! `marker_tree_compare.py` to cross-check paired checksums and enforce gates.
//! The reference array tracks byte AND char positions, unlike this char-only
//! tree; its timings are not a like-for-like GNU Emacs comparison.
use romacs_chars::{BytePos, CharPos};
use romacs_text::{Edit, MarkerTree, Text, TextPos};
use std::{
    hint::black_box,
    io::{BufRead, BufReader, Write},
    time::Instant,
};
#[allow(dead_code)]
#[path = "../src/markers.rs"]
mod markers;

fn control(command: &str) {
    if let Ok(path) = std::env::var("PERF_CTL") {
        let mut ctl = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        let ack = std::fs::OpenOptions::new()
            .read(true)
            .open(std::env::var("PERF_ACK").unwrap())
            .unwrap();
        writeln!(ctl, "{command}").unwrap();
        let mut line = String::new();
        BufReader::new(ack).read_line(&mut line).unwrap();
        assert!(line.starts_with("ack"));
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let backend = &args[1];
    let count: usize = args[2].parse().unwrap();
    let typing = args[3] == "typing";
    let loops = if typing { 200_000 } else { 10_000 };
    let mut tree = MarkerTree::new();
    let mut array = markers::Markers::default();
    let mut ts = vec![];
    let mut ms = vec![];
    for i in 1..=count {
        let p = i * (1_000_000 / count);
        if backend == "tree" {
            ts.push(tree.new_marker(p as u64, !typing && i % 2 == 1));
        } else {
            ms.push(array.new_marker(p, p, !typing && i % 2 == 1));
        }
    }
    let bytes = if backend == "tree" {
        tree.allocated_bytes()
    } else {
        // Exact capacity accounting for the current six Vec fields in markers.rs.
        std::mem::size_of::<markers::Markers>() + count.next_power_of_two() * (8 * 3 + 4 + 8)
    };
    let mut rng = 7u64;
    control("enable");
    let start = Instant::now();
    for i in 0..loops {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let at = if typing {
            500_000 + i
        } else {
            ((rng >> 33) as usize) % 999_000 + 1
        };
        if backend == "tree" {
            tree.insert(black_box(at as u64), if typing { 1 } else { 2 }, false);
            if !typing {
                tree.delete(at as u64 + 1, at as u64 + 2);
            }
        } else {
            array.adjust(
                Edit {
                    at: TextPos {
                        byte: BytePos(at),
                        char: CharPos(at),
                    },
                    del: (0, 0),
                    ins: if typing { (1, 1) } else { (2, 2) },
                },
                false,
                false,
            );
            if !typing {
                array.adjust(
                    Edit {
                        at: TextPos {
                            byte: BytePos(at + 1),
                            char: CharPos(at + 1),
                        },
                        del: (1, 1),
                        ins: (0, 0),
                    },
                    false,
                    false,
                );
            }
        }
    }
    let elapsed = start.elapsed();
    control("disable");
    let sum: u64 = if backend == "tree" {
        ts.into_iter().map(|s| tree.position(s).unwrap()).sum()
    } else {
        ms.into_iter()
            .map(|s| array.pos(s).unwrap().char.0 as u64)
            .sum()
    };
    println!(
        "backend={backend} markers={count} typing={typing} edits={} ns_per_edit={:.3} bytes={bytes} bytes_per_marker={:.3} checksum={sum}",
        loops * if typing { 1 } else { 2 },
        elapsed.as_nanos() as f64 / (loops * if typing { 1 } else { 2 }) as f64,
        bytes as f64 / count as f64
    );
}
