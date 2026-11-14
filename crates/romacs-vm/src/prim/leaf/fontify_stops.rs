//! Where redisplay calls `fontification-functions`: the positions at which
//! GNU's display iterator runs `handle_fontified_prop` (xdisp.c). Its
//! handlers run in order at each stop, `fontified` before `invisible`, so
//! it calls at shown text whose `fontified` is nil and once at the first
//! position of a hidden stretch; then `handle_invisible_prop` steps over the
//! stretch, adjacent ones merged, and nothing inside it is asked for. Hidden
//! is `TEXT_PROP_MEANS_INVISIBLE` of the `invisible` char property as the
//! window sees it (`Leaf::skip_hidden`, layout's own): text property or
//! overlay, through `category` and `char-property-alias-alist`, against
//! `buffer-invisibility-spec`. An ellipsis changes what the stretch shows,
//! not where it ends. `fontified` is read by buffer, as GNU reads it.
//!
//! The walk goes from one place where either property may change to the
//! next (`Vm::next_keys_change`): the runs of the text properties that
//! decide them and the overlays' ends, so it costs those, not the
//! characters nor every other property's runs.
use super::{Fault, Leaf};
use crate::Value;

type R<T> = std::result::Result<T, Fault>;

/// Shown stretches passed between polls for input.
const POLL_STEPS: u32 = 1024;

impl Leaf<'_> {
    /// The first position from POS, before END, of the current buffer as
    /// WINDOW shows it, at which redisplay calls `fontification-functions`;
    /// None when there is none. With PAST, POS is a position just called
    /// at, and the walk first leaves it as the iterator does: past its
    /// hidden stretch, or, where the call left it unfontified, past its run.
    /// WINDOW nil is the selected window.
    pub(crate) fn fontify_next(&mut self, window: Value, pos: i64, end: i64, past: bool) -> R<Option<i64>> {
        let window = self.vm.decode_window(window, true, false).map_err(|_| Fault::Wrong("window-live-p", window))?;
        let buffer = self.current_buffer()?;
        self.vm.mirror_shown(buffer).map_err(Fault::Error)?;
        let shape = self.buffer_shape()?;
        let end = end.min(shape.zv);
        let mut pos = pos.max(shape.begv);
        let (fontified, invisible) = (self.intern("fontified"), self.intern("invisible"));
        // The runs where either property may change.
        let mut keys = self.vm.deciding_keys(fontified).map_err(Fault::Error)?;
        for key in self.vm.deciding_keys(invisible).map_err(Fault::Error)? {
            if !keys.contains(&key) { keys.push(key); }
        }
        if past && pos < end {
            pos = if self.shown_invisible_kind(buffer, window, pos)? != 0 {
                self.skip_hidden(buffer, window, (pos, end), false)?.0
            } else if self.unfontified_at(buffer, pos, fontified)? {
                self.vm.next_keys_change(buffer, (pos, end), &keys).map_err(Fault::Error)?
            } else {
                pos
            };
        }
        let mut steps = 0u32;
        while pos < end {
            steps = steps.wrapping_add(1);
            if steps.is_multiple_of(POLL_STEPS) { self.vm.native_poll()?; }
            if self.unfontified_at(buffer, pos, fontified)? {
                return Ok(Some(pos));
            }
            pos = if self.shown_invisible_kind(buffer, window, pos)? != 0 {
                self.skip_hidden(buffer, window, (pos, end), false)?.0
            } else {
                self.vm.next_keys_change(buffer, (pos, end), &keys).map_err(Fault::Error)?
            };
        }
        Ok(None)
    }

    /// Whether POS of BUFFER has a nil `fontified` char property, read as
    /// `handle_fontified_prop` reads it: by buffer, every overlay counting.
    fn unfontified_at(&mut self, buffer: Value, pos: i64, fontified: Value) -> R<bool> {
        let at = Value::integer(pos).map_err(Fault::Error)?;
        Ok(self.vm.get_char_property_values(at, fontified, buffer).map_err(Fault::Error)? == Value::NIL)
    }
}
