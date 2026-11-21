//! The SQLite library behind GNU sqlite.c's primitives. This module is the
//! only unsafe boundary: `prim/leaf/sqlite.rs` and `prims/sqlite.rs` use the
//! safe types below. SQLite is linked from the bundled amalgamation, so no
//! system library is required.
use libsqlite3_sys as ffi;
use std::ffi::{CStr, c_int, c_void};
use std::sync::Arc;

/// The database's virtual machine instructions between two asks of a quit
/// probe: a few microseconds.
const PROGRESS_STEPS: c_int = 1000;

#[cfg(test)]
thread_local! {
    /// The instructions between two asks, which a test sets to place an ask
    /// at a chosen instruction of a statement.
    pub(crate) static PROGRESS_PERIOD: std::cell::Cell<c_int> = const { std::cell::Cell::new(PROGRESS_STEPS) };
}
#[cfg(test)]
fn progress_period() -> c_int {
    PROGRESS_PERIOD.with(std::cell::Cell::get)
}
#[cfg(not(test))]
fn progress_period() -> c_int {
    PROGRESS_STEPS
}

/// What the progress handler asks: STOP, while STATEMENT runs (null: none
/// does, as while a batch prepares its next).
struct Watch<'a> {
    stop: &'a crate::quit::NativeStop,
    statement: std::cell::Cell<*mut ffi::sqlite3_stmt>,
}

/// A database's mutex, held until dropped, as `sqlite3_exec` holds it for a
/// batch: each call inside only enters it again.
struct Held(*mut ffi::sqlite3_mutex);
impl Held {
    fn enter(db: *mut ffi::sqlite3) -> Self {
        // UNSAFE(this module is libsqlite3-sys's one boundary: the mutex is the library's)
        // SAFETY: DB is open; a null mutex (no threading) is entered as a no-op.
        Self(unsafe {
            let mutex = ffi::sqlite3_db_mutex(db);
            ffi::sqlite3_mutex_enter(mutex);
            mutex
        })
    }
}
impl Drop for Held {
    fn drop(&mut self) {
        // UNSAFE(this module is libsqlite3-sys's one boundary: the mutex is the library's)
        // SAFETY: the mutex was entered by `Held::enter`, on this thread.
        unsafe { ffi::sqlite3_mutex_leave(self.0) };
    }
}

/// BODY with DB's progress handler asking WATCH every [`PROGRESS_STEPS`]
/// virtual machine instructions, when its probe is armed. Once the probe
/// says to stop, the step running ends with `SQLITE_INTERRUPT`, which rolls
/// the statement back (and, for a write inside an explicit transaction, the
/// transaction), as `sqlite3_interrupt` does. Once the statement has halted,
/// its work is done, committed in autocommit mode, so the probe is not
/// asked: a stop there would report a quit with the write kept. The press
/// then waits for the caller's next poll.
fn watching<T>(db: *mut ffi::sqlite3, watch: &Watch, body: impl FnOnce() -> T) -> T {
    /// Removes the handler, however BODY ends, before the watch's borrow does.
    struct Installed(*mut ffi::sqlite3);
    impl Drop for Installed {
        fn drop(&mut self) {
            // UNSAFE(this module is libsqlite3-sys's one boundary: rusqlite's safe handler needs its own Connection)
            // SAFETY: the database is open; removing the handler takes no argument.
            unsafe { ffi::sqlite3_progress_handler(self.0, 0, None, std::ptr::null_mut()) };
        }
    }
    extern "C" fn progress(watch: *mut c_void) -> c_int {
        // UNSAFE(SQLite hands the handler its argument back as a raw pointer)
        // SAFETY: the argument is the watch `watching` borrows for as long as
        // the handler is installed (`Installed` removes it first).
        let watch = unsafe { &*watch.cast::<Watch>() };
        let statement = watch.statement.get();
        // UNSAFE(this module is libsqlite3-sys's one boundary: the statement's state is SQLite's)
        // SAFETY: a watched statement is live: its watch is cleared before it
        // is finalized; sqlite3_stmt_busy only reads it.
        let running = !statement.is_null() && unsafe { ffi::sqlite3_stmt_busy(statement) } != 0;
        c_int::from(running && watch.stop.due())
    }
    if !watch.stop.armed() { return body(); }
    // UNSAFE(this module is libsqlite3-sys's one boundary: rusqlite's safe handler needs its own Connection)
    // SAFETY: the database is open; WATCH outlives the installation, which
    // `Installed` ends before this function returns or unwinds.
    unsafe { ffi::sqlite3_progress_handler(db, progress_period(), Some(progress), std::ptr::from_ref(watch).cast_mut().cast()) };
    let installed = Installed(db);
    let out = body();
    drop(installed);
    out
}

