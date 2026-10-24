use super::*;

fn piece(n: usize) -> Chunk {
    Sum {
        bytes: n,
        chars: n.div_ceil(2),
        nl: n / 3,
    }
    .into()
}

fn verify(chunks: &Chunks, model: &[Chunk]) {
    chunks.check();
    assert_eq!(chunks.len(), model.len());
    assert_eq!(len(&chunks.root), model.len().saturating_sub(1));
    assert_eq!(chunks.tail.is_some(), !model.is_empty());
    let mut prefix = Sum::default();
    for (i, chunk) in model.iter().enumerate() {
        assert_eq!(chunks.prefix(i), prefix);
        assert_eq!(chunks[i].wide(), chunk.wide());
        for field in [
            (|s: Sum| s.bytes) as fn(Sum) -> usize,
            |s: Sum| s.chars,
            |s: Sum| s.nl,
        ] {
            for key in field(prefix)..field(prefix.plus(chunk.wide())) {
                assert_eq!(chunks.locate(key, field), (i, prefix));
            }
        }
        prefix = prefix.plus(chunk.wide());
    }
    assert_eq!(chunks.prefix(model.len()), prefix);
    assert_eq!(chunks.total(), prefix);
}

#[test]
fn tail_updates_and_index_do_no_tree_work() {
    for n in [1, 17, 4097] {
        let mut chunks = Chunks::from_pieces(vec![piece(4); n]);
        chunks.work = 0;
        super::super::TREE_LOOKUPS.with(|count| count.set(0));
        assert_eq!(chunks[n - 1].wide(), piece(4).wide());
        assert_eq!(super::super::TREE_LOOKUPS.with(|count| count.get()), 0);
        chunks.add(n - 1, piece(1).wide());
        chunks.set(n - 1, piece(6));
        chunks.replace_summary(n - 1, piece(6).wide(), piece(7).wide());
        let prefix = chunks.prefix(n - 1);
        let result = chunks.insert_summary(prefix.bytes + 1, piece(1).wide());
        assert_eq!(
            result.map(|(i, p, old)| (i, p, old.wide())),
            Some((n - 1, prefix, piece(7).wide()))
        );
        assert_eq!(chunks.work, 0, "tail updates cannot visit AVL ancestors");
        let mut model = vec![piece(4); n];
        model[n - 1] = piece(7).wide().plus(piece(1).wide()).into();
        verify(&chunks, &model);
    }
}

#[test]
fn tail_transitions_empty_and_boundary_selection() {
    for n in 0..12 {
        let mut model: Vec<_> = (0..n).map(|i| piece(i % 7)).collect();
        let mut chunks = Chunks::from_pieces(model.clone());
        verify(&chunks, &model);
        for i in 0..12 {
            chunks.insert(chunks.len(), piece(i % 7));
            model.push(piece(i % 7));
            verify(&chunks, &model);
        }
        let snapshot = chunks.clone();
        let saved = model.clone();
        while let Some(last) = model.pop() {
            assert_eq!(chunks.remove(model.len()).wide(), last.wide());
            verify(&chunks, &model);
        }
        verify(&snapshot, &saved);
        chunks.insert(0, Chunk::default());
        assert_eq!(chunks.insert_summary(0, piece(2).wide()).unwrap().0, 0);
        verify(&chunks, &[piece(2)]);
    }

    let mut chunks = Chunks::from_pieces(vec![piece(4), piece(3)]);
    let (i, prefix, old) = chunks.insert_summary(4, piece(1).wide()).unwrap();
    assert_eq!(
        (i, prefix, old.wide()),
        (0, Sum::default(), piece(4).wide())
    );
    verify(
        &chunks,
        &[piece(4).wide().plus(piece(1).wide()).into(), piece(3)],
    );
    let before = chunks.total();
    assert!(
        chunks
            .insert_summary(before.bytes, piece(2 * super::super::T).wide())
            .is_none()
    );
    assert_eq!(
        chunks.total(),
        before,
        "rejected tail update must be atomic"
    );
    chunks.check();
}

#[test]
fn mixed_operations_and_clones_match_flat_chunks() {
    let mut seed = 0x8a9d_7321_u64;
    let mut chunks = Chunks::default();
    let mut model = Vec::new();
    for step in 0..6000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let value = (seed >> 24) as usize;
        let i = value % (model.len() + 1);
        let chunk = piece(value % 9);
        match value % 8 {
            0 | 1 => {
                chunks.insert(i, chunk);
                model.insert(i, chunk);
            }
            2 if !model.is_empty() => {
                let i = i % model.len();
                assert_eq!(chunks.remove(i).wide(), model.remove(i).wide());
            }
            3 if !model.is_empty() => {
                let i = i % model.len();
                chunks.set(i, chunk);
                model[i] = chunk;
            }
            4 if !model.is_empty() => {
                let i = i % model.len();
                chunks.replace_summary(i, model[i].wide(), chunk.wide());
                model[i] = chunk;
            }
            5 if !model.is_empty() => {
                let i = i % model.len();
                if model[i].bytes < (2 * super::super::T) as u32 {
                    chunks.add(i, piece(1).wide());
                    model[i] = model[i].wide().plus(piece(1).wide()).into();
                }
            }
            6 if !model.is_empty() => {
                let key = value % (chunks.total().bytes + 1);
                let before = chunks.total();
                if let Some((i, prefix, old)) = chunks.insert_summary(key, chunk.wide()) {
                    assert_eq!(old.wide(), model[i].wide());
                    assert_eq!(
                        prefix,
                        model[..i]
                            .iter()
                            .fold(Sum::default(), |s, c| s.plus(c.wide()))
                    );
                    assert!(prefix.bytes <= key && key <= prefix.bytes + old.bytes as usize);
                    model[i] = old.wide().plus(chunk.wide()).into();
                } else {
                    assert_eq!(chunks.total(), before);
                }
            }
            _ => {
                let mut cloned = chunks.clone();
                cloned.insert(cloned.len(), chunk);
                let mut cloned_model = model.clone();
                cloned_model.push(chunk);
                verify(&cloned, &cloned_model);
                if step % 3 == 0 {
                    chunks = Chunks::from_pieces(model.clone());
                }
            }
        }
        verify(&chunks, &model);
    }
}
