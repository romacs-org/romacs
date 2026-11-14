//! GNU inotify.c's descriptor from a leaf: one inotify instance per VM,
//! made by the first watch and closed when Lisp drops the last
//! (lisp/romacs/vm/inotify.el keeps GNU's watch list). The keyboard waits poll it
//! (`Vm::file_notify_fd`); the records are read only when Lisp asks, so the
//! kernel's queue is GNU's keyboard buffer for these events.
use super::*;
use std::os::fd::{AsFd, AsRawFd, RawFd};

type Out = std::result::Result<Value, Fault>;

/// One `struct inotify_event': its descriptor, mask, cookie and name bytes
/// (empty for an event on the watched file itself).
#[derive(Debug, PartialEq)]
pub(crate) struct Record {
    pub wd: i32,
    pub mask: u32,
    pub cookie: u32,
    pub name: Vec<u8>,
}

/// The records in BYTES, as read(2) returned them from an inotify
/// descriptor: each a fixed header and LEN bytes of NUL-padded name.
pub(crate) fn records(bytes: &[u8]) -> Vec<Record> {
    const HEADER: usize = 16;
    let word = |at: usize| u32::from_ne_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
    let mut out = Vec::new();
    let mut at = 0;
    while at + HEADER <= bytes.len() {
        let len = word(at + 12) as usize;
        let end = (at + HEADER + len).min(bytes.len());
        let name = &bytes[at + HEADER..end];
        let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
        out.push(Record { wd: word(at) as i32, mask: word(at + 4), cookie: word(at + 8), name: name.to_vec() });
        at = end;
    }
    out
}

impl Vm {
    /// The inotify descriptor the keyboard waits poll, while any watch is.
    pub(crate) fn file_notify_fd(&self) -> Option<RawFd> {
        self.inotify.as_ref().map(|fd| fd.as_raw_fd())
    }
}

impl Leaf<'_> {
    /// report_file_notify_error: (file-notify-error STRING ERRNO-TEXT . DATA).
    fn file_notify_error(&mut self, what: &str, error: &std::io::Error, data: Value) -> Fault {
        let text = error.to_string();
        let text = text.rsplit_once(" (os error ").map_or(text.as_str(), |(s, _)| s);
        let data = if data == Value::NIL || self.pair(data).is_some() { data } else { self.cons(data, Value::NIL) };
        let text = self.string(text);
        let data = self.cons(text, data);
        let what = self.string(what);
        let data = self.cons(what, data);
        Fault::Signal(self.intern("file-notify-error"), data)
    }

    /// inotify_add_watch on FILE with MASK (IN_MASK_ADD and IN_EXCL_UNLINK
    /// already in it), making the instance first: the watch descriptor.
    pub(crate) fn inotify_add_watch(&mut self, file: Value, mask: u32) -> Out {
        let name = std::ffi::CString::new(self.file_name_bytes(file)?)
            .map_err(|_| Fault::Wrong("filenamep", file))?;
        if self.vm.inotify.is_none() {
            let fd = crate::host::inotify_init().map_err(|e| self.file_notify_error("File watching is not available", &e, Value::NIL))?;
            self.vm.inotify = Some(fd);
        }
        let fd = self.vm.inotify.as_ref().expect("made above").as_fd();
        match crate::host::inotify_add_watch(fd, &name, mask) {
            Ok(wd) => Ok(Value::integer(i64::from(wd)).expect("a watch descriptor is a fixnum")),
            Err(e) => Err(self.file_notify_error("Could not add watch for file", &e, file)),
        }
    }

    /// inotify_rm_watch of WD.
    pub(crate) fn inotify_rm_watch(&mut self, wd: i64) -> Out {
        let Some(fd) = self.vm.inotify.as_ref() else { return Ok(Value::NIL) };
        match crate::host::inotify_rm_watch(fd.as_fd(), wd as i32) {
            Ok(()) => Ok(Value::TRUE),
            Err(e) => {
                let wd = Value::integer(wd).map_err(Fault::Error)?;
                Err(self.file_notify_error("Could not rm watch", &e, wd))
            }
        }
    }

    /// The queued events as ((WD MASK COOKIE NAME) ...), NAME the decoded
    /// file name or nil; nil when none is queued or no watch is left.
    pub(crate) fn inotify_read(&mut self) -> Out {
        let Some(fd) = self.vm.inotify.as_ref() else { return Ok(Value::NIL) };
        let bytes = crate::host::inotify_read(fd.as_fd())
            .map_err(|e| self.file_notify_error("Error while reading file system events", &e, Value::NIL))?;
        let mut events = Vec::new();
        for record in records(&bytes) {
            // DECODE_FILE under a UTF-8 `file-name-coding-system'.
            let name = match record.name {
                name if name.is_empty() => Value::NIL,
                name if name.is_ascii() => self.make_string(name, false),
                name => self.make_string(crate::text::octets_as_multibyte(&name).0, true),
            };
            let fields = [
                Value::integer(i64::from(record.wd)).expect("fixnum"),
                Value::integer(i64::from(record.mask)).expect("fixnum"),
                Value::integer(i64::from(record.cookie)).expect("fixnum"),
                name,
            ];
            events.push(self.vm.list_value(&fields));
        }
        Ok(self.vm.list_value(&events))
    }

    /// Close the instance: GNU's when the watch list empties.
    pub(crate) fn inotify_close(&mut self) {
        self.vm.inotify = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(wd: i32, mask: u32, cookie: u32, name: &[u8], pad: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        for word in [wd as u32, mask, cookie, (name.len() + pad) as u32] {
            bytes.extend(word.to_ne_bytes());
        }
        bytes.extend(name);
        bytes.extend(std::iter::repeat_n(0, pad));
        bytes
    }

    #[test]
    fn records_split_headers_and_trim_name_padding() {
        let mut bytes = record(1, 2, 0, b"", 0);
        bytes.extend(record(3, 0x40, 77, b"a.txt", 11));
        assert_eq!(records(&bytes), vec![
            Record { wd: 1, mask: 2, cookie: 0, name: vec![] },
            Record { wd: 3, mask: 0x40, cookie: 77, name: b"a.txt".to_vec() },
        ]);
    }
}
