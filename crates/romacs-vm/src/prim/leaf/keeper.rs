//! The keeper from a leaf (`crate::keeper`).
use super::{Fault, Leaf, Value};
use romacs_keeper::proto::Row;
use romacs_keeper::spawn::{End, Spec, Status};
use romacs_keeper::Event;
use romacs_record::Handle;

type Out = std::result::Result<Value, Fault>;

fn failed(message: String) -> Fault { Fault::Error(message) }

fn natural(value: Value) -> std::result::Result<u64, Fault> {
    value.integer_value().and_then(|n| u64::try_from(n).ok()).ok_or(Fault::Wrong("natnump", value))
}

/// A natural number that fits T: a slot, a signal, a size.
fn small<T: TryFrom<u64>>(value: Value) -> std::result::Result<T, Fault> {
    T::try_from(natural(value)?).map_err(|_| Fault::Wrong("natnump", value))
}

fn handle(value: Value) -> std::result::Result<Handle, Fault> {
    Handle::from_raw(natural(value)?).ok_or(Fault::Wrong("romacs-keeper-handle-p", value))
}

fn integer(value: impl TryInto<i64>) -> Value {
    Value::integer(value.try_into().unwrap_or(i64::MAX)).expect("fits a fixnum")
}

fn path(bytes: &[u8]) -> std::path::PathBuf { std::path::PathBuf::from(String::from_utf8_lossy(bytes).into_owned()) }

