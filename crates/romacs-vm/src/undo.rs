//! undo.c: what text edits record in `buffer-undo-list` (`primitive-undo`
//! and the undo commands themselves are GNU's simple.el), and what a
//! collection does to the list.
//!
//! A live buffer's undo list is weak, as alloc.c's `mark_buffer` and
//! `compact_undo_list` have it: a mark passes over it, and once what everything
//! else holds is marked `Vm::undo_round` marks what stays. An entry
//! (MARKER . ADJUSTMENT) whose marker is unmarked does not stay, and nor does
//! what `truncate_undo_list` cuts. All of that is decided in the mark and written
//! after the last point a key press can end the collection (`UndoEdit`), so
//! that an abandoned mark leaves the lists as they were: GNU truncates first.
use super::*;
use data::Object;
use crate::coordinates::{ByteOffset, CharOffset};

/// undo.c's sizes, those of GNU on a 64-bit build, as the limits are in them.
const CONS_BYTES: i64 = 16;
const STRING_BYTES: i64 = 32;

/// The variables `truncate_undo_list` reads.
const UNDO_LIMITS: [&str; 4] = ["undo-limit", "undo-strong-limit", "undo-outer-limit", "undo-outer-limit-function"];

/// What `truncate_undo_list` reads from the buffer it makes current.
#[derive(Clone, Copy)]
pub(crate) struct UndoLimits {
    limit: i64,
    strong: i64,
    /// `undo-outer-limit` when it can be passed: not when it is no integer or
    /// a positive bignum, and always when a negative one (`Fnatnump`).
    outer: Option<i64>,
    /// `undo-outer-limit-function` is non-nil.
    offers: bool,
}

/// A collection's reading of those limits, taken once before it marks: what
/// every buffer sees, and what a buffer with a local value of one of them does.
/// Most collections find no local value, and so look at no table but to see
/// that it is not there.
pub(crate) struct UndoView {
    default: UndoLimits,
    local: HashMap<usize, UndoLimits>,
}

impl UndoView {
    fn of(&self, buffer: usize) -> UndoLimits {
        self.local.get(&buffer).copied().unwrap_or(self.default)
    }
}

/// `truncate_undo_list`'s decision on one list.
enum UndoCut {
    /// The whole list fits, or is not this collection's to cut.
    Keep,
    /// All after this cons goes.
    After(usize),
    /// Nothing is kept.
    Clear,
    /// The first record is past `undo-outer-limit`: the list is left to
    /// `undo-outer-limit-function`.
    Offer,
}

impl UndoCut {
    /// Cut at BOUNDARY, `truncate_undo_list`'s `last_boundary`.
    fn at(boundary: Option<usize>) -> Self {
        boundary.map_or(Self::Clear, Self::After)
    }
}

/// A change to an undo list a mark decided on, for after its last key poll.
#[derive(Debug, PartialEq)]
pub(crate) enum UndoEdit {
    /// This cons's cdr becomes the value.
    Cdr(usize, Value),
    /// This buffer's list becomes the value.
    List(usize, Value),
    /// `BUF_COMPACT` catches up with MODIFF.
    Compacted(usize),
    /// `undo-outer-limit-function` is to be asked: `Vm::take_undo_offer`
    /// finds which buffers. Until one answers, its `BUF_COMPACT` stays behind,
    /// so that a quit from the function leaves it to the next collection, as
    /// one out of GNU's `compact_buffer` does.
    Offer,
}

/// `truncate_undo_list`'s walk down a list: what it has passed, and its size.
struct UndoScan<'a, 'k> {
    vm: &'a Vm,
    size: i64,
    prev: Option<usize>,
    next: Value,
    /// How many more conses: a circular list comes to an end.
    fuel: usize,
    /// A mark's look for a key press, at every `UNDO_KEY_POLL` conses this
    /// scan has passed, so that a short list costs no look; and whether it
    /// found one, which ends the scan with nothing decided.
    key: Option<&'k mut dyn FnMut() -> bool>,
    passed: usize,
    stopped: bool,
}

