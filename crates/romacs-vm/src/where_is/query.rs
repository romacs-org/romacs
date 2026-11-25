//! `Fwhere_is_internal` itself: which of the sequences bound to a
//! definition run it, in which order, and which is its first key.
use super::{candidates, elements, equal, integer, stringp, symbolp, variable, Faulty, Stop, Walk, META, NON_META_MODIFIERS};
use crate::keymap::Walked;
use crate::prim::{Fault, Kind, Leaf};
use crate::{HashMap, Value};

/// `where-is-preferred-modifier` as modifier bits (`parse_solitary_modifier`).
fn preferred_modifier(cx: &mut Leaf<'_>) -> Faulty<i64> {
    let modifier = variable(cx, "where-is-preferred-modifier")?;
    if modifier == Value::NIL || cx.kind(modifier) != Kind::Symbol { return Ok(0); }
    Ok(match cx.symbol_name(modifier)?.as_str() {
        "A" | "alt" => 1 << 22,
        "s" | "super" => 1 << 23,
        "H" | "hyper" => 1 << 24,
        "S" | "shift" => 1 << 25,
        "C" | "ctrl" | "control" => 1 << 26,
        "M" | "meta" => META,
        "up" => 1,
        "down" => 2,
        "drag" => 4,
        "click" => 8,
        "double" => 16,
        "triple" => 32,
        _ => 0,
    })
}
/// `preferred_sequence_p`: 0 when EVENTS use a non-character or a modifier
/// other than MODIFIER and meta, 2 when they use MODIFIER, else 1.
fn preferred(events: &[Value], modifier: i64) -> u8 {
    let mut result = 1;
    for event in events {
        match event.integer_value().map(|c| c & NON_META_MODIFIERS) {
            Some(bits) if bits == modifier => result = 2,
            Some(0) => {}
            _ => return 0,
        }
    }
    result
}

/// A hash of a key sequence's EVENTS that `equal` sequences share, over all
/// of them: a character or a symbol by identity, any other event (a range,
/// a menu's string) as `sxhash-equal` hashes it.
fn sequence_hash(cx: &Leaf<'_>, events: &[Value]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    for &event in events {
        if event.tag() == 7 || event == Value::NIL || event == Value::TRUE {
            event.0.hash(&mut hasher);
        } else {
            event.integer_value().unwrap_or_else(|| cx.sxhash(event).integer_value().unwrap_or(0)).hash(&mut hasher);
        }
    }
    hasher.finish()
}
/// SEQUENCES without those `equal` to an earlier one: inherited keymaps
/// give a sequence more than once.
fn distinct(cx: &Leaf<'_>, sequences: Vec<Value>) -> Faulty<Vec<Value>> {
    let mut seen: HashMap<u64, Vec<Value>> = HashMap::default();
    let mut kept = Vec::new();
    for sequence in sequences {
        let alike = seen.entry(sequence_hash(cx, &cx.vector_values(sequence)?)).or_default();
        if !alike.iter().try_fold(false, |seen, &other| Ok::<_, Fault>(seen || equal(cx, other, sequence)?))? {
            alike.push(sequence);
            kept.push(sequence);
        }
    }
    Ok(kept)
}

/// Where a query is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Looking up what the definition is remapped to.
    Remap,
    /// Trying the definition's advertised bindings, those left in `later`.
    Advertised,
    /// Looking up the sequence at `at` of `pending`.
    Sequence,
    /// Looking up what `carried`, which that sequence runs, is remapped to.
    Remapped,
}
const STAGES: [Stage; 4] = [Stage::Remap, Stage::Advertised, Stage::Sequence, Stage::Remapped];

/// `Fwhere_is_internal` as it stands between two of its lookups.  Looking a
/// key sequence up applies menu filters, which are Lisp: the query is then
/// kept as `values` in the primitive's saved registers, and goes on from
/// them with the filter's result.  Nothing is found twice: the keymaps
/// searched, the sequences still to check and those found are the query's.
pub(crate) struct Query {
    definition: Value,
    firstonly: Value,
    noindirect: bool,
    no_remap: bool,
    /// The keymaps searched, a list.
    maps: Value,
    modifier: i64,
    stage: Stage,
    /// The sequences of this pass, a Lisp vector, and the next to look up.
    pending: Value,
    at: usize,
    /// The sequences of the commands remapped to the definition, a list,
    /// which make the second pass (`remapped`): a remapped command is not
    /// remapped again.
    later: Value,
    remapped: bool,
    /// The sequences found, newest first, a repeated one each time.
    found: Value,
    carried: Value,
}

