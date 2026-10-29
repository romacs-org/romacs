use super::*;
use crate::data::Object;

impl Vm {
    pub(crate) fn buffer_standard_case_roots(&mut self) -> crate::Result<[Value; 4]> {
        Leaf { vm: self }.standard_case_roots().map_err(|fault| {
            let (condition, data) = self.fault(fault);
            self.propagated_escape = Some(crate::threaded::Escape::Signal(condition, data));
            "Cannot initialize buffer case tables".to_owned()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(vm: &Vm, buffer: Value) -> [Value; 4] {
        match vm.object_if(buffer).unwrap() {
            Object::Buffer(b) => b.case_tables,
            Object::ByteBuffer(b) => b.case_tables,
            _ => panic!("buffer required"),
        }
    }

    #[test]
    fn casetab_reset_installs_current_standard_four_roots() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(set-case-table (make-char-table 'case-table)) (set-standard-case-table (make-char-table 'case-table)) (kill-all-local-variables)").unwrap();
        let buffer = vm.slots[vm.buffer_slot].value;
        assert_eq!(roots(&vm, buffer), vm.standard_case_tables);
    }

    #[test]
    fn casetab_new_and_reset_buffers_snapshot_current_standard_extras() {
        for action in ["(get-buffer-create \"new-case\")", "(progn (kill-all-local-variables) (current-buffer))"] {
            let mut vm = Vm::new();
            let source = format!("(let ((d (standard-case-table))) (current-buffer) (dotimes (i 3) (set-char-table-extra-slot d i (make-char-table 'case-table))) {action})");
            let buffer = vm.eval_source_raw(&source).unwrap();
            let down = vm.standard_case_tables[0];
            let Object::CharTable(t) = vm.object_if(down).unwrap() else { unreachable!() };
            assert_eq!(roots(&vm, buffer), [down, t.extras[0], t.extras[1], t.extras[2]]);
        }
    }

    #[test]
    fn casetab_creation_and_reset_regenerate_missing_standard_extras() {
        for action in ["(get-buffer-create \"new-case\")", "(progn (kill-all-local-variables) (current-buffer))"] {
            let mut vm = Vm::new();
            let source = format!("(let ((d (standard-case-table))) (current-buffer) (dotimes (i 3) (set-char-table-extra-slot d i nil)) {action})");
            let buffer = vm.eval_source_raw(&source).unwrap();
            let down = vm.standard_case_tables[0];
            let Object::CharTable(t) = vm.object_if(down).unwrap() else { unreachable!() };
            let expected = [down, t.extras[0], t.extras[1], t.extras[2]];
            assert!(expected.iter().all(|v| matches!(vm.object_if(*v), Some(Object::CharTable(_)))));
            assert_eq!(roots(&vm, buffer), expected);
        }
    }

    #[test]
    fn casetab_creation_and_reset_reject_malformed_standard_extras() {
        for action in ["(get-buffer-create \"new-case\")", "(kill-all-local-variables)"] {
            let mut vm = Vm::new();
            let source = format!("(let ((d (standard-case-table))) (current-buffer) (set-char-table-extra-slot d 0 17) (condition-case e {action} (error (list (car e) (car (cdr e)) (eq d (car (cdr (cdr e))))))))");
            let result = vm.eval_source_raw(&source).unwrap();
            assert_eq!(vm.display(result).unwrap(), "(wrong-type-argument case-table-p t)", "{action}");
        }
    }

    #[test]
    fn casetab_canonicalization_invalid_up_mapping_requires_character() {
        for invalid in ["nil", "-1", "4194304", "'bad"] {
            let mut vm = Vm::new();
            let source = format!("(let ((d (make-char-table 'case-table)) (up (make-char-table 'case-table))) (aset d 65 97) (aset up 97 {invalid}) (set-char-table-extra-slot d 0 up) (condition-case e (set-case-table d) (error e)))");
            let result = vm.eval_source_raw(&source).unwrap();
            let expected = format!("(wrong-type-argument characterp {})", if invalid == "'bad" { "bad" } else { invalid });
            assert_eq!(vm.display(result).unwrap(), expected);
        }
    }

    #[test]
    fn casetab_installed_upcase_is_independent_of_mutated_extras() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(let ((d (copy-sequence (standard-case-table)))) (set-case-table d) (set-char-table-extra-slot d 0 nil) (set-char-table-extra-slot d 1 nil) (set-char-table-extra-slot d 2 nil))").unwrap();
        vm.collect(&mut []);
        let mut leaf = Leaf { vm: &mut vm };
        let up = leaf.current_case_mapping(false).ok().unwrap();
        assert_eq!(
            leaf.char_table_element(up, 97).ok().unwrap().integer_value(),
            Some(65)
        );
        let down = leaf.current_case_mapping(true).ok().unwrap();
        assert_eq!(
            leaf.char_table_element(down, 65).ok().unwrap().integer_value(),
            Some(97)
        );
    }
}
use crate::char_table::{CharTable, CharTableNode, CharTableNodeHandle, CharTableSlot};

impl Leaf<'_> {
    pub fn case_table_p(&self, value: Value) -> bool {
        let Some(Object::CharTable(t)) = self.vm.object_if(value) else {
            return false;
        };
        if !self
            .vm
            .names
            .get("case-table")
            .is_some_and(|slot| t.purpose == Value::reference(*slot + 1, 7))
            || t.extras.len() < 3
        {
            return false;
        }
        let [up, canon, eqv] = [t.extras[0], t.extras[1], t.extras[2]];
        let table = |v| matches!(self.vm.object_if(v), Some(Object::CharTable(_)));
        (up == Value::NIL || table(up))
            && ((canon == Value::NIL && eqv == Value::NIL)
                || (table(canon) && (eqv == Value::NIL || table(eqv))))
    }

