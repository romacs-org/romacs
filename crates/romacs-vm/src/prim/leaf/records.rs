//! Records found by tag and reshaped in place, for redefining a struct in a
//! live session (`cl-defstruct`). A record's Value indexes the object
//! table, so new slots stored into its entry reach every reference, and
//! `eq`, weak tables and every holder keep it.
use super::*;

/// A record's slots at most, its tag included: GNU's `PSEUDOVECTOR_SIZE_BITS`
/// bound, which `make-record` and `record` keep (`Vm::allocate_record`).
const MAX_SLOTS: usize = 4095;

/// Where a reshaped record's new slot comes from.
#[derive(Clone, Copy)]
pub(crate) enum Source {
    /// The old record's slot at this index.
    Old(usize),
    /// The fill value at this position of the new slots.
    Fill,
}

impl Leaf<'_> {
    /// The private records whose slot 0 is one of TAGS, in table order: one
    /// walk of the table whatever the number of tags. A record unreachable
    /// but not yet swept is among them, and holding it again keeps it:
    /// what it holds is still allocated, since a minor sweeps only the young
    /// and an old entry holds a young value only through a store, which
    /// makes it DIRTY and so read again (table.rs).
    pub(crate) fn records_tagged(&mut self, tags: &[Value]) -> Value {
        let mut keys: Vec<u64> = tags.iter().map(|t| t.0).collect();
        keys.sort_unstable();
        let found: Vec<Value> = self.vm.objects.iter().enumerate()
            .filter_map(|(i, object)| match object {
                data::Object::Record(slots) if slots.first().is_some_and(|t| keys.binary_search(&t.0).is_ok()) => {
                    Some(Value::reference(i + 1, 0))
                }
                _ => None,
            })
            .collect();
        self.vm.list_value(&found)
    }

    /// Give each of RECORDS the slots TAG, then one per element of MAP: the
    /// old slot it names, or the record's fill value there (FILLS, one row a
    /// record). Every record is checked and every new slot vector built
    /// before the first is stored, so a refusal leaves all of them as they
    /// were. The number of records reshaped.
    pub(crate) fn records_reshape(
        &mut self,
        records: &[Value],
        tag: Value,
        map: &[Source],
        fills: &[&[Value]],
    ) -> std::result::Result<usize, Fault> {
        let count = map.len() + 1;
        if count > MAX_SLOTS {
            return Err(Fault::Error(format!("Attempt to reshape a record to {count} slots; max is {MAX_SLOTS}")));
        }
        let mut staged = Vec::with_capacity(records.len());
        for (n, &record) in records.iter().enumerate() {
            let i = self.writable_object(record)?;
            let Some(data::Object::Record(old)) = self.vm.objects.get(i) else {
                return Err(Fault::Wrong("recordp", record));
            };
            let fill = fills[n];
            let mut slots = Vec::with_capacity(count);
            slots.push(tag);
            for (at, source) in map.iter().enumerate() {
                slots.push(match *source {
                    Source::Old(k) => *old.get(k).ok_or_else(|| Fault::Range(record, i64::try_from(k).ok().and_then(Value::fixnum).unwrap_or(Value::NIL)))?,
                    Source::Fill => fill[at],
                });
            }
            staged.push((i, slots));
        }
        let reshaped = staged.len();
        for (i, slots) in staged {
            let entry = self.vm.objects.entry_mut(i);
            let grown = slots.len().saturating_sub(match entry { data::Object::Record(old) => old.len(), _ => 0 });
            *entry = data::Object::Record(slots);
            self.vm.charge_payload(grown * std::mem::size_of::<Value>());
        }
        Ok(reshaped)
    }
}
