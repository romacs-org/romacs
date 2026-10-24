use super::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        self.next() as usize % n
    }
}

// Independent absolute-offset GNU-semantics oracle, from anchors-peritext.
// Never resolve an expected position through the tree under test.
fn trace(seed: u64, mixed: bool, bulk: bool) {
    let mut rng = Rng(seed);
    let mut text = if mixed {
        "English Ελληνικά Кириллица\n".repeat(300)
    } else {
        "abcdefghij".repeat(30)
    };
    let mut t = AnchorTree::new(text.len() as u32);
    let mut oracle: Vec<(u32, Bias)> = Vec::new();
    let mut hs = Vec::new();
    if bulk {
        let mut positions: Vec<_> = text
            .char_indices()
            .step_by(7)
            .map(|(p, _)| (p as u32, Bias::Left))
            .collect();
        positions.extend(positions.clone().into_iter().map(|(p, _)| (p, Bias::Right)));
        positions.sort();
        (t, hs) = AnchorTree::from_sorted(text.len() as u32, &positions).unwrap();
        oracle = positions;
    }
    let mut readings = 0;
    for step in 0..3000 {
        let boundaries: Vec<_> = text
            .char_indices()
            .map(|(p, _)| p)
            .chain([text.len()])
            .collect();
        let len = boundaries.len() - 1;
        match rng.below(10) {
            0..=2 => {
                let p = boundaries[rng.below(len + 1)] as u32;
                let bias = if rng.next() & 1 == 1 {
                    Bias::Right
                } else {
                    Bias::Left
                };
                hs.push(t.create(p, bias).unwrap());
                oracle.push((p, bias));
            }
            3 if !hs.is_empty() => {
                let h = rng.below(hs.len());
                let p = boundaries[rng.below(len + 1)] as u32;
                let bias = if rng.next() & 1 == 1 {
                    Bias::Right
                } else {
                    Bias::Left
                };
                assert!(t.set(hs[h], p, bias));
                oracle[h] = (p, bias);
            }
            4..=6 => {
                let p = if !hs.is_empty() && rng.below(2) == 0 {
                    oracle[rng.below(hs.len())].0 as usize
                } else {
                    boundaries[rng.below(len + 1)]
                };
                let token = if mixed {
                    ["ξ", "Ж", "ab", "λЮ"][rng.below(4)]
                } else {
                    &"XYZWV"[..1 + rng.below(5)]
                };
                // Original trace remains unchanged for ASCII; extended mixed case
                // also exercises insert-before-markers.
                let before = mixed && rng.below(5) == 0;
                assert!(t.insert(p as u32, token.len() as u32, before));
                for (pos, bias) in &mut oracle {
                    if *pos > p as u32 || *pos == p as u32 && (*bias == Bias::Right || before) {
                        *pos += token.len() as u32;
                    }
                }
                text.insert_str(p, token);
            }
            _ if len > 0 => {
                let p = rng.below(len);
                let span = if rng.below(4) == 0 { 60 } else { 4 };
                let q = (p + 1 + rng.below(span)).min(len);
                let (a, b) = (boundaries[p] as u32, boundaries[q] as u32);
                assert!(t.delete(a, b));
                for (pos, _) in &mut oracle {
                    if *pos > a {
                        *pos = if *pos <= b { a } else { *pos - (b - a) };
                    }
                }
                text.replace_range(a as usize..b as usize, "");
            }
            _ => {}
        }
        assert_eq!(t.extent(), text.len() as u32);
        for (h, &(expected, bias)) in hs.iter().zip(&oracle) {
            readings += 1;
            assert_eq!(
                t.resolve(*h),
                Some(expected),
                "seed={seed} mixed={mixed} bulk={bulk} step={step} handle={h:?}"
            );
            assert_eq!(t.bias(*h), Some(bias));
        }
    }
    eprintln!("anchor oracle seed={seed} mixed={mixed} bulk={bulk} readings={readings}");
}
#[test]
fn original_probe_random_trace_gnu_oracle() {
    for seed in [77, 1234, 999_331] {
        trace(seed, false, false);
    }
}
#[test]
fn mixed_greek_english_cyrillic_random_trace() {
    for seed in [77, 1234, 999_331] {
        trace(seed, true, true);
    }
}
#[test]
fn stale_handles_and_checked_limits() {
    let mut t = AnchorTree::new(u32::MAX);
    let h = t.create(u32::MAX, Bias::Right).unwrap();
    assert!(!t.insert(0, 1, false));
    assert_eq!(t.resolve(h), Some(u32::MAX));
    assert!(t.delete(0, u32::MAX));
    assert_eq!(t.resolve(h), Some(0));
    assert!(t.remove(h));
    assert!(!t.remove(h));
    let new = t.create(0, Bias::Left).unwrap();
    assert_ne!(h, new);
    assert_eq!(t.resolve(h), None);
    assert!(!t.set(h, 0, Bias::Right));
    assert!(!t.set(new, 1, Bias::Right));
    assert_eq!(AnchorTree::bytes_per_slot(), 24);
}
#[test]
fn sorted_build_matches_individual_creation_after_edits() {
    let positions: Vec<_> = (0..=500)
        .flat_map(|p| [(p, Bias::Left), (p, Bias::Right)])
        .collect();
    let (mut bulk, hs) = AnchorTree::from_sorted(500, &positions).unwrap();
    let mut slow = AnchorTree::new(500);
    let slow_hs: Vec<_> = positions
        .iter()
        .map(|&(p, b)| slow.create(p, b).unwrap())
        .collect();
    for at in 0..250 {
        assert!(bulk.delete(at, at + 1));
        assert!(slow.delete(at, at + 1));
        assert!(bulk.insert(at, 1, false));
        assert!(slow.insert(at, 1, false));
        for (&a, &b) in hs.iter().zip(&slow_hs) {
            assert_eq!(bulk.resolve(a), slow.resolve(b));
        }
    }
    assert!(AnchorTree::from_sorted(2, &[(2, Bias::Right), (1, Bias::Left)]).is_none());
}
#[test]
fn collapse_then_insert_gnu_known_answer() {
    let mut t = AnchorTree::new(6);
    let left = t.create(4, Bias::Left).unwrap();
    let right = t.create(2, Bias::Right).unwrap();
    assert!(t.delete(2, 4));
    assert!(t.insert(2, 2, false));
    assert_eq!(t.resolve(left), Some(2));
    assert_eq!(t.resolve(right), Some(4));
}