/// What a query does next: look KEY up, or give its answer.
enum Next {
    Lookup(Value),
    Answer(Value),
}
impl Query {
    /// How many saved registers a query takes.
    pub(crate) const VALUES: usize = 13;
    pub(crate) fn new(cx: &mut Leaf<'_>, definition: Value, keymap: Value, firstonly: Value, noindirect: bool, no_remap: bool) -> Faulty<Self> {
        let maps = if cx.pair(keymap).is_some_and(|(first, _)| cx.keymapp(first)) {
            keymap
        } else if keymap != Value::NIL {
            let global = cx.global_map();
            let rest = cx.cons(global, Value::NIL);
            cx.cons(keymap, rest)
        } else {
            cx.current_active_maps(false, Value::NIL)?
        };
        Ok(Self {
            definition, firstonly, noindirect, no_remap, maps, modifier: preferred_modifier(cx)?, stage: Stage::Remap,
            pending: Value::NIL, at: 0, later: Value::NIL, remapped: false, found: Value::NIL, carried: Value::NIL,
        })
    }
    pub(crate) fn values(&self) -> Faulty<[Value; Self::VALUES]> {
        let flag = |flag: bool| if flag { Value::TRUE } else { Value::NIL };
        let count = |count: usize| integer(i64::try_from(count).unwrap_or(i64::MAX));
        let stage = STAGES.iter().position(|stage| *stage == self.stage).unwrap_or(0);
        Ok([self.definition, self.firstonly, flag(self.noindirect), flag(self.no_remap), self.maps, integer(self.modifier)?, count(stage)?,
            self.pending, count(self.at)?, self.later, flag(self.remapped), self.found, self.carried])
    }
    pub(crate) fn from_values(values: &[Value]) -> Self {
        let count = |value: Value| value.integer_value().and_then(|count| usize::try_from(count).ok()).unwrap_or(0);
        Self {
            definition: values[0], firstonly: values[1], noindirect: values[2] != Value::NIL, no_remap: values[3] != Value::NIL, maps: values[4],
            modifier: values[5].integer_value().unwrap_or(0), stage: STAGES[count(values[6]).min(STAGES.len() - 1)], pending: values[7],
            at: count(values[8]), later: values[9], remapped: values[10] != Value::NIL, found: values[11], carried: values[12],
        }
    }
    /// `[remap COMMAND]`, the key of COMMAND's remapping.
    fn remap_key(cx: &mut Leaf<'_>, command: Value) -> Value {
        let remap = cx.intern("remap");
        cx.vector(&[remap, command])
    }
    fn nomenus(&self, cx: &mut Leaf<'_>) -> bool {
        self.firstonly != Value::NIL && self.firstonly != cx.intern("non-ascii")
    }
    /// The sequences bound to DEFINITION, before shadowing.
    fn candidates(&self, cx: &mut Leaf<'_>, maps: &[Value], definition: Value) -> Faulty<Value> {
        let nomenus = self.nomenus(cx);
        candidates(cx, maps, definition, self.noindirect, nomenus)
    }

