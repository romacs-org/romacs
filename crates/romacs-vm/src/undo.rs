//! undo.c: what text edits record in `buffer-undo-list' (`primitive-undo'
//! and the undo commands themselves are GNU's simple.el).
use super::*;
use data::Object;

impl Vm {
    fn current_undo_buffer(&mut self) -> Result<usize> {
        self.buffer_index()
    }

    /// buffer_visited_file_modtime: 0 when unknown, -1 when the visited
    /// file did not exist, else the time in the current-time-list form.
    fn visited_modtime_value(&mut self, buffer: usize) -> Result<Value> {
        let state = match &self.objects[buffer] {
            Object::Buffer(b) => b.per_buffer.1,
            Object::ByteBuffer(b) => b.per_buffer.1,
            _ => return Err("buffer required".into()),
        };
        Ok(match state {
            forwarded::VisitedModtime::Unknown => Value::integer(0)?,
            forwarded::VisitedModtime::Missing => Value::integer(-1)?,
            forwarded::VisitedModtime::Known { seconds, nanos, .. } => {
                let slot = self.slot("current-time-list");
                let list = self.variable_value(slot)?;
                if list == Value::NIL || list == Value::UNBOUND {
                    let ticks = i64::try_from(i128::from(seconds) * 1_000_000_000 + i128::from(nanos))
                        .map_err(|_| "VM2 undo timestamps requiring bignum ticks are unsupported")?;
                    let ticks = Value::integer(ticks)?;
                    let hz = Value::integer(1_000_000_000)?;
                    self.conses.push((ticks, hz));
                    Value::reference(self.conses.len() - 1, 3)
                } else {
                    let parts = [seconds.div_euclid(65536), seconds.rem_euclid(65536), nanos / 1000, nanos % 1000 * 1000];
                    let parts = parts.into_iter().map(Value::integer).collect::<Result<Vec<_>>>()?;
                    self.list_value(&parts)
                }
            }
        })
    }

    fn push_undo(&mut self, buffer: usize, entry: Value) {
        let list = self.undo_list_of(buffer);
        self.conses.push((entry, list));
        self.set_undo_list_of(buffer, Value::reference(self.conses.len() - 1, 3));
    }

    /// record_first_change: (t . VISITED-MODTIME), unless undo is off.
    pub(crate) fn record_first_change(&mut self) -> Result<()> {
        let buffer = self.current_undo_buffer()?;
        if self.undo_list_of(buffer) == Value::TRUE { return Ok(()); }
        let modtime = self.visited_modtime_value(buffer)?;
        self.conses.push((Value::TRUE, modtime));
        let entry = Value::reference(self.conses.len() - 1, 3);
        self.push_undo(buffer, entry);
        Ok(())
    }

    /// record_property_change: (nil PROP VALUE BEG . END) for characters
    /// BEG..END of the current buffer, after a first-change entry if the
    /// buffer is unmodified.
    pub(crate) fn record_property_change(&mut self, beg: i64, end: i64, prop: Value, value: Value) -> Result<()> {
        let buffer = self.current_undo_buffer()?;
        if self.undo_list_of(buffer) == Value::TRUE { return Ok(()); }
        if !self.buffer_state(self.slots[self.buffer_slot].value)?.modified() { self.record_first_change()?; }
        let (beg, end) = (Value::integer(beg)?, Value::integer(end)?);
        self.conses.push((beg, end));
        let mut entry = Value::reference(self.conses.len() - 1, 3);
        for item in [value, prop, Value::NIL] {
            self.conses.push((item, entry));
            entry = Value::reference(self.conses.len() - 1, 3);
        }
        self.push_undo(buffer, entry);
        Ok(())
    }

    /// Fundo_boundary: a nil boundary unless one is already on top.
    pub(crate) fn undo_boundary(&mut self) -> Result<Value> {
        let buffer = self.current_undo_buffer()?;
        let list = self.undo_list_of(buffer);
        if list == Value::TRUE { return Ok(Value::NIL); }
        if self.cons_pair(list).is_ok_and(|(car, _)| car != Value::NIL) {
            self.push_undo(buffer, Value::NIL);
        }
        let cause = self.slot("undo-auto--last-boundary-cause");
        let explicit = Value::reference(self.slot("explicit") + 1, 7);
        self.slots[cause].value = explicit;
        Ok(Value::NIL)
    }
}