impl<'a, 'k> UndoScan<'a, 'k> {
    fn new(vm: &'a Vm, list: Value, key: Option<&'k mut dyn FnMut() -> bool>) -> Self {
        Self { vm, size: 0, prev: None, next: list, fuel: vm.conses.len(), key, passed: 0, stopped: false }
    }
    /// The entry at NEXT, while there is a cons there to look at.
    fn entry(&self) -> Option<Value> {
        self.vm.plain_cons(self.next).filter(|_| self.fuel > 0).map(|(_, car, _)| car)
    }
    /// Count the entry at NEXT and move past it.
    fn pass(&mut self) {
        let (i, car, cdr) = self.vm.plain_cons(self.next).expect("a cons, as `entry` found");
        self.size += self.vm.undo_entry_bytes(car);
        self.prev = Some(i);
        self.next = cdr;
        self.fuel -= 1;
        self.passed += 1;
        if self.passed.is_multiple_of(UNDO_KEY_POLL) && self.key.as_mut().is_some_and(|key| key()) {
            (self.stopped, self.fuel) = (true, 0);
        }
    }
    /// Past a leading boundary and the first record.
    fn first_record(mut self) -> Self {
        if self.entry() == Some(Value::NIL) { self.pass(); }
        while self.entry().is_some_and(|entry| entry != Value::NIL) { self.pass(); }
        self
    }
    /// The list ended in something other than nil. An image cons is an end,
    /// not this: a collection reads no image page.
    fn dotted(&self) -> bool {
        self.next != Value::NIL && self.next.tag() != 3
    }
}

/// What a mark's undo rounds have decided, written only once the mark can no
/// longer be abandoned: the conses cut after, kept across rounds so that a
/// buffer reached later, sharing a tail with one cut earlier, keeps the cut;
/// and the edits.
#[derive(Default)]
pub(super) struct UndoDecided {
    pub(super) cuts: HashSet<usize>,
    pub(super) edits: Vec<UndoEdit>,
}

/// A key press ended a scan: the mark is given up, and nothing was decided.
struct KeyPressed;

/// Conses of an undo list a walk passes between its looks for a key press.
const UNDO_KEY_POLL: usize = 4096;

impl Vm {
    /// V's index and halves if it is a cons of the table: what an undo list's
    /// walk reads, a collection's among them, which reads no image page.
    fn plain_cons(&self, v: Value) -> Option<(usize, Value, Value)> {
        if Self::image_value(v) { return None; }
        let i = v.index_if(3)?;
        let (car, cdr) = self.conses[i];
        Some((i, car, cdr))
    }

    /// What ENTRY and its link count for, as undo.c adds them up.
    fn undo_entry_bytes(&self, entry: Value) -> i64 {
        let Some((_, car, _)) = self.plain_cons(entry) else {
            return CONS_BYTES + if entry.tag() == 3 { CONS_BYTES } else { 0 };
        };
        let text = if Self::image_value(car) { None } else { self.str_view(car) };
        2 * CONS_BYTES + text.map_or(0, |text| STRING_BYTES - 1 + i64::try_from(text.chars).unwrap_or(0))
    }

    /// `truncate_undo_list`'s decision on LIST under LIMITS; none when KEY, a
    /// mark's look for a key press, ended the scan.
    fn undo_cut(&self, list: Value, limits: UndoLimits, key: Option<&mut dyn FnMut() -> bool>) -> Option<UndoCut> {
        let mut scan = UndoScan::new(self, list, key).first_record();
        if scan.stopped { return None; }
        if limits.offers && limits.outer.is_some_and(|outer| outer < scan.size) {
            return Some(UndoCut::Offer);
        }
        let mut boundary = scan.entry().and(scan.prev);
        while let Some(entry) = scan.entry() {
            // Past the higher limit the cut comes before this record; past
            // the lower one, after it.
            if entry == Value::NIL {
                if scan.size > limits.strong { return Some(UndoCut::at(boundary)); }
                boundary = scan.prev;
                if scan.size > limits.limit { return Some(UndoCut::at(boundary)); }
            }
            scan.pass();
        }
        if scan.stopped { return None; }
        Some(if scan.dotted() { UndoCut::at(boundary) } else { UndoCut::Keep })
    }

