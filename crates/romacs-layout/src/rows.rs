//! The line cache: each display line a layout read lately, by its
//! content, with its rows in each context it was wrapped in. Rows are
//! found when first asked for and no further; a line that changed is
//! wrapped again only from the row that read the change; the whole is
//! held to a byte budget, least recently used first out.
use crate::faces::Faces;
use crate::line::{LineText, LineView, Run};
use crate::wrap::{Ask, Context, Pointed, Restart, Row, Shift, Standing, Started, Walk, Walked};
use romacs_display_protocol::call_count::add;
use romacs_font::FontResolver;
use rustc_hash::FxHashMap;
use std::rc::Rc;

/// A line no longer than this is wrapped whole when it changes: it is a
/// row or two. A longer one's earlier version is found by its first or its
/// last this many bytes, one of which an edit leaves as they were.
const RESUME_MIN: usize = 64;

/// The bytes the cache holds before it drops what was least recently used.
pub(crate) const BUDGET: usize = 32 << 20;

/// The hints a window keeps before it starts over.
const HINTS: usize = 4096;

/// A line's rows in one context: those found so far and, until the line's
/// last row is found, the state the next one starts from.
struct Layout {
    context: Context,
    rows: Vec<Row>,
    next: Option<Restart>,
    /// The rows of the line's earlier version after the change, until a
    /// new row meets one of them.
    resume: Option<Resume>,
}

/// What is left of an earlier version's layout while its line is wrapped
/// again past the change: its rows from the first one not kept (ROWS, at
/// their old bytes), what followed them, how bytes after the change moved
/// (SHIFT), and the first byte of the new line from which the two versions
/// are the same again (FROM).
struct Resume {
    rows: Vec<Row>,
    next: Option<Restart>,
    shift: Shift,
    from: usize,
}

struct Entry {
    line: Rc<LineText>,
    /// The lines of the window that made it which began or ended as it
    /// does when it was made: earlier versions of it, perhaps.
    earlier: Vec<u64>,
    layouts: Vec<Layout>,
    /// The pass that last read it.
    used: u64,
    bytes: usize,
}

/// What the layouts did, counted: lines some row of which was wrapped,
/// lines whose rows were all there, rows wrapped, and rows of an earlier
/// version of a line kept when it was wrapped again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub lines_laid: u64,
    pub lines_served: u64,
    pub rows_wrapped: u64,
    pub rows_reused: u64,
    /// Lines resumed from an earlier version, and the times a resumed
    /// line met that version's rows again.
    pub lines_resumed: u64,
    pub resyncs: u64,
}

pub(crate) struct Cache {
    entries: FxHashMap<u64, Entry>,
    bytes: usize,
    budget: usize,
    pass: u64,
    /// Each window's lines longer than `RESUME_MIN`, by the hash of their
    /// first and of their last bytes, and the pass that last read one.
    recent: FxHashMap<u64, (u64, FxHashMap<u64, u64>)>,
    pub(crate) counts: Counts,
    /// The counts the last pass handed on.
    published: Counts,
}

impl Default for Cache {
    fn default() -> Self {
        Cache { entries: FxHashMap::default(), bytes: 0, budget: BUDGET, pass: 0, recent: FxHashMap::default(), counts: Counts::default(), published: Counts::default() }
    }
}

/// The first byte at which lines A and B differ in anything a row reads:
/// their characters, the face of a character, a raw mark, a stretch, an
/// image, a wrap flip or a line number's label; the shorter one's length
/// when it is the other's start.
fn first_difference(a: &LineText, b: &LineText) -> usize {
    let common = common_prefix(a.text.as_bytes(), b.text.as_bytes());
    let mut at = (0..=common).rev().find(|&at| a.text.is_char_boundary(at)).unwrap_or(0);
    let run = |a: Option<&Run>, b: Option<&Run>| match (a, b) {
        (Some(a), Some(b)) if a.start == b.start && a.slot == b.slot => a.end.min(b.end),
        (Some(a), Some(b)) => a.start.min(b.start),
        (Some(one), None) | (None, Some(one)) => one.start,
        (None, None) => usize::MAX,
    };
    let differ = (0..a.runs.len().max(b.runs.len())).find(|&i| a.runs.get(i) != b.runs.get(i));
    if let Some(i) = differ { at = at.min(run(a.runs.get(i), b.runs.get(i))); }
    // A slot whose face differs differs where it first shows.
    let same = a.faces.iter().zip(b.faces.iter()).take_while(|(a, b)| Rc::ptr_eq(a, b)).count();
    if same < a.faces.len().max(b.faces.len()) {
        if same == 0 { return 0; }
        let first = |line: &LineText| line.runs.iter().find(|run| run.slot as usize >= same).map_or(usize::MAX, |run| run.start);
        at = at.min(first(a)).min(first(b));
    }
    at.min(first_listed(&a.raw, &b.raw, |&byte| byte)).min(first_listed(&a.flips, &b.flips, |&byte| byte))
        .min(first_listed(&a.labels, &b.labels, |label| label.0))
        .min(first_listed(&a.stretches, &b.stretches, |s| s.0)).min(first_listed(&a.images, &b.images, |i| i.byte))
}

