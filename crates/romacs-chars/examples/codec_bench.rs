use romacs_chars::{char_string, chars, multibyte_chars_in_text};
use std::{hint::black_box, time::Instant};

fn bench(name: &str, bytes: usize, f: impl Fn() -> u64) {
    let mut samples = [0.0; 11];
    let mut answer = 0;
    for sample in &mut samples {
        let start = Instant::now();
        for _ in 0..50 {
            answer = black_box(f());
        }
        *sample = start.elapsed().as_secs_f64() * 1000.0 / 50.0;
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name}: bytes={bytes} answer={answer} ms={:.6} [{:.6}, {:.6}]",
        samples[5], samples[0], samples[10]
    );
}

fn main() {
    for (name, codes) in [
        ("ascii", &b"editor text\n".map(u32::from)[..]),
        ("raw", &[0x3fff80, 0x3fffff][..]),
        ("extended", &[0x200000, 0x3fff7f][..]),
        (
            "mixed",
            &[97, 98, 0x3bb, 0x96ea, 0x1f600, 0x3fff80, 0x200000][..],
        ),
    ] {
        let mut pattern = Vec::new();
        for &c in codes {
            let mut form = [0; 5];
            let len = char_string(c, &mut form);
            pattern.extend_from_slice(&form[..len]);
        }
        let data = pattern.repeat((1024 * 1024) / pattern.len());
        bench(&format!("{name}/decode"), data.len(), || {
            chars(black_box(&data)).map(u64::from).sum()
        });
        bench(&format!("{name}/count"), data.len(), || {
            multibyte_chars_in_text(black_box(&data)) as u64
        });
    }
}
