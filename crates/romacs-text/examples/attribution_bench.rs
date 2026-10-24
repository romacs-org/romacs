//! Attribution only: identical gap movements without index/marker bookkeeping.
use romacs_chars::BytePos;
use romacs_text::Text;
use std::{hint::black_box, time::Instant};
fn report(name: &str, mut times: Vec<f64>, moved: usize) {
    times.sort_by(f64::total_cmp);
    println!(
        "{name}: {:.6} ms [{:.6}, {:.6}] n={} moved={moved} GB/s={:.3}",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1],
        times.len(),
        moved as f64 / times[times.len() / 2] / 1e6
    );
}
fn main() {
    let mut expected = vec![b'x'; 1000000];
    let mut r = 7usize;
    for _ in 0..100000 {
        r = (r * 1103515245 + 12345) % 1000000;
        expected.insert(r, b'y');
    }
    let mut gap_times = vec![];
    let mut index_times = vec![];
    let mut moved = 0;
    for sample in 0..12 {
        let mut bytes = vec![b'x'; 1500000];
        let (mut start, mut end) = (1000000usize, 1500000usize);
        let mut r = 7usize;
        moved = 0;
        let begin = Instant::now();
        for _ in 0..100000 {
            r = (r * 1103515245 + 12345) % 1000000;
            let p = black_box(r);
            let size = end - start;
            moved += start.abs_diff(p);
            if p < start {
                bytes.copy_within(p..start, end - (start - p));
            } else {
                bytes.copy_within(end..end + p - start, start);
            }
            start = p;
            end = p + size;
            bytes[start..start + 2].copy_from_slice(b"yz");
            start += 2;
            // Mirror Text's one-byte gap move preceding the deletion of z.
            bytes.copy_within(start - 1..start, end - 1);
            start -= 1;
            moved += 1;
        }
        let elapsed = begin.elapsed().as_secs_f64() * 1000.;
        if sample > 0 {
            gap_times.push(elapsed);
        }
        let mut logical = bytes[..start].to_vec();
        logical.extend_from_slice(&bytes[end..]);
        assert_eq!(logical, expected);
        let mut t = Text::from_bytes(&vec![b'x'; 200000], true);
        let begin = Instant::now();
        for i in 0..20000 {
            t.insert(BytePos(7 * i), b"q", false);
        }
        let elapsed = begin.elapsed().as_secs_f64() * 1000.;
        if sample > 0 {
            index_times.push(elapsed);
        }
        assert_eq!(t.len_bytes(), 220000);
    }
    report("gap-only-random-1e5", gap_times, moved);
    report("markers-row-without-markers", index_times, 0);
    println!(
        "answer=gap bytes={} y={} (byte-equal naive model); markerless bytes=220000",
        expected.len(),
        expected.iter().filter(|&&b| b == b'y').count()
    );
}