/// The byte of the first entry where lists A and B, by byte, differ.
fn first_listed<T: PartialEq>(a: &[T], b: &[T], byte: impl Fn(&T) -> usize) -> usize {
    match (0..a.len().max(b.len())).find(|&i| a.get(i) != b.get(i)) {
        Some(i) => a.get(i).map_or(usize::MAX, &byte).min(b.get(i).map_or(usize::MAX, &byte)),
        None => usize::MAX,
    }
}

/// LIST's entries from byte FROM on, by BYTE.
fn tail<T>(list: &[T], from: usize, byte: impl Fn(&T) -> usize) -> &[T] { &list[list.partition_point(|item| byte(item) < from)..] }

/// The first byte of NEW from which it is OLD again, moved by their
/// difference in length: their characters, and everything else a row
/// reads there, the line's end included; none when their ends differ.
/// Never before FROM, where they first differ.
fn common_end(old: &LineText, new: &LineText, from: usize) -> Option<usize> {
    if (old.extend, old.newline) != (new.extend, new.newline) || old.faces.len() != new.faces.len()
        || !old.faces.iter().zip(new.faces.iter()).all(|(a, b)| Rc::ptr_eq(a, b)) { return None; }
    let room = old.text.len().min(new.text.len()) - from;
    let common = common_suffix(&old.text.as_bytes()[old.text.len() - room..], &new.text.as_bytes()[new.text.len() - room..]);
    let mut start = new.text.len() - common;
    while !new.text.is_char_boundary(start) { start += 1; }
    let shift = Shift { old: old.text.len(), new: new.text.len() };
    // What the lists say from START on must agree, moved.
    let old_start = shift.back(start);
    let same = tail(&old.raw, old_start, |&b| b).iter().map(|&b| shift.at(b)).eq(tail(&new.raw, start, |&b| b).iter().copied())
        && tail(&old.flips, old_start, |&b| b).iter().map(|&b| shift.at(b)).eq(tail(&new.flips, start, |&b| b).iter().copied())
        && tail(&old.labels, old_start, |l| l.0).iter().map(|l| (shift.at(l.0), l.1)).eq(tail(&new.labels, start, |l| l.0).iter().copied())
        && tail(&old.stretches, old_start, |s| s.0).iter().map(|s| (shift.at(s.0), s.1)).eq(tail(&new.stretches, start, |s| s.0).iter().copied())
        && tail(&old.images, old_start, |i| i.byte).iter().map(|i| romacs_layout_input::InlineImage { byte: shift.at(i.byte), ..*i })
            .eq(tail(&new.images, start, |i| i.byte).iter().copied());
    // Each character's face: the runs reaching past START, cut there.
    let cut = |line: &LineText, from: usize, moved: &dyn Fn(usize) -> usize| -> Vec<Run> {
        line.runs.iter().filter(|run| run.end > from)
            .map(|run| Run { start: moved(run.start.max(from)), end: moved(run.end), slot: run.slot }).collect()
    };
    (same && cut(old, old_start, &|byte| shift.at(byte)) == cut(new, start, &|byte| byte)).then_some(start)
}

