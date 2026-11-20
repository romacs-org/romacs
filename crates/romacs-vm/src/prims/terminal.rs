use super::*;

// Start a terminal running SHELL in DIRECTORY, bound to the current buffer, COLUMNS
// wide and ROWS tall, each at most 65535 (the GUI host takes at least 2 columns and 1 row),
// or with both nil the size of the stable text area of the window that controls the
// buffer.  Returns the session number the other `romacs--term-' functions take, once the
// host has the session: SHELL can still fail to start after, as a program that ended.
// Signals an error when a size is invalid or the host refuses it, when with both nil the
// buffer is not shown in a window or that window is too small, when the buffer already owns
// a terminal, or when no terminal host is attached.
defsubr! { name = "romacs--term-create", gnu = "romacs:terminal", effect = Global,
fn terminal_create(cx, shell: Value, directory: Value; optional columns: Value, rows: Value) {
    cx.terminal_create(shell, directory, columns, rows)
}}

// (romacs--term-adopt HANDLE &optional COLUMNS ROWS): bind the current
// buffer to the terminal whose helper process keeper handle HANDLE names.
defsubr! { name = "romacs--term-adopt", gnu = "romacs:terminal", effect = Global,
fn terminal_adopt(cx, handle: Integer; optional columns: Value, rows: Value) {
    cx.terminal_adopt(handle.0, columns, rows)
}}

// (romacs--term-helper-version SESSION): the record version its helper
// process speaks, or nil.
defsubr! { name = "romacs--term-helper-version", gnu = "romacs:terminal", effect = Reads,
fn terminal_helper_version(cx, session: Integer) {
    cx.terminal_helper_version(session.0)
}}

// (romacs--term-transport-counts SESSION): (FULLS . DELTAS), or nil.
defsubr! { name = "romacs--term-transport-counts", gnu = "romacs:terminal", effect = Reads,
fn terminal_transport_counts(cx, session: Integer) {
    cx.terminal_transport_counts(session.0)
}}

// Send BYTES, a string, to terminal SESSION as input.  Signals
// `romacs-terminal-not-running' when its program has ended before the send, an error when
// the host's write fails, as it does when the program ends meanwhile, and an error for an
// unknown SESSION.  Returns nil.
defsubr! { name = "romacs--term-send", gnu = "romacs:terminal", effect = Global,
fn terminal_send(cx, session: Integer, bytes: Value) {
    cx.terminal_send(session.0, bytes)
}}

// Make WINDOW (nil for the selected window) follow terminal SESSION again when FOLLOW
// is non-nil, moving it to the terminal's cursor and screen at once when it shows the
// terminal's buffer and a screen has been published; or stop following it when FOLLOW is
// nil.  An unknown SESSION signals an error.  Returns FOLLOW.
defsubr! { name = "romacs--term-follow", gnu = "romacs:terminal", effect = Global,
fn terminal_follow(cx, session: Integer, follow: Value; optional window: Value) {
    cx.terminal_follow(session.0, follow, window)
}}

// Stop following the terminal in each window showing one with a published normal
// screen, whose point left the terminal's cursor, or whose start is before the screen or
// past the beginning of the cursor's line.  `romacs-term--track' runs it after each command,
// so a command in any window lets that window look back through the terminal's output.
// Returns nil.
defsubr! { name = "romacs--term-track", gnu = "romacs:terminal", effect = Global,
fn terminal_track(cx) {
    cx.terminal_track()
}}

// Paste the string TEXT into terminal SESSION as Alacritty does: between bracketed
// paste marks, less ESC and C-c so the text can neither end the paste nor interrupt, when
// the program asked for them; else with each line end a carriage return, as a typed RET
// is.  Signals as `romacs--term-send' does.  Returns nil.
defsubr! { name = "romacs--term-paste", gnu = "romacs:terminal", effect = Global,
fn terminal_paste(cx, session: Integer, text: Value) {
    cx.terminal_paste(session.0, text)
}}

// Close terminal SESSION: its buffer no longer owns a terminal, and the host is asked
// to end its program, without waiting for it to end.  A SESSION closed already, or never
// opened, is left alone; a negative one signals an error.  Returns nil.
defsubr! { name = "romacs--term-close", gnu = "romacs:terminal", effect = Global,
fn terminal_close(cx, session: Integer) {
    cx.terminal_close(session.0)
}}

// Whether the program in terminal SESSION still runs.  An unknown SESSION signals an
// error.
defsubr! { name = "romacs--term-running-p", gnu = "romacs:terminal", effect = Reads,
fn terminal_running(cx, session: Integer) {
    cx.terminal_running(session.0)
}}

// The bytes a terminal sends for the input EVENT, a unibyte string: empty for a list
// event, such as a mouse click; cursor keys in application-cursor mode when
// APPLICATION-CURSOR is non-nil.  A key symbol it has no bytes for, such as a lone modifier,
// a key with a Super, Hyper or Alt modifier, a control character with no terminal code, or
// an invalid character, signals an error.
defsubr! { name = "romacs--term-encode-event", gnu = "romacs:terminal", effect = Reads,
fn terminal_encode_event(cx, event: Value; optional application_cursor: Value) {
    cx.terminal_encode_event(event, application_cursor != Value::NIL)
}}

// Send the input EVENT to terminal SESSION as the bytes a terminal sends for it, in
// the cursor mode the terminal is in now; a list event, which has none, sends nothing.
// Signals as `romacs--term-encode-event' does for EVENT; an unknown SESSION signals an
// error for any EVENT, and when there are bytes to send it signals as `romacs--term-send'
// does.  Returns nil.
defsubr! { name = "romacs--term-send-event", gnu = "romacs:terminal", effect = Global,
fn terminal_send_event(cx, session: Integer, event: Value) {
    cx.terminal_send_event(session.0, event)
}}

// Terminal SESSION's state as a plist: `:running', `:alternate', `:bracketed-paste',
// `:application-cursor', `:following' (for the selected window), `:shell-foreground', and,
// once the terminal has published a screen, `:cursor' and `:screen-start', positions in
// its buffer.  An unknown SESSION signals an error.
defsubr! { name = "romacs--term-state", gnu = "romacs:terminal", effect = Reads,
fn terminal_state(cx, session: Integer) {
    cx.terminal_state(session.0)
}}
