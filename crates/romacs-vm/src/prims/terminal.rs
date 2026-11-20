use super::*;

// Start a terminal running SHELL in DIRECTORY, bound to the current buffer, COLUMNS
// wide and ROWS tall, or with both nil the size of the window that controls the buffer.
// Returns the session number the other `romacs--term-' functions take.  Signals an error
// when the buffer is not shown in a window or that window is too small, when the buffer
// already owns a terminal, or when no terminal host is attached.
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
// `romacs-terminal-not-running' once its program has ended.  Returns nil.
defsubr! { name = "romacs--term-send", gnu = "romacs:terminal", effect = Global,
fn terminal_send(cx, session: Integer, bytes: Value) {
    cx.terminal_send(session.0, bytes)
}}

// Make WINDOW (nil for the selected window) follow terminal SESSION again at once when
// FOLLOW is non-nil, or stop following it when FOLLOW is nil.  Returns FOLLOW.
defsubr! { name = "romacs--term-follow", gnu = "romacs:terminal", effect = Global,
fn terminal_follow(cx, session: Integer, follow: Value; optional window: Value) {
    cx.terminal_follow(session.0, follow, window)
}}

// Stop following the terminal in each window showing one on its normal screen whose
// point left the terminal's cursor, or whose start left the screen.  `romacs-term--track'
// runs it after each command, so a command in any window lets that window look back
// through the terminal's output.  Returns nil.
defsubr! { name = "romacs--term-track", gnu = "romacs:terminal", effect = Global,
fn terminal_track(cx) {
    cx.terminal_track()
}}

// Paste the string TEXT into terminal SESSION as Alacritty does: between bracketed
// paste marks, less ESC and C-c so the text can neither end the paste nor interrupt, when
// the program asked for them; else with each line end a carriage return, as a typed RET
// is.  Signals `romacs-terminal-not-running' as `romacs--term-send' does.
defsubr! { name = "romacs--term-paste", gnu = "romacs:terminal", effect = Global,
fn terminal_paste(cx, session: Integer, text: Value) {
    cx.terminal_paste(session.0, text)
}}

// Close terminal SESSION: its program is ended and its buffer no longer owns a
// terminal.  Returns nil.
defsubr! { name = "romacs--term-close", gnu = "romacs:terminal", effect = Global,
fn terminal_close(cx, session: Integer) {
    cx.terminal_close(session.0)
}}

// Whether the program in terminal SESSION still runs.
defsubr! { name = "romacs--term-running-p", gnu = "romacs:terminal", effect = Reads,
fn terminal_running(cx, session: Integer) {
    cx.terminal_running(session.0)
}}

// The bytes a terminal sends for the input EVENT, a unibyte string, empty for an event
// that sends none; cursor keys in application-cursor mode when APPLICATION-CURSOR is
// non-nil.
defsubr! { name = "romacs--term-encode-event", gnu = "romacs:terminal", effect = Reads,
fn terminal_encode_event(cx, event: Value; optional application_cursor: Value) {
    cx.terminal_encode_event(event, application_cursor != Value::NIL)
}}

// Send the input EVENT to terminal SESSION as the bytes a terminal sends for it, in
// the cursor mode the terminal is in now; an event with no bytes sends nothing.  Signals
// `romacs-terminal-not-running' as `romacs--term-send' does.  Returns nil.
defsubr! { name = "romacs--term-send-event", gnu = "romacs:terminal", effect = Global,
fn terminal_send_event(cx, session: Integer, event: Value) {
    cx.terminal_send_event(session.0, event)
}}

// Terminal SESSION's state as a plist: `:running', `:alternate', `:bracketed-paste',
// `:application-cursor', `:following' (for the selected window), `:shell-foreground', and,
// once the terminal has published a screen, `:cursor' and `:screen-start', positions in
// its buffer.
defsubr! { name = "romacs--term-state", gnu = "romacs:terminal", effect = Reads,
fn terminal_state(cx, session: Integer) {
    cx.terminal_state(session.0)
}}
