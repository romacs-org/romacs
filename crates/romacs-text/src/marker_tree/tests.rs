use super::*;
use proptest::prelude::*;

fn check(tree: &MarkerTree, model: &[(MarkerSlot, u64, bool)]) {
    assert_eq!(tree.len(), model.len());
    for &(slot, p, after) in model {
        assert_eq!(tree.position(slot), Some(p));
        assert_eq!(tree.insertion_type(slot), Some(after));
    }
    let mut expected = model.to_vec();
    expected.sort_by_key(|&(s, p, a)| (p, a, s.0));
    let mut actual: Vec<_> = tree.range(0..u64::MAX).collect();
    actual.sort_by_key(|&(s, p, a)| (p, a, s.0));
    assert_eq!(actual, expected);
    let mut previous = None;
    for &id in &tree.order {
        let l = &tree.leaves[id as usize];
        assert!((1..=CAP).contains(&l.n));
        for i in 0..l.n {
            let key = l.key(i);
            assert!(previous.is_none_or(|p| p <= key));
            previous = Some(key);
            assert_eq!(tree.slots[l.slot[i] as usize], (id << 6) | i as u32);
        }
    }
    assert_eq!(tree.order.len() + tree.free_leaves.len(), tree.leaves.len());
    assert_eq!(tree.free_slots.len() + tree.len(), tree.slots.len());
}

fn check_cached_search(tree: &MarkerTree) {
    let mut keys = vec![(0, false), (u64::MAX, true)];
    for &id in &tree.order {
        let leaf = &tree.leaves[id as usize];
        for i in 0..leaf.n {
            let p = leaf.key(i).0;
            keys.extend([(p, false), (p, true), (p.saturating_add(1), false)]);
        }
    }
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        let expected = tree.order.partition_point(|&id| {
            let l = &tree.leaves[id as usize];
            l.key(l.n - 1) < key
        });
        for hint in (0..=tree.order.len() + 1).chain([usize::MAX]) {
            tree.boundary.set(Some(hint));
            assert_eq!(tree.lower_leaf(key), expected, "key={key:?} hint={hint}");
        }
    }
}

#[test]
fn cached_boundary_survives_every_mutation_and_stale_rank() {
    let mut tree = MarkerTree::new();
    let mut model: Vec<_> = (0..193)
        .map(|p| (tree.new_marker(p, p % 2 != 0), p, p % 2 != 0))
        .collect();
    check_cached_search(&tree);
    assert_eq!(tree.order.len(), 4);
    assert!(
        tree.order[..3]
            .iter()
            .all(|&id| tree.leaves[id as usize].n == CAP)
    );
    // Split before cached ranks; collapse creates ties spanning full leaves.
    for _ in 0..70 {
        model.push((tree.new_marker(1, true), 1, true));
    }
    check_cached_search(&tree);
    tree.replace(0, 200, 3);
    model_replace(&mut model, 0, 200, 3);
    check_cached_search(&tree);
    tree.insert(0, 7, false);
    model_insert(&mut model, 0, 7, false);
    check_cached_search(&tree);
    tree.set(model[0].0, u64::from(u32::MAX) + 100);
    model[0].1 = u64::from(u32::MAX) + 100;
    tree.set_insertion_type(model[1].0, false);
    model[1].2 = false;
    check_cached_search(&tree);
    check(&tree, &model);
    // Remove/merge leaves, empty the tree, then reuse IDs at unrelated keys.
    while let Some((slot, _, _)) = model.pop() {
        tree.free(slot);
        check_cached_search(&tree);
    }
    check_cached_search(&tree.clone());
    for p in [2, 2, 0, u64::from(u32::MAX) + 200] {
        model.push((tree.new_marker(p, false), p, false));
        check_cached_search(&tree);
    }
    check(&tree, &model);
}

fn model_insert(model: &mut [(MarkerSlot, u64, bool)], at: u64, n: u64, bm: bool) {
    for (_, p, a) in model {
        if *p > at || (*p == at && (*a || bm)) {
            *p += n;
        }
    }
}
fn model_replace(model: &mut [(MarkerSlot, u64, bool)], from: u64, to: u64, n: u64) {
    if from == to {
        model_insert(model, from, n, false);
        return;
    }
    for (_, p, _) in model {
        if *p >= to {
            *p = *p - (to - from) + n;
        } else if *p > from {
            *p = from;
        }
    }
}

