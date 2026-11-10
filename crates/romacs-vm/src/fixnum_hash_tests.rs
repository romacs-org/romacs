use super::*;
use data::Object;

#[test]
fn immediate_hashes_and_position_hashes_remain_distinct_after_compaction() {
    for test in ["eq", "eql", "equal"] {
        let mut vm = Vm::new();
        vm.set_gc_moving(true);
        vm.read_source_raw("[\"garbage\"]").unwrap();
        let source = format!(
            "(setq s109-key (make-hash-table)) (setq s109-h (make-hash-table :test '{test})) (puthash s109-key 99 s109-h) (puthash 1152921504606846976 1 s109-h) (puthash 2305843009213693951 2 s109-h) (puthash -2305843009213693952 3 s109-h) s109-h"
        );
        let table = vm.eval_source_raw(&source);
        assert!(table.is_ok(), "S109 mixed-key setup: {table:?}");
        let table = table.unwrap();
        let Object::HashTable(before) = vm.object(table).unwrap() else {
            panic!("table")
        };
        assert_eq!(
            before.moving,
            [true, false, false, false],
            "S109 position-hashed classification"
        );
        let hashes = before.hashes[1..].to_vec();
        let mut roots = [table];
        vm.clear_source_cache();
        vm.collect(&mut roots);
        assert_ne!(roots[0], table, "S109 table really moved");
        let Object::HashTable(after) = vm.object(roots[0]).unwrap() else {
            panic!("table")
        };
        assert!(after.stale, "S109 moving key must invalidate buckets");
        assert_eq!(after.hashes[1..], hashes, "S109 immediate hashes unchanged");
        let result = vm.eval_source_raw("(list (gethash s109-key s109-h) (gethash 1152921504606846976 s109-h) (gethash 2305843009213693951 s109-h) (gethash -2305843009213693952 s109-h) (hash-table-count s109-h))").and_then(|v| vm.display(v));
        assert_eq!(
            result,
            Ok("(99 1 2 3 4)".into()),
            "S109 rehash preserves moving and immediate keys"
        );
        let Object::HashTable(after) = vm.object(roots[0]).unwrap() else {
            panic!("table")
        };
        assert!(!after.stale, "S109 lookup rebuilds stale index");
    }
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    let wide = Value::integer(Value::MAX_FIXNUM).unwrap();
    vm.cells.extend([Value::NIL, wide]);
    vm.conses.push((Value::integer(1).unwrap(), wide));
    let cell = Value::reference(1, 5);
    let mut roots = [cell, wide, Value::TRUE, Value::reference(0, 3)];
    vm.collect(&mut roots);
    assert_ne!(roots[0], cell, "S109 cell index actually moved");
    assert_eq!(roots[0].tag(), 5, "S109 cell identity after forwarding");
    assert_eq!(roots[0].payload(), 0, "S109 one-based cell decoded index");
    assert_eq!(vm.cells.iter().copied().collect::<Vec<_>>(), [wide], "S109 cell payload remains immediate");
    assert_eq!(roots[1], wide, "S109 integer bits never forwarded");
    assert_eq!(roots[2], Value::TRUE, "S109 t is not cell zero");
    assert_eq!(
        vm.cons_pair(roots[3]).unwrap(),
        (Value::integer(1).unwrap(), wide),
        "S109 odd integer car is not a cell"
    );
}
