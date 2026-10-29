//! GNU process.c and callproc.c primitives that need no Lisp callback
//! (state in `prim/leaf/process.rs`, descriptors in `subprocess.rs`). What
//! runs filters, sentinels, timers or hooks is Lisp in rlisp/vm/subprocess.el,
//! composed from the `romacs--proc-*' leaves below.
use super::*;
use crate::prim::CallOutput;
use crate::subprocess::Slot;

fn check_process(cx: &Leaf<'_>, v: Value) -> std::result::Result<(), Fault> {
    cx.process(v).map(|_| ())
}

/// get_process: a process, a process or buffer name, a buffer, or nil for
/// the current buffer.
pub(crate) fn get_process(cx: &mut Leaf<'_>, name: Value) -> Out {
    let obj = if matches!(cx.kind(name), Kind::String) {
        let mut obj = cx.process_named(name)?;
        if obj == Value::NIL {
            obj = cx.lifecycle_get(name)?;
        }
        if obj == Value::NIL {
            return Err(Fault::Error(format!("Process {} does not exist", cx.string_text(name)?)));
        }
        obj
    } else if name == Value::NIL {
        cx.current_buffer()?
    } else {
        name
    };
    if cx.lifecycle_buffer_p(obj) {
        if !cx.lifecycle_live_p(obj) {
            return Err(Fault::Error("Attempt to get process for a dead buffer".into()));
        }
        let proc = cx.buffer_process(obj);
        if proc == Value::NIL {
            let name = cx.buffer_state(obj).and_then(|s| s.name.clone()).unwrap_or_default();
            return Err(Fault::Error(format!("Buffer {name} has no process")));
        }
        return Ok(proc);
    }
    check_process(cx, obj)?;
    Ok(obj)
}

fn slot_of(cx: &Leaf<'_>, field: Value) -> std::result::Result<Option<Slot>, Fault> {
    Ok(Some(match cx.symbol_name(field)?.as_str() {
        "name" => Slot::Name, "command" => Slot::Command, "filter" => Slot::Filter,
        "sentinel" => Slot::Sentinel, "log" => Slot::Log, "buffer" => Slot::Buffer,
        "childp" => Slot::Childp, "plist" => Slot::Plist, "type" => Slot::Type, "mark" => Slot::Mark,
        "status" => Slot::Status, "decode" => Slot::Decode, "encode" => Slot::Encode,
        "stderrproc" => Slot::Stderrproc, "tty-name" => Slot::TtyName,
        _ => return Ok(None),
    }))
}

