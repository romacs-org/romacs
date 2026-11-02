//! GNU inotify.c's system calls (`prim/leaf/inotify.rs`), under the
//! `inotify-*' functions and the event dispatch of rlisp/vm/inotify.el.
use super::*;

// (romacs--inotify-add-watch FILE MASK): FILE's watch descriptor, an
// integer, for the inotify MASK.
defsubr! { name = "romacs--inotify-add-watch", gnu = "inotify.c:370", effect = Global,
fn inotify_add_watch(cx, file: Value, mask: Integer) {
    let mask = u32::try_from(mask.0).map_err(|_| Fault::Wrong("fixnump", Value::integer(mask.0).unwrap_or(Value::NIL)))?;
    cx.inotify_add_watch(file, mask)
}}

// (romacs--inotify-rm-watch WD): remove watch descriptor WD.
defsubr! { name = "romacs--inotify-rm-watch", gnu = "inotify.c:272", effect = Global,
fn inotify_rm_watch(cx, wd: Integer) {
    cx.inotify_rm_watch(wd.0)
}}

// (romacs--inotify-read): the queued events, ((WD MASK COOKIE NAME) ...),
// without waiting.
defsubr! { name = "romacs--inotify-read", gnu = "inotify.c:327", effect = Global,
fn inotify_read(cx) {
    cx.inotify_read()
}}

// (romacs--inotify-close): close the instance, its last watch gone.
defsubr! { name = "romacs--inotify-close", gnu = "inotify.c:272", effect = Global,
fn inotify_close(cx) {
    cx.inotify_close();
    Ok(Value::NIL)
}}
