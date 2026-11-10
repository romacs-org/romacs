//! sqlite.c: databases and result sets as Lisp `sqlite' objects over
//! `crate::sqlite', with GNU's value binding, row conversion and signals.
use super::*;
use crate::sqlite::{Bind, Column, Connection, Handle, State, Statement, Step};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

type Out = super::super::Out;
type Result<T> = std::result::Result<T, Fault>;

/// GNU's `db_count': each in-memory database is named `:memory:N'.
static MEMORY_COUNT: AtomicUsize = AtomicUsize::new(0);

/// sqlite-load-extension's allowlist (sqlite.c).
const EXTENSIONS: &[&str] = &[
    "base64", "cksumvfs", "compress", "csv", "csvtable", "fts3", "icu", "pcre", "percentile", "regexp",
    "rot13", "rtree", "sha1", "uuid", "vec0", "vector0", "vfslog", "vss0", "zipfile",
];

/// encode_string: a multibyte string's internal bytes as UTF-8 for SQLite
/// (encode_string_utf_8 with HANDLE-8-BIT and HANDLE-OVER-UNI t): a raw-byte
/// character becomes its byte, every other character keeps its bytes. A
/// unibyte string passes unchanged.
fn encoded(bytes: &[u8], multibyte: bool) -> Vec<u8> {
    if !multibyte {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let (code, width) = crate::lstring::string_char(&bytes[at..]);
        let width = width.max(1);
        if code >= crate::lstring::BYTE8_OFFSET + 0x80 {
            out.push((code - crate::lstring::BYTE8_OFFSET) as u8);
        } else {
            out.extend_from_slice(&bytes[at..at + width]);
        }
        at += width;
    }
    out
}

/// SQL as a C string ends at its first NUL.
fn c_string(bytes: &[u8]) -> &[u8] {
    bytes.split(|&b| b == 0).next().unwrap_or_default()
}

/// sqlite-load-extension: MODULE's file name, less a `libsqlite3_mod_'
/// prefix, is an allowed name followed by a shared-library suffix.
fn extension_allowed(module: &str) -> bool {
    let name = module.rsplit('/').next().unwrap_or(module);
    let name = name.strip_prefix("libsqlite3_mod_").unwrap_or(name);
    EXTENSIONS.iter().any(|allow| {
        name.len() > allow.len() && name.starts_with(allow) && {
            let suffix = &name[allow.len()..];
            suffix == ".so" || suffix == ".dylib" || suffix.eq_ignore_ascii_case(".dll")
        }
    })
}

fn locked(code: i32) -> bool {
    code == crate::sqlite::BUSY || code == crate::sqlite::LOCKED
}

