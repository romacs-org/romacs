use super::*;
use crate::prim::Out;

impl Leaf<'_> {
    pub(crate) fn process_start(&mut self, argv: Value, input: Value, directory: Value, seconds: i64) -> Out {
        let values = self.vm.list_values(argv).map_err(Fault::Error)?;
        let argv = values.into_iter().map(|v| self.string_text(v)).collect::<std::result::Result<Vec<_>, _>>()?;
        let input = self.string_view(input)?.bytes.to_vec();
        let directory = std::path::PathBuf::from(self.string_text(directory)?);
        let seconds = u64::try_from(seconds).map_err(|_| Fault::Error("Invalid process timeout".into()))?;
        let id = self.vm.process_jobs.start(crate::process_jobs::Request {
            argv, input, directory, timeout: std::time::Duration::from_secs(seconds),
        }).map_err(Fault::Error)?;
        Value::integer(i64::try_from(id).map_err(|_| Fault::Error("Process identity exhausted".into()))?).map_err(Fault::Error)
    }

    pub(crate) fn process_result(&mut self, id: i64) -> Out {
        let id = u64::try_from(id).map_err(|_| Fault::Error("Invalid process identity".into()))?;
        let Some(output) = self.vm.process_jobs.result(id).map_err(Fault::Error)? else { return Ok(Value::NIL); };
        let status = output.status.map(|n| Value::integer(n as i64).unwrap()).unwrap_or(Value::NIL);
        let stdout = self.string(&String::from_utf8_lossy(&output.stdout));
        let stderr = self.string(&String::from_utf8_lossy(&output.stderr));
        let error = output.error.as_deref().map(|s| self.string(s)).unwrap_or(Value::NIL);
        let tail = self.cons(error, Value::NIL);
        let tail = self.cons(stderr, tail);
        let tail = self.cons(stdout, tail);
        Ok(self.cons(status, tail))
    }

    pub(crate) fn process_ready(&mut self) -> Out {
        let ids = self.vm.process_jobs.ready();
        let mut list = Value::NIL;
        for id in ids.into_iter().rev() {
            let id = Value::integer(id as i64).map_err(Fault::Error)?;
            list = self.cons(id, list);
        }
        Ok(list)
    }

    pub(crate) fn process_forget(&mut self, id: i64) -> Out {
        self.vm.process_jobs.forget(u64::try_from(id).map_err(|_| Fault::Error("Invalid process identity".into()))?)
            .map_err(Fault::Error)?;
        Ok(Value::NIL)
    }

    pub(crate) fn process_json_read(&mut self, source: Value) -> Out {
        let source = self.string_text(source)?;
        let value: serde_json::Value = serde_json::from_str(&source).map_err(|e| Fault::Error(format!("Invalid JSON: {e}")))?;
        self.process_json_value(&value)
    }

    fn process_json_value(&mut self, value: &serde_json::Value) -> Out {
        use serde_json::Value as Json;
        Ok(match value {
            Json::Null => self.intern("json-null"),
            Json::Bool(true) => Value::TRUE,
            Json::Bool(false) => self.intern("json-false"),
            Json::String(text) => self.string(text),
            Json::Number(n) => {
                if let Some(n) = n.as_i64() { Value::integer(n).map_err(Fault::Error)? }
                else if let Some(n) = n.as_u64() {
                    Value::integer(i64::try_from(n).map_err(|_| Fault::Error("JSON integer out of range".into()))?).map_err(Fault::Error)?
                } else { self.float(n.as_f64().ok_or_else(|| Fault::Error("Invalid JSON number".into()))?) }
            }
            Json::Array(items) => {
                let mut list = Value::NIL;
                for item in items.iter().rev() { let item = self.process_json_value(item)?; list = self.cons(item, list); }
                list
            }
            Json::Object(items) => {
                let mut list = Value::NIL;
                for (key, item) in items.iter().rev() {
                    let key = self.string(key);
                    let item = self.process_json_value(item)?;
                    let pair = self.cons(key, item);
                    list = self.cons(pair, list);
                }
                list
            }
        })
    }
}