    /// GNU reset_buffer installs the standard down table and its current extras.
    pub(crate) fn standard_case_roots(&mut self) -> std::result::Result<[Value; 4], Fault> {
        let down = self.standard_case_table()?;
        let Object::CharTable(t) = self.vm.object_if(down).unwrap() else { unreachable!() };
        let roots = [down, t.extras[0], t.extras[1], t.extras[2]];
        if roots[1..].iter().any(|v| !matches!(self.vm.object_if(*v), Some(Object::CharTable(_)))) {
            self.install_case_table(down, true)?;
            return Ok(self.vm.standard_case_tables);
        }
        Ok(roots)
    }

    // Snapshot all pre-existing buffers before the standard roots can change.
    fn initialize_case_roots(&mut self) -> std::result::Result<(), Fault> {
        self.standard_case_table()?;
        let roots = self.vm.standard_case_tables;
        for object in &mut self.vm.objects {
            let buffer_roots = match object {
                Object::Buffer(b) if b.state.live => &mut b.case_tables,
                Object::ByteBuffer(b) if b.state.live => &mut b.case_tables,
                _ => continue,
            };
            if buffer_roots[0] == Value::NIL {
                *buffer_roots = roots;
            }
        }
        Ok(())
    }

    pub fn current_case_mapping(&mut self, down: bool) -> super::super::Out {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let uninitialized = match &self.vm.objects[index] {
            Object::Buffer(b) => b.case_tables[0] == Value::NIL,
            Object::ByteBuffer(b) => b.case_tables[0] == Value::NIL,
            _ => unreachable!(),
        };
        if uninitialized { self.initialize_case_roots()?; }
        let roots = match &mut self.vm.objects[index] {
            Object::Buffer(b) => &mut b.case_tables,
            Object::ByteBuffer(b) => &mut b.case_tables,
            _ => unreachable!(),
        };
        Ok(roots[if down { 0 } else { 1 }])
    }

    /// The current buffer's case-canon table, the one completion folds with.
    pub(crate) fn completion_case_canon(&mut self) -> std::result::Result<Value, Fault> {
        self.current_case_mapping(true)?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        Ok(match &self.vm.objects[index] {
            Object::Buffer(b) => b.case_tables[2],
            Object::ByteBuffer(b) => b.case_tables[2],
            _ => unreachable!(),
        })
    }

    pub(crate) fn completion_case_ranges(&mut self) -> std::result::Result<Vec<(u32, u32, i64)>, Fault> {
        self.current_case_mapping(true)?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let canon = match &self.vm.objects[index] {
            Object::Buffer(b) => b.case_tables[2],
            Object::ByteBuffer(b) => b.case_tables[2],
            _ => unreachable!(),
        };
        let native = self.native_case_mapping(canon, 2);
        Ok(self.case_ranges(canon)?.into_iter().filter_map(|(start, end, value)| {
            if native && value == Value::NIL {
                (start < 128).then_some((start as u32, end.min(127) as u32, -1))
            } else {
                Some((start as u32, end as u32, value.integer_value().filter(|&n| n >= 0).unwrap_or(-1)))
            }
        }).collect())
    }

    pub(crate) fn native_case_mapping(&self, table: Value, kind: usize) -> bool {
        let Some(Object::CharTable(table)) = self.vm.object_if(table) else { return false };
        self.vm.native_case_roots[kind].upgrade().is_some_and(|root| std::sync::Arc::ptr_eq(&root, &table.root))
    }

    pub(crate) fn completion_upcase(&mut self, code: u32) -> std::result::Result<u32, Fault> {
        self.case_mapping_character(code, false)
    }

    pub(crate) fn case_mapping_character(&mut self, code: u32, down: bool) -> std::result::Result<u32, Fault> {
        let table = self.current_case_mapping(down)?;
        let kind = if down { 0 } else { 1 };
        if self.char_table_element(table, code as usize)? == Value::NIL && (code < 128 || !self.native_case_mapping(table, kind)) {
            return Ok(code);
        }
        Ok(self.case_convert(Value::integer(i64::from(code)).unwrap(), down)?.integer_value().unwrap() as u32)
    }