impl Leaf<'_> {
    /// build_string: unibyte when ASCII.
    fn sqlite_string(&mut self, text: &str) -> Value {
        self.make_string(text.as_bytes().to_vec(), !text.is_ascii())
    }
    fn sqlite_list(&mut self, values: Vec<Value>) -> Value {
        values.into_iter().rev().fold(Value::NIL, |tail, v| self.cons(v, tail))
    }
    fn sqlite_signal(&mut self, is_locked: bool, data: Value) -> Fault {
        let condition = self.intern(if is_locked { "sqlite-locked-error" } else { "sqlite-error" });
        let data = self.cons(data, Value::NIL);
        Fault::Signal(condition, data)
    }
    fn sqlite_message(&mut self, message: &str) -> Fault {
        let message = self.sqlite_string(message);
        self.sqlite_signal(false, message)
    }
    fn sqlite_handle(&self, object: Value) -> Option<&Handle> {
        match self.vm.object_if(object) {
            Some(data::Object::Sqlite(handle)) => Some(handle),
            _ => None,
        }
    }
    fn sqlite_handle_mut(&mut self, object: Value) -> &mut Handle {
        let index = object.index(0).expect("checked sqlite object") - 1;
        match self.vm.objects.entry_mut(index) {
            data::Object::Sqlite(handle) => handle,
            _ => unreachable!("checked sqlite object"),
        }
    }
    /// check_sqlite: OBJECT is a live database, or with SET a live set.
    fn sqlite_check(&mut self, object: Value, set: bool) -> Result<()> {
        let Some(handle) = self.sqlite_handle(object) else { return Err(Fault::Wrong("sqlitep", object)) };
        let message = match (&handle.state, set) {
            (State::Set { .. }, false) => Some("Invalid database object"),
            (State::Database(_), true) => Some("Invalid set object"),
            (State::Database(None), false) => Some("Database closed"),
            (State::Set { statement: None, .. }, true) => Some("Statement closed"),
            _ => None,
        };
        match message {
            Some(message) => Err(self.sqlite_message(message)),
            None => Ok(()),
        }
    }
    fn sqlite_connection(&mut self, db: Value) -> Result<Arc<Connection>> {
        self.sqlite_check(db, false)?;
        match &self.sqlite_handle(db).expect("checked").state {
            State::Database(Some(connection)) => Ok(connection.clone()),
            _ => unreachable!("checked"),
        }
    }
    fn sqlite_query(&mut self, query: Value) -> Result<Vec<u8>> {
        let view = self.string_view(query)?;
        Ok(encoded(view.bytes(), view.is_multibyte()))
    }

    pub(crate) fn sqlite_open(&mut self, file: Option<Vec<u8>>, readonly: Value, disable_uri: Value) -> Out {
        let (name, memory) = match file {
            Some(file) => (file, false),
            None => (format!(":memory:{}", MEMORY_COUNT.fetch_add(1, Ordering::Relaxed) + 1).into_bytes(), true),
        };
        let name = c_string(&name);
        let Some(connection) = Connection::open(name, readonly != Value::NIL, disable_uri == Value::NIL, memory) else {
            return Ok(Value::NIL);
        };
        let handle = Handle { name: String::from_utf8_lossy(name).into_owned(), state: State::Database(Some(connection)) };
        Ok(self.vm.allocate_object(data::Object::Sqlite(Box::new(handle))))
    }

    pub(crate) fn sqlite_close(&mut self, db: Value) -> Out {
        // Closing what is already closed (or a finalized set) does nothing.
        if let Some(handle) = self.sqlite_handle(db)
            && matches!(handle.state, State::Database(None) | State::Set { statement: None, .. }) {
            return Ok(Value::TRUE);
        }
        self.sqlite_check(db, false)?;
        // The database closes now unless a live set still holds it.
        self.sqlite_handle_mut(db).state = State::Database(None);
        Ok(Value::TRUE)
    }

    /// bind_values: reset STATEMENT, then convert and bind each of VALUES (a
    /// list or vector, whose length is checked first) in turn; the message
    /// GNU reports for the first value it cannot bind.
    fn sqlite_bind(&mut self, statement: &mut Statement, values: Value) -> Result<Option<String>> {
        let items = if self.kind(values) == Kind::Vector {
            self.vector_values(values)?
        } else {
            // list_length: an improper or circular list signals first.
            let mut items = vec![];
            let mut seen = HashSet::default();
            let mut tail = values;
            while let Some((item, rest)) = self.pair(tail) {
                if !seen.insert(tail.0) { return Err(Fault::Circular(values)); }
                items.push(item);
                tail = rest;
            }
            if tail != Value::NIL { return Err(Fault::Wrong("listp", tail)); }
            items
        };
        statement.reset();
        for (index, value) in items.into_iter().enumerate() {
            let Some(bind) = self.sqlite_value(value)? else { return Ok(Some("invalid argument".into())) };
            if let Err(message) = statement.bind(index + 1, &bind) {
                return Ok(Some(message));
            }
        }
        Ok(None)
    }
    /// One bind_values value, or None when it has no SQLite form.
    fn sqlite_value(&mut self, value: Value) -> Result<Option<Bind>> {
        let (coding_system, binary, false_) = (self.intern("coding-system"), self.intern("binary"), self.intern("false"));
        Ok(Some(match self.kind(value) {
            Kind::String | Kind::Bytes => {
                let view = self.string_view(value)?;
                let coding = if view.bytes().is_empty() { Value::NIL } else {
                    let raw = self.vm.annotation_raw(value, 0).map_err(Fault::Error)?;
                    self.vm.annotation_resolve(&raw, coding_system).map_err(Fault::Error)?
                };
                if coding == binary {
                    if view.len() != view.bytes().len() {
                        return Err(self.sqlite_message("BLOB values must be unibyte"));
                    }
                    Bind::Blob(view.bytes().to_vec())
                } else if coding == Value::NIL {
                    Bind::Text(encoded(view.bytes(), view.is_multibyte()))
                } else {
                    let text = self.coding_convert_string(value, coding, Value::NIL, Value::NIL, true)?;
                    Bind::Text(self.string_view(text)?.bytes().to_vec())
                }
            }
            Kind::Integer | Kind::Bignum => {
                // bignum_to_intmax: 0 when out of range.
                Bind::Integer(self.bigint_value(value).and_then(|n| i64::try_from(n).ok()).unwrap_or(0))
            }
            Kind::Float => Bind::Float(self.float_value(value)?),
            _ if value == Value::NIL => Bind::Null,
            _ if value == Value::TRUE => Bind::Integer(1),
            _ if value == false_ => Bind::Integer(0),
            _ => return Ok(None),
            }))
    }

    /// The checks, prepare and bind shared by execute and select. WHOLE
    /// prepares all of QUERY rather than QUERY as a C string; CAN_LOCK lets a
    /// busy or locked prepare signal `sqlite-locked-error'.
    fn sqlite_prepare(&mut self, db: Value, query: Value, values: Value, whole: bool, can_lock: bool) -> Result<Statement> {
        let connection = self.sqlite_connection(db)?;
        if !matches!(self.kind(query), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", query)); }
        if !(values == Value::NIL || self.pair(values).is_some() || self.kind(values) == Kind::Vector) {
            return Err(self.sqlite_message("VALUES must be a list or a vector"));
        }
        let sql = self.sqlite_query(query)?;
        let mut statement = match connection.prepare(if whole { &sql } else { c_string(&sql) }) {
            Ok(statement) => statement,
            Err(e) => {
                let errstr = self.sqlite_string(&e.errstr);
                let errmsg = match &e.errmsg { Some(m) => self.sqlite_string(m), None => Value::NIL };
                let code = Value::integer(e.code).map_err(Fault::Error)?;
                let extended = Value::integer(e.extended).map_err(Fault::Error)?;
                let data = self.sqlite_list(vec![errstr, errmsg, code, extended]);
                return Err(self.sqlite_signal(can_lock && locked(e.code as i32), data));
            }
        };
        if values != Value::NIL && let Some(message) = self.sqlite_bind(&mut statement, values)? {
            return Err(self.sqlite_message(&message));
        }
        Ok(statement)
    }

    /// row_to_value: text decoded as `utf-8' without recording the coding
    /// system (code_convert_string_norecord), always multibyte.
    fn sqlite_row(&mut self, statement: &Statement) -> Out {
        let columns = statement.row();
        let inhibit = self.intern("inhibit-eol-conversion");
        let inhibit = self.variable_value(inhibit)?.is_some_and(|v| v != Value::NIL);
        let mut values = Vec::with_capacity(columns.len());
        for column in columns {
            values.push(match column {
                Column::Integer(n) => self.bigint(n.into()),
                Column::Float(x) => self.float(x),
                Column::Blob(bytes) => self.make_string(bytes, false),
                Column::Null => Value::NIL,
                Column::Text(bytes) => {
                    let text = super::coding::decode_utf8_detecting_eol(&bytes, inhibit);
                    self.make_string(text, true)
                }
            });
        }
        Ok(self.sqlite_list(values))
    }
    fn sqlite_column_names(&mut self, statement: &Statement) -> Value {
        let names: Vec<Value> = statement.column_names().iter().map(|n| self.sqlite_string(n)).collect();
        self.sqlite_list(names)
    }

    pub(crate) fn sqlite_execute(&mut self, db: Value, query: Value, values: Value) -> Out {
        let mut statement = self.sqlite_prepare(db, query, values, false, true)?;
        match statement.step() {
            Step::Row => {
                let mut rows = vec![self.sqlite_row(&statement)?];
                while matches!(statement.step(), Step::Row) {
                    rows.push(self.sqlite_row(&statement)?);
                }
                Ok(self.sqlite_list(rows))
            }
            Step::Done => Value::integer(statement.connection().changes()).map_err(Fault::Error),
            Step::Error(code) => {
                let message = statement.connection().errmsg();
                let message = self.sqlite_string(&message);
                Err(self.sqlite_signal(locked(code), message))
            }
        }
    }

    pub(crate) fn sqlite_select(&mut self, db: Value, query: Value, values: Value, return_type: Value) -> Out {
        let mut statement = self.sqlite_prepare(db, query, values, true, false)?;
        if return_type == self.intern("set") {
            let name = self.sqlite_handle(db).expect("checked").name.clone();
            let set = Handle { name, state: State::Set { statement: Some(statement), eof: false } };
            return Ok(self.vm.allocate_object(data::Object::Sqlite(Box::new(set))));
        }
        let mut rows = vec![];
        // A step that fails ends the rows, as in GNU.
        while matches!(statement.step(), Step::Row) {
            rows.push(self.sqlite_row(&statement)?);
        }
        let rows = self.sqlite_list(rows);
        if return_type == self.intern("full") {
            let columns = self.sqlite_column_names(&statement);
            return Ok(self.cons(columns, rows));
        }
        Ok(rows)
    }

    /// sqlite_exec of SQL in DB: t when it succeeded, else nil.
    pub(crate) fn sqlite_exec(&mut self, db: Value, sql: &[u8]) -> Out {
        let connection = self.sqlite_connection(db)?;
        Ok(if connection.exec(sql) { Value::TRUE } else { Value::NIL })
    }
    pub(crate) fn sqlite_execute_batch(&mut self, db: Value, statements: Value) -> Out {
        self.sqlite_check(db, false)?;
        if !matches!(self.kind(statements), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", statements)); }
        let sql = self.sqlite_query(statements)?;
        self.sqlite_exec(db, &sql)
    }
    /// sqlite-pragma sends `PRAGMA ' and PRAGMA's internal bytes unencoded.
    pub(crate) fn sqlite_pragma(&mut self, db: Value, pragma: Value) -> Out {
        self.sqlite_check(db, false)?;
        if !matches!(self.kind(pragma), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", pragma)); }
        let mut sql = b"PRAGMA ".to_vec();
        sql.extend_from_slice(self.string_view(pragma)?.bytes());
        self.sqlite_exec(db, &sql)
    }
    /// check_sqlite for a database, before a caller's own argument checks.
    pub(crate) fn sqlite_database(&mut self, db: Value) -> Result<()> {
        self.sqlite_check(db, false)
    }
    pub(crate) fn sqlite_extension_allowed(&self, module: &str) -> bool {
        extension_allowed(module)
    }
    pub(crate) fn sqlite_extension_refused(&mut self) -> Fault {
        self.sqlite_message("Module name not on allowlist")
    }
    /// sqlite-load-extension of the expanded, encoded PATH, once MODULE
    /// passed the allowlist.
    pub(crate) fn sqlite_load_extension(&mut self, db: Value, path: &[u8]) -> Out {
        let connection = self.sqlite_connection(db)?;
        Ok(if connection.load_extension(c_string(path)) { Value::TRUE } else { Value::NIL })
    }

    pub(crate) fn sqlite_next(&mut self, set: Value) -> Out {
        self.sqlite_check(set, true)?;
        // The statement leaves the set while its row is built from VM
        // allocations, then returns.
        let State::Set { statement, eof } = &mut self.sqlite_handle_mut(set).state else { unreachable!("checked") };
        if *eof { return Ok(Value::NIL); }
        let mut statement = statement.take().expect("checked");
        let step = statement.step();
        *eof = matches!(step, Step::Done);
        let result = match step {
            Step::Done => Ok(Value::NIL),
            Step::Row => self.sqlite_row(&statement),
            Step::Error(_) => {
                let message = statement.connection().errmsg();
                Err(self.sqlite_message(&message))
            }
        };
        if let State::Set { statement: slot, .. } = &mut self.sqlite_handle_mut(set).state {
            *slot = Some(statement);
        }
        result
    }

    pub(crate) fn sqlite_columns(&mut self, set: Value) -> Out {
        self.sqlite_check(set, true)?;
        let State::Set { statement: Some(statement), .. } = &self.sqlite_handle(set).expect("checked").state else { unreachable!("checked") };
        let names = statement.column_names();
        let names: Vec<Value> = names.iter().map(|n| self.sqlite_string(n)).collect();
        Ok(self.sqlite_list(names))
    }

    pub(crate) fn sqlite_more_p(&mut self, set: Value) -> Out {
        self.sqlite_check(set, true)?;
        let eof = matches!(self.sqlite_handle(set).expect("checked").state, State::Set { eof: true, .. });
        Ok(if eof { Value::NIL } else { Value::TRUE })
    }

    pub(crate) fn sqlite_finalize(&mut self, set: Value) -> Out {
        self.sqlite_check(set, true)?;
        if let State::Set { statement, .. } = &mut self.sqlite_handle_mut(set).state {
            *statement = None;
        }
        Ok(Value::TRUE)
    }

    pub(crate) fn sqlite_version(&mut self) -> Value {
        let version = crate::sqlite::version();
        self.sqlite_string(&version)
    }

    pub(crate) fn sqlitep(&self, object: Value) -> Value {
        if self.sqlite_handle(object).is_some() { Value::TRUE } else { Value::NIL }
    }
}