defsubr! { name = "processp", gnu = "process.c:1051", effect = Pure,
fn processp(cx, object: Value) {
    Ok(if cx.processp(object) { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "get-process", gnu = "process.c:1058", effect = Reads,
fn get_process_prim(cx, name: Value) {
    if cx.processp(name) { return Ok(name); }
    cx.string_view(name)?;
    cx.process_named(name)
}}

defsubr! { name = "process-list", gnu = "process.c:1791", effect = Reads,
fn process_list(cx) { Ok(cx.process_list()) }}

defsubr! { name = "get-buffer-process", gnu = "process.c:8422", effect = Reads,
fn get_buffer_process(cx, buffer: Value) {
    if buffer == Value::NIL { return Ok(Value::NIL); }
    let buffer = cx.lifecycle_get(buffer)?;
    if buffer == Value::NIL { return Ok(Value::NIL); }
    Ok(cx.buffer_process(buffer))
}}

defsubr! { name = "process-status", gnu = "process.c:1205", effect = Reads,
fn process_status(cx, process: Value) {
    let process = if matches!(cx.kind(process), Kind::String) { cx.process_named(process)? } else { get_process(cx, process)? };
    if process == Value::NIL { return Ok(Value::NIL); }
    cx.process_status(process)
}}

defsubr! { name = "process-exit-status", gnu = "process.c:1251", effect = Reads,
fn process_exit_status(cx, process: Value) { cx.process_exit_status(process) }}

defsubr! { name = "process-id", gnu = "process.c:1265", effect = Reads,
fn process_id(cx, process: Value) { cx.process_numeric(process, "pid") }}

defsubr! { name = "process-name", gnu = "process.c:1279", effect = Reads,
fn process_name(cx, process: Value) { cx.process_slot(process, Slot::Name) }}

defsubr! { name = "process-command", gnu = "process.c:1289", effect = Reads,
fn process_command(cx, process: Value) { cx.process_slot(process, Slot::Command) }}

defsubr! { name = "process-tty-name", gnu = "process.c:1301", effect = Reads,
fn process_tty_name(cx, process: Value; optional stream: Value) {
    let tty = cx.process_slot(process, Slot::TtyName)?;
    let pty_in = cx.process_numeric(process, "pty-in")? != Value::NIL;
    let pty_out = cx.process_numeric(process, "pty-out")? != Value::NIL;
    let stderr_proc = cx.process_slot(process, Slot::Stderrproc)?;
    if stream == Value::NIL { return Ok(tty); }
    let on = match cx.symbol_name(stream).ok().as_deref() {
        Some("stdin") => pty_in,
        Some("stdout") => pty_out,
        Some("stderr") => pty_out && stderr_proc == Value::NIL,
        _ => {
            let message = cx.string("Unknown stream");
            let data = cx.cons(stream, Value::NIL);
            let data = cx.cons(message, data);
            return Err(Fault::Signal(cx.intern("error"), data));
        }
    };
    Ok(if on { tty } else { Value::NIL })
}}

defsubr! { name = "process-buffer", gnu = "process.c:1362", effect = Reads,
fn process_buffer(cx, process: Value) { cx.process_slot(process, Slot::Buffer) }}

defsubr! { name = "process-mark", gnu = "process.c:1372", effect = Reads,
fn process_mark(cx, process: Value) { cx.process_slot(process, Slot::Mark) }}

defsubr! { name = "process-filter", gnu = "process.c:1454", effect = Reads,
fn process_filter(cx, process: Value) { cx.process_slot(process, Slot::Filter) }}

defsubr! { name = "process-sentinel", gnu = "process.c:1485", effect = Reads,
fn process_sentinel(cx, process: Value) { cx.process_slot(process, Slot::Sentinel) }}

/// pset_childp (p, plist_put (p->childp, KEY, VALUE)) for connections.
fn put_childp(cx: &mut Leaf<'_>, process: Value, key: &str, value: Value) -> std::result::Result<(), Fault> {
    if !cx.connection_p(process) { return Ok(()); }
    let childp = cx.process_slot(process, Slot::Childp)?;
    let key = cx.intern(key);
    let childp = plist_put(cx, childp, key, value)?;
    cx.set_process_slot(process, Slot::Childp, childp)?;
    Ok(())
}

/// Fplist_put on a fresh tail: replace KEY's value, or append KEY VALUE.
fn plist_put(cx: &mut Leaf<'_>, plist: Value, key: Value, value: Value) -> Out {
    let mut tail = plist;
    while let Some((k, rest)) = cx.pair(tail) {
        let Some((_, next)) = cx.pair(rest) else { break };
        if k == key {
            cx.setcar(rest, value)?;
            return Ok(plist);
        }
        tail = next;
    }
    let cell = cx.cons(value, Value::NIL);
    let cell = cx.cons(key, cell);
    if plist == Value::NIL { return Ok(cell); }
    let mut last = plist;
    while let Some((_, rest)) = cx.pair(last) {
        if cx.pair(rest).is_none() { break; }
        last = rest;
    }
    cx.setcdr(last, cell)?;
    Ok(plist)
}

fn plist_get(cx: &Leaf<'_>, plist: Value, key: Value) -> Value {
    let mut tail = plist;
    while let Some((k, rest)) = cx.pair(tail) {
        let Some((v, next)) = cx.pair(rest) else { break };
        if k == key { return v; }
        tail = next;
    }
    Value::NIL
}

defsubr! { name = "set-process-filter", gnu = "process.c:1405", effect = Global,
fn set_process_filter(cx, process: Value, filter: Value) {
    check_process(cx, process)?;
    let filter = if filter == Value::NIL { cx.intern("internal-default-process-filter") } else { filter };
    if cx.process_infd(process) {
        let listen = cx.intern("listen");
        let status = cx.process_slot(process, Slot::Status)?;
        let old = cx.process_slot(process, Slot::Filter)?;
        let command = cx.process_slot(process, Slot::Command)?;
        if filter == Value::TRUE && status != listen {
            cx.set_process_reading(process, false);
        } else if old == Value::TRUE && command != Value::TRUE {
            cx.set_process_reading(process, true);
        }
    }
    cx.set_process_slot(process, Slot::Filter, filter)?;
    put_childp(cx, process, ":filter", filter)?;
    Ok(filter)
}}

defsubr! { name = "set-process-sentinel", gnu = "process.c:1464", effect = Global,
fn set_process_sentinel(cx, process: Value, sentinel: Value) {
    check_process(cx, process)?;
    let sentinel = if sentinel == Value::NIL { cx.intern("internal-default-process-sentinel") } else { sentinel };
    cx.set_process_slot(process, Slot::Sentinel, sentinel)?;
    put_childp(cx, process, ":sentinel", sentinel)?;
    Ok(sentinel)
}}

defsubr! { name = "set-process-window-size", gnu = "process.c:1543", effect = Global,
fn set_process_window_size(cx, process: Value, height: Value, width: Value) {
    check_process(cx, process)?;
    let dim = |v: Value| v.integer_value().filter(|n| (0..=i64::from(u16::MAX)).contains(n)).map(|n| n as u16)
        .ok_or(Fault::Wrong("natnump", v));
    let (h, w) = (dim(height)?, dim(width)?);
    cx.process_window_size(process, h, w)
}}

defsubr! { name = "set-process-inherit-coding-system-flag", gnu = "process.c:1564", effect = Global,
fn set_process_inherit_coding_system_flag(cx, process: Value, flag: Value) {
    cx.set_process_flag(process, "inherit-coding", flag != Value::NIL)?;
    Ok(flag)
}}

defsubr! { name = "process-inherit-coding-system-flag", gnu = "process.c:8443", effect = Reads,
fn process_inherit_coding_system_flag(cx, process: Value) { cx.process_numeric(process, "inherit-coding") }}

defsubr! { name = "set-process-query-on-exit-flag", gnu = "process.c:1590", effect = Global,
fn set_process_query_on_exit_flag(cx, process: Value, flag: Value) {
    cx.set_process_flag(process, "kill-without-query", flag == Value::NIL)?;
    Ok(flag)
}}

defsubr! { name = "process-query-on-exit-flag", gnu = "process.c:1604", effect = Reads,
fn process_query_on_exit_flag(cx, process: Value) {
    Ok(if cx.process_numeric(process, "kill-without-query")? == Value::NIL { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "process-contact", gnu = "process.c:1614", effect = Reads,
fn process_contact(cx, process: Value; optional key: Value, no_block: Value) {
    let contact = cx.process_slot(process, Slot::Childp)?;
    let network = cx.process_slot(process, Slot::Type)? == cx.intern("network");
    let serial = cx.process_slot(process, Slot::Type)? == cx.intern("serial");
    let pipe = cx.process_slot(process, Slot::Type)? == cx.intern("pipe");
    if (!network && !serial && !pipe) || key == Value::TRUE { return Ok(contact); }
    if key == Value::NIL && (network || serial) {
        let (a, b) = if network { (":host", ":service") } else { (":port", ":speed") };
        let (a, b) = (cx.intern(a), cx.intern(b));
        let (a, b) = (plist_get(cx, contact, a), plist_get(cx, contact, b));
        let tail = cx.cons(b, Value::NIL);
        return Ok(cx.cons(a, tail));
    }
    if key == Value::NIL { return Ok(Value::TRUE); }
    Ok(plist_get(cx, contact, key))
}}

defsubr! { name = "process-plist", gnu = "process.c:1671", effect = Reads,
fn process_plist(cx, process: Value) { cx.process_slot(process, Slot::Plist) }}

defsubr! { name = "set-process-plist", gnu = "process.c:1680", effect = Global,
fn set_process_plist(cx, process: Value, plist: Value) {
    check_process(cx, process)?;
    if plist != Value::NIL && cx.pair(plist).is_none() { return Err(Fault::Wrong("listp", plist)); }
    cx.set_process_slot(process, Slot::Plist, plist)
}}

defsubr! { name = "process-type", gnu = "process.c:1704", effect = Reads,
fn process_type(cx, process: Value) {
    let process = get_process(cx, process)?;
    cx.process_slot(process, Slot::Type)
}}

defsubr! { name = "process-coding-system", gnu = "process.c:8052", effect = Reads,
fn process_coding_system(cx, process: Value) {
    let decode = cx.process_slot(process, Slot::Decode)?;
    let encode = cx.process_slot(process, Slot::Encode)?;
    Ok(cx.cons(decode, encode))
}}

defsubr! { name = "process-running-child-p", gnu = "process.c:7046", effect = Reads,
fn process_running_child_p(cx; optional process: Value) {
    let process = get_process(cx, process)?;
    cx.process_running_child(process)
}}

// waiting_for_user_input_p is READ_KBD of the innermost wait; VM2's waits
// never read the keyboard (subprocess.el), so it is always 0.
defsubr! { name = "waiting-for-user-input-p", gnu = "process.c:8484", effect = Reads,
fn waiting_for_user_input_p(_cx) { Ok(Value::NIL) }}

defsubr! { name = "internal-default-signal-process", gnu = "process.c:7365", effect = Global,
fn internal_default_signal_process(cx, process: Value, sigcode: Value; optional remote: Value) {
    let mut process = process;
    if matches!(cx.kind(process), Kind::String) {
        let found = cx.process_named(process)?;
        process = if found != Value::NIL { found } else {
            match cx.string_text(process)?.parse::<i64>() {
                Ok(n) => Value::integer(n).map_err(Fault::Error)?,
                Err(_) => return Ok(Value::NIL),
            }
        };
    } else if process.integer_value().is_none() && !matches!(cx.kind(process), Kind::Float) {
        process = get_process(cx, process)?;
    }
    if process == Value::NIL { return Ok(Value::NIL); }
    let pid = if let Some(n) = process.integer_value() { n } else if matches!(cx.kind(process), Kind::Float) {
        cx.float_value(process)? as i64
    } else {
        let pid = cx.process_numeric(process, "pid")?.integer_value().unwrap_or(0);
        if pid <= 0 {
            let name = cx.string_text(cx.process_slot(process, Slot::Name)?)?;
            return Err(Fault::Error(format!("Cannot signal process {name}")));
        }
        pid
    };
    let signo = if let Some(n) = sigcode.integer_value() { n as i32 } else {
        if cx.kind(sigcode) != Kind::Symbol { return Err(Fault::Wrong("symbolp", sigcode)); }
        let name = cx.symbol_name(sigcode)?;
        crate::subprocess::signal_number(&name).ok_or_else(|| Fault::Error(format!("Undefined signal name {name}")))?
    };
    let result = if crate::subprocess::kill(pid as i32, signo).is_ok() { 0 } else { -1 };
    Ok(Value::integer(result).map_err(Fault::Error)?)
}}

// Internal leaves for rlisp/vm/subprocess.el.

defsubr! { name = "romacs--get-process", gnu = "romacs:process", effect = Reads,
fn romacs_get_process(cx, name: Value) { get_process(cx, name) }}

defsubr! { name = "romacs--proc-make", gnu = "romacs:process", effect = Global,
fn proc_make(cx, name: Value) { cx.make_process(name) }}

defsubr! { name = "romacs--proc-get", gnu = "romacs:process", effect = Reads,
fn proc_get(cx, process: Value, field: Value) {
    // GNU reads p->status only after update_status.
    if cx.symbol_name(field)? == "status" { check_process(cx, process)?; cx.update_status(process); }
    match slot_of(cx, field)? {
        Some(slot) => cx.process_slot(process, slot),
        None => { let name = cx.symbol_name(field)?; cx.process_numeric(process, &name) }
    }
}}

defsubr! { name = "romacs--proc-set", gnu = "romacs:process", effect = Global,
fn proc_set(cx, process: Value, field: Value, value: Value) {
    match slot_of(cx, field)? {
        Some(slot) => cx.set_process_slot(process, slot, value),
        None => {
            let name = cx.symbol_name(field)?;
            check_process(cx, process)?;
            if name == "reading" { cx.set_process_reading(process, value != Value::NIL); }
            else { cx.set_process_flag(process, &name, value != Value::NIL)?; }
            Ok(value)
        }
    }
}}

fn byte_list(cx: &Leaf<'_>, list: Value) -> std::result::Result<Vec<Vec<u8>>, Fault> {
    let mut out = Vec::new();
    let mut tail = list;
    while let Some((item, rest)) = cx.pair(tail) {
        out.push(cx.string_view(item)?.bytes().to_vec());
        tail = rest;
    }
    Ok(out)
}

defsubr! { name = "romacs--proc-spawn", gnu = "process.c:2201", effect = Global,
fn proc_spawn(cx, process: Value, argv: Value, env: Value, dir: Value, stderrproc: Value) {
    let argv = byte_list(cx, argv)?;
    let env = byte_list(cx, env)?;
    let dir = cx.string_view(dir)?.bytes().to_vec();
    cx.process_spawn(process, &argv, &env, &dir, stderrproc)
}}

defsubr! { name = "romacs--proc-pty", gnu = "process.c:2380", effect = Global,
fn proc_pty(cx, process: Value) { cx.process_pty_only(process) }}

defsubr! { name = "romacs--proc-pipe", gnu = "process.c:2441", effect = Global,
fn proc_pipe(cx, process: Value) { cx.process_pipe(process) }}

defsubr! { name = "romacs--proc-network", gnu = "process.c:3827", effect = Global,
fn proc_network(cx, process: Value, server: Value, nowait: Value, host: Value, port: Integer, family: Value, contact: Value) {
    let host = cx.string_text(host)?;
    let family = if family == cx.intern("ipv4") { Some(false) } else if family == cx.intern("ipv6") { Some(true) } else { None };
    let port = u16::try_from(port.0).map_err(|_| Fault::Error(format!("Invalid service port {}", port.0)))?;
    cx.process_network(process, server != Value::NIL, nowait != Value::NIL, host, port, family, contact)
}}

defsubr! { name = "romacs--proc-accept", gnu = "process.c:5000", effect = Global,
fn proc_accept(cx, server: Value) { cx.process_accept(server) }}

defsubr! { name = "romacs--proc-attach", gnu = "romacs:process", effect = Global,
fn proc_attach(cx, process: Value, connection: Integer) { cx.process_attach(process, connection.0) }}

defsubr! { name = "romacs--proc-remove", gnu = "process.c:977", effect = Global,
fn proc_remove(cx, process: Value) { cx.remove_process(process) }}

defsubr! { name = "romacs--proc-deactivate", gnu = "process.c:4841", effect = Global,
fn proc_deactivate(cx, process: Value) {
    check_process(cx, process)?;
    cx.deactivate_process(process);
    Ok(Value::NIL)
}}

defsubr! { name = "romacs--proc-select", gnu = "process.c:5358", effect = Global,
fn proc_select(cx, wait_proc: Value, just_this_one: Value, seconds: Value) {
    let seconds = if seconds == Value::NIL { None } else { Some(cx.float_value(seconds).or_else(|_| seconds.integer_value().map(|n| n as f64).ok_or(Fault::Wrong("numberp", seconds)))?) };
    cx.process_select(wait_proc, just_this_one != Value::NIL, seconds)
}}

defsubr! { name = "romacs--proc-activity", gnu = "romacs:process", effect = Reads,
fn proc_activity(cx) { Value::integer(cx.process_activity() as i64).map_err(Fault::Error) }}

defsubr! { name = "romacs--proc-input-pending", gnu = "process.c:5566", effect = Global,
fn proc_input_pending(cx) { Ok(if cx.process_input_pending()? { Value::TRUE } else { Value::NIL }) }}

defsubr! { name = "romacs--proc-read", gnu = "process.c:6259", effect = Global,
fn proc_read(cx, process: Value) { cx.process_read(process) }}

defsubr! { name = "romacs--proc-read-done", gnu = "process.c:6029", effect = Global,
fn proc_read_done(cx, process: Value, nread: Integer, errno: Value) {
    cx.process_read_done(process, nread.0, errno.integer_value().unwrap_or(0))?;
    Ok(Value::NIL)
}}

defsubr! { name = "romacs--proc-ticks-pending", gnu = "process.c:5545", effect = Reads,
fn proc_ticks_pending(cx) { Ok(if cx.process_ticks_pending() { Value::TRUE } else { Value::NIL }) }}

defsubr! { name = "romacs--proc-ticks-acknowledge", gnu = "process.c:7859", effect = Global,
fn proc_ticks_acknowledge(cx) { cx.process_ticks_acknowledge(); Ok(Value::NIL) }}

defsubr! { name = "romacs--proc-changed", gnu = "process.c:7867", effect = Global,
fn proc_changed(cx, process: Value) {
    check_process(cx, process)?;
    Ok(if cx.process_changed(process) { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "romacs--proc-tick", gnu = "romacs:process", effect = Global,
fn proc_tick(cx, process: Value) { check_process(cx, process)?; cx.process_tick(process); Ok(Value::NIL) }}

defsubr! { name = "romacs--proc-status-finish", gnu = "process.c:7889", effect = Global,
fn proc_status_finish(cx, process: Value) { cx.process_status_finish(process) }}

defsubr! { name = "romacs--proc-status-message", gnu = "process.c:813", effect = Reads,
fn proc_status_message(cx, process: Value) { cx.update_status(process); cx.status_message(process) }}

defsubr! { name = "romacs--proc-reap", gnu = "process.c:7701", effect = Global,
fn proc_reap(cx) { cx.reap_children(); Ok(Value::NIL) }}

defsubr! { name = "romacs--proc-send", gnu = "process.c:6740", effect = Global,
fn proc_send(cx, process: Value, bytes: Value) {
    let bytes = if bytes == Value::NIL { None } else { Some(cx.string_view(bytes)?.bytes().to_vec()) };
    cx.process_send(process, bytes)
}}

defsubr! { name = "romacs--proc-send-eof", gnu = "process.c:7441", effect = Global,
fn proc_send_eof(cx, process: Value) { cx.process_send_eof(process)?; Ok(process) }}

defsubr! { name = "romacs--proc-signal", gnu = "process.c:7096", effect = Global,
fn proc_signal(cx, process: Value, signo: Integer, current_group: Value) {
    cx.process_send_signal(process, signo.0 as i32, current_group)
}}

defsubr! { name = "romacs--proc-kill-for-delete", gnu = "process.c:1129", effect = Global,
fn proc_kill_for_delete(cx, process: Value) {
    check_process(cx, process)?;
    Ok(if cx.process_kill_for_delete(process)? { Value::TRUE } else { Value::NIL })
}}

// get_current_directory's check: report_file_error ("Setting current
// directory", CURDIR) unless DIR is an accessible directory.
defsubr! { name = "romacs--check-directory", gnu = "callproc.c:140", effect = Reads,
fn check_directory(cx, dir: Value, curdir: Value) {
    let bytes = cx.string_view(dir)?.bytes().to_vec();
    match crate::subprocess::accessible_directory(&bytes) {
        Ok(()) => Ok(Value::NIL),
        Err(error) => {
            let data = cx.cons(curdir, Value::NIL);
            Err(cx.file_errno_data("Setting current directory", &error, data))
        }
    }
}}

defsubr! { name = "romacs--decode-output", gnu = "callproc.c:812", effect = Global,
fn decode_output(cx, bytes: Value, coding: Value) {
    let bytes = cx.string_view(bytes)?.bytes().to_vec();
    cx.decode_output(bytes, coding)
}}

defsubr! { name = "romacs--signal-description", gnu = "callproc.c:933", effect = Pure,
fn signal_description(cx, signal: Integer) {
    let text = crate::subprocess::signal_description(signal.0 as i32).unwrap_or_else(|| "unknown".into());
    // code_convert_string_norecord (..., Vlocale_coding_system, 0): decoded,
    // so multibyte.
    Ok(cx.make_string(text.into_bytes(), true))
}}

fn call_output(cx: &Leaf<'_>, spec: Value) -> std::result::Result<CallOutput, Fault> {
    Ok(if spec == Value::TRUE { CallOutput::Same }
        else if spec == Value::NIL { CallOutput::Discard }
        else if matches!(cx.kind(spec), Kind::Symbol) { CallOutput::Pipe }
        else { CallOutput::File(cx.string_view(spec)?.bytes().to_vec()) })
}

// OUTPUT: `pipe', nil (discard) or a file name; ERROR: t (with output), nil
// or a file name.
defsubr! { name = "romacs--call-process-run", gnu = "callproc.c:351", effect = Global,
fn call_process_run(cx, argv: Value, env: Value, dir: Value, infile: Value, output: Value, error: Value, wait: Value) {
    let argv = byte_list(cx, argv)?;
    let env = byte_list(cx, env)?;
    let dir = cx.string_view(dir)?.bytes().to_vec();
    let infile = cx.string_view(infile)?.bytes().to_vec();
    let output = if output == Value::TRUE { CallOutput::Pipe } else { call_output(cx, output)? };
    let error = call_output(cx, error)?;
    cx.call_process_run(&argv, &env, &dir, &infile, output, error, wait != Value::NIL)
}}

defsubr! { name = "romacs--wait-status", gnu = "romacs:process", effect = Pure,
fn wait_status(_cx, status: Integer) {
    // (signal . N) or (exit . N), from a wait status.
    let w = status.0 as i32;
    let (kind, n) = if libc::WIFSIGNALED(w) { ("signal", libc::WTERMSIG(w)) } else { ("exit", libc::WEXITSTATUS(w)) };
    let kind = _cx.intern(kind);
    Ok(_cx.cons(kind, Value::integer(n.into()).map_err(Fault::Error)?))
}}
