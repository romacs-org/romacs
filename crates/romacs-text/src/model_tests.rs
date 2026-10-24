use super::test_support::iterations;
use super::*;
use proptest::prelude::*;
fn char_count(b: &[u8], mb: bool) -> usize {
    b.iter().filter(|&&b| !mb || char_head_p(b)).count()
}
fn exercise(ops: &[(u8, usize, usize, u8)], mb: bool) {
    let mut t = Text::new(mb);
    let mut model = Vec::new();
    let mut ms: Vec<(MarkerId, usize, bool, bool)> = vec![];
    let (mut tick, mut ctick) = (0, 0);
    for &(op, a, b, v) in ops {
        let mut bounds: Vec<_> = model
            .iter()
            .enumerate()
            .filter(|&(_, &b)| !mb || char_head_p(b))
            .map(|(i, _)| i)
            .collect();
        bounds.push(model.len());
        let f = bounds[a % bounds.len()];
        let z = bounds[b % bounds.len()];
        let (f, z) = (f.min(z), f.max(z));
        let mut encoded = [0; 5];
        let codes = [97, 10, 0xe9, 0x20ac, 0x1f600, 0x234567, 0x3fff80];
        let n = if mb {
            romacs_chars::char_string(codes[v as usize % codes.len()], &mut encoded)
        } else {
            encoded[0] = v;
            1
        };
        let s = if v == 0 && op & 128 != 0 {
            &[][..]
        } else {
            &encoded[..n]
        };
        let batch = s.repeat((a >> 16) % 80 + 1);
        let s = batch.as_slice();
        let which = op % 9;
        if which < 3 {
            let end = if which == 0 { f } else { z };
            let inserted = if which == 1 { &[][..] } else { s };
            let deleted = end - f;
            let dc = char_count(&model[f..end], mb);
            let ic = char_count(inserted, mb);
            let e = match which {
                0 => t.insert(BytePos(f), inserted, op & 16 != 0),
                1 => t.delete(BytePos(f), BytePos(end)),
                _ => t.replace(BytePos(f), BytePos(end), inserted),
            };
            assert_eq!(
                e,
                Edit {
                    at: TextPos {
                        byte: BytePos(f),
                        char: CharPos(char_count(&model[..f], mb))
                    },
                    del: (deleted, dc),
                    ins: (inserted.len(), ic)
                }
            );
            for &mut (_, ref mut p, after, live) in &mut ms {
                if !live {
                    continue;
                }
                let old = *p;
                if deleted == 0 {
                    if old > f || (old == f && (after || (which == 0 && op & 16 != 0))) {
                        *p += inserted.len();
                    }
                } else if old > end || (which == 2 && old == end) {
                    *p = old - deleted + inserted.len();
                } else if old > f {
                    *p = f;
                }
            }
            model.splice(f..end, inserted.iter().copied());
            if dc + ic > 0 {
                tick += (dc + ic).ilog2() as u64 + 1;
                ctick = tick;
            }
        } else if which == 3 {
            ms.push((t.marker_new(BytePos(f), v & 1 != 0), f, v & 1 != 0, true));
        } else if which == 8 {
            t.bump_modiff(a % 1024);
            tick += if a % 1024 == 0 {
                1
            } else {
                (a % 1024).ilog2() as u64 + 1
            };
        } else if !ms.is_empty() {
            let i = a % ms.len();
            let (id, p, after, live) = &mut ms[i];
            match which {
                4 => {
                    assert_eq!(t.marker_set(*id, BytePos(f)), *live);
                    if *live {
                        *p = f;
                    }
                }
                5 => {
                    assert_eq!(t.marker_set_type(*id, v & 1 != 0), *live);
                    if *live {
                        *after = v & 1 != 0;
                    }
                }
                _ => {
                    assert_eq!(t.marker_free(*id), *live);
                    *live = false;
                }
            }
        }
        let mut got = vec![];
        t.copy_to(BytePos(0), BytePos(t.len_bytes()), &mut got);
        assert_eq!(got, model);
        assert_eq!((t.modiff(), t.chars_modiff()), (tick, ctick));
        assert_eq!(t.is_multibyte(), mb);
        let mut c = 0;
        let mut nl = 0;
        for (i, &byte) in model.iter().enumerate() {
            assert_eq!(t.byte_at(BytePos(i)), byte);
            assert_eq!(t.newlines_before(BytePos(i)), nl);
            nl += usize::from(byte == b'\n');
            let floor = if !mb {
                i
            } else {
                (0..=i).rev().find(|&j| char_head_p(model[j])).unwrap()
            };
            assert_eq!(t.floor_char_boundary(BytePos(i)), BytePos(floor));
            if !mb || char_head_p(byte) {
                assert_eq!(t.byte_of_char(CharPos(c)), BytePos(i));
                assert_eq!(t.char_of_byte(BytePos(i)), CharPos(c));
                let expected = if mb {
                    string_char(&model[i..])
                } else {
                    (byte as u32, 1)
                };
                assert_eq!(t.char_at(BytePos(i)), expected);
                c += 1;
            }
        }
        assert_eq!(t.byte_of_char(CharPos(c)), BytePos(model.len()));
        assert_eq!(t.len_chars(), c);
        assert_eq!(t.len_newlines(), model.iter().filter(|&&b| b == 10).count());
        assert_eq!(
            t.newlines_before(BytePos(f.min(model.len()))),
            model[..f.min(model.len())]
                .iter()
                .filter(|&&b| b == 10)
                .count()
        );
        let from = a % (model.len() + 1);
        let lim = b % (model.len() + 1);
        let (lo, hi) = (from.min(lim), from.max(lim));
        let (a, b) = t.segments(BytePos(lo), BytePos(hi));
        assert_eq!([a, b].concat(), model[lo..hi]);
        let mut appended = vec![42];
        t.copy_to(BytePos(lo), BytePos(hi), &mut appended);
        assert_eq!(&appended[1..], &model[lo..hi]);
        assert_eq!(
            t.count_newlines(BytePos(lo), BytePos(hi)),
            model[lo..hi].iter().filter(|&&b| b == 10).count()
        );
        for sign in [-1isize, 1] {
            let (from, limit) = if sign > 0 { (lo, hi) } else { (hi, lo) };
            let want = v as usize % 5;
            let mut hits: Vec<_> = (lo..hi).filter(|&i| model[i] == 10).collect();
            if sign < 0 {
                hits.reverse();
            }
            let found = hits.len().min(want);
            let p = if want == 0 {
                from
            } else if found < want {
                limit
            } else {
                hits[want - 1] + usize::from(sign > 0)
            };
            assert_eq!(
                t.find_newline(BytePos(from), BytePos(limit), sign * want as isize),
                (BytePos(p), found)
            );
        }
        for &(id, p, _, live) in &ms {
            assert_eq!(
                t.marker_pos(id),
                live.then(|| TextPos {
                    byte: BytePos(p),
                    char: CharPos(char_count(&model[..p], mb))
                })
            );
        }
        assert_eq!(t.marker_count(), ms.iter().filter(|m| m.3).count());
        t.check_invariants();
    }
}
proptest! {
 #![proptest_config(ProptestConfig::with_cases(iterations(32, 256) as u32))]
 #[test] fn differential(ops in prop::collection::vec((any::<u8>(),any::<usize>(),any::<usize>(),any::<u8>()),0..200),mb in any::<bool>()) {exercise(&ops,mb);}
}
#[test]
fn fixed_seed_20k() {
    let mut seed = 7u64;
    let ops: Vec<_> = (0..iterations(1024, 20000))
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (
                seed as u8,
                (seed >> 8) as usize,
                (seed >> 32) as usize,
                (seed >> 48) as u8,
            )
        })
        .collect();
    exercise(&ops, true);
    exercise(&ops, false);
}
#[test]
fn unibyte_complete_alphabet() {
    let bytes: Vec<_> = (0..=255).collect();
    let t = Text::from_bytes(&bytes, false);
    for p in 0..256 {
        assert_eq!(t.char_at(BytePos(p)), (p as u32, 1));
        assert_eq!(t.char_of_byte(BytePos(p)), CharPos(p));
        assert_eq!(t.byte_of_char(CharPos(p)), BytePos(p));
    }
    t.check_invariants();
}
#[test]
fn marker_rules_and_stale_ids() {
    let mut t = Text::from_bytes(b"abcdef", true);
    let a = t.marker_new(BytePos(2), false);
    let b = t.marker_new(BytePos(2), true);
    t.insert(BytePos(2), b"XY", false);
    assert_eq!(t.marker_pos(a).unwrap().byte, BytePos(2));
    assert_eq!(t.marker_pos(b).unwrap().byte, BytePos(4));
    t.insert(BytePos(2), b"Z", true);
    assert_eq!(t.marker_pos(a).unwrap().byte, BytePos(3));
    assert!(t.marker_free(a));
    let _ = t.marker_new(BytePos(0), false);
    assert_eq!(t.marker_pos(a), None);
    assert!(!t.marker_free(a));
    t.check_invariants();
}
