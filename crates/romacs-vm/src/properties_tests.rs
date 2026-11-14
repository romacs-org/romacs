use super::*;
/// The key tests call NAME.
fn named(name: &str) -> Key {
    Key::test(name)
}
fn check(tree: &Tree) -> (usize, usize, u32) {
    match tree {
        Tree::Leaf(runs, len, present, uniform) => {
            assert!(!runs.is_empty() && runs.len() <= LEAF);
            assert_eq!(*uniform, runs.windows(2).all(|w| w[0].observed() == w[1].observed()), "leaf uniform summary");
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
            uniform,
        } => {
            let (a, b, h) = check(left);
            let (c, d, j) = check(right);
            assert!(h.abs_diff(j) <= 1, "annotation AVL balance");
            assert_eq!(*uniform, left.is_uniform() && right.is_uniform()
                && left.edge(true).observed() == right.edge(false).observed(), "branch uniform summary");
            assert_eq!((*len, *present, *height), (a + c, b + d, 1 + h.max(j)));
            (*len, *present, *height)
        }
    }
}
#[test]
fn absence_nil_and_value_are_independent_per_key() {
    let mut p = Properties::new();
    assert_eq!(p.get_property(ByteOffset(3), named("face")), None);
    p.put(ByteOffset(2)..ByteOffset(8), named("face"), Value::NIL);
    p.put(ByteOffset(0)..ByteOffset(10), named("fontified"), Value::TRUE);
    assert_eq!(p.get_property(ByteOffset(3), named("face")), Some(Value::NIL));
    assert_eq!(p.get_property(ByteOffset(0), named("face")), None);
    assert_eq!(
        p.get_property(ByteOffset(0), named("fontified")),
        Some(Value::TRUE)
    );
    p.remove(ByteOffset(0)..ByteOffset(10), named("face"));
    assert!(!p.has_key(named("face")));
    assert!(p.has_key(named("fontified")));
}
#[test]
fn replacement_preserves_stamp_and_new_keys_prepend() {
    let mut p = Properties::new();
    let r = ByteOffset(0)..ByteOffset(10);
    p.put(r.clone(), named("a"), Value::TRUE);
    p.put(r.clone(), named("b"), Value::NIL);
    p.put(r.clone(), named("a"), Value::integer(3).unwrap());
    assert_eq!(
        p.properties_at(ByteOffset(1)),
        vec![(named("b"), Value::NIL), (named("a"), Value::integer(3).unwrap())]
    );
    p.remove(r.clone(), named("a"));
    p.put(r, named("a"), Value::TRUE);
    assert_eq!(p.properties_at(ByteOffset(1))[0].0, named("a"));
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
            let v: Values = [(named("x"), Value::integer(i).unwrap())].into_iter().collect();
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
                assert_eq!(new.get_property(*at, *key), Some(*value));
            }
        }
        for tree in new.trees() {
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
        let name = ["face", "fontified", "x"].map(named)[key];
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
            for (k, name) in ["face", "fontified", "x"].map(named).into_iter().enumerate() {
                assert_eq!(
                    p.get_property(ByteOffset(at), name),
                    row[k],
                    "operation {i} byte {at} key {name:?}"
                );
            }
        }
        for tree in p.trees() {
            check(tree);
        }
    }
}
#[test]
fn boundaries_ignore_stamp_but_not_presence_or_identity() {
    let mut p = Properties::new();
    p.put(ByteOffset(0)..ByteOffset(2), named("face"), Value::NIL);
    p.put(ByteOffset(2)..ByteOffset(4), named("face"), Value::NIL);
    assert_eq!(p.next_change(ByteOffset(0), ByteOffset(8)), ByteOffset(4));
    p.put(ByteOffset(3)..ByteOffset(4), named("face"), Value::TRUE);
    assert_eq!(p.next_change(ByteOffset(0), ByteOffset(8)), ByteOffset(3));
    assert_eq!(p.next_change(ByteOffset(3), ByteOffset(8)), ByteOffset(4));
    assert_eq!(p.next_change(ByteOffset(4), ByteOffset(8)), ByteOffset(8));
    p.put(ByteOffset(6)..ByteOffset(7), named("x"), Value::NIL);
    assert_eq!(p.next_change(ByteOffset(4), ByteOffset(8)), ByteOffset(6));
}
#[test]
fn exhausted_stamp_counter_compacts_live_ranks_without_changing_order() {
    let mut p = Properties::new();
    p.put(ByteOffset(0)..ByteOffset(4), named("a"), Value::NIL);
    p.put(ByteOffset(0)..ByteOffset(4), named("b"), Value::TRUE);
    p.stamp = u32::MAX;
    p.put(ByteOffset(0)..ByteOffset(4), named("c"), Value::TRUE);
    assert_eq!(
        p.properties_at(ByteOffset(1)),
        vec![(named("c"), Value::TRUE), (named("b"), Value::TRUE), (named("a"), Value::NIL)]
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
            named("x"),
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
        let name = ["face", "fontified", "x"].map(named)[(seed >> 32) as usize % 3];
        // Few distinct values, so runs with equal values and different
        // stamps abut.
        let value = Value::integer((seed >> 40) as i64 % 3).unwrap();
        if i % 4 == 3 {
            p.remove(ByteOffset(start)..ByteOffset(end), name);
        } else {
            p.put(ByteOffset(start)..ByteOffset(end), name, value);
        }
        let state = |at: usize| ["face", "fontified", "x"].map(named).map(|k| p.get_property(ByteOffset(at), k));
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
#[test]
fn slice_matches_the_per_byte_oracle() {
    let mut p = Properties::new();
    let len = 600;
    let mut seed = 5u64;
    for i in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let start = seed as usize % len;
        let end = (start + 1 + ((seed >> 20) as usize % 30)).min(len);
        let name = ["face", "fontified"].map(named)[(seed >> 32) as usize % 2];
        p.put(ByteOffset(start)..ByteOffset(end), name, Value::integer(i).unwrap());
        let a = (seed >> 41) as usize % (len + 10);
        let b = (a + ((seed >> 50) as usize % 300)).min(len + 10);
        let part = p.slice(ByteOffset(a)..ByteOffset(b));
        for tree in part.trees() {
            check(tree);
        }
        for at in 0..b.saturating_sub(a) + 5 {
            for name in ["face", "fontified"].map(named) {
                let expected = if a + at < b { p.get_property(ByteOffset(a + at), name) } else { None };
                assert_eq!(part.get_property(ByteOffset(at), name), expected, "operation {i} slice {a}..{b} at {at} {name:?}");
            }
        }
        assert_eq!(part.properties_at(ByteOffset(0)), if a < b { p.properties_at(ByteOffset(a)) } else { vec![] });
    }
}
#[test]
fn runs_of_keys_end_only_where_those_keys_change() {
    let mut p = Properties::new();
    let one = Value::integer(1).unwrap();
    for at in 0..10 {
        p.put(ByteOffset(at)..ByteOffset(at + 1), named("face"), Value::integer(at as i64 % 2).unwrap());
    }
    p.put(ByteOffset(2)..ByteOffset(5), named("read-only"), Value::TRUE);
    p.put(ByteOffset(4)..ByteOffset(7), named("category"), one);
    let keys = ["read-only", "category", "absent"].map(named);
    let runs = p.runs_of(ByteOffset(1)..ByteOffset(9), &keys);
    let starts: Vec<_> = runs.iter().map(|(at, _)| at.0).collect();
    assert_eq!(starts, vec![1, 2, 4, 5, 7]);
    fn keys_of(values: &[(Key, Value)]) -> Vec<Key> {
        values.iter().map(|(k, _)| *k).collect()
    }
    assert_eq!(keys_of(&runs[0].1), Vec::<Key>::new());
    assert_eq!(keys_of(&runs[2].1), vec![named("read-only"), named("category")]);
    assert_eq!(runs[3].1, vec![(named("category"), one)]);
    // Every run agrees with the full properties at its start, on KEYS.
    for (at, values) in &runs {
        let full: Vec<_> = p.properties_at(*at).into_iter().filter(|(k, _)| keys.contains(k)).collect();
        assert_eq!(full.len(), values.len());
        assert!(full.iter().all(|(k, v)| values.iter().any(|(key, value)| key == k && value == v)));
    }
    assert!(p.runs_of(ByteOffset(3)..ByteOffset(3), &keys).is_empty());
    assert_eq!(p.runs_of(ByteOffset(0)..ByteOffset(10), &[named("absent")]).len(), 1);
}
#[test]
fn next_change_matches_the_per_byte_oracle_across_uniform_stretches() {
    // Few distinct values, put and edited in short pieces, so long
    // stretches read one value under many stamps: the searches pass such
    // stretches whole, and must still stop at every change, bounded by
    // their limit, after edits and removals as well.
    let mut p = Properties::new();
    let mut len = 300;
    let mut seed = 29u64;
    for i in 0..1500 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let start = seed as usize % (len + 1);
        let end = (start + ((seed >> 20) as usize % 30)).min(len);
        let name = ["face", "fontified", "x"].map(named)[(seed >> 32) as usize % 3];
        let value = Value::integer((seed >> 40) as i64 % 2).unwrap();
        match i % 5 {
            4 => p.remove(ByteOffset(start)..ByteOffset(end), name),
            3 => {
                let inserted = (seed >> 44) as usize % 4;
                p.replace(ByteOffset(start)..ByteOffset(end), inserted);
                len = len - (end - start) + inserted;
            }
            _ => p.put(ByteOffset(start)..ByteOffset(end), name, value),
        }
        for tree in p.trees() {
            check(tree);
        }
        let state = |at: usize| ["face", "fontified", "x"].map(named).map(|k| p.get_property(ByteOffset(at), k));
        for at in 0..len {
            for limit in [len, (at + 7).min(len)] {
                let mut expected = at + 1;
                while expected < limit && state(expected) == state(at) {
                    expected += 1;
                }
                assert_eq!(p.next_change(ByteOffset(at), ByteOffset(limit)), ByteOffset(expected.min(limit)),
                           "operation {i} at {at} limit {limit}");
            }
        }
    }
}

