use super::*;
fn check(tree: &Tree) -> (usize, usize, u32) {
    match tree {
        Tree::Leaf(runs, len, present) => {
            assert!(!runs.is_empty() && runs.len() <= LEAF);
            assert!(runs.iter().all(|r| r.len > 0));
            assert_eq!(*len, runs.iter().map(|r| r.len as usize).sum::<usize>());
            assert_eq!(
                *present,
                runs.iter()
                    .filter(|r| r.stamp != 0)
                    .map(|r| r.len as usize)
                    .sum::<usize>()
            );
            (
                runs.iter().map(|r| r.len as usize).sum(),
                runs.iter()
                    .filter(|r| r.stamp != 0)
                    .map(|r| r.len as usize)
                    .sum(),
                1,
            )
        }
        Tree::Branch {
            left,
            right,
            len,
            present,
            height,
        } => {
            let (a, b, h) = check(left);
            let (c, d, j) = check(right);
            assert!(h.abs_diff(j) <= 1, "annotation AVL balance");
            assert_eq!((*len, *present, *height), (a + c, b + d, 1 + h.max(j)));
            (*len, *present, *height)
        }
    }
}
#[test]
fn absence_nil_and_value_are_independent_per_key() {
    let mut p = Properties::new();
    assert_eq!(p.get_property(ByteOffset(3), "face"), None);
    p.put(ByteOffset(2)..ByteOffset(8), "face", Value::NIL);
    p.put(ByteOffset(0)..ByteOffset(10), "fontified", Value::TRUE);
    assert_eq!(p.get_property(ByteOffset(3), "face"), Some(Value::NIL));
    assert_eq!(p.get_property(ByteOffset(0), "face"), None);
    assert_eq!(
        p.get_property(ByteOffset(0), "fontified"),
        Some(Value::TRUE)
    );
    p.remove(ByteOffset(0)..ByteOffset(10), "face");
    assert!(!p.layers.contains_key("face"));
    assert!(p.layers.contains_key("fontified"));
}
#[test]
fn replacement_preserves_stamp_and_new_keys_prepend() {
    let mut p = Properties::new();
    let r = ByteOffset(0)..ByteOffset(10);
    p.put(r.clone(), "a", Value::TRUE);
    p.put(r.clone(), "b", Value::NIL);
    p.put(r.clone(), "a", Value::integer(3).unwrap());
    assert_eq!(
        p.properties_at(ByteOffset(1)),
        vec![("b", Value::NIL), ("a", Value::integer(3).unwrap())]
    );
    p.remove(r.clone(), "a");
    p.put(r, "a", Value::TRUE);
    assert_eq!(p.properties_at(ByteOffset(1))[0].0, "a");
}
#[test]
fn relative_layers_match_the_old_map_after_every_operation() {
    let mut new = Properties::new();
    let mut old = BTreeMap::new();
    let mut seed = 7u64;
    let mut len = 8192usize;
    for i in 0..5000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        if i % 3 == 0 {
            let p = ByteOffset(seed as usize % (len + 1));
            let v = BTreeMap::from([("x".into(), Value::integer(i).unwrap())]);
            assert_eq!(new.insert(p, v.clone()), old.insert(p, v));
        } else {
            let start = seed as usize % (len + 1);
            let end = (start + ((seed >> 32) as usize % 17)).min(len);
            let added = (seed >> 40) as usize % 23;
            new.replace(ByteOffset(start)..ByteOffset(end), added);
            old = old
                .into_iter()
                .filter_map(|(p, v)| {
                    if (start..end).contains(&p.0) {
                        None
                    } else {
                        Some((
                            ByteOffset(if p.0 >= end {
                                p.0 - (end - start) + added
                            } else {
                                p.0
                            }),
                            v,
                        ))
                    }
                })
                .collect();
            len = len - (end - start) + added;
        }
        assert_eq!(new.to_map(), old, "property state after operation {i}");
        for (at, values) in &old {
            for (key, value) in values {
                assert_eq!(new.get_property(*at, key), Some(*value));
            }
        }
        for tree in new.layers.values().flatten() {
            check(tree);
        }
    }
}
#[test]
fn range_writes_removals_and_edits_match_per_byte_oracle() {
    let mut p = Properties::new();
    let mut oracle = vec![[None; 3]; 256];
    let mut seed = 3u64;
    for i in 0..3000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let start = seed as usize % (oracle.len() + 1);
        let end = (start + ((seed >> 20) as usize % 80)).min(oracle.len());
        let key = (seed >> 32) as usize % 3;
        let name = ["face", "fontified", "x"][key];
        if i % 3 == 0 {
            let n = (seed >> 40) as usize % 12;
            p.replace(ByteOffset(start)..ByteOffset(end), n);
            oracle.splice(start..end, vec![[None; 3]; n]);
        } else if i % 3 == 1 {
            let value = if seed & 4 == 0 {
                Value::NIL
            } else {
                Value::integer(i).unwrap()
            };
            p.put(ByteOffset(start)..ByteOffset(end), name, value);
            for row in &mut oracle[start..end] {
                row[key] = Some(value);
            }
        } else {
            p.remove(ByteOffset(start)..ByteOffset(end), name);
            for row in &mut oracle[start..end] {
                row[key] = None;
            }
        }
        for (at, row) in oracle.iter().enumerate() {
            for (k, name) in ["face", "fontified", "x"].iter().enumerate() {
                assert_eq!(
                    p.get_property(ByteOffset(at), name),
                    row[k],
                    "operation {i} byte {at} key {name}"
                );
            }
        }
        for tree in p.layers.values().flatten() {
            check(tree);
        }
    }
}
#[test]
fn boundaries_ignore_stamp_but_not_presence_or_identity() {
    let mut p = Properties::new();
    p.put(ByteOffset(0)..ByteOffset(2), "face", Value::NIL);
    p.put(ByteOffset(2)..ByteOffset(4), "face", Value::NIL);
    assert_eq!(p.next_change(ByteOffset(0), ByteOffset(8)), ByteOffset(4));
    p.put(ByteOffset(3)..ByteOffset(4), "face", Value::TRUE);
    assert_eq!(p.next_change(ByteOffset(0), ByteOffset(8)), ByteOffset(3));
    assert_eq!(p.next_change(ByteOffset(3), ByteOffset(8)), ByteOffset(4));
    assert_eq!(p.next_change(ByteOffset(4), ByteOffset(8)), ByteOffset(8));
    p.put(ByteOffset(6)..ByteOffset(7), "x", Value::NIL);
    assert_eq!(p.next_change(ByteOffset(4), ByteOffset(8)), ByteOffset(6));
}
#[test]
fn exhausted_stamp_counter_compacts_live_ranks_without_changing_order() {
    let mut p = Properties::new();
    p.put(ByteOffset(0)..ByteOffset(4), "a", Value::NIL);
    p.put(ByteOffset(0)..ByteOffset(4), "b", Value::TRUE);
    p.stamp = u32::MAX;
    p.put(ByteOffset(0)..ByteOffset(4), "c", Value::TRUE);
    assert_eq!(
        p.properties_at(ByteOffset(1)),
        vec![("c", Value::TRUE), ("b", Value::TRUE), ("a", Value::NIL)]
    );
    assert_eq!(p.stamp, 3);
}
#[test]
fn packed_run_capacity_and_large_lengths() {
    assert_eq!(size_of::<Run>(), 16);
    let mut p = Properties::new();
    for at in 0..8192 {
        p.put(
            ByteOffset(at)..ByteOffset(at + 1),
            "x",
            Value::integer(at as i64).unwrap(),
        );
    }
    let bytes = p.storage_bytes();
    eprintln!(
        "8192 runs: {bytes} bytes, {} bytes/run including tree/key reservation",
        bytes as f64 / 8192.0
    );
    assert!(bytes <= 8192 * (16 + size_of::<Value>()));
    let mut work = 0;
    let n = u32::MAX as usize + 17;
    let t = Tree::uniform(n, 1, Value::NIL, &mut work).unwrap();
    assert_eq!(t.len(), n);
    assert_eq!(t.at(n - 1).unwrap().value, Value::NIL);
    check(&t);
}
#[test]
fn previous_change_matches_the_per_byte_oracle() {
    let mut p = Properties::new();
    let len = 200;
    let mut seed = 11u64;
    for i in 0..1500 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let start = seed as usize % len;
        let end = (start + ((seed >> 20) as usize % 40)).min(len);
        let name = ["face", "fontified", "x"][(seed >> 32) as usize % 3];
        // Few distinct values, so runs with equal values and different
        // stamps abut.
        let value = Value::integer((seed >> 40) as i64 % 3).unwrap();
        if i % 4 == 3 {
            p.remove(ByteOffset(start)..ByteOffset(end), name);
        } else {
            p.put(ByteOffset(start)..ByteOffset(end), name, value);
        }
        let state = |at: usize| ["face", "fontified", "x"].map(|k| p.get_property(ByteOffset(at), k));
        for at in 0..=len {
            for limit in [0, at / 2] {
                let mut expected = at.saturating_sub(1);
                while expected > 0 && state(expected - 1) == state(at - 1) {
                    expected -= 1;
                }
                let expected = if at == 0 { limit } else { expected.max(limit) };
                assert_eq!(p.previous_change(ByteOffset(at), ByteOffset(limit)), ByteOffset(expected), "operation {i} at {at} limit {limit}");
            }
        }
    }
}
