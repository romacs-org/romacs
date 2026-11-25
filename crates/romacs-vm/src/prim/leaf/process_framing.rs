//! `set-process-framing': a process's output split into whole frames in
//! the read itself (crate::framing), so its filter is called once a read
//! with the messages it completed, and the size of what it holds unsent.
use super::*;
use crate::framing::{Frame, Framer, Kind};
use crate::prim::Out;
use crate::subprocess::Slot;

type Result<T> = std::result::Result<T, Fault>;

fn fixnum(n: usize) -> Result<Value> {
    Value::integer(i64::try_from(n).unwrap_or(i64::MAX)).map_err(Fault::Error)
}

impl Leaf<'_> {
    /// Frame PROC's output by KIND (`lines', `content-length', or nil for
    /// none), each frame at most LIMIT bytes (nil: no limit).  Bytes read
    /// but not yet given to Lisp move between the framer and the decoder's
    /// carryover, so turning framing on or off loses none.
    pub(crate) fn process_set_framing(&mut self, proc: Value, kind: Value, limit: Value) -> Out {
        let kind = match kind {
            Value::NIL => None,
            _ if kind == self.intern("lines") => Some(Kind::Lines),
            _ if kind == self.intern("content-length") => Some(Kind::ContentLength),
            _ => return Err(Fault::Error(format!("Unknown process framing: {}", self.symbol_name(kind).unwrap_or_default()))),
        };
        let limit = match limit {
            Value::NIL => None,
            _ => Some(usize::try_from(limit.integer_value().filter(|&n| n >= 0).ok_or(Fault::Wrong("natnump", limit))?)
                .map_err(|_| Fault::Wrong("natnump", limit))?),
        };
        let name = self.string_text(self.process(proc)?.get(Slot::Name))?;
        let Some(channel) = self.channel_mut(proc) else {
            return Err(Fault::Error(format!("Process {name} has no output to frame")));
        };
        let mut rest = channel.framer.take().map(Framer::into_rest).unwrap_or_default();
        rest.append(&mut channel.carryover);
        match kind {
            // Framed with the next read's bytes.
            Some(kind) => channel.framer = Some(Framer::with_rest(kind, limit, rest)),
            None => channel.carryover = rest,
        }
        Ok(Value::NIL)
    }

    /// PROC's framing: (KIND . LIMIT), or nil.
    pub(crate) fn process_framing(&mut self, proc: Value) -> Out {
        self.process(proc)?;
        let Some((kind, limit)) = self.channel(proc).and_then(|c| c.framer.as_ref()).map(|f| (f.kind(), f.limit())) else {
            return Ok(Value::NIL);
        };
        let kind = self.sym(match kind { Kind::Lines => "lines", Kind::ContentLength => "content-length" });
        let limit = limit.map_or(Ok(Value::NIL), fixnum)?;
        Ok(self.cons(kind, limit))
    }

    /// The bytes PROC's channel holds unsent, written as its peer reads.
    pub(crate) fn process_unsent(&mut self, proc: Value) -> Out {
        self.process(proc)?;
        fixnum(self.channel(proc).map_or(0, |c| c.pending.len()))
    }

    /// process_read for a framed PROC: BUF, the bytes just read (none at
    /// end of file), framed.  (NBYTES . FRAMES): each frame a string
    /// decoded by PROC's coding system, or `over' or `malformed'.
    pub(super) fn process_read_framed(&mut self, proc: Value, buf: &[u8]) -> Out {
        let eof = buf.is_empty();
        let mut frames = Vec::new();
        if let Some(channel) = self.channel_mut(proc)
            && !(eof && channel.last_block)
            && let Some(framer) = channel.framer.as_mut()
        {
            framer.push(buf, &mut frames);
            if eof {
                framer.finish(&mut frames);
                channel.last_block = true;
            }
        }
        self.process_mut(proc).nbytes_read += buf.len() as u64;
        let list = self.frames_list(proc, frames)?;
        let nbytes = fixnum(buf.len())?;
        Ok(self.cons(nbytes, list))
    }

    /// FRAMES as Lisp, in order.
    fn frames_list(&mut self, proc: Value, frames: Vec<Frame>) -> Out {
        if frames.is_empty() {
            return Ok(Value::NIL);
        }
        let coding = self.process(proc)?.get(Slot::Decode);
        let raw = self.coding_raw(coding)?;
        let mut items = Vec::with_capacity(frames.len());
        for frame in frames {
            items.push(match frame {
                Frame::Bytes(bytes) if raw => self.make_string(bytes, false),
                Frame::Bytes(bytes) => self.decode_output(bytes, coding)?,
                Frame::Over => self.sym("over"),
                Frame::Malformed => self.sym("malformed"),
            });
        }
        Ok(self.list(&items))
    }

    /// Whether CODING leaves bytes as they are: nil, or raw text with Unix
    /// line ends (`no-conversion', `binary').
    fn coding_raw(&mut self, coding: Value) -> Result<bool> {
        if coding == Value::NIL {
            return Ok(true);
        }
        let spec = self.coding_spec_checked(coding)?;
        let attrs = self.vector_get(spec, fixnum(0)?)?;
        let kind = self.vector_get(attrs, fixnum(3)?)?;
        let eol = self.vector_get(spec, fixnum(2)?)?;
        Ok(kind == self.intern("raw-text") && eol == self.intern("unix"))
    }
}