fn put_syntax(p: &mut Properties, range: Range<usize>, value: i64) {
    p.put(ByteOffset(range.start)..ByteOffset(range.end), Key::SYNTAX_TABLE, Value::integer(value).unwrap());
}
/// The log's ranges as (START, END), taken.
fn syntax_log(p: &mut Properties) -> Vec<(usize, usize)> {
    let changes = p.take_syntax_changes();
    assert!(!changes.all && !changes.hull, "{changes:?}");
    changes.ranges.iter().map(|r| (r.start, r.end)).collect()
}
#[test]
fn the_syntax_log_past_its_count_is_the_hull_of_its_ranges() {
    // COUNT one-byte puts three bytes apart, in a scrambled order.
    let log = |count: usize| {
        let mut p = Properties::new();
        for i in 0..count {
            let at = 1000 + 3 * (i * 37 % count);
            put_syntax(&mut p, at..at + 1, 1);
        }
        p.take_syntax_changes()
    };
    let kept = log(64);
    let mut starts: Vec<_> = kept.ranges.iter().map(|r| (r.start, r.len())).collect();
    starts.sort_unstable();
    assert_eq!(starts, (0..64).map(|i| (1000 + 3 * i, 1)).collect::<Vec<_>>());
    assert!(!kept.hull && !kept.all);
    for count in [65, 500] {
        let hull = log(count);
        let ends: Vec<_> = hull.ranges.iter().map(|r| (r.start, r.end)).collect();
        assert_eq!(ends, [(1000, 998 + 3 * count)], "{count}: from the first put's start to the last one's end");
        assert!(hull.hull && !hull.all, "{count}");
    }
}
#[test]
fn the_syntax_log_holds_only_the_runs_a_write_changes() {
    let mut p = Properties::new();
    put_syntax(&mut p, 10..20, 2);
    put_syntax(&mut p, 20..30, 3);
    put_syntax(&mut p, 30..40, 2);
    assert_eq!(syntax_log(&mut p), [(10, 40)], "neighbours are one range");
    // The value there, over all of it and over a part: no change.
    put_syntax(&mut p, 10..20, 2);
    put_syntax(&mut p, 32..38, 2);
    p.put(ByteOffset(0)..ByteOffset(50), named("face"), Value::TRUE);
    assert!(!p.has_syntax_changes());
    // Where it had none and where it had another.
    put_syntax(&mut p, 5..40, 2);
    assert_eq!(syntax_log(&mut p), [(5, 10), (20, 30)]);
    put_syntax(&mut p, 0..50, 2);
    assert_eq!(syntax_log(&mut p), [(0, 5), (40, 50)]);
    // A remove changes the runs that had a value.
    p.remove(ByteOffset(50)..ByteOffset(90), Key::SYNTAX_TABLE);
    assert!(!p.has_syntax_changes());
    put_syntax(&mut p, 60..70, 4);
    p.take_syntax_changes();
    p.remove(ByteOffset(45)..ByteOffset(100), Key::SYNTAX_TABLE);
    assert_eq!(syntax_log(&mut p), [(45, 50), (60, 70)]);
    p.remove(ByteOffset(45)..ByteOffset(100), Key::SYNTAX_TABLE);
    assert!(!p.has_syntax_changes());
    // nil is a value: over text with none it is a change, again it is not.
    p.put(ByteOffset(60)..ByteOffset(70), Key::SYNTAX_TABLE, Value::NIL);
    assert_eq!(syntax_log(&mut p), [(60, 70)]);
    p.put(ByteOffset(60)..ByteOffset(70), Key::SYNTAX_TABLE, Value::NIL);
    assert!(!p.has_syntax_changes());
}
#[test]
fn a_reset_marks_every_syntax_property_changed() {
    let mut p = Properties::new();
    p.put(ByteOffset(0)..ByteOffset(5), named("face"), Value::TRUE);
    p.reset();
    assert!(!p.has_syntax_changes(), "it had no `syntax-table' property");
    for i in 0..70 {
        put_syntax(&mut p, 10 * i..10 * i + 1, 1);
    }
    p.reset();
    put_syntax(&mut p, 3..4, 1);
    let changes = p.take_syntax_changes();
    assert!(changes.all && changes.ranges.is_empty(), "{changes:?}");
    assert!(!p.has_syntax_changes());
}
