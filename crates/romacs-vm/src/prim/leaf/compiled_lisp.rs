//! Background compiles of compiled Lisp (`crate::compiled_lisp`): what
//! lisp/session/romacs-precompile.el's switch asks.
use super::*;

impl Leaf<'_> {
    pub(crate) fn compiled_lisp_background(&mut self, on: bool, delay: Value) -> crate::prim::Out {
        if !crate::compiled_lisp::compiles() {
            return Ok(Value::NIL);
        }
        if !on {
            crate::compiled_lisp::stop_background();
            return Ok(Value::TRUE);
        }
        let seconds = match delay.integer_value() {
            Some(n) => n as f64,
            None => self.float_value(delay)?,
        };
        let load_path = self.vm.load_path_roots().map_err(Fault::Error)?;
        crate::compiled_lisp::start_background(load_path, std::time::Duration::from_secs_f64(seconds.max(0.0)));
        Ok(Value::TRUE)
    }

    pub(crate) fn compiled_lisp_stats(&mut self) -> crate::prim::Out {
        let (stats, store, identity) = crate::compiled_lisp::stats();
        let int = |n: u64| Value::integer(i64::try_from(n).unwrap_or(i64::MAX)).expect("a count");
        let mut misses = Value::NIL;
        for (path, miss) in &stats.misses {
            let file = self.string(&path.to_string_lossy());
            let reason = self.intern(miss.name());
            let pair = self.cons(file, reason);
            misses = self.cons(pair, misses);
        }
        let store = store.map_or(Value::NIL, |store| self.string(&store.to_string_lossy()));
        let identity = identity.map_or(Value::NIL, |identity| self.string(identity));
        let items = [
            ("taken", int(stats.taken)), ("compiled", int(stats.compiled)),
            ("source-on-miss", int(stats.source_on_miss)), ("source-off", int(stats.source_off)),
            ("misses", misses), ("store", store), ("identity", identity),
        ];
        let mut plist = Value::NIL;
        for (key, value) in items.into_iter().rev() {
            let key = self.intern(&format!(":{key}"));
            plist = self.cons(value, plist);
            plist = self.cons(key, plist);
        }
        Ok(plist)
    }
}