impl Cache {
    /// Forget every line: the font rules changed.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.recent.clear();
        self.bytes = 0;
    }

    /// A layout pass begins: what it reads is in use until it ends.
    pub(crate) fn begin(&mut self) { self.pass += 1; }

    /// A layout pass ended: its counts go to the thread's (`call_count`),
    /// and over the budget, the lines least recently read go, down to
    /// three quarters of it, and the hints of windows not laid out for a
    /// while.
    pub(crate) fn finish(&mut self) {
        let (now, then) = (self.counts, std::mem::replace(&mut self.published, self.counts));
        add("layout.lines-laid", now.lines_laid - then.lines_laid);
        add("layout.lines-served", now.lines_served - then.lines_served);
        add("layout.rows-wrapped", now.rows_wrapped - then.rows_wrapped);
        add("layout.rows-reused", now.rows_reused - then.rows_reused);
        let pass = self.pass;
        self.recent.retain(|_, (used, _)| *used + 64 >= pass);
        if self.bytes <= self.budget { return; }
        let mut old: Vec<(u64, u64)> = self.entries.iter().filter(|(_, entry)| entry.used < pass).map(|(&key, entry)| (entry.used, key)).collect();
        old.sort_unstable();
        for (_, key) in old {
            if self.bytes <= self.budget / 4 * 3 { break; }
            if let Some(entry) = self.entries.remove(&key) { self.bytes -= entry.bytes; }
        }
    }

    /// The key of the line VIEW read, its characters TEXT, laid out for
    /// WINDOW: its entry is made if it is new, and read by this pass.
    pub(crate) fn key(&mut self, view: &LineView, text: &str, window: u64) -> u64 {
        let key = view.hash(text);
        let pass = self.pass;
        match self.entries.get_mut(&key) {
            Some(entry) if view.is(text, &entry.line) => entry.used = pass,
            _ => {
                let line = Rc::new(view.own(text));
                let bytes = line.bytes();
                // A line of another content under the same hash gives way.
                if let Some(other) = self.entries.insert(key, Entry { line, earlier: vec![], layouts: vec![], used: pass, bytes }) { self.bytes -= other.bytes; }
                self.bytes += bytes;
            }
        }
        if text.len() > RESUME_MIN {
            let (used, recent) = self.recent.entry(window).or_default();
            *used = pass;
            if recent.len() > HINTS { recent.clear(); }
            let ends = ends(text);
            let entry = self.entries.get_mut(&key).expect("made above");
            if entry.layouts.is_empty() && entry.earlier.is_empty() {
                entry.earlier = ends.iter().filter_map(|end| recent.get(end)).copied().filter(|&known| known != key).collect();
                entry.earlier.dedup();
            }
            for end in ends { recent.insert(end, key); }
        }
        key
    }

    pub(crate) fn line(&self, key: u64) -> Rc<LineText> { self.entries[&key].line.clone() }

    /// The layout of KEY's line in CONTEXT, begun if it has none: from the
    /// rows of an earlier version of the line (`Entry::earlier`), where one
    /// is kept, else from the line's start. At most two contexts are kept
    /// for a line.
    pub(crate) fn layout(&mut self, key: u64, context: &Context) -> usize {
        let entry = &self.entries[&key];
        if let Some(at) = entry.layouts.iter().position(|layout| layout.context == *context) { return at; }
        let rise = { let default = entry.line.faces[0].metrics; (default.0, default.1) };
        let resumed = self.resumed(key, context);
        // The version it resumed from is dead unless shown again: first out.
        if let Some((old, _)) = &resumed && let Some(entry) = self.entries.get_mut(old) { entry.used = 0; }
        let resumed = resumed.map(|(_, layout)| layout);
        self.counts.rows_reused += resumed.as_ref().map_or(0, |layout| layout.rows.len()) as u64;
        self.counts.lines_resumed += u64::from(resumed.is_some());
        let layout = resumed.unwrap_or(Layout { context: *context, rows: vec![], next: Some(Restart::start(rise)), resume: None });
        let grown: usize = layout.rows.iter().map(Row::bytes).sum();
        let entry = self.entries.get_mut(&key).expect("its entry");
        if entry.layouts.len() == 2 {
            let dropped: usize = entry.layouts.remove(0).rows.iter().map(Row::bytes).sum();
            entry.bytes -= dropped;
            self.bytes -= dropped;
        }
        entry.layouts.push(layout);
        entry.bytes += grown;
        self.bytes += grown;
        entry.layouts.len() - 1
    }

    /// A layout of KEY's line in CONTEXT begun from an earlier version's:
    /// its rows that read nothing at or after the first difference.
    fn resumed(&self, key: u64, context: &Context) -> Option<(u64, Layout)> {
        let entry = &self.entries[&key];
        let new = &entry.line;
        if new.text.len() <= RESUME_MIN || !new.simple { return None; }
        // A version that shares half of the line with it.
        let (from_key, old_line, old, at) = entry.earlier.iter().find_map(|known| {
            let entry = self.entries.get(known)?;
            if !entry.line.simple { return None; }
            // Its rows found so far hold, resumed or not.
            let layout = entry.layouts.iter().find(|layout| layout.context == *context)?;
            let at = first_difference(&entry.line, new);
            let end = common_end(&entry.line, new, at).map_or(0, |from| new.text.len() - from);
            (2 * (at + end) >= new.text.len().min(entry.line.text.len())).then_some((*known, &entry.line, layout, at))
        })?;
        let keep = old.rows.partition_point(|row| row.horizon <= at);
        let rows = old.rows[..keep].to_vec();
        let next = match old.rows.get(keep) {
            Some(row) => Some(row.restart),
            None => old.next,
        };
        // Every row was kept and the old line had no more: nothing at the
        // difference was read, and the line is laid out again whole.
        next?;
        let resume = common_end(old_line, new, at).filter(|_| keep < old.rows.len()).map(|from| Resume {
            rows: old.rows[keep..].to_vec(), next: old.next, shift: Shift { old: old_line.text.len(), new: new.text.len() }, from });
        Some((from_key, Layout { context: *context, rows, next, resume }))
    }

    /// Wrap the rows of layout LAYOUT of KEY's line through row INDEX.
    fn extend(&mut self, key: u64, layout: usize, until: impl Fn(&[Row]) -> bool, faces: &mut Faces, resolver: &mut FontResolver) {
        let entry = self.entries.get_mut(&key).expect("its entry");
        let line = entry.line.clone();
        let layout = &mut entry.layouts[layout];
        let mut grown = 0;
        while !until(&layout.rows) && let Some(restart) = layout.next {
            let context = layout.context;
            let mut walk = Walk { line: &line, context: &context, faces, resolver };
            let (row, next) = wrap(&mut walk, &restart);
            grown += row.bytes();
            layout.rows.push(row);
            layout.next = next;
            self.counts.rows_wrapped += 1;
            grown += meet(layout, &mut self.counts);
        }
        entry.bytes += grown;
        self.bytes += grown;
    }

    /// Row INDEX of layout LAYOUT of KEY's line, wrapped now if it was not
    /// yet, with the rows before it, and with its glyphs when PAINT; none
    /// past the line's last row.
    pub(crate) fn row(&mut self, key: u64, layout: usize, index: usize, paint: bool, faces: &mut Faces, resolver: &mut FontResolver) -> Option<Row> {
        self.extend(key, layout, |rows| rows.len() > index, faces, resolver);
        let entry = self.entries.get_mut(&key).expect("its entry");
        let line = entry.line.clone();
        let lay = &mut entry.layouts[layout];
        let context = lay.context;
        let row = lay.rows.get_mut(index)?;
        if paint && row.ops.is_none() {
            let mut walk = Walk { line: &line, context: &context, faces, resolver };
            let standing = walk.standing(&row.restart, false);
            let (drawn, _, _, _) = drawn(&mut walk, &standing, &row.restart, &Ask::default());
            let grown = drawn.bytes() - row.bytes();
            *row = drawn;
            entry.bytes += grown;
            self.bytes += grown;
        }
        Some(row.clone())
    }

    /// The index of the row of layout LAYOUT of KEY's line that starts at
    /// RESTART's byte in its state, when the line has one.
    pub(crate) fn find(&mut self, key: u64, layout: usize, restart: &Restart, faces: &mut Faces, resolver: &mut FontResolver) -> Option<usize> {
        let at = restart.at;
        self.extend(key, layout, |rows| rows.last().is_some_and(|row| row.restart.at >= at), faces, resolver);
        let rows = &self.entries[&key].layouts[layout].rows;
        rows.binary_search_by_key(&at, |row| row.restart.at).ok().filter(|&index| rows[index].restart == *restart)
    }

    /// The index of the row of layout LAYOUT of KEY's line that holds AT:
    /// the last whose first byte is at or before it.
    pub(crate) fn locate(&mut self, key: u64, layout: usize, at: usize, faces: &mut Faces, resolver: &mut FontResolver) -> usize {
        self.extend(key, layout, |rows| rows.last().is_some_and(|row| row.restart.at > at), faces, resolver);
        let rows = &self.entries[&key].layouts[layout].rows;
        rows.partition_point(|row| row.restart.at <= at).saturating_sub(1)
    }

    /// Whether a row before row INDEX of layout LAYOUT of KEY's line has a
    /// glyph of another font's metrics, and those rows' heights before
    /// they were settled. The rows are there: `locate` found INDEX.
    pub(crate) fn above(&self, key: u64, layout: usize, index: usize) -> (bool, f32) {
        let rows = &self.entries[&key].layouts[layout].rows[..index];
        (rows.iter().any(|row| row.mixed), rows.iter().map(|row| row.raw_height).sum())
    }
}

