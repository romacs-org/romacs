//! Raw reader constructors. No Lisp evaluation, collection, or normalization.
use super::*;

impl Vm {
    pub(crate) fn literal_table_slot(&self, value: Value) -> CharTableSlot {
        match self.object_if(value) {
            Some(data::Object::SubCharTable(node)) => CharTableSlot::Node(Arc::clone(node)),
            _ => CharTableSlot::Value(value),
        }
    }

    /// Construct #^[DEFAULT PARENT PURPOSE ASCII CONTENTS(64) EXTRAS...].
    /// The caller roots the field values; this allocation window never collects.
    /// Raw ASCII and child identities are retained, not inferred from contents.
    pub(crate) fn char_table_from_literal(&mut self, fields: &[Value]) -> Result<Value> {
        if fields.len() < 68 { return Err("Invalid size char-table".into()); }
        let root = Arc::new(Mutex::new(CharTableNode {
            depth: 0, min_char: 0,
            slots: fields[4..68].iter().map(|&v| self.literal_table_slot(v)).collect(),
        }));
        let table = CharTable {
            default: fields[0], parent: fields[1], purpose: fields[2],
            ascii: self.literal_table_slot(fields[3]), root, extras: fields[68..].to_vec(),
            edits: 0, serial: super::fresh_serial(),
        };
        Ok(self.allocate_object(data::Object::CharTable(Box::new(table))))
    }

    /// Construct standalone #^^[DEPTH MIN CONTENTS...], preserving every field.
    /// Validation order and messages mirror GNU lread.c. Minimum need not align.
    pub(crate) fn sub_char_table_from_literal(&mut self, fields: &[Value]) -> Result<Value> {
        if fields.len() < 2 { return Err("Invalid size of sub-char-table".into()); }
        let depth = fields[0].integer_value().filter(|n| (1..=3).contains(n))
            .ok_or("Invalid depth in sub-char-table")? as usize;
        if fields.len() - 2 != CharTableNode::SIZES[depth] {
            return Err("Invalid size in sub-char-table".into());
        }
        let min_char = fields[1].integer_value().filter(|n| (0..CharTable::LEN as i64).contains(n))
            .ok_or("Invalid minimum character in sub-char-table")? as usize;
        let node = Arc::new(Mutex::new(CharTableNode {
            depth, min_char,
            slots: fields[2..].iter().map(|&v| self.literal_table_slot(v)).collect(),
        }));
        Ok(self.allocate_object(data::Object::SubCharTable(node)))
    }
}
