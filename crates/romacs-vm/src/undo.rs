//! undo.c: what text edits record in `buffer-undo-list' (`primitive-undo'
//! and the undo commands themselves are GNU's simple.el).
use super::*;
use data::Object;
use crate::coordinates::CharOffset;

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
        self.record_first_change_in(buffer)
    }
    fn record_first_change_in(&mut self, buffer: usize) -> Result<()> {
        if self.undo_list_of(buffer) == Value::TRUE { return Ok(()); }
        let modtime = self.visited_modtime_value(buffer)?;
        self.conses.push((Value::TRUE, modtime));
        let entry = Value::reference(self.conses.len() - 1, 3);
        self.push_undo(buffer, entry);
        Ok(())
    }

    /// record_point for a change at BEG of buffer BUFFER, UNMODIFIED before
    /// it: the first-change entry, then, right after a boundary, point as
    /// it was before the last command or undo-boundary when the change does
    /// not put it back there itself.
    fn record_point(&mut self, buffer: usize, beg: i64, unmodified: bool) -> Result<()> {
        let list = self.undo_list_of(buffer);
        let at_boundary = self.cons_pair(list).map_or(true, |(car, _)| car == Value::NIL);
        let (point, owner) = self.command_point;
        let records_point = at_boundary && point != beg && owner == Value::reference(buffer + 1, 0);
        // undo-inhibit-record-point stops both records; read it only when
        // one would be made.
        if !unmodified && !records_point { return Ok(()); }
        let inhibit = self.change_slots().inhibit_record_point;
        if !matches!(self.variable_value(inhibit)?, Value::NIL | Value::UNBOUND) { return Ok(()); }
        if unmodified { self.record_first_change_in(buffer)?; }
        if records_point {
            let point = Value::integer(point)?;
            self.push_undo(buffer, point);
        }
        Ok(())
    }

    /// record_insert: LENGTH characters inserted at BEG, extending a
    /// directly preceding (BEG0 . BEG) insertion entry.
    pub(crate) fn record_insert(&mut self, buffer: usize, beg: i64, length: i64, unmodified: bool) -> Result<()> {
        if self.undo_list_of(buffer) == Value::TRUE { return Ok(()); }
        self.record_point(buffer, beg, unmodified)?;
        let list = self.undo_list_of(buffer);
        if let Ok((elt, _)) = self.cons_pair(list)
            && !Self::image_value(elt)
            && let Ok((start, end)) = self.cons_pair(elt)
            && start.integer_value().is_some()
            && end.integer_value() == Some(beg)
        {
            self.conses[elt.index(3)?].1 = Value::integer(beg + length)?;
            return Ok(());
        }
        self.conses.push((Value::integer(beg)?, Value::integer(beg + length)?));
        let entry = Value::reference(self.conses.len() - 1, 3);
        self.push_undo(buffer, entry);
        Ok(())
    }

    /// record_delete: TEXT deleted from BEG, (TEXT . BEG), or (TEXT . -BEG)
    /// when point was at its end; with RECORD_MARKERS, first each marker in
    /// the text that reinserting it would not put back, (MARKER . ADJUSTMENT).
    pub(crate) fn record_delete(&mut self, buffer: usize, beg: i64, text: Value, record_markers: bool) -> Result<()> {
        if self.undo_list_of(buffer) == Value::TRUE { return Ok(()); }
        let unmodified = !self.buffer_state(Value::reference(buffer + 1, 0))?.modified();
        let length = self.str_view(text).ok_or("deleted text required")?.len() as i64;
        let point = match &self.objects[buffer] {
            Object::Buffer(b) => b.char_of_byte(b.point)?.0 as i64 + 1,
            Object::ByteBuffer(b) => b.point.0 as i64 + 1,
            _ => return Err("buffer required".into()),
        };
        self.record_point(buffer, beg, unmodified)?;
        let position = if point == beg + length { -beg } else { beg };
        if record_markers {
            for (marker, at, advances) in self.markers_between(buffer, beg, beg + length)? {
                let adjustment = if advances { beg + length } else { beg } - at;
                if adjustment != 0 {
                    self.conses.push((marker, Value::integer(adjustment)?));
                    let entry = Value::reference(self.conses.len() - 1, 3);
                    self.push_undo(buffer, entry);
                }
            }
        }
        self.conses.push((text, Value::integer(position)?));
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

    /// record_change: the character at BEG of buffer BUFFER is about to be
    /// replaced by one other, recorded as its deletion and an insertion.
    pub(crate) fn record_change(&mut self, buffer: usize, beg: i64) -> Result<()> {
        if self.undo_list_of(buffer) == Value::TRUE { return Ok(()); }
        let object = match &self.objects[buffer] {
            Object::Buffer(b) => {
                let (a, z) = (b.byte_of_char(CharOffset((beg - 1) as usize))?, b.byte_of_char(CharOffset(beg as usize))?);
                let mut s = data::LispString::new(b.slice(a..z)?);
                s.properties = b.properties.slice(a..z);
                Object::String(Box::new(s))
            }
            Object::ByteBuffer(b) => Object::Bytes(vec![b.bytes[(beg - 1) as usize]]),
            _ => return Err("buffer required".into()),
        };
        let text = self.allocate_object(object);
        self.record_delete(buffer, beg, text, false)?;
        self.record_insert(buffer, beg, 1, false)
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
        self.note_command_point()?;
        Ok(Value::NIL)
    }

    /// point_before_last_command_or_undo and its buffer, as the command
    /// loop and undo-boundary record them for record_point.
    pub(crate) fn note_command_point(&mut self) -> Result<()> {
        let buffer = self.current_undo_buffer()?;
        let point = match &self.objects[buffer] {
            Object::Buffer(b) => b.char_of_byte(b.point)?.0 as i64 + 1,
            Object::ByteBuffer(b) => b.point.0 as i64 + 1,
            _ => return Err("buffer required".into()),
        };
        self.command_point = (point, Value::reference(buffer + 1, 0));
        Ok(())
    }
}
