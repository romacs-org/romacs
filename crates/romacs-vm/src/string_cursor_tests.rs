use crate::{
    Vm,
    coordinates::CharOffset,
    data::{Object, LispString},
};

#[test]
fn review_ascii_aset_flags_and_cost() {
    let mut s = LispString::new(&"a".repeat(262144));
    s.set_code(CharOffset(0), 97).unwrap();
    assert_eq!(s.flag_scanned.get(), 0, "ASCII to ASCII must not scan the string for flags");
    s.set_code(CharOffset(0), 233).unwrap();
    assert!(s.is_utf8() && s.is_multibyte() && !s.is_ascii());
    s.set_code(CharOffset(0), 97).unwrap();
    assert!(s.is_utf8() && s.is_multibyte() && s.is_ascii());
    s.set_code(CharOffset(0), 4194303).unwrap();
    assert!(!s.is_utf8() && !s.is_ascii());
    s.set_code(CharOffset(0), 97).unwrap();
    assert!(s.is_utf8() && s.is_ascii() && s.is_multibyte());
    // D001 intentionally permits width changes GNU rejects. The slot must follow
    // the encoded character class without changing multibyteness or identity.
    let mut vm = Vm::new();
    let raw = vm.eval_source_raw("(progn (setq review-aset-string (make-string 2 97 t)) (aset review-aset-string 0 4194303) review-aset-string)").unwrap();
    assert!(matches!(vm.object(raw).unwrap(), Object::RawString(_)));
    let ascii = vm.eval_source_raw("(progn (aset review-aset-string 0 97) review-aset-string)").unwrap();
    assert_eq!(ascii, raw);
    let Object::String(s) = vm.object(ascii).unwrap() else { panic!("restored UTF8 slot"); };
    assert!(s.is_utf8() && s.is_ascii() && s.is_multibyte());
}

#[test]
fn populated_string_cursors_survive_compaction_and_copy() {
    let mut vm = Vm::new();
    vm.allocate_object(Object::Bytes(vec![0]));
    let s = LispString::new(&"aλ🦀".repeat(1000));
    assert_eq!(s.get(CharOffset(1234)), Some('λ'));
    assert_eq!(s.get(CharOffset(2345)), Some('🦀'));
    let mut roots = [vm.allocate_object(Object::String(Box::new(s)))];
    vm.collect(&mut roots);
    let Object::String(s) = vm.object(roots[0]).unwrap() else {
        panic!()
    };
    assert_eq!(s.get(CharOffset(1235)), Some('🦀'));
    assert_eq!(s.get(CharOffset(2344)), Some('λ'));
    let mut copy = s.clone();
    copy.set(CharOffset(0), '🦀').unwrap();
    assert_eq!(copy.get(CharOffset(2344)), Some('λ'));
    assert_eq!(s.get(CharOffset(0)), Some('a'));
}

#[test]
fn string_coordinates_are_cached_and_mutation_invalidates() {
    let mut s = LispString::new(&"aλ🦀".repeat(1000));
    for i in 0..s.len() {
        assert_eq!(s.get(CharOffset(i)), Some(['a', 'λ', '🦀'][i % 3]));
    }
    assert!(
        s.scanned.get() <= 3000,
        "sequential indexing must scan at most once"
    );
    for _ in 0..100 {
        assert_eq!(s.get(CharOffset(0)), Some('a'));
        assert_eq!(s.get(CharOffset(2999)), Some('🦀'));
    }
    s.scanned.set(0);
    for _ in 0..100 {
        for i in [750, 2250] {
            assert_eq!(s.get(CharOffset(i)), Some('a'));
        }
    }
    assert!(
        s.scanned.get() < 5000,
        "two alternating readers must not thrash"
    );
    s.set(CharOffset(0), '🦀').unwrap();
    assert_eq!(s.get(CharOffset(2250)), Some('a'));
    s.set(CharOffset(2250), 'λ').unwrap();
    assert_eq!(s.get(CharOffset(2250)), Some('λ'));
    assert_eq!(s.get(CharOffset(2251)), Some('λ'));
    assert_eq!(s.get(CharOffset(3000)), None);
    let expected: Vec<_> = s.as_str().chars().collect();
    for i in (0..s.len()).rev() {
        assert_eq!(s.get(CharOffset(i)), Some(expected[i]));
    }
    let mut seed = 17usize;
    for _ in 0..1000 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let i = seed % s.len();
        assert_eq!(s.get(CharOffset(i)), Some(expected[i]));
    }
}