/// The row of WALK's line from RESTART and what follows it, measured, or
/// with its glyphs when its height hangs on them.
fn wrap(walk: &mut Walk, restart: &Restart) -> (Row, Option<Restart>) {
    let standing = walk.standing(restart, false);
    if let Walked::Row { row, next, unsettled: false, .. } = walk.row(&standing, restart, Ask::default()) { return (row, next); }
    let (row, next, _, _) = drawn(walk, &standing, restart, &Ask::default());
    (row, next)
}

/// The row of WALK's line from STANDING with its glyphs, what follows it,
/// point on it when asked, and how it began.
pub(crate) fn drawn(walk: &mut Walk, standing: &Standing, restart: &Restart, ask: &Ask) -> (Row, Option<Restart>, Option<Pointed>, Started) {
    let mut ops = vec![];
    let walked = walk.row(standing, restart, Ask { ops: Some(&mut ops), clip: ask.clip, point: ask.point, until: ask.until, starting: ask.starting });
    let Walked::Row { mut row, next, point, started, .. } = walked else { unreachable!("a walk with no stop answers a row") };
    row.ops = Some(ops.into());
    (row, next, point, started)
}

/// Where the last row of LAYOUT ends, take the earlier version's rows
/// when one of them starts there in the same state: nothing they read
/// differs. The bytes the rows taken hold.
fn meet(layout: &mut Layout, counts: &mut Counts) -> usize {
    let Some(resume) = &layout.resume else { return 0 };
    let Some(next) = layout.next else { layout.resume = None; return 0 };
    if next.at < resume.from { return 0; }
    let shift = resume.shift;
    let old_at = shift.back(next.at);
    let found = resume.rows.binary_search_by_key(&old_at, |row| row.restart.at).ok().filter(|&at| resume.rows[at].restart.moved(shift) == next);
    let Some(at) = found else {
        // Past every row the earlier version had found: go on alone.
        if resume.rows.last().is_none_or(|last| last.restart.at < old_at) { layout.resume = None; }
        return 0;
    };
    let moved = |byte: usize| if byte == usize::MAX { byte } else { shift.at(byte) };
    counts.rows_reused += (resume.rows.len() - at) as u64;
    counts.resyncs += 1;
    let rows: Vec<Row> = resume.rows[at..].iter().map(|row| Row { restart: row.restart.moved(shift), end: moved(row.end), horizon: moved(row.horizon), ..row.clone() }).collect();
    let bytes = rows.iter().map(Row::bytes).sum();
    layout.next = resume.next.map(|old| old.moved(shift));
    layout.rows.extend(rows);
    layout.resume = None;
    bytes
}

