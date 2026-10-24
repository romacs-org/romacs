#[cfg(test)]
mod tests {
    use crate::map_keymap::*;
    use crate::*;
    #[test]
    fn map_keymap_cursor_only_detached_subtree_moves_and_deduplicates() {
        for detached in [false, true] {
            let mut vm = Vm::new();
            for _ in 0..3 {
                vm.allocate_object(data::Object::String(Box::new(data::LispString::new(
                    "dead",
                ))));
            }
            let payload = vm.allocate_object(data::Object::String(Box::new(
                data::LispString::new("retained"),
            )));
            let mut table = char_table::CharTable::new(Value::NIL);
            table.set(65537, payload).unwrap();
            let root = table.root.clone();
            let CharTableSlot::Node(child) = root.lock().unwrap().slots[1].clone() else {
                panic!()
            };
            if detached {
                table.set_range(65536, 131071, Value::NIL).unwrap();
            }
            let top = vm.allocate_object(data::Object::CharTable(Box::new(table)));
            let cursor = Cursor {
                stack: vec![MapFrame::Table {
                    tail: Value::NIL,
                    table: Chartab {
                        table: top,
                        frames: vec![
                            Frame::new(root, top, 0, char_table::CharTable::LEN as i64 - 1),
                            Frame::new(child.clone(), top, 65536, 131071),
                        ],
                        val: Value::NIL,
                        from: 65536,
                        to: 131071,
                        final_emitted: false,
                    },
                }],
                internal: false,
                result: Value::NIL,
            };
            let handle = vm.allocate_object(data::Object::MapKeymapCursor(Box::new(cursor)));
            let mut roots = [handle];
            vm.collect(&mut roots);
            assert_ne!(roots[0], handle, "collection must actually compact");
            let kept = CharTableNode::get(&child, 65537);
            assert_ne!(kept, payload);
            assert_eq!(vm.str_view(kept).unwrap().bytes, b"retained");
            assert_eq!(vm.heap_objects(), 3);
            vm.collect(&mut roots);
            assert_eq!(
                vm.str_view(CharTableNode::get(&child, 65537))
                    .unwrap()
                    .bytes,
                b"retained"
            );
            // An external Arc is deliberately NOT a Lisp root.
            drop(child);
            vm.collect(&mut []);
            assert_eq!(vm.heap_objects(), 0);
        }
    }
}