    /// Whether ENTRY is (MARKER . ADJUSTMENT) with a marker the mark has not
    /// reached (`compact_undo_list`).
    fn undo_entry_dead(&self, entry: Value) -> bool {
        let Some((_, marker, _)) = self.plain_cons(entry) else { return false };
        let Some(i) = marker.index_if(0).filter(|_| !Self::image_value(marker)).and_then(|i| i.checked_sub(1)) else { return false };
        matches!(self.objects[i], Object::Marker(_)) && !self.objects.marked(i)
    }

    /// `compact_buffer`'s truncation and `compact_undo_list`, as decisions: the
    /// mark has drained, and OWNERS are the buffers it reached that own an undo
    /// list of their own (`owns_undo_list`). What stays goes to PENDING and the
    /// changes to EDITS. Marks of objects do not change here, so what one
    /// buffer's walk finds does not depend on another's, and the threads' mark
    /// and a verifying one agree.
    /// False when KEY, asked between buffers and every few thousand conses
    /// of a walk, says a key press ends the mark: nothing was written, and
    /// what was decided is dropped with the mark.
    pub(super) fn undo_round(&self, mut owners: Vec<usize>, view: &UndoView, marking: &gc_mark::Marking, pending: &mut Vec<Value>, decided: &mut UndoDecided, key: &mut dyn FnMut() -> bool) -> bool {
        owners.sort_unstable();
        let mut lists = Vec::new();
        for buffer in owners {
            if key() { return false; }
            let Ok(list) = self.undo_decide(buffer, view, decided, key) else { return false };
            lists.extend(list);
        }
        for (buffer, list) in lists {
            if key() || !self.undo_walk(buffer, list, marking, pending, decided, key) { return false; }
        }
        true
    }

    /// `compact_buffer` on BUFFER: its cut, if it changed since the last, into
    /// DECIDED; its list if any of it stays. An error, with nothing decided
    /// for this buffer, when KEY ended its scan.
    fn undo_decide(&self, buffer: usize, view: &UndoView, decided: &mut UndoDecided, key: &mut dyn FnMut() -> bool) -> std::result::Result<Option<(usize, Value)>, KeyPressed> {
        let UndoDecided { cuts, edits } = decided;
        let list = self.own_undo_list(buffer);
        let Ok(state) = self.buffer_state(Value::reference(buffer + 1, 0)) else { return Ok(None) };
        let changed = state.compact != state.modiff;
        if list == Value::TRUE {
            if changed { edits.push(UndoEdit::Compacted(buffer)); }
            return Ok(None);
        }
        if !changed { return Ok(Some((buffer, list))); }
        let cut = self.undo_cut(list, view.of(buffer), Some(key)).ok_or(KeyPressed)?;
        edits.push(if matches!(cut, UndoCut::Offer) { UndoEdit::Offer } else { UndoEdit::Compacted(buffer) });
        match cut {
            UndoCut::Keep | UndoCut::Offer => {}
            UndoCut::After(i) => {
                cuts.insert(i);
                edits.push(UndoEdit::Cdr(i, Value::NIL));
            }
            UndoCut::Clear => {
                edits.push(UndoEdit::List(buffer, Value::NIL));
                return Ok(None);
            }
        }
        Ok(Some((buffer, list)))
    }