/// An open database. Each statement holds it, so `sqlite3_close` runs once
/// the database object and every statement prepared on it have gone.
pub(crate) struct Connection {
    db: *mut ffi::sqlite3,
}
// SAFETY: databases are opened SQLITE_OPEN_FULLMUTEX (serialized mode).
unsafe impl Send for Connection {}
unsafe impl Sync for Connection {}
impl Drop for Connection {
    fn drop(&mut self) {
        // SAFETY: `db` came from sqlite3_open_v2 and is closed only here.
        unsafe { ffi::sqlite3_close(self.db) };
    }
}

/// A prepared statement, finalized when dropped.
pub(crate) struct Statement {
    stmt: *mut ffi::sqlite3_stmt,
    connection: Arc<Connection>,
}
// SAFETY: a statement is used by one VM at a time; its database is serialized.
unsafe impl Send for Statement {}
impl Drop for Statement {
    fn drop(&mut self) {
        // SAFETY: `stmt` came from sqlite3_prepare_v2 and is finalized only here.
        unsafe { ffi::sqlite3_finalize(self.stmt) };
    }
}

/// A value bound to a `?` parameter.
pub(crate) enum Bind {
    Text(Vec<u8>),
    Blob(Vec<u8>),
    Integer(i64),
    Float(f64),
    Null,
}

