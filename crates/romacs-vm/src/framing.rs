//! Message framing of a process's output (`set-process-framing'): a pure
//! byte splitter that turns the bytes a channel reads into whole frames, so
//! a protocol's filter is called with messages, not chunks.  Each byte is
//! scanned once and moved at most once, whatever a frame's length or the
//! chunks it came in.  Nothing here touches Lisp or a descriptor.

/// How frames are delimited.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// One frame a line, ended by LF; the LF is not part of it.
    Lines,
    /// LSP's base protocol: `Name: value' header lines ended by CRLF, a
    /// blank CRLF line, then Content-Length bytes of body.
    ContentLength,
}

/// What a frame came to.
#[derive(PartialEq, Eq, Debug)]
pub(crate) enum Frame {
    Bytes(Vec<u8>),
    /// A frame over the limit, skipped: never held whole.
    Over,
    /// A header block over `HEADER_LIMIT' bytes: the rest of the stream is
    /// dropped, since no frame boundary can be found in it again.
    Malformed,
}

/// The most bytes one Content-Length header block may take.
const HEADER_LIMIT: usize = 8192;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// Lines: within a line. Content-Length: within a header block.
    Frame,
    /// In a body of this many bytes.
    Body(usize),
    /// Skipping an over-limit frame: a line to its LF, or this many bytes.
    Skip(usize),
    /// The stream can no longer be framed.
    Broken,
}

/// One process's framing: its kind and limit, and the bytes of the frame
/// not yet complete.
#[derive(Debug)]
pub(crate) struct Framer {
    kind: Kind,
    limit: Option<usize>,
    buf: Vec<u8>,
    /// Bytes before this offset were consumed by frames this push.
    start: usize,
    /// Bytes before this offset were searched for the delimiter.
    scan: usize,
    state: State,
}

impl Framer {
    pub(crate) fn new(kind: Kind, limit: Option<usize>) -> Self {
        Framer { kind, limit, buf: Vec::new(), start: 0, scan: 0, state: State::Frame }
    }
    /// A framer whose stream begins with REST, bytes read before it was
    /// made, framed with the next push.
    pub(crate) fn with_rest(kind: Kind, limit: Option<usize>, rest: Vec<u8>) -> Self {
        Framer { buf: rest, ..Framer::new(kind, limit) }
    }
    pub(crate) fn kind(&self) -> Kind {
        self.kind
    }
    pub(crate) fn limit(&self) -> Option<usize> {
        self.limit
    }
    fn over(&self, len: usize) -> bool {
        self.limit.is_some_and(|limit| len > limit)
    }

    /// Frame BYTES, the stream's next, appending each frame they complete
    /// to OUT in order.
    pub(crate) fn push(&mut self, bytes: &[u8], out: &mut Vec<Frame>) {
        if self.state == State::Broken {
            return;
        }
        self.buf.extend_from_slice(bytes);
        match self.kind {
            Kind::Lines => self.lines(out),
            Kind::ContentLength => self.content_length(out),
        }
        // Only what frames consumed is moved, once: a partial frame never
        // is again until a frame before it completes.
        if self.start > 0 {
            self.buf.drain(..self.start);
            self.scan -= self.start;
            self.start = 0;
        }
    }

    /// The stream ended: a last line with no LF is a frame; an incomplete
    /// body or header is dropped.
    pub(crate) fn finish(&mut self, out: &mut Vec<Frame>) {
        if self.kind == Kind::Lines && self.state == State::Frame && !self.buf.is_empty() {
            out.push(Frame::Bytes(std::mem::take(&mut self.buf)));
        }
        self.buf.clear();
        self.start = 0;
        self.scan = 0;
    }

    /// The bytes of the frame not yet complete, given back when framing
    /// stops, so the stream loses nothing.
    pub(crate) fn into_rest(mut self) -> Vec<u8> {
        if matches!(self.state, State::Skip(_) | State::Broken) {
            return Vec::new();
        }
        self.buf.drain(..self.start);
        self.buf
    }

    fn lines(&mut self, out: &mut Vec<Frame>) {
        loop {
            let found = memchr::memchr(b'\n', &self.buf[self.scan..]).map(|at| self.scan + at);
            match (self.state, found) {
                (State::Skip(_), Some(_)) => self.state = State::Frame,
                (State::Skip(_), None) => {}
                (_, Some(end)) => {
                    let len = end - self.start;
                    out.push(if self.over(len) { Frame::Over } else { Frame::Bytes(self.buf[self.start..end].to_vec()) });
                }
                (_, None) if self.over(self.buf.len() - self.start) => {
                    out.push(Frame::Over);
                    self.state = State::Skip(0);
                }
                (_, None) => {}
            }
            match found {
                Some(end) => {
                    self.start = end + 1;
                    self.scan = self.start;
                }
                None => {
                    self.scan = self.buf.len();
                    if matches!(self.state, State::Skip(_)) {
                        // Nothing of a skipped line is kept.
                        self.start = self.buf.len();
                    }
                    return;
                }
            }
        }
    }

    fn content_length(&mut self, out: &mut Vec<Frame>) {
        loop {
            let avail = self.buf.len() - self.start;
            match self.state {
                State::Frame => match self.header(out) {
                    Some(len) if self.over(len) => {
                        out.push(Frame::Over);
                        self.state = State::Skip(len);
                    }
                    Some(len) => self.state = State::Body(len),
                    None => return,
                },
                State::Body(len) if avail >= len => {
                    out.push(Frame::Bytes(self.buf[self.start..self.start + len].to_vec()));
                    self.start += len;
                    self.scan = self.start;
                    self.state = State::Frame;
                }
                State::Skip(len) if avail >= len => {
                    self.start += len;
                    self.scan = self.start;
                    self.state = State::Frame;
                }
                State::Skip(len) => {
                    self.start += avail;
                    self.scan = self.start;
                    self.state = State::Skip(len - avail);
                    return;
                }
                State::Body(_) | State::Broken => return,
            }
        }
    }

    /// The next header block's Content-Length, its block consumed, or None
    /// while it is incomplete.  A block with no readable length is dropped
    /// (a server's stray output); one over `HEADER_LIMIT' breaks the stream.
    fn header(&mut self, out: &mut Vec<Frame>) -> Option<usize> {
        loop {
            let from = self.scan.saturating_sub(3).max(self.start);
            let Some(end) = memchr::memmem::find(&self.buf[from..], b"\r\n\r\n").map(|at| from + at) else {
                self.scan = self.buf.len();
                if self.buf.len() - self.start > HEADER_LIMIT {
                    out.push(Frame::Malformed);
                    self.state = State::Broken;
                    self.start = self.buf.len();
                }
                return None;
            };
            let length = content_length(&self.buf[self.start..end]);
            self.start = end + 4;
            self.scan = self.start;
            if length.is_some() {
                return length;
            }
        }
    }
}

/// The Content-Length of a header BLOCK (CRLF-separated `Name: value'
/// lines), its name matched case-blind; lines of no header are ignored.
fn content_length(block: &[u8]) -> Option<usize> {
    block.split(|&b| b == b'\n').find_map(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let colon = memchr::memchr(b':', line)?;
        let (name, value) = (&line[..colon], &line[colon + 1..]);
        if !name.eq_ignore_ascii_case(b"Content-Length") {
            return None;
        }
        std::str::from_utf8(value).ok()?.trim().parse().ok()
    })
}

#[cfg(test)]
#[path = "framing_tests.rs"]
mod tests;