    /// `compact_undo_list` on BUFFER's LIST, as far as CUTS, and the marking that
    /// follows it: each link that stays is marked and its entry is for PENDING;
    /// the others are unlinked in EDITS, unmarked. At a cons already marked the
    /// rest of the list is too. What it ends in is traced as it is, and so is
    /// what is left of a circle of dead entries.
    fn undo_walk(&self, buffer: usize, list: Value, marking: &gc_mark::Marking, pending: &mut Vec<Value>, decided: &mut UndoDecided, key: &mut dyn FnMut() -> bool) -> bool {
        let UndoDecided { cuts, edits } = decided;
        let (mut tail, mut kept) = (list, None);
        // The dead entries passed since the last kept one: meeting one of
        // them again is a circle of dead entries, and the walk is over.
        let mut dropped = HashSet::default();
        for step in 0..self.conses.len() {
            if step % UNDO_KEY_POLL == UNDO_KEY_POLL - 1 && key() { return false; }
            let Some((i, car, cdr)) = self.plain_cons(tail) else { break };
            let next = if cuts.contains(&i) { Value::NIL } else { cdr };
            if self.undo_entry_dead(car) {
                if !dropped.insert(i) { break; }
                edits.push(kept.map_or(UndoEdit::List(buffer, next), |cons| UndoEdit::Cdr(cons, next)));
            } else {
                if !marking.conses.claim(i) { return true; }
                if car != Value::NIL { pending.push(car); }
                kept = Some(i);
                dropped.clear();
            }
            tail = next;
        }
        pending.push(tail);
        true
    }

    /// The changes a mark decided on, now that it is sure to collect.
    pub(super) fn apply_undo_edits(&mut self, edits: Vec<UndoEdit>) {
        for edit in edits {
            match edit {
                UndoEdit::Cdr(i, next) => self.conses.set_cdr(i, next),
                UndoEdit::List(buffer, list) => self.set_undo_list_of(buffer, list),
                UndoEdit::Compacted(buffer) => self.note_undo_compacted(Value::reference(buffer + 1, 0)),
                UndoEdit::Offer => self.undo_offers_due = true,
            }
        }
    }

    /// `BUF_COMPACT` (BUFFER) = `BUF_MODIFF` (BUFFER).
    fn note_undo_compacted(&mut self, buffer: Value) {
        let state = self.buffer_state_mut(buffer).expect("a buffer");
        state.compact = state.modiff;
    }

    /// The limits as every buffer sees them, and as those with a local value
    /// of one do, read from where they are kept: a mark can run no Lisp.
    pub(super) fn undo_view(&mut self) -> UndoView {
        let object = |vm: &mut Vm, name: &str| {
            let fwd = vm.forwarded.lookup(name).expect("declared");
            vm.forwarded_value(fwd).expect("a global object")
        };
        let (outer, function) = (object(self, "undo-outer-limit"), object(self, "undo-outer-limit-function"));
        let default = UndoLimits {
            limit: self.forwarded.integer("undo-limit"),
            strong: self.forwarded.integer("undo-strong-limit"),
            outer: self.undo_outer_limit(outer),
            offers: function != Value::NIL,
        };
        let mut local = HashMap::default();
        for name in UNDO_LIMITS {
            let table = self.names.get(name).map_or(Value::NIL, |&slot| self.slots[slot].local);
            let Some(Object::HashTable(table)) = self.object_if(table) else { continue };
            for (_, buffer, binding) in table.entries() {
                let (Ok(buffer), Ok(binding)) = (buffer.index(0), binding.index(7)) else { continue };
                let limits = local.entry(buffer - 1).or_insert(default);
                self.set_undo_limit(limits, name, self.slots[binding - 1].value);
            }
        }
        UndoView { default, local }
    }

    fn undo_outer_limit(&self, value: Value) -> Option<i64> {
        match self.object_if(value) {
            Some(Object::Bignum(n)) => (n.sign() == num_bigint::Sign::Minus).then_some(i64::MIN),
            _ => value.integer_value(),
        }
    }

    /// LIMITS with VALUE for variable NAME; a locally void one stays as it is.
    fn set_undo_limit(&self, limits: &mut UndoLimits, name: &str, value: Value) {
        if value == Value::UNBOUND { return; }
        match name {
            "undo-limit" => limits.limit = value.integer_value().unwrap_or(limits.limit),
            "undo-strong-limit" => limits.strong = value.integer_value().unwrap_or(limits.strong),
            "undo-outer-limit" => limits.outer = self.undo_outer_limit(value),
            _ => limits.offers = value != Value::NIL,
        }
    }