/// A result column's value.
pub(crate) enum Column {
    Integer(i64),
    Float(f64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
    Null,
}

/// What `sqlite3_step` reported.
pub(crate) enum Step {
    Row,
    Done,
    /// Any other result code.
    Error(c_int),
}

/// sqlite_prepare_errdata: (ERRSTR ERRMSG CODE EXTENDED-CODE).
pub(crate) struct PrepareError {
    pub errstr: String,
    pub errmsg: Option<String>,
    pub code: i64,
    pub extended: i64,
}

fn text(pointer: *const std::ffi::c_char) -> Option<String> {
    // SAFETY: SQLite returns NUL-terminated strings valid until the next call.
    (!pointer.is_null()).then(|| unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned())
}

pub(crate) const BUSY: c_int = ffi::SQLITE_BUSY;
/// Whether result CODE is `SQLITE_INTERRUPT`: the progress handler's own
/// stop (`Statement::step`), the only error a quit may stand for.
pub(crate) fn interrupted(code: c_int) -> bool {
    code & 0xff == ffi::SQLITE_INTERRUPT
}
/// The most memory the library has had in use since the last ask, which resets it
/// (`SQLITE_STATUS_MEMORY_USED`).
#[cfg(test)]
pub(crate) fn memory_highwater() -> i64 {
    let (mut current, mut highest) = (0, 0);
    // UNSAFE(this module is libsqlite3-sys's one boundary: the counter is SQLite's)
    // SAFETY: both pointers are to live locals; the call only writes them.
    unsafe { ffi::sqlite3_status64(ffi::SQLITE_STATUS_MEMORY_USED, &raw mut current, &raw mut highest, 1) };
    highest
}
pub(crate) const LOCKED: c_int = ffi::SQLITE_LOCKED;

pub(crate) fn version() -> String {
    // SAFETY: sqlite3_libversion returns a static string.
    text(unsafe { ffi::sqlite3_libversion() }).unwrap_or_default()
}

impl Connection {
    /// sqlite3_open_v2 of NAME (UTF-8 file name or `:memory:N`); None on failure.
    pub(crate) fn open(name: &[u8], readonly: bool, uri: bool, memory: bool) -> Option<Arc<Self>> {
        let mut flags = if readonly { ffi::SQLITE_OPEN_READONLY } else { ffi::SQLITE_OPEN_CREATE | ffi::SQLITE_OPEN_READWRITE };
        flags |= ffi::SQLITE_OPEN_FULLMUTEX;
        if uri { flags |= ffi::SQLITE_OPEN_URI; }
        if memory { flags |= ffi::SQLITE_OPEN_MEMORY; }
        let name = std::ffi::CString::new(name).ok()?;
        let mut db = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated and `db` receives the handle.
        let result = unsafe { ffi::sqlite3_open_v2(name.as_ptr(), &mut db, flags, std::ptr::null()) };
        if result != ffi::SQLITE_OK {
            // SAFETY: sqlite3_open_v2 may return a handle even on failure.
            if !db.is_null() { unsafe { ffi::sqlite3_close(db) }; }
            return None;
        }
        Some(Arc::new(Self { db }))
    }
    pub(crate) fn address(&self) -> usize {
        self.db as usize
    }
    pub(crate) fn errmsg(&self) -> String {
        // SAFETY: `db` is open.
        text(unsafe { ffi::sqlite3_errmsg(self.db) }).unwrap_or_default()
    }
    pub(crate) fn changes(&self) -> i64 {
        // SAFETY: `db` is open.
        i64::from(unsafe { ffi::sqlite3_changes(self.db) })
    }
    /// The statements of SQL up to its first NUL in turn, each stepped to
    /// its end with STOP asked as it runs (`Statement::step`), as
    /// `sqlite3_exec` runs them without a callback; the result code of the
    /// first that failed. The batch is one buffer ended by a NUL, each
    /// statement prepared from where the last one ended, so the library
    /// copies none of it (a prepare given a length with no NUL copies the rest).
    pub(crate) fn exec(self: &Arc<Self>, sql: &[u8], stop: &crate::quit::NativeStop) -> Result<(), c_int> {
        let mut batch = sql.split(|&b| b == 0).next().unwrap_or_default().to_vec();
        batch.push(0);
        let _held = Held::enter(self.db);
        // One handler for the whole batch, watching each statement in turn.
        let watch = Watch { stop, statement: std::cell::Cell::new(std::ptr::null_mut()) };
        watching(self.db, &watch, || {
            let mut rest = &batch[..];
            // Up to the NUL, which stays at the end of REST.
            while rest.len() > 1 {
                let (mut statement, used) = self.prepare_next(rest, -1).map_err(|e| c_int::try_from(e.code).unwrap_or(ffi::SQLITE_ERROR))?;
                let used = used.min(rest.len() - 1);
                // A prepare that consumed nothing leaves nothing it could run.
                if used == 0 { break; }
                rest = &rest[used..];
                // Only blanks or a comment: nothing to run.
                if statement.stmt.is_null() { continue; }
                watch.statement.set(statement.stmt);
                let outcome = loop {
                    match statement.step_unwatched() {
                        Step::Row => {}
                        Step::Done => break Ok(()),
                        Step::Error(code) => break Err(code),
                    }
                };
                // Unwatched before it is finalized.
                watch.statement.set(std::ptr::null_mut());
                drop(statement);
                outcome?;
            }
            Ok(())
        })
    }
    /// `sqlite3_prepare_v2` of the first statement in SQL.
    pub(crate) fn prepare(self: &Arc<Self>, sql: &[u8]) -> Result<Statement, PrepareError> {
        let length = c_int::try_from(sql.len()).unwrap_or(c_int::MAX);
        self.prepare_next(sql, length).map(|(statement, _)| statement)
    }
    /// `sqlite3_prepare_v2` of the first statement in SQL, read for LENGTH
    /// bytes (or, with -1, to its NUL), and the bytes of SQL it used. An
    /// EXPLAIN writes nothing, whatever it explains.
    fn prepare_next(self: &Arc<Self>, sql: &[u8], length: c_int) -> Result<(Statement, usize), PrepareError> {
        let mut stmt = std::ptr::null_mut();
        let mut tail = std::ptr::null();
        // SAFETY: `db` is open; SQL is read for LENGTH bytes or, with -1, up
        // to the NUL it then ends with (`exec`), and TAIL is left within them.
        let code = unsafe { ffi::sqlite3_prepare_v2(self.db, sql.as_ptr().cast(), length, &mut stmt, std::ptr::from_mut(&mut tail)) };
        if code != ffi::SQLITE_OK {
            // SAFETY: finalizing null is a no-op; the error calls read `db`.
            unsafe { ffi::sqlite3_finalize(stmt) };
            return Err(PrepareError {
                errstr: text(unsafe { ffi::sqlite3_errstr(code) }).unwrap_or_default(),
                errmsg: text(unsafe { ffi::sqlite3_errmsg(self.db) }),
                code: i64::from(code),
                extended: i64::from(unsafe { ffi::sqlite3_extended_errcode(self.db) }),
            });
        }
        // SQLite left TAIL within SQL, at or past its start.
        let used = tail.addr().saturating_sub(sql.as_ptr().addr()).min(sql.len());
        Ok((Statement { stmt, connection: self.clone() }, used))
    }
}

impl Statement {
    /// Whether the statement writes nothing (`sqlite3_stmt_readonly`), or
    /// only explains one that would (`sqlite3_stmt_isexplain`). A writing
    /// statement (INSERT ... RETURNING) quits only through the library's own
    /// interrupt, which rolls it back, never between its rows, where
    /// finalizing it would commit its write.
    pub(crate) fn readonly(&self) -> bool {
        // UNSAFE(this module is libsqlite3-sys's one boundary: the statement's flags are SQLite's)
        // SAFETY: `stmt` is a live statement, or null for blank SQL, which
        // both calls answer for; they only read it.
        unsafe { ffi::sqlite3_stmt_readonly(self.stmt) != 0 || ffi::sqlite3_stmt_isexplain(self.stmt) != 0 }
    }
    pub(crate) fn address(&self) -> usize {
        self.stmt as usize
    }
    pub(crate) fn connection(&self) -> &Arc<Connection> {
        &self.connection
    }
    pub(crate) fn reset(&mut self) {
        // SAFETY: `stmt` is a live statement.
        unsafe { ffi::sqlite3_reset(self.stmt) };
    }
    /// Bind VALUE to parameter INDEX (from 1); the database's message when
    /// SQLite refuses it.
    pub(crate) fn bind(&mut self, index: usize, value: &Bind) -> Result<(), String> {
        let index = c_int::try_from(index).unwrap_or(c_int::MAX);
        // SAFETY: `stmt` is a live statement; text and blob buffers are read
        // for their exact 64-bit lengths and copied (SQLITE_TRANSIENT) before
        // the call returns.
        let result = unsafe {
            match value {
                Bind::Text(bytes) => ffi::sqlite3_bind_text64(self.stmt, index, bytes.as_ptr().cast(), bytes.len() as u64, ffi::SQLITE_TRANSIENT(), ffi::SQLITE_UTF8 as u8),
                Bind::Blob(bytes) => ffi::sqlite3_bind_blob64(self.stmt, index, bytes.as_ptr().cast(), bytes.len() as u64, ffi::SQLITE_TRANSIENT()),
                Bind::Integer(n) => ffi::sqlite3_bind_int64(self.stmt, index, *n),
                Bind::Float(x) => ffi::sqlite3_bind_double(self.stmt, index, *x),
                Bind::Null => ffi::sqlite3_bind_null(self.stmt, index),
            }
        };
        if result == ffi::SQLITE_OK { Ok(()) } else { Err(self.connection.errmsg()) }
    }
    /// `sqlite3_step`, with STOP asked as the statement runs (`watching`).
    pub(crate) fn step(&mut self, stop: &crate::quit::NativeStop) -> Step {
        let watch = Watch { stop, statement: std::cell::Cell::new(self.stmt) };
        let db = self.connection.db;
        watching(db, &watch, || self.step_unwatched())
    }
    /// The statement's virtual machine instructions so far
    /// (`SQLITE_STMTSTATUS_VM_STEP`).
    #[cfg(test)]
    pub(crate) fn instructions(&self) -> i64 {
        // UNSAFE(this module is libsqlite3-sys's one boundary: the counter is SQLite's)
        // SAFETY: `stmt` is a live statement; the counter is only read.
        i64::from(unsafe { ffi::sqlite3_stmt_status(self.stmt, ffi::SQLITE_STMTSTATUS_VM_STEP, 0) })
    }
    fn step_unwatched(&mut self) -> Step {
        // SAFETY: `stmt` is a live statement.
        match unsafe { ffi::sqlite3_step(self.stmt) } {
            ffi::SQLITE_ROW => Step::Row,
            ffi::SQLITE_DONE | ffi::SQLITE_OK => Step::Done,
            code => Step::Error(code),
        }
    }
    /// row_to_value's columns of the current row.
    pub(crate) fn row(&self) -> Vec<Column> {
        // SAFETY: called after sqlite3_step returned SQLITE_ROW; column
        // buffers are copied before the next SQLite call.
        unsafe {
            let count = ffi::sqlite3_column_count(self.stmt);
            (0..count).map(|i| {
                let bytes = |pointer: *const u8| {
                    let length = ffi::sqlite3_column_bytes(self.stmt, i) as usize;
                    if pointer.is_null() { Vec::new() } else { std::slice::from_raw_parts(pointer, length).to_vec() }
                };
                match ffi::sqlite3_column_type(self.stmt, i) {
                    ffi::SQLITE_INTEGER => Column::Integer(ffi::sqlite3_column_int64(self.stmt, i)),
                    ffi::SQLITE_FLOAT => Column::Float(ffi::sqlite3_column_double(self.stmt, i)),
                    ffi::SQLITE_BLOB => Column::Blob(bytes(ffi::sqlite3_column_blob(self.stmt, i).cast())),
                    ffi::SQLITE_TEXT => Column::Text(bytes(ffi::sqlite3_column_text(self.stmt, i))),
                    _ => Column::Null,
                }
            }).collect()
        }
    }
    pub(crate) fn column_names(&self) -> Vec<String> {
        // SAFETY: `stmt` is a live statement.
        unsafe {
            let count = ffi::sqlite3_column_count(self.stmt);
            (0..count).map(|i| text(ffi::sqlite3_column_name(self.stmt, i)).unwrap_or_default()).collect()
        }
    }
}

impl Connection {
    /// sqlite-load-extension: load PATH with extension loading enabled only
    /// for the call; true when it loaded.
    pub(crate) fn load_extension(&self, path: &[u8]) -> bool {
        let Ok(path) = std::ffi::CString::new(path) else { return false };
        // SAFETY: `db` is open; loading is re-disabled before returning.
        unsafe {
            let enable = ffi::SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION;
            if ffi::sqlite3_db_config(self.db, enable, 1 as c_int, std::ptr::null_mut::<c_int>()) != ffi::SQLITE_OK {
                return false;
            }
            let result = ffi::sqlite3_load_extension(self.db, path.as_ptr(), std::ptr::null(), std::ptr::null_mut());
            ffi::sqlite3_db_config(self.db, enable, 0 as c_int, std::ptr::null_mut::<c_int>());
            result == ffi::SQLITE_OK
        }
    }
}

/// The Lisp `sqlite` object: a database or a result set (`set` mode).
pub(crate) struct Handle {
    /// The database's file or `:memory:N` name; a set shares its database's.
    pub(crate) name: String,
    pub(crate) state: State,
}

/// A copy owns nothing: `copy-sequence' refuses sqlite objects, so no Lisp
/// operation copies one, and a copy cannot share the statement.
impl Clone for Handle {
    fn clone(&self) -> Self {
        let state = match &self.state {
            State::Database(_) => State::Database(None),
            State::Set { eof, .. } => State::Set { statement: None, eof: *eof },
        };
        Handle { name: self.name.clone(), state }
    }
}

pub(crate) enum State {
    /// None once `sqlite-close` closed it.
    Database(Option<Arc<Connection>>),
    /// None once `sqlite-finalize` finalized it.
    Set { statement: Option<Statement>, eof: bool },
}

impl Handle {
    /// Whether it still holds SQLite's connection or statement.
    pub(crate) fn is_live(&self) -> bool {
        match &self.state {
            State::Database(db) => db.is_some(),
            State::Set { statement, .. } => statement.is_some(),
        }
    }
}

impl Handle {
    /// print.c: `#<sqlite db=PTR [stmt=PTR] name=NAME>`, a closed pointer as
    /// glibc's `(nil)`.
    pub(crate) fn describe(&self) -> String {
        let pointer = |p: usize| if p == 0 { "(nil)".to_string() } else { format!("{p:#x}") };
        match &self.state {
            State::Database(db) => format!("#<sqlite db={} name={}>", pointer(db.as_ref().map_or(0, |d| d.address())), self.name),
            State::Set { statement, .. } => format!(
                "#<sqlite db={} stmt={} name={}>",
                pointer(statement.as_ref().map_or(0, |s| s.connection().address())),
                pointer(statement.as_ref().map_or(0, |s| s.address())),
                self.name
            ),
        }
    }
    pub(crate) fn storage_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.name.capacity()
    }
}
