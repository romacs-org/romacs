//! What the reader asks of the VM: symbols under shorthands, and buffer streams.
use crate::coordinates::ByteOffset;
use crate::{Result, Value, Vm, data::Object};

impl Vm {
    /// lread.c oblookup_considering_shorthand: the name TOKEN reads as under
    /// `read-symbol-shorthands', the first (SHORTHAND . LONGHAND) pair of
    /// strings whose SHORTHAND prefixes it, or None. A name made only of
    /// `^*+-/<=>_|' (symbol_char_span) is exempt, as is `#_' syntax.
    pub(super) fn shorthand_longhand(&mut self, token: &str) -> Result<Option<String>> {
        let Some(&slot) = self.names.get("read-symbol-shorthands") else { return Ok(None) };
        let shorthands = self.variable_value(slot)?;
        if matches!(shorthands, Value::NIL | Value::UNBOUND) || token.bytes().all(|b| b"^*+-/<=>_|".contains(&b)) {
            return Ok(None);
        }
        let mut tail = shorthands;
        for _ in 0..=self.list_limit() {
            let Ok((pair, next)) = self.cons_pair(tail) else { break };
            if let Ok((short, long)) = self.cons_pair(pair)
                && let (Some(short), Some(long)) = (self.str_view(short), self.str_view(long))
                && let Some(rest) = token.as_bytes().strip_prefix(short.bytes.as_ref())
            {
                let mut name = long.bytes.to_vec();
                name.extend_from_slice(rest);
                return Ok(Some(String::from_utf8_lossy(&name).into_owned()));
            }
            tail = next;
        }
        Ok(None)
    }
    /// A fresh symbol outside the obarray, as `make-symbol' makes.
    pub(crate) fn uninterned_symbol(&mut self, name: &str) -> Value {
        let value = self.generated_symbol(crate::expand::generated_id());
        let slot = value.index_if(7).unwrap() - 1;
        self.symbol_names[slot] = name.to_owned();
        value
    }
    /// Interning is stable for this VM's lifetime, independent of moving GC.
    pub(crate) fn intern(&mut self, name: &str) -> Value {
        match name {
            "nil" => Value::NIL,
            "t" => Value::TRUE,
            _ => {
                let slot = self.slot(name);
                Value::reference(slot + 1, 7)
            }
        }
    }
    fn reader_buffer_location(&self, source: Value) -> Result<(Value, usize, bool)> {
        if let Some(Object::Marker(m)) = self.object_if(source) {
            let anchor = m.anchor.ok_or("Reader refuses a detached marker")?;
            self.require_live_buffer(m.buffer)?;
            let at = self.marker_anchors(m.buffer)?.resolve(anchor).ok_or("invalid marker anchor")? as usize;
            Ok((m.buffer, at, true))
        } else {
            self.require_live_buffer(source)?;
            let at = match self.object(source)? {
                Object::Buffer(b) => b.point.0,
                Object::ByteBuffer(b) => b.point.0,
                _ => unreachable!(),
            };
            Ok((source, at, false))
        }
    }
    fn reader_buffer_position(
        &mut self,
        source: Value,
        buffer: Value,
        at: usize,
        marker: bool,
    ) -> Result<()> {
        if marker {
            let Object::Marker(m) = self.object(source)? else {
                unreachable!()
            };
            let (anchor, bias) = (m.anchor.unwrap(), m.bias);
            if !self.marker_anchors_mut(buffer)?.set(anchor, at as u32, bias) {
                return Err("invalid marker anchor".into());
            }
        } else {
            match self.object_mut(buffer)? {
                Object::Buffer(b) => b.point = ByteOffset(at),
                Object::ByteBuffer(b) => b.point = ByteOffset(at),
                _ => unreachable!(),
            }
        }
        Ok(())
    }
    pub(super) fn reader_buffer_get(&mut self, source: Value) -> Result<Option<u32>> {
        let (buffer, at, marker) = self.reader_buffer_location(source)?;
        let (code, width) = match self.object(buffer)? {
            Object::Buffer(b) => {
                if at >= b.accessible().end.0 {
                    return Ok(None);
                }
                let c = b.character_at(ByteOffset(at))?.ok_or("invalid buffer reader point")? as u32;
                (c, crate::lstring::char_string(c, &mut [0; 5]))
            }
            Object::ByteBuffer(b) => {
                if at >= b.restriction.end.0 {
                    return Ok(None);
                }
                let c = *b.bytes.get(at).ok_or("invalid byte buffer reader point")? as u32;
                (if c >= 128 { c + 0x3fff00 } else { c }, 1)
            }
            _ => unreachable!(),
        };
        self.reader_buffer_position(source, buffer, at + width, marker)?;
        Ok(Some(code))
    }
    pub(super) fn reader_buffer_unget(&mut self, source: Value, code: u32) -> Result<()> {
        let (buffer, at, marker) = self.reader_buffer_location(source)?;
        let width = match self.object(buffer)? {
            Object::Buffer(_) => crate::lstring::char_string(code, &mut [0; 5]),
            _ => 1,
        };
        self.reader_buffer_position(
            source,
            buffer,
            at.checked_sub(width)
                .ok_or("reader unread before buffer start")?,
            marker,
        )
    }
}