    /// The limits BUFFER sees, outside a collection.
    fn undo_limits_in(&mut self, buffer: Value) -> Result<UndoLimits> {
        let mut limits = UndoLimits { limit: 0, strong: 0, outer: None, offers: false };
        for name in UNDO_LIMITS {
            let slot = self.slot(name);
            let value = self.value_in_buffer(slot, buffer)?;
            self.set_undo_limit(&mut limits, name, value);
        }
        Ok(limits)
    }

    /// What `undo-outer-limit-function` is to be asked next: (BUFFER . SIZE),
    /// the first live buffer not in ASKED, the list of those a run has asked
    /// about already, that changed since it was last compacted and whose
    /// first record is past the limit, as things stand now. Nil when there
    /// is none, and at once, with ASKED nil, when no collection has noted any
    /// since the last run.
    pub(crate) fn take_undo_offer(&mut self, asked: Value) -> Result<Value> {
        // The first of a run takes the collection's note with it, so that a
        // quit leaves the rest to the next collection; the run's later ones,
        // which have asked about some, look whatever the note says.
        if !std::mem::take(&mut self.undo_offers_due) && asked == Value::NIL { return Ok(Value::NIL); }
        // As many as there are conses: a circular list comes to an end.
        let mut excluded = HashSet::default();
        let mut rest = asked;
        for _ in 0..self.conses.len() {
            let Some((_, buffer, next)) = self.plain_cons(rest) else { break };
            excluded.insert(buffer.0);
            rest = next;
        }
        // Looked for afresh each time: what the function did about one buffer,
        // or what happened while it asked, may have changed another's list.
        // One asked about already is not asked again in this run, whatever
        // has happened to its list since.
        for buffer in self.buffer_order() {
            if excluded.contains(&buffer.0) { continue; }
            let Some(size) = self.undo_offer_size(buffer)? else { continue };
            return Ok(self.alloc_cons(buffer, Value::integer(size)?));
        }
        Ok(Value::NIL)
    }

    /// The first record's size if the function is to be asked about BUFFER.
    fn undo_offer_size(&mut self, buffer: Value) -> Result<Option<i64>> {
        let index = buffer.index(0)? - 1;
        let state = self.buffer_state(buffer)?;
        let indirect = matches!(&self.objects[index], Object::Buffer(b) if !b.owns_undo_list());
        let list = self.own_undo_list(index);
        if state.compact == state.modiff || indirect || list == Value::TRUE { return Ok(None); }
        let limits = self.undo_limits_in(buffer)?;
        let size = UndoScan::new(self, list, None).first_record().size;
        Ok((limits.offers && limits.outer.is_some_and(|outer| outer < size)).then_some(size))
    }

