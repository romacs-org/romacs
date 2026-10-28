//! Declaration metadata, following GNU byte-run.el's executable bodies.
use super::*;

impl Vm {
    pub(super) fn metadata_primitive(&mut self, p: Primitive, start: usize, n: usize) -> Result<Value> {
        use Primitive::*;
        if !match p {
            MakeObsolete | SetAdvertisedCallingConvention => n == 3,
            MakeObsoleteVariable => (3..=4).contains(&n),
            _ => false,
        } {
            return Err("wrong primitive arity".into());
        }
        let args = self.stack[start..start + n].to_vec();
        if p == SetAdvertisedCallingConvention {
            // Resolve exactly as indirect-function: non-symbols are themselves,
            // unbound symbols yield nil, aliases follow function cells.
            let mut function = args[0];
            let mut seen = HashSet::default();
            while function.tag() == 7 && function != Value::UNBOUND {
                let slot = function.index(7)? - 1;
                if !seen.insert(slot) { return Err("cyclic function indirection".into()); }
                function = self.slots[slot].function;
                if function == Value::UNBOUND { function = Value::NIL; }
            }
            let slot = self.slot("advertised-signature-table");
            if self.slots[slot].value == Value::UNBOUND {
                let table = hash::HashTable::new(hash::Test::Eq, hash::Weakness::Key, 65);
                let table = self.allocate_object(data::Object::HashTable(Box::new(table)));
                self.slots[slot].value = table;
                self.specials.insert("advertised-signature-table".into());
            }
            let table = self.slots[slot].value;
            // Hash kernels do not collect; stack arguments are roots at the
            // next wait point, and the table lives in the ordinary traced slot.
            self.declaration_hash_put(table, function, args[1])?;
            return Ok(args[1]);
        }
        if args[0] == Value::NIL || args[0] == Value::TRUE {
            return Err("cannot make constant obsolete".into());
        }
        if args[0].tag() != 7 || args[0] == Value::UNBOUND {
            return Err("symbol required".into());
        }
        let property = self.intern(if p == MakeObsolete { "byte-obsolete-info" } else { "byte-obsolete-variable" });
        let access = if p == MakeObsoleteVariable && n == 4 { args[3] } else { Value::NIL };
        let metadata = self.list_value(&[args[1], access, args[2]]);
        self.put_symbol_property(args[0], property, metadata)?;
        Ok(args[0])
    }
}