impl Leaf<'_> {
    fn link(&mut self) -> std::result::Result<&mut crate::keeper::Link, Fault> {
        self.vm.keeper.as_mut().ok_or_else(|| failed("This session has no keeper attached".into()))
    }

    fn byte_strings(&mut self, list: Value) -> std::result::Result<Vec<Vec<u8>>, Fault> {
        self.vm.list_values(list).map_err(Fault::Error)?.into_iter().map(|item| self.string_bytes(item)).collect()
    }

    fn plist(&mut self, items: &[(&str, Value)]) -> Value {
        items.iter().rev().fold(Value::NIL, |tail, (key, value)| {
            let key = self.intern(key);
            let tail = self.cons(*value, tail);
            self.cons(key, tail)
        })
    }

    fn keeper_status(&mut self, status: Status) -> Value {
        let (name, value) = match status {
            Status::Running => ("running", None),
            Status::Exited(code) => ("exited", Some(code)),
            Status::Signaled(signal) => ("signaled", Some(signal)),
            Status::Stopped(signal) => ("stopped", Some(signal)),
            Status::Continued => ("continued", None),
        };
        let tail = value.map_or(Value::NIL, |value| { let value = integer(value); self.cons(value, Value::NIL) });
        let name = self.intern(name);
        self.cons(name, tail)
    }

    /// A handle's row as a property list.
    fn keeper_row(&mut self, row: &Row) -> Value {
        let boolean = |on: bool| if on { Value::TRUE } else { Value::NIL };
        let process = |pid: u32| if pid == 0 { Value::NIL } else { integer(pid) };
        let slots: Vec<Value> = (0..64i64).filter(|slot| row.open >> slot & 1 == 1).map(|slot: i64| integer(slot)).collect();
        let items = [
            (":handle", integer(row.handle.raw())),
            (":kind", self.string(&row.tag)),
            (":version", integer(row.version)),
            (":metadata", self.make_string(row.metadata.clone(), false)),
            (":kept", boolean(row.kept)),
            (":deadline", if row.deadline == 0 { Value::NIL } else { self.float(std::time::Duration::from_millis(row.deadline).as_secs_f64()) }),
            (":pid", process(row.pid)),
            (":status", self.keeper_status(row.status)),
            (":owner", process(row.owner)),
            (":owner-alive", boolean(row.owner_alive)),
            (":open", self.vm.list_value(&slots)),
        ];
        self.plist(&items)
    }

    /// The spec `spawn' reads from ARGS: ARGV ENV DIRECTORY UMASK SIGMASK
    /// SETSID FDS, where FDS is a list of (NUMBER . END) and END one of
    /// `null', `reads', `writes', `socket', `pty'.
    fn keeper_spec(&mut self, given: &[Value]) -> std::result::Result<Spec, Fault> {
        let [argv, env, directory, umask, sigmask, setsid, fds] = given else { return Err(failed("romacs--keeper-call spawn: wrong arguments".into())) };
        let mut slots = Vec::new();
        for slot in self.vm.list_values(*fds).map_err(Fault::Error)? {
            let (number, end) = self.pair(slot).ok_or(Fault::Wrong("consp", slot))?;
            let end = match self.symbol_name(end)?.as_str() {
                "null" => End::Null, "reads" => End::Reads, "writes" => End::Writes, "socket" => End::Socket, "pty" => End::Pty,
                _ => return Err(Fault::Wrong("romacs-keeper-end-p", end)),
            };
            slots.push((small(number)?, end));
        }
        Ok(Spec { argv: self.byte_strings(*argv)?, env: self.byte_strings(*env)?, cwd: self.string_bytes(*directory)?,
                  umask: small(*umask)?, limits: Vec::new(), sigmask: natural(*sigmask)?,
                  setsid: *setsid != Value::NIL, fds: slots, pty_size: (80, 24) })
    }

    fn keeper_handed(&mut self, handed: std::result::Result<(Row, Vec<std::os::fd::OwnedFd>), String>) -> Out {
        let (row, fds) = handed.map_err(failed)?;
        self.link()?.hold(&row, fds);
        Ok(self.keeper_row(&row))
    }

    /// The operations on the link itself and on the keeper as a whole.
    fn keeper_session_op(&mut self, name: &str, args: &[Value]) -> Option<Out> {
        let arg = |index: usize| args.get(index).copied().unwrap_or(Value::NIL);
        let done = |result: std::result::Result<(), String>| result.map(|()| Value::NIL).map_err(failed);
        Some((|| match name {
            "attach" => {
                // A second link would drop the descriptors this one holds.
                if let Some(link) = &self.vm.keeper {
                    return Err(failed(format!("This session is attached to the keeper with pid {}; detach first", link.client.keeper.pid)));
                }
                let client = crate::host::keeper_attach(&path(&self.string_bytes(arg(0))?)).map_err(failed)?;
                let (pid, epoch) = (client.keeper.pid, client.keeper.epoch);
                self.vm.keeper = Some(crate::keeper::Link::new(client));
                let items = [(":pid", integer(pid)), (":incarnation", self.string(&format!("{epoch:016x}")))];
                Ok(self.plist(&items))
            }
            "attached" => Ok(self.vm.keeper.as_ref().map_or(Value::NIL, |link| integer(link.client.keeper.pid))),
            "socket" => {
                let runtime = crate::host::getenv("XDG_RUNTIME_DIR").ok_or_else(|| failed("XDG_RUNTIME_DIR is not set: no place for a keeper's socket".into()))?;
                let session = String::from_utf8_lossy(&self.string_bytes(arg(0))?).into_owned();
                let socket = romacs_keeper::run::socket_path(std::path::Path::new(&runtime), &session);
                Ok(self.string(&socket.to_string_lossy()))
            }
            "detach" => {
                let result = self.link()?.client.detach(arg(0) != Value::NIL);
                self.vm.keeper = None;
                done(result)
            }
            "list" => {
                let rows = self.link()?.client.list().map_err(failed)?;
                let rows: Vec<Value> = rows.iter().map(|row| self.keeper_row(row)).collect();
                Ok(self.vm.list_value(&rows))
            }
            "configure" => { let idle = natural(arg(0))?; done(self.link()?.client.configure(idle)) }
            "close-orphans" => Ok(integer(self.link()?.client.close_orphans().map_err(failed)?)),
            "exec" => { let program = path(&self.string_bytes(arg(0))?); done(self.link()?.client.exec(&program)) }
            "shutdown" => { let result = self.link()?.client.shutdown(); self.vm.keeper = None; done(result) }
            "info" => {
                let info = self.link()?.client.info().map_err(failed)?;
                let items = [(":pid", integer(info.pid)), (":executable", self.string(&format!("{:016x}", info.executable))),
                             (":idle-milliseconds", integer(info.idle_ms)), (":handles", integer(info.handles))];
                Ok(self.plist(&items))
            }
            "executable-hash" => {
                let program = path(&self.string_bytes(arg(0))?);
                Ok(self.string(&format!("{:016x}", romacs_keeper::server::executable_hash(&program))))
            }
            "events" => {
                let events = self.link()?.events().map_err(failed)?;
                let events: Vec<Value> = events.iter().map(|event| match event {
                    Event::Status(row) => { let row = self.keeper_row(row); let tag = self.intern("status"); self.cons(tag, row) }
                    Event::Exec => { let tag = self.intern("exec"); self.cons(tag, Value::NIL) }
                }).collect();
                Ok(self.vm.list_value(&events))
            }
            _ => Err(Fault::Wrong("", Value::NIL)),
        })()).filter(|out| !matches!(out, Err(Fault::Wrong("", _))))
    }

    /// SECONDS, nil for none, as a wait.
    fn keeper_seconds(&self, seconds: Value) -> std::result::Result<std::time::Duration, Fault> {
        Ok(match seconds.integer_value() {
            _ if seconds == Value::NIL => std::time::Duration::ZERO,
            Some(whole) => std::time::Duration::from_secs(u64::try_from(whole).unwrap_or(0)),
            None => std::time::Duration::try_from_secs_f64(self.float_value(seconds)?).unwrap_or_default(),
        })
    }

    /// The operations on one handle.
    fn keeper_handle_op(&mut self, name: &str, args: &[Value]) -> Option<Out> {
        let arg = |index: usize| args.get(index).copied().unwrap_or(Value::NIL);
        let done = |result: std::result::Result<(), String>| result.map(|()| Value::NIL).map_err(failed);
        Some((|| match name {
            "spawn" => {
                let (kind, metadata) = (String::from_utf8_lossy(&self.string_bytes(arg(0))?).into_owned(), self.string_bytes(arg(2))?);
                let spec = self.keeper_spec(args.get(4..).unwrap_or(&[]))?;
                let handed = self.link()?.client.spawn(&kind, small(arg(1))?, &metadata, arg(3) != Value::NIL, &spec);
                self.keeper_handed(handed)
            }
            "claim" => {
                let supersedes = if arg(1) == Value::NIL { 0 } else { small(arg(1))? };
                let handed = self.link()?.client.claim(handle(arg(0))?, supersedes);
                self.keeper_handed(handed)
            }
            "read" => {
                let timeout = self.keeper_seconds(arg(3))?;
                match self.link()?.read(natural(arg(0))?, small(arg(1))?, small(arg(2))?, timeout).map_err(failed)? {
                    None => Ok(Value::TRUE),
                    Some(bytes) if bytes.is_empty() => Ok(Value::NIL),
                    Some(bytes) => Ok(self.make_string(bytes, false)),
                }
            }
            "write" => {
                let (bytes, timeout) = (self.string_bytes(arg(2))?, self.keeper_seconds(arg(3))?);
                Ok(integer(self.link()?.write(natural(arg(0))?, small(arg(1))?, &bytes, timeout).map_err(failed)?))
            }
            "close-slot" => {
                let (kept, slot) = (handle(arg(0))?, small::<u32>(arg(1))?);
                let link = self.link()?;
                // The keeper's copy first, the session's on its word: end
                // of file reaches the child when both are closed.
                link.client.close_slot(kept, slot).map_err(failed)?;
                link.drop_slot(kept.raw(), small(arg(1))?);
                Ok(Value::NIL)
            }
            "close" => {
                let (kept, signal) = (handle(arg(0))?, if arg(1) == Value::NIL { 0 } else { small(arg(1))? });
                let link = self.link()?;
                link.client.close(kept, signal).map_err(failed)?;
                link.forget(kept.raw());
                Ok(Value::NIL)
            }
            "signal" => done(self.link()?.client.signal(handle(arg(0))?, small(arg(1))?, arg(2) != Value::NIL)),
            "set-kept" => done(self.link()?.client.set_kept(handle(arg(0))?, arg(1) != Value::NIL)),
            "set-metadata" => { let metadata = self.string_bytes(arg(1))?; done(self.link()?.client.set_metadata(handle(arg(0))?, &metadata)) }
            "set-deadline" => {
                let at = if arg(1) == Value::NIL { 0 } else {
                    let since = std::time::Duration::try_from_secs_f64(self.float_value(arg(1))?).map_err(|_| Fault::Wrong("natnump", arg(1)))?;
                    u64::try_from(since.as_millis()).unwrap_or(u64::MAX).max(1)
                };
                done(self.link()?.client.set_deadline(handle(arg(0))?, at))
            }
            _ => Err(Fault::Wrong("", Value::NIL)),
        })()).filter(|out| !matches!(out, Err(Fault::Wrong("", _))))
    }

    pub(crate) fn keeper_call(&mut self, op: Value, args: &[Value]) -> Out {
        let name = self.symbol_name(op)?;
        self.keeper_session_op(&name, args).or_else(|| self.keeper_handle_op(&name, args))
            .unwrap_or(Err(Fault::Wrong("romacs-keeper-operation-p", op)))
    }
}