/// The hashes of TEXT's first and last `RESUME_MIN` bytes, told apart.
fn ends(text: &str) -> [u64; 2] {
    use std::hash::Hasher;
    let bytes = text.as_bytes();
    let hash = |part: &[u8], tag: u64| {
        let mut hasher = rustc_hash::FxHasher::default();
        hasher.write_u64(tag);
        hasher.write(part);
        hasher.finish()
    };
    [hash(&bytes[..RESUME_MIN], 0), hash(&bytes[bytes.len() - RESUME_MIN..], 1)]
}

/// How many bytes A and B share at their start, compared a block at a time.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 64;
    let n = a.len().min(b.len());
    let mut at = 0;
    while at + BLOCK <= n && a[at..at + BLOCK] == b[at..at + BLOCK] { at += BLOCK; }
    at + a[at..n].iter().zip(&b[at..n]).take_while(|(a, b)| a == b).count()
}

/// How many bytes A and B, of one length, share at their end.
fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 64;
    let n = a.len().min(b.len());
    let (a, b) = (&a[a.len() - n..], &b[b.len() - n..]);
    let mut end = n;
    while end >= BLOCK && a[end - BLOCK..end] == b[end - BLOCK..end] { end -= BLOCK; }
    n - end + a[..end].iter().rev().zip(b[..end].iter().rev()).take_while(|(a, b)| a == b).count()
}

#[cfg(test)]
mod tests {
    #[test]
    fn common_ends_count_bytes() {
        let a: Vec<u8> = (0..300u32).map(|i| (i % 7) as u8).collect();
        for cut in [0, 1, 63, 64, 65, 200, 299] {
            let mut b = a.clone();
            b[cut] ^= 0xff;
            assert_eq!(super::common_prefix(&a, &b), cut);
            assert_eq!(super::common_suffix(&a, &b), 299 - cut);
        }
        assert_eq!(super::common_prefix(&a, &a[..100]), 100);
        assert_eq!(super::common_suffix(&a, &a), 300);
    }
}
