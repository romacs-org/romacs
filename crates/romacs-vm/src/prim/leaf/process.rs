//! GNU process.c and callproc.c state for prims/process.rs: process objects,
//! Vprocess_alist, channels, status conversion and output decoding. Nothing
//! here calls Lisp; rlisp/vm/subprocess.el composes these with filters,
//! sentinels and timers.
use super::*;
use crate::anchors::Bias;
use crate::annotations::Marker;
use crate::prim::Out;
use crate::subprocess::{self as sp, Channel, Process, Slot};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};

type Result<T> = std::result::Result<T, Fault>;

/// Vprocess_alist: ((NAME . PROCESS) ...), newest first.
const ALIST: &str = "#:vm2-process-alist";

fn fixnum(n: i64) -> Value {
    Value::integer(n).expect("process integer is a fixnum")
}

impl Leaf<'_> {
    fn process_if(&self, v: Value) -> Option<&Process> {
        match self.vm.object_if(v) {
            Some(data::Object::Process(p)) => Some(p),
            _ => None,
        }
    }
    /// CHECK_PROCESS, then the process.
    pub(crate) fn process(&self, v: Value) -> Result<&Process> {
        self.process_if(v).ok_or(Fault::Wrong("processp", v))
    }
    fn process_mut(&mut self, v: Value) -> &mut Process {
        let index = v.index(0).expect("checked process") - 1;
        match &mut self.vm.objects[index] {
            data::Object::Process(p) => p,
            _ => unreachable!("checked process"),
        }
    }
    pub(crate) fn processp(&self, v: Value) -> bool {
        self.process_if(v).is_some()
    }
    pub(crate) fn process_slot(&self, v: Value, slot: Slot) -> Out {
        Ok(self.process(v)?.get(slot))
    }
    pub(crate) fn set_process_slot(&mut self, v: Value, slot: Slot, value: Value) -> Out {
        self.process(v)?;
        self.process_mut(v).set(slot, value);
        Ok(value)
    }
    fn channel(&self, v: Value) -> Option<&Channel> {
        let serial = self.process_if(v)?.serial;
        self.vm.subprocesses.get(serial)
    }
    fn channel_mut(&mut self, v: Value) -> Option<&mut Channel> {
        let serial = self.process_if(v)?.serial;
        self.vm.subprocesses.get_mut(serial)
    }
    /// p->infd >= 0.
    pub(crate) fn process_infd(&self, v: Value) -> bool {
        self.channel(v).is_some_and(|c| c.infd.is_some())
    }
    fn sym(&mut self, name: &str) -> Value {
        self.intern(name)
    }
    fn list(&mut self, items: &[Value]) -> Value {
        items.iter().rev().fold(Value::NIL, |tail, &v| self.cons(v, tail))
    }

    /// FOR_EACH_PROCESS: the processes of Vprocess_alist, in order.
    pub(crate) fn process_values(&mut self) -> Vec<Value> {
        let mut tail = self.hidden_value(ALIST);
        let mut out = Vec::new();
        while let Some((pair, rest)) = self.pair(tail) {
            if let Some((_, process)) = self.pair(pair) {
                out.push(process);
            }
            tail = rest;
        }
        out
    }
    pub(crate) fn process_list(&mut self) -> Value {
        let processes = self.process_values();
        self.list(&processes)
    }
    /// Fget_process on a string: (cdr (assoc NAME Vprocess_alist)).
    pub(crate) fn process_named(&mut self, name: Value) -> Result<Value> {
        let wanted = self.string_view(name)?;
        let mut tail = self.hidden_value(ALIST);
        while let Some((pair, rest)) = self.pair(tail) {
            if let Some((key, process)) = self.pair(pair)
                && self.string_view(key).is_ok_and(|k| k.bytes() == wanted.bytes())
            {
                return Ok(process);
            }
            tail = rest;
        }
        Ok(Value::NIL)
    }
    /// Fget_buffer_process: the first process of Vprocess_alist on BUFFER.
    pub(crate) fn buffer_process(&mut self, buffer: Value) -> Value {
        self.process_values().into_iter()
            .find(|&p| self.process_if(p).is_some_and(|p| p.get(Slot::Buffer) == buffer))
            .unwrap_or(Value::NIL)
    }

    /// make_process: a running process named NAME, made unique, with the
    /// default filter and sentinel, pushed onto Vprocess_alist.
    pub(crate) fn make_process(&mut self, name: Value) -> Out {
        let base = self.string_view(name)?;
        let mut unique = name;
        let mut i = 1;
        while self.process_named(unique)? != Value::NIL {
            let mut bytes = base.bytes().to_vec();
            bytes.extend_from_slice(format!("<{i}>").as_bytes());
            unique = self.make_string(bytes, base.is_multibyte());
            i += 1;
        }
        let readmax = {
            let symbol = self.intern("read-process-output-max");
            self.variable_value(symbol)?.and_then(|v| v.integer_value()).unwrap_or(65536)
        };
        let mut process = Process::new(0, readmax.clamp(1, i64::from(i32::MAX)) as usize);
        let mark = self.vm.allocate_object(data::Object::Marker(Box::new(Marker {
            buffer: Value::NIL, anchor: None, bias: Bias::Left,
        })));
        process.set(Slot::Name, unique);
        process.set(Slot::Status, self.sym("run"));
        process.set(Slot::Mark, mark);
        process.set(Slot::Filter, self.sym("internal-default-process-filter"));
        process.set(Slot::Sentinel, self.sym("internal-default-process-sentinel"));
        let value = self.vm.allocate_object(data::Object::Process(Box::new(process)));
        let pair = self.cons(unique, value);
        let alist = self.hidden_value(ALIST);
        let alist = self.cons(pair, alist);
        self.set_hidden_value(ALIST, alist);
        Ok(value)
    }
    /// remove_process: drop PROC from Vprocess_alist and deactivate it.
    pub(crate) fn remove_process(&mut self, proc: Value) -> Out {
        self.process(proc)?;
        let mut kept = Vec::new();
        let mut tail = self.hidden_value(ALIST);
        let mut removed = false;
        while let Some((pair, rest)) = self.pair(tail) {
            if !removed && self.pair(pair).is_some_and(|(_, p)| p == proc) {
                removed = true;
            } else {
                kept.push(pair);
            }
            tail = rest;
        }
        let alist = self.list(&kept);
        self.set_hidden_value(ALIST, alist);
        self.deactivate_process(proc);
        Ok(Value::NIL)
    }
    /// deactivate_process: close every descriptor.
    pub(crate) fn deactivate_process(&mut self, proc: Value) {
        let serial = self.process_mut(proc).serial;
        self.vm.subprocesses.remove(serial);
        self.process_mut(proc).serial = 0;
    }
    fn attach_channel(&mut self, proc: Value, channel: Channel) {
        let serial = self.vm.subprocesses.insert(channel);
        self.process_mut(proc).serial = serial;
    }

    /// The process `kind' predicates of GNU: NETCONN1_P, PIPECONN1_P ...
    fn process_type_is(&mut self, proc: Value, name: &str) -> bool {
        let wanted = self.intern(name);
        self.process_if(proc).is_some_and(|p| p.get(Slot::Type) == wanted)
    }
    pub(crate) fn connection_p(&mut self, proc: Value) -> bool {
        self.process_type_is(proc, "network") || self.process_type_is(proc, "serial") || self.process_type_is(proc, "pipe")
    }

    /// update_status.
    pub(crate) fn update_status(&mut self, proc: Value) {
        if let Some(raw) = self.process_mut(proc).raw_status.take() {
            let status = self.status_convert(raw);
            self.process_mut(proc).set(Slot::Status, status);
        }
    }
    /// status_convert.
    fn status_convert(&mut self, w: i32) -> Value {
        if libc::WIFSTOPPED(w) {
            let stop = self.sym("stop");
            self.list(&[stop, fixnum(libc::WSTOPSIG(w).into())])
        } else if libc::WIFEXITED(w) {
            let exit = self.sym("exit");
            self.list(&[exit, fixnum(libc::WEXITSTATUS(w).into())])
        } else if libc::WIFSIGNALED(w) {
            let signal = self.sym("signal");
            let core = if libc::WCOREDUMP(w) { Value::TRUE } else { Value::NIL };
            let tail = self.cons(fixnum(libc::WTERMSIG(w).into()), core);
            self.cons(signal, tail)
        } else {
            self.sym("run")
        }
    }
    /// decode_status: (SYMBOL CODE CORE).
    fn decode_status(&mut self, status: Value) -> (Value, Value, bool) {
        let mut l = status;
        if let Some((head, _)) = self.pair(l)
            && head == self.intern("connect")
        {
            l = head;
        }
        match self.pair(l) {
            None => (l, fixnum(0), false),
            Some((symbol, tail)) => {
                let (code, rest) = self.pair(tail).unwrap_or((Value::NIL, Value::NIL));
                (symbol, code, rest != Value::NIL)
            }
        }
    }
    /// status_message.
    pub(crate) fn status_message(&mut self, proc: Value) -> Out {
        let status = self.process(proc)?.get(Slot::Status);
        let (symbol, code, core) = self.decode_status(status);
        let name = self.symbol_name(symbol)?;
        let suffix = if core { " (core dumped)\n" } else { "\n" };
        let text = match name.as_str() {
            "signal" | "stop" => {
                let signal = code.integer_value().unwrap_or(0) as i32;
                let mut text = sp::signal_description(signal).unwrap_or_else(|| "unknown".into());
                if let Some(first) = text.chars().next() {
                    let lower: String = first.to_lowercase().collect();
                    text.replace_range(..first.len_utf8(), &lower);
                }
                // Decoded by locale-coding-system: multibyte.
                return Ok(self.make_string((text + suffix).into_bytes(), true));
            }
            "exit" => {
                let code = code.integer_value().unwrap_or(0);
                if self.process_type_is(proc, "network") {
                    if code == 0 { "deleted\n".into() } else { "connection broken by remote peer\n".into() }
                } else if code == 0 {
                    "finished\n".into()
                } else {
                    format!("exited abnormally with code {code}{suffix}")
                }
            }
            "failed" => {
                let code = if let Some(n) = code.integer_value() { n.to_string() } else { self.string_text(code).unwrap_or_default() };
                format!("failed with code {code}\n")
            }
            _ => return Ok(self.make_string(name.into_bytes(), false)),
        };
        let multibyte = !text.is_ascii();
        Ok(self.make_string(text.into_bytes(), multibyte))
    }
    /// Fprocess_status, for a process object.
    pub(crate) fn process_status(&mut self, proc: Value) -> Out {
        self.update_status(proc);
        let status = self.process(proc)?.get(Slot::Status);
        let mut symbol = self.pair(status).map_or(status, |(head, _)| head);
        if self.connection_p(proc) {
            let command = self.process(proc)?.get(Slot::Command);
            if symbol == self.intern("exit") {
                symbol = self.sym("closed");
            } else if command == Value::TRUE {
                symbol = self.sym("stop");
            } else if symbol == self.intern("run") {
                symbol = self.sym("open");
            }
        }
        Ok(symbol)
    }
    pub(crate) fn process_exit_status(&mut self, proc: Value) -> Out {
        self.process(proc)?;
        self.update_status(proc);
        let status = self.process(proc)?.get(Slot::Status);
        Ok(match self.pair(status) {
            Some((_, tail)) => self.pair(tail).map_or(Value::NIL, |(code, _)| code),
            None => fixnum(0),
        })
    }
    pub(crate) fn process_numeric(&mut self, proc: Value, field: &str) -> Out {
        let p = self.process(proc)?;
        Ok(match field {
            "pid" => if p.pid != 0 { fixnum(p.pid) } else { Value::NIL },
            "kill-without-query" => if p.kill_without_query { Value::TRUE } else { Value::NIL },
            "inherit-coding" => if p.inherit_coding { Value::TRUE } else { Value::NIL },
            "pty-in" => if p.pty_in { Value::TRUE } else { Value::NIL },
            "pty-out" => if p.pty_out { Value::TRUE } else { Value::NIL },
            "alive" => if p.alive { Value::TRUE } else { Value::NIL },
            "nbytes-read" => fixnum(p.nbytes_read as i64),
            "infd" => if self.process_infd(proc) { Value::TRUE } else { Value::NIL },
            "outfd" => if self.channel(proc).is_some_and(|c| c.outfd.is_some()) { Value::TRUE } else { Value::NIL },
            "raw-status-new" => if p.raw_status.is_some() { Value::TRUE } else { Value::NIL },
            _ => return Err(Fault::Error(format!("unknown process field {field}"))),
        })
    }
    pub(crate) fn set_process_flag(&mut self, proc: Value, field: &str, on: bool) -> Result<()> {
        self.process(proc)?;
        let p = self.process_mut(proc);
        match field {
            "kill-without-query" => p.kill_without_query = on,
            "inherit-coding" => p.inherit_coding = on,
            "pty-in" => p.pty_in = on,
            "pty-out" => p.pty_out = on,
            _ => return Err(Fault::Error(format!("unknown process flag {field}"))),
        }
        Ok(())
    }
    /// add_process_read_fd / delete_read_fd.
    pub(crate) fn set_process_reading(&mut self, proc: Value, on: bool) {
        if let Some(c) = self.channel_mut(proc) {
            c.reading = on;
        }
    }

    /// GNU p->tick = ++process_tick.
    pub(crate) fn process_tick(&mut self, proc: Value) {
        self.vm.subprocesses.tick += 1;
        let tick = self.vm.subprocesses.tick;
        self.process_mut(proc).tick = tick;
    }
    /// status_notify's per-process test: true, and acknowledged, when PROC's
    /// tick moved.
    pub(crate) fn process_changed(&mut self, proc: Value) -> bool {
        let p = self.process_mut(proc);
        if p.tick == p.update_tick {
            return false;
        }
        p.update_tick = p.tick;
        true
    }
    pub(crate) fn process_ticks_pending(&self) -> bool {
        self.vm.subprocesses.tick != self.vm.subprocesses.update_tick
    }
    pub(crate) fn process_ticks_acknowledge(&mut self) {
        self.vm.subprocesses.update_tick = self.vm.subprocesses.tick;
    }
    /// status_notify after the drain: update the status, deactivate or
    /// remove a finished process, and return the sentinel's message.
    pub(crate) fn process_status_finish(&mut self, proc: Value) -> Out {
        self.update_status(proc);
        let message = self.status_message(proc)?;
        let status = self.process(proc)?.get(Slot::Status);
        let symbol = self.pair(status).map_or(status, |(head, _)| head);
        if [self.intern("signal"), self.intern("exit"), self.intern("closed")].contains(&symbol) {
            let symbol = self.intern("delete-exited-processes");
            if self.variable_value(symbol)?.unwrap_or(Value::NIL) != Value::NIL {
                self.remove_process(proc)?;
            } else {
                self.deactivate_process(proc);
            }
        }
        let p = self.process_mut(proc);
        p.update_tick = p.tick;
        Ok(message)
    }

    /// handle_child_signal, polled: record every live child's status change.
    pub(crate) fn reap_children(&mut self) {
        self.vm.subprocesses.reap_deleted();
        for proc in self.process_values() {
            let Some(p) = self.process_if(proc) else { continue };
            if !p.alive || p.pid <= 0 {
                continue;
            }
            let Some(status) = sp::wait_status(p.pid as i32, libc::WUNTRACED | libc::WCONTINUED) else { continue };
            self.process_tick(proc);
            let p = self.process_mut(proc);
            p.raw_status = Some(status);
            if libc::WIFSIGNALED(status) || libc::WIFEXITED(status) {
                p.alive = false;
                self.set_process_reading(proc, false);
                if let Some(c) = self.channel_mut(proc) {
                    c.pidfd = None;
                }
            }
        }
    }

    /// One pass of wait_reading_process_output's select: reap, poll for at
    /// most SECONDS (nil: forever), flush queued writes, and return the
    /// process whose channel is readable (round robin), (connect PROC ERRNO
    /// REMOTE LOCAL) for a finished :nowait connection, t when something
    /// else woke the wait or a status changed, or nil on timeout.
    pub(crate) fn process_select(&mut self, wait_proc: Value, just_this: bool, seconds: Option<f64>) -> Out {
        // Reaping closes a child's pidfd and stops reading it, so nothing
        // would wake the poll for its status change: hand that change to
        // status_notify before blocking, as GNU's SIGCHLD wakeup does.
        let tick = self.vm.subprocesses.tick;
        self.reap_children();
        if self.vm.subprocesses.tick != tick {
            return Ok(Value::TRUE);
        }
        // A :nowait connection refused at creation reports as soon as the
        // wait looks.
        for proc in self.process_values() {
            if wait_proc != Value::NIL && just_this && proc != wait_proc {
                continue;
            }
            if let Some(errno) = self.channel_mut(proc).and_then(|c| c.connect.take()) {
                let connect = self.sym("connect");
                return Ok(self.list(&[connect, proc, fixnum(errno.into()), Value::NIL, Value::NIL]));
            }
        }
        let mut fds: Vec<(RawFd, i16)> = Vec::new();
        let mut owners: Vec<(Value, Wake)> = Vec::new();
        let candidates = if wait_proc != Value::NIL && just_this { vec![wait_proc] } else { self.process_values() };
        for proc in candidates {
            let Some(channel) = self.channel(proc) else { continue };
            let connecting = channel.connecting.is_some();
            if connecting && let Some(fd) = &channel.infd {
                fds.push((fd.as_raw_fd(), libc::POLLOUT));
                owners.push((proc, Wake::Connect));
            }
            if !connecting && (channel.reading || (just_this && proc == wait_proc)) && let Some(fd) = &channel.infd {
                fds.push((fd.as_raw_fd(), libc::POLLIN));
                owners.push((proc, Wake::Read));
            }
            if !channel.pending.is_empty() && let Some(fd) = &channel.outfd {
                fds.push((fd.as_raw_fd(), libc::POLLOUT));
                owners.push((proc, Wake::Write));
            }
            if let Some(fd) = &channel.pidfd {
                fds.push((fd.as_raw_fd(), libc::POLLIN));
                owners.push((proc, Wake::Child));
            }
        }
        if wait_proc != Value::NIL && just_this && !self.process_infd(wait_proc) {
            return Ok(self.sym("terminated"));
        }
        let timeout = seconds.map(|s| std::time::Duration::from_secs_f64(s.clamp(0.0, 1e7)));
        let revents = sp::poll(&fds, timeout).map_err(|e| self.file_errno("Failed select", &e))?;
        if revents.iter().all(|&r| r == 0) {
            return Ok(Value::NIL);
        }
        self.reap_children();
        let mut readable = Vec::new();
        let mut connected = Vec::new();
        for (&r, &(proc, wake)) in revents.iter().zip(&owners) {
            if r == 0 {
                continue;
            }
            match wake {
                Wake::Read => readable.push(proc),
                Wake::Write => self.process_flush(proc)?,
                Wake::Connect => connected.push(proc),
                Wake::Child => {}
            }
        }
        for proc in connected {
            if let Some(ready) = self.process_connect_poll(proc) {
                return Ok(ready);
            }
        }
        if readable.is_empty() {
            return Ok(Value::TRUE);
        }
        let last = self.vm.subprocesses.last_read;
        let prioritize = {
            let symbol = self.intern("process-prioritize-lower-fds");
            self.variable_value(symbol)?.unwrap_or(Value::NIL) != Value::NIL
        };
        let serial = |cx: &Self, p: Value| cx.process_if(p).map_or(0, |p| p.serial);
        let chosen = if prioritize { None } else { readable.iter().copied().find(|&p| serial(self, p) > last) }
            .unwrap_or(readable[0]);
        self.vm.subprocesses.last_read = serial(self, chosen);
        Ok(chosen)
    }
    /// The wait's handling of PROC's writable NON_BLOCKING_CONNECT_FD:
    /// SO_ERROR, and on failure GNU's retry of the next address. Returns
    /// (connect PROC ERRNO REMOTE LOCAL) once the connection is done, with
    /// the connected addresses when ERRNO is 0, or None while the next
    /// address is connecting.
    fn process_connect_poll(&mut self, proc: Value) -> Option<Value> {
        let channel = self.channel_mut(proc)?;
        let fd = channel.infd.as_ref()?.as_raw_fd();
        // Borrowed, never closed here: the channel owns the descriptor.
        let stream = std::mem::ManuallyDrop::new(unsafe { <std::net::TcpStream as std::os::fd::FromRawFd>::from_raw_fd(fd) });
        let mut errno = match stream.take_error() {
            Ok(None) => 0,
            Ok(Some(error)) | Err(error) => error.raw_os_error().unwrap_or(libc::ECONNREFUSED),
        };
        while errno != 0 && let Some(next) = channel.connecting.as_mut().filter(|rest| !rest.is_empty()).map(|rest| rest.remove(0)) {
            match sp::connect_nonblocking(next).and_then(|fd| Ok((sp::dup(&fd)?, fd))) {
                Ok((outfd, infd)) => {
                    channel.infd = Some(infd);
                    channel.outfd = Some(outfd);
                    return None;
                }
                Err(error) => errno = error.raw_os_error().unwrap_or(libc::ECONNREFUSED),
            }
        }
        channel.connecting = None;
        let addresses = (errno == 0).then(|| (stream.peer_addr().ok(), stream.local_addr().ok()));
        let (remote, local) = addresses.unwrap_or((None, None));
        let remote = remote.map_or(Value::NIL, |a| self.sockaddr_vector(a));
        let local = local.map_or(Value::NIL, |a| self.sockaddr_vector(a));
        let connect = self.sym("connect");
        Some(self.list(&[connect, proc, fixnum(errno.into()), remote, local]))
    }
    /// A zero-timeout poll of every reading channel: is any input waiting?
    pub(crate) fn process_input_pending(&mut self) -> Result<bool> {
        let fds: Vec<(RawFd, i16)> = self.process_values().into_iter()
            .filter_map(|p| self.channel(p).filter(|c| c.reading).and_then(|c| c.infd.as_ref()).map(|fd| (fd.as_raw_fd(), libc::POLLIN)))
            .collect();
        let revents = sp::poll(&fds, Some(std::time::Duration::ZERO)).map_err(|e| self.file_errno("Failed select", &e))?;
        Ok(revents.iter().any(|&r| r != 0))
    }

    /// read_process_output's read and decode: (NBYTES . TEXT), NBYTES < 0
    /// with TEXT the errno when the read failed, NBYTES = 0 at end of file.
    pub(crate) fn process_read(&mut self, proc: Value) -> Out {
        let readmax = self.process(proc)?.readmax;
        let Some(channel) = self.channel_mut(proc) else { return Ok(self.cons(fixnum(-1), fixnum(libc::EBADF.into()))) };
        let Some(fd) = channel.infd.as_ref().map(|f| f.as_raw_fd()) else { return Ok(self.cons(fixnum(-1), fixnum(libc::EBADF.into()))) };
        let mut buf = vec![0; readmax];
        let nbytes = match sp::read(fd, &mut buf) {
            Ok(n) => n,
            Err(error) => {
                let errno = error.raw_os_error().unwrap_or(libc::EIO);
                return Ok(self.cons(fixnum(-1), fixnum(errno.into())));
            }
        };
        buf.truncate(nbytes);
        let channel = self.channel_mut(proc).expect("read channel");
        let mut bytes = std::mem::take(&mut channel.carryover);
        let carried = bytes.len();
        if nbytes == 0 {
            if channel.last_block {
                return Ok(self.cons(fixnum(0), Value::NIL));
            }
            channel.last_block = true;
        }
        let last = channel.last_block;
        bytes.extend_from_slice(&buf);
        self.process_mut(proc).nbytes_read += nbytes as u64;
        let text = self.process_decode(proc, bytes, last)?;
        Ok(self.cons(fixnum((nbytes + carried) as i64), text))
    }
    /// decode_coding_c_string into a string with PROC's decoding coding
    /// system, holding as carryover unless LAST an incomplete UTF-8 tail or
    /// the CR a DOS end of line may continue; then
    /// read_process_output_set_last_coding_system.
    fn process_decode(&mut self, proc: Value, mut bytes: Vec<u8>, last: bool) -> Out {
        let coding = self.process(proc)?.get(Slot::Decode);
        if !last {
            let mut keep = if self.coding_utf8_family(coding)? { incomplete_utf8_tail(&bytes) } else { 0 };
            if keep == 0 && bytes.last() == Some(&b'\r') && self.coding_eol_dos(coding)? {
                keep = 1;
            }
            let tail = bytes.split_off(bytes.len() - keep);
            self.channel_mut(proc).expect("decoding channel").carryover = tail;
        }
        let text = self.decode_output(bytes, coding)?;
        let last_used = self.intern("last-coding-system-used");
        let used = self.variable_value(last_used)?.unwrap_or(Value::NIL);
        if self.process(proc)?.get(Slot::Decode) != used {
            self.process_mut(proc).set(Slot::Decode, used);
            if self.process(proc)?.get(Slot::Encode) == Value::NIL && self.channel(proc).is_some_and(|c| c.outfd.is_some()) {
                self.process_mut(proc).set(Slot::Encode, used);
            }
        }
        Ok(text)
    }
    fn coding_spec_checked(&mut self, coding: Value) -> Result<Value> {
        let coding = if coding == Value::NIL { self.sym("no-conversion") } else { coding };
        let spec = self.coding_spec(coding)?;
        if spec == Value::NIL {
            return Err(Fault::Wrong("coding-system-p", coding));
        }
        Ok(spec)
    }
    /// Whether CODING decodes UTF-8 (or detects it, for `undecided').
    fn coding_utf8_family(&mut self, coding: Value) -> Result<bool> {
        let spec = self.coding_spec_checked(coding)?;
        let attrs = self.vector_get(spec, fixnum(0))?;
        let kind = self.vector_get(attrs, fixnum(3))?;
        Ok(kind == self.intern("undecided") || kind == self.intern("utf-8"))
    }
    /// Whether CODING's end of line is DOS (not undetermined: GNU detects
    /// that from what has arrived).
    fn coding_eol_dos(&mut self, coding: Value) -> Result<bool> {
        let spec = self.coding_spec_checked(coding)?;
        let eol = self.vector_get(spec, fixnum(2))?;
        Ok(eol == self.intern("dos"))
    }
    /// decode_coding_c_string of BYTES with CODING (nil: no-conversion),
    /// setting `last-coding-system-used'. `undecided' detects ASCII and
    /// UTF-8 only (VM2-PROC2); other bytes are refused, not guessed.
    pub(crate) fn decode_output(&mut self, bytes: Vec<u8>, coding: Value) -> Out {
        let coding = if coding == Value::NIL { self.sym("no-conversion") } else { coding };
        let spec = self.coding_spec_checked(coding)?;
        let attrs = self.vector_get(spec, fixnum(0))?;
        let kind = self.vector_get(attrs, fixnum(3))?;
        let eol = self.vector_get(spec, fixnum(2))?;
        let last_used = self.intern("last-coding-system-used");
        let name = if kind == self.intern("undecided") {
            if std::str::from_utf8(&bytes).is_err() {
                return Err(Fault::Error("VM2 cannot detect the coding system of non-UTF-8 process output".into()));
            }
            if bytes.is_ascii() && !bytes.contains(&b'\r') {
                // Nothing to detect: the text, and the coding system stays.
                let text = self.make_string(bytes, true);
                self.variable_set(last_used, coding)?;
                return Ok(text);
            }
            let suffix = if eol == self.intern("unix") { "-unix" } else if eol == self.intern("dos") { "-dos" } else if eol == self.intern("mac") { "-mac" } else { "" };
            self.sym(&format!("utf-8{suffix}"))
        } else {
            coding
        };
        let raw = self.make_string(bytes, false);
        self.coding_convert_string(raw, name, Value::NIL, Value::NIL, false)
    }
    /// wait_reading_process_output's handling of a read of NREAD bytes
    /// (ERRNO when negative) from PROC's channel.
    pub(crate) fn process_read_done(&mut self, proc: Value, nread: i64, errno: i64) -> Result<()> {
        if nread > 0 || (nread < 0 && (errno == i64::from(libc::EAGAIN) || errno == i64::from(libc::EINTR))) {
            return Ok(());
        }
        let network = self.process_type_is(proc, "network") || self.process_type_is(proc, "serial");
        let pipe = self.process_type_is(proc, "pipe");
        let run = self.intern("run");
        if nread < 0 && errno == i64::from(libc::EIO) {
            self.set_process_reading(proc, false);
            if self.process(proc)?.pid == -2 {
                self.process_tick(proc);
                let failed = self.sym("failed");
                self.process_mut(proc).set(Slot::Status, failed);
            }
        } else if nread == 0 && !network && !pipe {
            self.set_process_reading(proc, false);
        } else if nread == 0 && pipe {
            self.process_tick(proc);
            self.deactivate_process(proc);
            if self.process(proc)?.get(Slot::Status) == run {
                let exit = self.sym("exit");
                let status = self.list(&[exit, fixnum(0)]);
                self.process_mut(proc).set(Slot::Status, status);
            }
        } else {
            self.process_tick(proc);
            self.deactivate_process(proc);
            self.update_status(proc);
            if self.process(proc)?.get(Slot::Status) == run {
                let exit = self.sym("exit");
                let status = self.list(&[exit, fixnum(256)]);
                self.process_mut(proc).set(Slot::Status, status);
            }
        }
        Ok(())
    }

    /// send_process's writes of BYTES (nil: only the queue): t when all was
    /// written, nil when the rest is queued because the channel is full.
    pub(crate) fn process_send(&mut self, proc: Value, bytes: Option<Vec<u8>>) -> Out {
        self.update_status(proc);
        let status = self.process(proc)?.get(Slot::Status);
        if status != self.intern("run") {
            let name = self.string_text(self.process(proc)?.get(Slot::Name))?;
            let message = self.status_message(proc)?;
            let message = self.string_text(message)?;
            return Err(Fault::Error(format!("Process {name} not running: {message}")));
        }
        let name = self.string_text(self.process(proc)?.get(Slot::Name))?;
        let Some(channel) = self.channel_mut(proc).filter(|c| c.outfd.is_some()) else {
            return Err(Fault::Error(format!("Output file descriptor of {name} is closed")));
        };
        if let Some(bytes) = bytes {
            channel.pending.extend_from_slice(&bytes);
        }
        self.process_flush(proc)?;
        Ok(if self.channel(proc).is_some_and(|c| c.pending.is_empty()) { Value::TRUE } else { Value::NIL })
    }
    fn process_flush(&mut self, proc: Value) -> Result<()> {
        let Some(channel) = self.channel_mut(proc) else { return Ok(()) };
        let Some(fd) = channel.outfd.as_ref().map(|f| f.as_raw_fd()) else { return Ok(()) };
        while !channel.pending.is_empty() {
            match sp::write(fd, &channel.pending) {
                Ok(0) => break,
                Ok(n) => { channel.pending.drain(..n); }
                Err(error) if error.raw_os_error() == Some(libc::EPIPE) => {
                    channel.outfd = None;
                    channel.pending.clear();
                    let name = self.string_text(self.process(proc)?.get(Slot::Name))?;
                    return Err(Fault::Error(format!("Process {name} no longer connected to pipe; closed it")));
                }
                Err(error) => {
                    channel.pending.clear();
                    return Err(self.file_errno_value("Writing to process", &error, proc));
                }
            }
        }
        Ok(())
    }
    /// Fprocess_send_eof's channel work, after the status check.
    pub(crate) fn process_send_eof(&mut self, proc: Value) -> Result<()> {
        if self.process(proc)?.pty_in {
            let eof = self.channel(proc).and_then(|c| c.outfd.as_ref()).map_or(4, |fd| sp::tty_eof_char(fd.as_raw_fd()));
            self.process_send(proc, Some(vec![eof]))?;
            return Ok(());
        }
        let network = self.process_type_is(proc, "network");
        let null = std::fs::OpenOptions::new().write(true).open("/dev/null")
            .map_err(|e| self.file_errno("Opening null device", &e))?;
        if let Some(channel) = self.channel_mut(proc) {
            if let Some(fd) = &channel.outfd
                && network
            {
                unsafe { libc::shutdown(fd.as_raw_fd(), libc::SHUT_WR) };
            }
            channel.pending.clear();
            channel.outfd = Some(OwnedFd::from(null));
        }
        Ok(())
    }

    /// report_file_errno: (file-error STRING STRERROR) or file-missing.
    pub(crate) fn file_errno(&mut self, string: &str, error: &std::io::Error) -> Fault {
        self.file_errno_data(string, error, Value::NIL)
    }
    fn file_errno_value(&mut self, string: &str, error: &std::io::Error, data: Value) -> Fault {
        let data = self.cons(data, Value::NIL);
        self.file_errno_data(string, error, data)
    }
    /// report_file_errno with DATA, a list spliced after the error text.
    pub(crate) fn file_errno_data(&mut self, string: &str, error: &std::io::Error, data: Value) -> Fault {
        let errno = error.raw_os_error().unwrap_or(0);
        let condition = match errno {
            libc::ENOENT => "file-missing",
            libc::EEXIST => "file-already-exists",
            libc::EACCES => "permission-denied",
            _ => "file-error",
        };
        let text = strerror(errno);
        let message = self.string(string);
        let text = self.make_string(text.clone().into_bytes(), !text.is_ascii());
        let tail = self.cons(text, data);
        let data = self.cons(message, tail);
        Fault::Signal(self.intern(condition), data)
    }

    /// create_process: spawn ARGV (encoded strings) with ENV in DIR, wiring
    /// pipes or a pty per PROC's pty flags and STDERRPROC's pipe.
    pub(crate) fn process_spawn(&mut self, proc: Value, argv: &[Vec<u8>], env: &[Vec<u8>], dir: &[u8], stderrproc: Value) -> Out {
        let (pty_in, pty_out) = { let p = self.process(proc)?; (p.pty_in, p.pty_out) };
        let mut pty = None;
        if pty_in || pty_out {
            match sp::allocate_pty() {
                Ok((master, name)) => {
                    let tty = sp::open_tty(&name).map_err(|e| self.file_errno("Opening pty", &e))?;
                    pty = Some((master, tty, name));
                }
                // allocate_pty failing leaves pipes for both directions.
                Err(_) => {}
            }
        }
        let (pty_in, pty_out) = if pty.is_some() { (pty_in, pty_out) } else { (false, false) };
        let mut channel = Channel::default();
        let (forkin, forkout);
        let mut child_ends: Vec<OwnedFd> = Vec::new();
        if let (true, Some((master, tty, _))) = (pty_in, &pty) {
            forkin = tty.as_raw_fd();
            channel.outfd = Some(sp::dup(master).map_err(|e| self.file_errno("Creating pipe", &e))?);
        } else {
            let (read, write) = sp::pipe().map_err(|e| self.file_errno("Creating pipe", &e))?;
            forkin = read.as_raw_fd();
            child_ends.push(read);
            channel.outfd = Some(write);
        }
        if let (true, Some((master, tty, _))) = (pty_out, &pty) {
            forkout = tty.as_raw_fd();
            channel.infd = Some(sp::dup(master).map_err(|e| self.file_errno("Creating pipe", &e))?);
        } else {
            let (read, write) = sp::pipe().map_err(|e| self.file_errno("Creating pipe", &e))?;
            forkout = write.as_raw_fd();
            child_ends.push(write);
            channel.infd = Some(read);
        }
        let mut forkerr = forkout;
        if stderrproc != Value::NIL {
            if let Some(c) = self.channel_mut(stderrproc) {
                c.outfd = None;
                c.child_stdin = None;
                if let Some(fd) = &c.child_stdout {
                    forkerr = fd.as_raw_fd();
                }
            }
        }
        for fd in [&channel.infd, &channel.outfd].into_iter().flatten() {
            sp::set_nonblocking(fd.as_raw_fd()).map_err(|e| self.file_errno("Creating pipe", &e))?;
        }
        if pty_out && let Some((_, tty, _)) = &pty {
            sp::child_setup_tty(tty.as_raw_fd());
        }
        let run = self.sym("run");
        self.process_mut(proc).set(Slot::Status, run);
        let spawn = sp::Spawn { argv, env, dir, stdin: forkin, stdout: forkout, stderr: forkerr, controlling_tty: pty_in };
        let pid = sp::spawn(&spawn).map_err(|e| self.file_errno("Doing vfork", &e))?;
        drop(child_ends);
        channel.pidfd = sp::pidfd_open(pid);
        channel.pid = pid;
        let filter = self.process(proc)?.get(Slot::Filter);
        let command = self.process(proc)?.get(Slot::Command);
        channel.reading = filter != Value::TRUE && command != Value::TRUE;
        if let Some((_, _, name)) = pty {
            let name = self.make_string(name, false);
            self.process_mut(proc).set(Slot::TtyName, name);
        }
        let p = self.process_mut(proc);
        p.pid = i64::from(pid);
        p.alive = true;
        p.pty_in = pty_in;
        p.pty_out = pty_out;
        self.attach_channel(proc, channel);
        if stderrproc != Value::NIL && let Some(c) = self.channel_mut(stderrproc) {
            c.child_stdout = None;
        }
        Ok(proc)
    }
    /// create_pty: a pty-only process (make-process without :command).
    pub(crate) fn process_pty_only(&mut self, proc: Value) -> Out {
        let (pty_in, pty_out) = { let p = self.process(proc)?; (p.pty_in, p.pty_out) };
        if (pty_in || pty_out) && let Ok((master, name)) = sp::allocate_pty() {
            let tty = sp::open_tty(&name).map_err(|e| self.file_errno("Opening pty", &e))?;
            sp::set_nonblocking(master.as_raw_fd()).map_err(|e| self.file_errno("Opening pty", &e))?;
            let mut channel = Channel::default();
            channel.outfd = Some(sp::dup(&master).map_err(|e| self.file_errno("Opening pty", &e))?);
            channel.infd = Some(master);
            channel.child_stdin = Some(tty);
            channel.reading = self.process(proc)?.get(Slot::Filter) != Value::TRUE;
            let name = self.make_string(name, false);
            let run = self.sym("run");
            let p = self.process_mut(proc);
            p.pty_in = true;
            p.pty_out = true;
            p.set(Slot::Status, run);
            p.set(Slot::TtyName, name);
            self.attach_channel(proc, channel);
        }
        self.process_mut(proc).pid = -2;
        Ok(proc)
    }
    /// make-pipe-process's two pipes.
    pub(crate) fn process_pipe(&mut self, proc: Value) -> Out {
        let (stdin_read, outfd) = sp::pipe().map_err(|e| self.file_errno("Creating pipe", &e))?;
        let (infd, stdout_write) = sp::pipe().map_err(|e| self.file_errno("Creating pipe", &e))?;
        for fd in [&infd, &outfd] {
            sp::set_nonblocking(fd.as_raw_fd()).map_err(|e| self.file_errno("Creating pipe", &e))?;
        }
        let channel = Channel { infd: Some(infd), outfd: Some(outfd), child_stdin: Some(stdin_read), child_stdout: Some(stdout_write), ..Channel::default() };
        self.attach_channel(proc, channel);
        Ok(proc)
    }
    /// The pid of a live real process (process_send_signal's checks).
    fn signal_target(&mut self, proc: Value) -> Result<()> {
        let name = self.string_text(self.process(proc)?.get(Slot::Name))?;
        if !self.process_type_is(proc, "real") {
            return Err(Fault::Error(format!("Process {name} is not a subprocess")));
        }
        if !self.process_infd(proc) {
            return Err(Fault::Error(format!("Process {name} is not active")));
        }
        Ok(())
    }
    /// process_send_signal. CURRENT_GROUP is nil, t or `lambda'. Returns t
    /// when SIGCONT made the status `run' (the caller runs status_notify).
    pub(crate) fn process_send_signal(&mut self, proc: Value, signo: i32, current_group: Value) -> Out {
        self.signal_target(proc)?;
        let (pid, pty_in) = { let p = self.process(proc)?; (p.pid as i32, p.pty_in) };
        let current_group = if pty_in { current_group } else { Value::NIL };
        let mut gid = pid;
        if current_group != Value::NIL {
            let fd = self.channel(proc).and_then(|c| c.infd.as_ref()).map(|f| f.as_raw_fd());
            if let Some(fd) = fd {
                let mut t: libc::termios = unsafe { std::mem::zeroed() };
                unsafe { libc::tcgetattr(fd, &mut t) };
                let ch = match signo {
                    libc::SIGINT => Some(t.c_cc[libc::VINTR]),
                    libc::SIGQUIT => Some(t.c_cc[libc::VQUIT]),
                    libc::SIGTSTP => Some(t.c_cc[libc::VSUSP]),
                    _ => None,
                };
                if let Some(ch) = ch.filter(|&c| c != 0xff && c != 0) {
                    self.process_send(proc, Some(vec![ch]))?;
                    return Ok(Value::NIL);
                }
                gid = sp::tty_pgrp(fd).unwrap_or(pid);
            }
            if current_group == self.intern("lambda") && gid == pid {
                return Ok(Value::NIL);
            }
        }
        let mut continued = Value::NIL;
        if signo == libc::SIGCONT {
            let run = self.sym("run");
            let p = self.process_mut(proc);
            p.raw_status = None;
            p.set(Slot::Status, run);
            self.process_tick(proc);
            continued = Value::TRUE;
        }
        if self.process(proc)?.alive {
            let _ = sp::kill(-gid, signo);
        }
        Ok(continued)
    }
    /// Fprocess_running_child_p.
    pub(crate) fn process_running_child(&mut self, proc: Value) -> Out {
        self.signal_target(proc)?;
        let pid = self.process(proc)?.pid as i32;
        let gid = self.channel(proc).and_then(|c| c.infd.as_ref()).and_then(|fd| sp::tty_pgrp(fd.as_raw_fd())).unwrap_or(-1);
        Ok(if gid == pid { Value::NIL } else if gid != -1 { fixnum(gid.into()) } else { Value::TRUE })
    }
    /// delete-process's kill of a real process: record_kill_process, then
    /// GNU's status (signal 9) unless it already ended.
    pub(crate) fn process_kill_for_delete(&mut self, proc: Value) -> Result<bool> {
        self.process_mut(proc).raw_status = None;
        let (alive, pid) = { let p = self.process(proc)?; (p.alive, p.pid as i32) };
        if alive {
            let p = self.process_mut(proc);
            p.alive = false;
            let _ = sp::kill(-pid, libc::SIGKILL);
            self.vm.subprocesses.deleted.push((pid, None));
        }
        if !self.process_infd(proc) {
            return Ok(false);
        }
        let status = self.process(proc)?.get(Slot::Status);
        let symbol = self.pair(status).map_or(status, |(head, _)| head);
        if symbol != self.intern("signal") && symbol != self.intern("exit") {
            let signal = self.sym("signal");
            let status = self.list(&[signal, fixnum(libc::SIGKILL.into())]);
            self.process_mut(proc).set(Slot::Status, status);
        }
        self.process_tick(proc);
        Ok(true)
    }
    /// Fset_process_window_size.
    pub(crate) fn process_window_size(&mut self, proc: Value, height: u16, width: u16) -> Out {
        self.process(proc)?;
        if self.process_type_is(proc, "network") {
            return Ok(Value::NIL);
        }
        let Some(fd) = self.channel(proc).and_then(|c| c.infd.as_ref()).map(|f| f.as_raw_fd()) else { return Ok(Value::NIL) };
        let size = libc::winsize { ws_row: height, ws_col: width, ws_xpixel: 0, ws_ypixel: 0 };
        Ok(if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &size) } < 0 { Value::NIL } else { Value::TRUE })
    }

    /// The network half of make-network-process for a stream socket:
    /// listen (SERVER) or connect to HOST:SERVICE in FAMILY. Returns
    /// (LOCAL REMOTE PORT); signals GNU's file-error with CONTACT.
    pub(crate) fn process_network(&mut self, proc: Value, server: bool, nowait: bool, host: String, port: u16, family: Option<bool>, contact: Value) -> Out {
        use std::net::ToSocketAddrs;
        let addrs: Vec<std::net::SocketAddr> = match (host.as_str(), port).to_socket_addrs() {
            Ok(addrs) => addrs.filter(|a| family.is_none_or(|v6| a.is_ipv6() == v6)).collect(),
            Err(_) => Vec::new(),
        };
        if addrs.is_empty() {
            let message = self.string(if server { "make server process failed" } else { "make client process failed" });
            let text = self.string("Name or service not known");
            let tail = self.cons(text, contact);
            let data = self.cons(message, tail);
            return Err(Fault::Signal(self.intern("file-error"), data));
        }
        let mut last_error = None;
        let mut socket: Option<(OwnedFd, std::net::SocketAddr, Option<std::net::SocketAddr>)> = None;
        let mut addrs = addrs.into_iter();
        for addr in addrs.by_ref() {
            let result = if server {
                std::net::TcpListener::bind(addr).and_then(|l| Ok((l.local_addr()?, None, OwnedFd::from(l))))
            } else if nowait {
                // GNU's remote is the address tried; the peer is not known yet.
                sp::connect_nonblocking(addr).map(std::net::TcpStream::from)
                    .and_then(|s| Ok((s.local_addr()?, Some(addr), OwnedFd::from(s))))
            } else {
                std::net::TcpStream::connect(addr).and_then(|s| Ok((s.local_addr()?, Some(s.peer_addr()?), OwnedFd::from(s))))
            };
            match result {
                Ok((local, remote, fd)) => { socket = Some((fd, local, remote)); break; }
                Err(error) => last_error = Some(error),
            }
        }
        let Some((fd, local, remote)) = socket else {
            let error = last_error.expect("an address was tried");
            if nowait {
                // A :nowait connection fails later, in the wait.
                let errno = error.raw_os_error().unwrap_or(libc::ECONNREFUSED);
                self.attach_channel(proc, Channel { connect: Some(errno), ..Channel::default() });
                return Ok(Value::NIL);
            }
            return Err(self.file_errno_data(if server { "make server process failed" } else { "make client process failed" }, &error, contact));
        };
        sp::set_nonblocking(fd.as_raw_fd()).map_err(|e| self.file_errno("Setting socket", &e))?;
        let mut channel = Channel::default();
        channel.outfd = Some(sp::dup(&fd).map_err(|e| self.file_errno("Setting socket", &e))?);
        channel.infd = Some(fd);
        let filter = self.process(proc)?.get(Slot::Filter);
        channel.reading = server || (filter != Value::TRUE && !nowait);
        if nowait {
            channel.connecting = Some(addrs.collect());
        }
        self.attach_channel(proc, channel);
        let local_vec = self.sockaddr_vector(local);
        let remote_vec = remote.map_or(Value::NIL, |r| self.sockaddr_vector(r));
        Ok(self.list(&[local_vec, remote_vec, fixnum(local.port().into())]))
    }
    /// conv_sockaddr_to_lisp: [A B C D PORT] or [X1 ... X8 PORT].
    fn sockaddr_vector(&mut self, addr: std::net::SocketAddr) -> Value {
        let mut items: Vec<Value> = match addr.ip() {
            std::net::IpAddr::V4(ip) => ip.octets().iter().map(|&b| fixnum(b.into())).collect(),
            std::net::IpAddr::V6(ip) => ip.segments().iter().map(|&s| fixnum(s.into())).collect(),
        };
        items.push(fixnum(addr.port().into()));
        self.vector(&items)
    }
    /// server_accept_connection's accept: nil when it would block,
    /// (error . ERRNO), or (CONNECTION HOST SERVICE REMOTE LOCAL), where
    /// CONNECTION is a channel handle for `romacs--process-attach'.
    pub(crate) fn process_accept(&mut self, server: Value) -> Out {
        let Some(fd) = self.channel(server).and_then(|c| c.infd.as_ref()).map(|f| f.as_raw_fd()) else { return Ok(Value::NIL) };
        let listener = std::mem::ManuallyDrop::new(unsafe { <std::net::TcpListener as std::os::fd::FromRawFd>::from_raw_fd(fd) });
        let (stream, remote) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(Value::NIL),
            Err(error) => {
                let error_symbol = self.sym("error");
                return Ok(self.cons(error_symbol, fixnum(error.raw_os_error().unwrap_or(0).into())));
            }
        };
        let local = stream.local_addr().ok();
        let fd = OwnedFd::from(stream);
        sp::set_nonblocking(fd.as_raw_fd()).map_err(|e| self.file_errno("Setting socket", &e))?;
        let mut channel = Channel::default();
        channel.outfd = Some(sp::dup(&fd).map_err(|e| self.file_errno("Setting socket", &e))?);
        channel.infd = Some(fd);
        let serial = self.vm.subprocesses.insert(channel);
        let host = match remote.ip() {
            std::net::IpAddr::V4(ip) => ip.to_string(),
            std::net::IpAddr::V6(ip) => ip.segments().iter().map(|s| format!("{s:x}")).collect::<Vec<_>>().join(":"),
        };
        let host = self.string(&host);
        let remote_vec = self.sockaddr_vector(remote);
        let local_vec = local.map_or(Value::NIL, |l| self.sockaddr_vector(l));
        let v6 = if remote.is_ipv6() { Value::TRUE } else { Value::NIL };
        Ok(self.list(&[fixnum(serial as i64), host, fixnum(remote.port().into()), remote_vec, local_vec, v6]))
    }
    /// Give PROC the accepted channel CONNECTION.
    pub(crate) fn process_attach(&mut self, proc: Value, connection: i64) -> Out {
        self.process(proc)?;
        let serial = u64::try_from(connection).map_err(|_| Fault::Error("invalid connection".into()))?;
        if self.vm.subprocesses.get(serial).is_none() {
            return Err(Fault::Error("invalid connection".into()));
        }
        self.deactivate_process(proc);
        self.process_mut(proc).serial = serial;
        let filter = self.process(proc)?.get(Slot::Filter);
        self.set_process_reading(proc, filter != Value::TRUE);
        Ok(proc)
    }

    /// call_process's child: run ARGV synchronously with INFILE on stdin.
    /// OUTPUT is `pipe' (collect), `discard', or a file; ERROR is t (as
    /// output), nil (discard) or a file. With WAIT nil, record the child
    /// for reaping and return nil; else (STATUS . BYTES), STATUS a wait
    /// status.
    pub(crate) fn call_process_run(&mut self, argv: &[Vec<u8>], env: &[Vec<u8>], dir: &[u8], infile: &[u8],
                                   output: CallOutput, error: CallOutput, wait: bool) -> Out {
        use std::os::unix::ffi::OsStrExt;
        let path = |b: &[u8]| std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b));
        let input = std::fs::File::open(path(infile)).map_err(|e| {
            let name = self.make_string(infile.to_vec(), false);
            self.file_errno_value("Opening process input file", &e, name)
        })?;
        let open_out = |cx: &mut Self, spec: &CallOutput, what: &str| -> Result<Option<OwnedFd>> {
            Ok(match spec {
                CallOutput::Discard => Some(std::fs::OpenOptions::new().write(true).open("/dev/null")
                    .map_err(|e| cx.file_errno("Opening null device", &e))?.into()),
                CallOutput::File(name) => Some(std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(path(name))
                    .map_err(|e| { let file = cx.make_string(name.clone(), false); cx.file_errno_value(what, &e, file) })?.into()),
                CallOutput::Pipe | CallOutput::Same => None,
            })
        };
        let mut reader = None;
        let out_fd = match open_out(self, &output, "Opening process output file")? {
            Some(fd) => fd,
            None => {
                let (read, write) = sp::pipe().map_err(|e| self.file_errno("Creating process pipe", &e))?;
                reader = Some(read);
                write
            }
        };
        let err_fd = match error {
            CallOutput::Same => None,
            ref spec => open_out(self, spec, "Cannot redirect stderr")?,
        };
        let spawn = sp::Spawn {
            argv, env, dir, stdin: input.as_raw_fd(), stdout: out_fd.as_raw_fd(),
            stderr: err_fd.as_ref().map_or(out_fd.as_raw_fd(), |f| f.as_raw_fd()), controlling_tty: false,
        };
        let pid = sp::spawn(&spawn).map_err(|e| self.file_errno("Doing vfork", &e))?;
        drop((input, out_fd, err_fd));
        if !wait {
            self.vm.subprocesses.deleted.push((pid, None));
            return Ok(Value::NIL);
        }
        let mut bytes = Vec::new();
        if let Some(read) = reader {
            let mut buf = vec![0; 64 * 1024];
            loop {
                match sp::read(read.as_raw_fd(), &mut buf) {
                    Ok(0) => break,
                    Ok(n) => bytes.extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
        }
        let status = sp::wait_blocking(pid).map_err(|e| self.file_errno("Waiting for process", &e))?;
        let text = self.make_string(bytes, false);
        Ok(self.cons(fixnum(status.into()), text))
    }
}

/// Why a descriptor is in process_select's poll.
#[derive(Clone, Copy)]
enum Wake {
    Read,
    Write,
    Connect,
    Child,
}

/// Where call_process sends a child stream.
pub(crate) enum CallOutput {
    Pipe,
    Discard,
    Same,
    File(Vec<u8>),
}

/// The length of an incomplete UTF-8 sequence ending BYTES.
fn incomplete_utf8_tail(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(4) {
        let byte = bytes[bytes.len() - back];
        if byte & 0xc0 == 0x80 {
            continue;
        }
        let need = if byte >= 0xf0 { 4 } else if byte >= 0xe0 { 3 } else if byte >= 0xc0 { 2 } else { 1 };
        return if need > back { back } else { 0 };
    }
    0
}

/// emacs_strerror.
pub(crate) fn strerror(errno: i32) -> String {
    let text = unsafe { libc::strerror(errno) };
    if text.is_null() {
        return format!("Unknown error {errno}");
    }
    unsafe { std::ffi::CStr::from_ptr(text) }.to_string_lossy().into_owned()
}