#[test]
fn exhaustive_small_edit_boundaries() {
    for bits in 0..16 {
        for from in 0..=4 {
            for to in from..=4 {
                for n in 0..=3 {
                    for bm in [false, true] {
                        let mut tree = MarkerTree::new();
                        let mut model: Vec<_> = (0..4)
                            .map(|p| {
                                let a = bits >> p & 1 != 0;
                                (tree.new_marker(p, a), p, a)
                            })
                            .collect();
                        let mut insert_tree = tree.clone();
                        let mut insert_model = model.clone();
                        insert_tree.insert(from, n, bm);
                        model_insert(&mut insert_model, from, n, bm);
                        check(&insert_tree, &insert_model);
                        tree.replace(from, to, n);
                        model_replace(&mut model, from, to, n);
                        check(&tree, &model);
                    }
                }
            }
        }
    }
}

#[test]
fn ties_across_leaves_and_collapse_repartition() {
    let mut tree = MarkerTree::new();
    let mut model: Vec<_> = (0..513)
        .map(|i| {
            let p = i % 3 + 5;
            let a = i % 2 != 0;
            (tree.new_marker(p, a), p, a)
        })
        .collect();
    tree.delete(4, 8);
    model_replace(&mut model, 4, 8, 0);
    check(&tree, &model);
    tree.insert(4, 20, false);
    model_insert(&mut model, 4, 20, false);
    check(&tree, &model);
    for &(slot, _, _) in &model {
        assert!(tree.free(slot));
    }
    assert!(tree.is_empty());
    assert!(tree.range(0..100).next().is_none());
    assert_eq!(tree.position(model[0].0), None);
    assert!(!tree.free(model[0].0));
    for p in 0..513 {
        tree.new_marker(p, false);
    }
    assert_eq!(tree.slots.len(), 513);
}

#[test]
fn wide_spans_and_large_insert_split_without_truncation() {
    let mut tree = MarkerTree::new();
    let wide = u64::from(u32::MAX) + 123;
    let mut model = vec![];
    for p in [0, 1, wide, wide + 1, u64::MAX - wide - 10] {
        model.push((tree.new_marker(p, true), p, true));
    }
    tree.insert(1, wide, false);
    model_insert(&mut model, 1, wide, false);
    check(&tree, &model);
    tree.replace(0, wide * 2, 2);
    model_replace(&mut model, 0, wide * 2, 2);
    check(&tree, &model);
    assert_eq!(std::mem::size_of::<ProposedMarkerObject>(), 16);
}

#[test]
fn narrow_edits_visit_leaves_not_the_marker_array() {
    for n in [10_000, 100_000] {
        let mut tree = MarkerTree::new();
        for p in 0..n {
            tree.new_marker(p * 100, p % 2 != 0);
        }
        for i in 0..100 {
            let p = i * 997;
            tree.insert(p, 2, false);
            assert!(tree.work <= CAP + n as usize / 16, "{}", tree.work);
            tree.delete(p + 1, p + 2);
            assert!(tree.work <= CAP + n as usize / 16, "{}", tree.work);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn every_operation_matches_naive(ops in prop::collection::vec((0u8..9, any::<u16>(), any::<u16>(), any::<bool>()), 1..400)) {
        let mut tree = MarkerTree::new();
        let mut model = vec![];
        for i in 0..160 {
            let p = (i % 9) as u64;
            let a = i % 2 != 0;
            model.push((tree.new_marker(p, a), p, a));
        }
        for (op, x, y, flag) in ops {
            let p = u64::from(x % 512);
            let q = u64::from(y % 512);
            let i = x as usize % model.len().max(1);
            match op {
                0 => model.push((tree.new_marker(p, flag), p, flag)),
                1 if !model.is_empty() => { let old = model.remove(i); prop_assert!(tree.free(old.0)); },
                2 if !model.is_empty() => { prop_assert!(tree.set(model[i].0, p)); model[i].1 = p; },
                3 if !model.is_empty() => { prop_assert!(tree.set_insertion_type(model[i].0, flag)); model[i].2 = flag; },
                4 => { tree.insert(p, q, flag); model_insert(&mut model, p, q, flag); },
                5 => { tree.delete(p.min(q), p.max(q)); model_replace(&mut model, p.min(q), p.max(q), 0); },
                6 => { let n = u64::from(x % 33); tree.replace(p.min(q), p.max(q), n); model_replace(&mut model, p.min(q), p.max(q), n); },
                _ => {
                    let mut actual: Vec<_> = tree.range(p.min(q)..p.max(q)).collect();
                    let mut expected: Vec<_> = model.iter().copied().filter(|&(_, pos, _)| pos >= p.min(q) && pos < p.max(q)).collect();
                    actual.sort_by_key(|&(s,p,a)| (p,a,s.0));
                    expected.sort_by_key(|&(s,p,a)| (p,a,s.0));
                    prop_assert_eq!(actual, expected);
                }
            }
            check(&tree, &model);
        }
    }
}
