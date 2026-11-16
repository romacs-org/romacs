use super::*;
use crate::prim::Out;

impl Leaf<'_> {
    /// `romacs--display-animates': t, nil, or `unknown' while the host has
    /// not chosen its adapter or none is attached (a read's wait lends it
    /// out, and redisplay there runs Lisp).
    pub(crate) fn display_animates(&mut self) -> Out {
        let animates = self.vm.host_input.as_mut().and_then(|host| host.display_animates());
        Ok(match animates {
            Some(true) => Value::TRUE,
            Some(false) => Value::NIL,
            None => self.intern("unknown"),
        })
    }
}

impl Leaf<'_> {
    /// `romacs-startup-progress': STAGE (a symbol or string), LABEL and, of
    /// STATE, its counts (:done of :of) and whether it failed (:status
    /// `failed') told to the display, for its launch screen; and STAGE's
    /// latest state, (STAGE :label LABEL . STATE) with STAGE as a symbol,
    /// kept in `romacs-progress-state' in place of its earlier one.  t when
    /// the launch screen showed it.
    pub(crate) fn startup_progress(&mut self, stage: Value, label: Value, state: Value) -> Out {
        let text = |leaf: &mut Self, v: Value| if leaf.kind(v) == Kind::Symbol { leaf.symbol_name(v) } else { leaf.string_text(v) };
        let (shown, said) = (text(self, stage)?, if label == Value::NIL { String::new() } else { text(self, label)? });
        let count = |leaf: &mut Self, key: &str| {
            let key = leaf.intern(key);
            leaf.plist_value(state, key).integer_value().and_then(|n| u64::try_from(n).ok())
        };
        let counts = count(self, ":done").zip(count(self, ":of"));
        let status = { let key = self.intern(":status"); self.plist_value(state, key) };
        let failed = self.kind(status) == Kind::Symbol && self.symbol_name(status).is_ok_and(|name| name == "failed");
        let took = self.vm.window_system_host().is_some_and(|host| host.startup_progress(&shown, &said, counts, failed));
        // Kept by the stage's symbol, a string stage's too, so two equal
        // names are one stage.
        let stage = if self.kind(stage) == Kind::Symbol { stage } else { self.intern(&shown) };
        let slot = self.vm.slot("romacs-progress-state");
        let old = self.vm.slots[slot].value;
        let mut others = Vec::new();
        let mut tail = old;
        let mut seen = std::collections::HashSet::new();
        while let Some((entry, rest)) = self.pair(tail) {
            if !seen.insert(tail.0) { return Err(Fault::Circular(old)); }
            if self.pair(entry).map(|(key, _)| key) != Some(stage) { others.push(entry); }
            tail = rest;
        }
        let key = self.intern(":label");
        let labelled = self.cons(label, state);
        let labelled = self.cons(key, labelled);
        let mut alist = Value::NIL;
        for entry in others.into_iter().rev() { alist = self.cons(entry, alist); }
        let entry = self.cons(stage, labelled);
        self.vm.slots[slot].value = self.cons(entry, alist);
        Ok(if took { Value::TRUE } else { Value::NIL })
    }
}

impl Leaf<'_> {
    /// `romacs--debug-lose-device': t when the open display took the request.
    pub(crate) fn debug_lose_device(&mut self) -> Out {
        let lost = self.vm.window_system_host().is_some_and(|host| host.lose_device());
        Ok(if lost { Value::TRUE } else { Value::NIL })
    }
}

impl Leaf<'_> {
    /// `romacs--native-window': ACTION `query', `show' or `hide' asked of
    /// the host; t when its window shows after the request, nil when it
    /// does not or the host has no native windows.
    /// (X . Y) of the pointer over the host frame, else nil.
    pub(crate) fn pointer(&mut self) -> Out {
        let Some((x, y)) = self.vm.host_input.as_mut().and_then(|host| host.pointer()) else { return Ok(Value::NIL) };
        let [x, y] = [x, y].map(|n| Value::integer(i64::from(n)).expect("a pixel is a fixnum"));
        Ok(self.cons(x, y))
    }
    pub(crate) fn native_window(&mut self, action: Value) -> Out {
        let action = match self.symbol_name(action).ok().as_deref() {
            Some("query") => crate::NativeWindow::Query,
            Some("show") => crate::NativeWindow::Show,
            Some("hide") => crate::NativeWindow::Hide,
            _ => return Err(crate::prim::Fault::Error("romacs--native-window: ACTION is query, show or hide".into())),
        };
        let shown = self.vm.host_input.as_mut().and_then(|host| host.native_window(action));
        Ok(if shown == Some(true) { Value::TRUE } else { Value::NIL })
    }
}