    /// The rest of `truncate_undo_list` and `compact_buffer` once the function
    /// has answered about BUFFER: nil makes the cut, under the limits it sees.
    pub(crate) fn answer_undo_offer(&mut self, buffer: Value, answer: Value) -> Result<()> {
        self.buffer_state(buffer)?;
        let index = buffer.index(0)? - 1;
        let list = self.undo_list_of(index);
        if answer == Value::NIL && list != Value::TRUE {
            let limits = UndoLimits { offers: false, ..self.undo_limits_in(buffer)? };
            match self.undo_cut(list, limits, None) {
                Some(UndoCut::After(i)) => self.conses.set_cdr(i, Value::NIL),
                Some(UndoCut::Clear) => self.set_undo_list_of(index, Value::NIL),
                Some(UndoCut::Keep | UndoCut::Offer) | None => {}
            }
        }
        self.note_undo_compacted(buffer);
        Ok(())
    }

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
                    self.alloc_cons(ticks, hz)
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
        let cons = self.alloc_cons(entry, list);
        self.set_undo_list_of(buffer, cons);
    }

    /// record_first_change: (t . VISITED-MODTIME), unless undo is off.
    pub(crate) fn record_first_change(&mut self) -> Result<()> {
        let buffer = self.current_undo_buffer()?;
        self.record_first_change_in(buffer)
    }
    fn record_first_change_in(&mut self, buffer: usize) -> Result<()> {
        if self.undo_list_of(buffer) == Value::TRUE { return Ok(()); }
        let modtime = self.visited_modtime_value(buffer)?;
        let entry = self.alloc_cons(Value::TRUE, modtime);
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
            self.conses.set_cdr(elt.index(3)?, Value::integer(beg + length)?);
            return Ok(());
        }
        let entry = self.alloc_cons(Value::integer(beg)?, Value::integer(beg + length)?);
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
                    let entry = self.alloc_cons(marker, Value::integer(adjustment)?);
                    self.push_undo(buffer, entry);
                }
            }
        }
        let entry = self.alloc_cons(text, Value::integer(position)?);
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
        let mut entry = self.alloc_cons(beg, end);
        for item in [value, prop, Value::NIL] {
            entry = self.alloc_cons(item, entry);
        }
        self.push_undo(buffer, entry);
        Ok(())
    }

    /// record_change: the character at BEG of buffer BUFFER is about to be
    /// replaced by one other, recorded as its deletion and an insertion.
    pub(crate) fn record_change(&mut self, buffer: usize, beg: i64) -> Result<()> {
        self.record_change_span(buffer, beg, 1)
    }

    /// record_change of LENGTH characters from BEG: their text, with its
    /// properties, recorded as deleted and as many inserted.
    pub(crate) fn record_change_span(&mut self, buffer: usize, beg: i64, length: i64) -> Result<()> {
        if self.undo_list_of(buffer) == Value::TRUE || length == 0 { return Ok(()); }
        let (from, to) = ((beg - 1) as usize, (beg - 1 + length) as usize);
        let object = match &self.objects[buffer] {
            Object::Buffer(b) => {
                let (a, z) = (b.byte_of_char(CharOffset(from))?, b.byte_of_char(CharOffset(to))?);
                data::string_object(b.lisp_string(a..z)?)
            }
            Object::ByteBuffer(b) => b.text_object(ByteOffset(from)..ByteOffset(to)),
            _ => return Err("buffer required".into()),
        };
        let text = self.allocate_object(object);
        self.record_delete(buffer, beg, text, false)?;
        self.record_insert(buffer, beg, length, false)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A truncation scan looks for a key press as it goes, every
    /// `UNDO_KEY_POLL` conses, and one that arrives ends it with nothing
    /// decided: a first record of any length does not hold a key up.
    #[test]
    fn a_key_press_ends_the_scan_of_one_long_record() {
        let mut vm = Vm::new();
        let mut list = Value::NIL;
        for _ in 0..3 * UNDO_KEY_POLL {
            let entry = vm.alloc_cons(Value::integer(1).unwrap(), Value::integer(2).unwrap());
            list = vm.alloc_cons(entry, list);
        }
        let limits = UndoLimits { limit: i64::MAX, strong: i64::MAX, outer: None, offers: false };
        let mut looks = 0;
        assert!(matches!(vm.undo_cut(list, limits, Some(&mut || { looks += 1; false })), Some(UndoCut::Keep)));
        assert_eq!(looks, 3, "one look for each poll's length of the record");
        let short = vm.alloc_cons(Value::NIL, Value::NIL);
        let mut looks = 0;
        assert!(vm.undo_cut(short, limits, Some(&mut || { looks += 1; false })).is_some());
        assert_eq!(looks, 0, "and none for a list shorter than that, whatever the heap's size");
        let mut looks = 0;
        assert!(vm.undo_cut(list, limits, Some(&mut || { looks += 1; looks == 2 })).is_none(), "ended by the second look");
        assert_eq!(looks, 2);
        assert!(matches!(vm.undo_cut(list, limits, None), Some(UndoCut::Keep)), "and with no key to look for it runs to its end");
    }
}