    /// Go on to the query's value, or to the next menu filter to apply.
    /// Each lookup is `shadow_lookup`: one `lookup-key` over the keymaps'
    /// list, of the key itself (the walk takes a range event's first
    /// character when it reaches it).
    pub(crate) fn advance(&mut self, cx: &mut Leaf<'_>) -> Walk<Value> {
        loop {
            let key = match self.next(cx)? {
                Next::Answer(answer) => return Ok(answer),
                Next::Lookup(key) => key,
            };
            let binding = match cx.key_walk(self.maps, key, false)? {
                (_, Walked::Done(binding)) => binding,
                (walk, Walked::Filter { filter, object }) => return Err(Stop::Filter { walk: Box::new(walk), filter, object }),
            };
            if let Some(answer) = self.looked_up(cx, binding)? { return Ok(answer); }
        }
    }
    /// The stages up to the next lookup, or the query's answer.
    fn next(&mut self, cx: &mut Leaf<'_>) -> Faulty<Next> {
        loop {
            match self.stage {
                // A remapped command's keys are those of the command it is remapped to.
                Stage::Remap if symbolp(cx, self.definition) => return Ok(Next::Lookup(Self::remap_key(cx, self.definition))),
                Stage::Remap => { self.looked_up(cx, Value::NIL)?; }
                Stage::Advertised => match cx.pair(self.later) {
                    Some((key, _)) => return Ok(Next::Lookup(key)),
                    None if self.later != Value::NIL => return Ok(Next::Lookup(self.later)),
                    None => {
                        let maps = elements(cx, self.maps)?;
                        (self.pending, self.at, self.stage) = (self.candidates(cx, &maps, self.definition)?, 0, Stage::Sequence);
                    }
                },
                Stage::Sequence if self.at < cx.vector_len(self.pending)? => return Ok(Next::Lookup(self.sequence(cx)?)),
                Stage::Sequence if self.remapped => return Ok(Next::Answer(self.answer(cx)?)),
                Stage::Sequence => {
                    let later = elements(cx, self.later)?;
                    (self.pending, self.at, self.later, self.remapped) = (cx.vector_from(later), 0, Value::NIL, true);
                }
                Stage::Remapped => return Ok(Next::Lookup(Self::remap_key(cx, self.carried))),
            }
        }
    }
    /// The stage that asked for a lookup goes on with its BINDING, nil when
    /// the key is too long for the keymaps: the query's answer when that is
    /// its first key, else None.
    pub(crate) fn looked_up(&mut self, cx: &mut Leaf<'_>, binding: Value) -> Faulty<Option<Value>> {
        let binding = if binding.integer_value().is_some() { Value::NIL } else { binding };
        match self.stage {
            Stage::Remap => {
                if !self.no_remap && binding != Value::NIL { self.definition = binding; }
                let advertises = self.firstonly != Value::NIL && cx.kind(self.definition) == Kind::Symbol;
                self.later = if advertises { let property = cx.intern(":advertised-binding"); cx.symbol_property(self.definition, property)? } else { Value::NIL };
                self.stage = Stage::Advertised;
                Ok(None)
            }
            // The first advertised binding that runs the definition is its key.
            Stage::Advertised => {
                let (key, rest) = cx.pair(self.later).unwrap_or((self.later, Value::NIL));
                if binding == self.definition { return Ok(Some(key)); }
                self.later = rest;
                Ok(None)
            }
            // A binding another keymap shadows does not run the definition.
            Stage::Sequence if self.remapped && binding != Value::NIL && symbolp(cx, binding) => {
                (self.carried, self.stage) = (binding, Stage::Remapped);
                Ok(None)
            }
            Stage::Sequence => {
                let sequence = self.sequence(cx)?;
                self.runs(cx, sequence, binding)
            }
            Stage::Remapped => {
                let value = if binding == Value::NIL { self.carried } else { binding };
                let sequence = self.sequence(cx)?;
                (self.carried, self.stage) = (Value::NIL, Stage::Sequence);
                self.runs(cx, sequence, value)
            }
        }
    }
    /// The sequence at `at` of `pending`.
    fn sequence(&self, cx: &mut Leaf<'_>) -> Faulty<Value> {
        cx.vector_get(self.pending, integer(i64::try_from(self.at).unwrap_or(i64::MAX))?)
    }
    /// The query's value once every sequence is checked: those found, each
    /// once, or with FIRSTONLY the one it prefers.
    fn answer(&self, cx: &mut Leaf<'_>) -> Faulty<Value> {
        let mut found = elements(cx, self.found)?;
        found.reverse();
        if self.firstonly == Value::NIL {
            let found = distinct(cx, found)?;
            return Ok(found.iter().rev().fold(Value::NIL, |tail, &sequence| cx.cons(sequence, tail)));
        }
        // No sequence uses the preferred modifier: the best of the others,
        // the first found.
        let plain = if self.modifier == 0 { None } else {
            found.iter().copied().find(|&sequence| cx.vector_values(sequence).is_ok_and(|events| preferred(&events, self.modifier) > 0))
        };
        Ok(plain.or(found.first().copied()).unwrap_or(Value::NIL))
    }
    /// SEQUENCE, the one at `at`, runs VALUE: note it when that is the
    /// definition, and give it when it is the first key asked for.
    fn runs(&mut self, cx: &mut Leaf<'_>, mut sequence: Value, value: Value) -> Faulty<Option<Value>> {
        self.at += 1;
        if !equal(cx, value, self.definition)? { return Ok(None); }
        let mut events = cx.vector_values(sequence)?;
        // The keys of a command remapped to the definition come later.
        if let ([first, command], false, false) = (&events[..], self.no_remap, self.remapped)
            && *first == cx.intern("remap")
            && symbolp(cx, *command)
        {
            let maps = elements(cx, self.maps)?;
            let sequences = self.candidates(cx, &maps, *command)?;
            for sequence in cx.vector_values(sequences)?.into_iter().rev() { self.later = cx.cons(sequence, self.later); }
            return Ok(None);
        }
        // One entry stands for a menu's strings; the cache shares SEQUENCE.
        if events.last().is_some_and(|&last| stringp(cx, last)) {
            *events.last_mut().expect("an event") = cx.string("(any string)");
            sequence = cx.vector(&events);
        }
        let non_key = cx.intern("non-key-event");
        let ignored = matches!(events[..], [event] if cx.kind(event) == Kind::Symbol && cx.symbol_property(event, non_key)? != Value::NIL);
        if !ignored { self.found = cx.cons(sequence, self.found); }
        let first = self.firstonly == cx.intern("non-ascii") || self.firstonly != Value::NIL && preferred(&events, self.modifier) == 2;
        Ok(first.then_some(sequence))
    }
}