    fn case_set(
        &mut self,
        table: Value,
        from: usize,
        to: usize,
        value: Value,
    ) -> std::result::Result<(), Fault> {
        let Object::CharTable(t) = &mut self.vm.objects[table.index(0).map_err(Fault::Error)? - 1]
        else {
            unreachable!()
        };
        t.set_range(from, to, value).map_err(Fault::Error)
    }

    // Collect effective ranges including default/parent fallback, without a
    // four-million-character scan for sparse Unicode tables.
    fn case_ranges(&self, table: Value) -> std::result::Result<Vec<(usize, usize, Value)>, Fault> {
        fn boundaries(node: &CharTableNodeHandle, out: &mut Vec<usize>) {
            let node = node.lock().unwrap();
            let width = CharTableNode::WIDTHS[node.depth];
            for (i, slot) in node.slots.iter().enumerate() {
                out.push(node.min_char + i * width);
                if let CharTableSlot::Node(child) = slot {
                    boundaries(child, out);
                }
            }
        }
        let mut points = vec![0, CharTable::LEN];
        let mut at = table;
        let mut seen = HashSet::default();
        while let Some(Object::CharTable(t)) = self.vm.object_if(at) {
            if !seen.insert(at.0) {
                return Err(Fault::Error("cyclic char-table parent".into()));
            }
            CharTableNode::validate_traversal(&t.root, 0, 0).map_err(Fault::Error)?;
            boundaries(&t.root, &mut points);
            at = t.parent;
        }
        points.sort_unstable();
        points.dedup();
        let mut ranges: Vec<(usize, usize, Value)> = Vec::new();
        for pair in points.windows(2) {
            let value = self.char_table_element(table, pair[0])?;
            if let Some(last) = ranges.last_mut() {
                if last.2 == value {
                    last.1 = pair[1] - 1;
                    continue;
                }
            }
            ranges.push((pair[0], pair[1] - 1, value));
        }
        Ok(ranges)
    }

    fn case_inverse(&mut self, source: Value, target: Value) -> std::result::Result<(), Fault> {
        let ranges = self.case_ranges(source)?;
        for &(from, to, elt) in &ranges {
            if elt.integer_value().is_some_and(|n| n >= 0) {
                for c in from..=to {
                    self.case_set(target, c, c, Value::integer(c as i64).unwrap())?;
                }
            }
        }
        for (from, to, elt) in ranges {
            if let Some(n) = elt.integer_value().filter(|n| *n >= 0) {
                let n = usize::try_from(n).map_err(|_| Fault::Range(source, elt))?;
                for c in from..=to {
                    let old = self.char_table_element(target, n)?;
                    self.case_set(target, n, n, Value::integer(c as i64).unwrap())?;
                    self.case_set(target, c, c, old)?;
                }
            }
        }
        Ok(())
    }

    pub fn install_case_table(&mut self, table: Value, standard: bool) -> super::super::Out {
        if !self.case_table_p(table) {
            return Err(Fault::Wrong("case-table-p", table));
        }
        self.initialize_case_roots()?;
        let Object::CharTable(t) = self.vm.object_if(table).unwrap() else {
            unreachable!()
        };
        let mut roots = [table, t.extras[0], t.extras[1], t.extras[2]];
        let purpose = t.purpose;
        if roots[1] == Value::NIL {
            roots[1] = self.make_char_table(purpose, Value::NIL)?;
            self.case_inverse(table, roots[1])?;
            self.char_table_extra(table, Value::integer(0).unwrap(), Some(roots[1]))?;
        }
        if roots[2] == Value::NIL {
            roots[2] = self.make_char_table(purpose, Value::NIL)?;
            self.char_table_extra(table, Value::integer(1).unwrap(), Some(roots[2]))?;
            for (from, to, elt) in self.case_ranges(table)? {
                if let Some(n) = elt.integer_value().filter(|n| *n >= 0) {
                    let up = self.char_table_element(roots[1], n as usize)?;
                    let index = up
                        .integer_value()
                        .filter(|n| (0..CharTable::LEN as i64).contains(n))
                        .ok_or(Fault::Wrong("characterp", up))?;
                    let canonical = self.char_table_element(table, index as usize)?;
                    self.case_set(roots[2], from, to, canonical)?;
                }
            }
        }
        if roots[3] == Value::NIL {
            roots[3] = self.make_char_table(purpose, Value::NIL)?;
            self.case_inverse(roots[2], roots[3])?;
            self.char_table_extra(table, Value::integer(2).unwrap(), Some(roots[3]))?;
        }
        self.char_table_extra(roots[2], Value::integer(2).unwrap(), Some(roots[3]))?;
        if standard {
            self.vm.standard_case_tables = roots;
        } else {
            let index = self.vm.buffer_index().map_err(Fault::Error)?;
            match &mut self.vm.objects[index] {
                Object::Buffer(b) => b.case_tables = roots,
                Object::ByteBuffer(b) => b.case_tables = roots,
                _ => unreachable!(),
            }
        }
        Ok(table)
    }
}
