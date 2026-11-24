//! Frozen regions from a leaf (`crate::frozen`): a freeze or thaw never
//! collects, so the Values it holds are valid throughout.
use super::*;

impl Leaf<'_> {
    pub(crate) fn freeze(&mut self, object: Value) -> std::result::Result<Value, Fault> {
        self.vm.freeze(object).map_err(|e| self.vm.freeze_fault(e))
    }
    pub(crate) fn thaw(&mut self, object: Value) -> std::result::Result<Value, Fault> {
        self.vm.thaw(object).map_err(|e| self.vm.freeze_fault(e))
    }
    pub(crate) fn seed_build(&mut self, roots: Value, table: Value, limit: Value) -> std::result::Result<Value, Fault> {
        let limit = limit.as_integer().ok().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| Fault::Wrong("natnump", limit))?;
        self.vm.seed_build(roots, table, limit).map_err(Fault::Error)
    }
    pub(crate) fn frozen_p(&self, object: Value) -> bool {
        Vm::frozen(object)
    }
    pub(crate) fn frozen_regions(&mut self) -> Value {
        let rows: Vec<[i64; 4]> = self.vm.regions.iter().enumerate()
            .filter_map(|(id, r)| r.as_ref().map(|r| [id as i64, r.conses.len() as i64, r.objects.len() as i64, r.bytes as i64]))
            .collect();
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let values: Vec<Value> = row.iter().map(|&n| Value::integer(n).expect("count fits")).collect();
            items.push(self.vm.list_value(&values));
        }
        self.vm.list_value(&items)
    }
}
