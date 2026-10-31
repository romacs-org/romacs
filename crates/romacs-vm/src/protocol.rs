//! Structured agent boundary. A batch is sequential and non-transactional;
//! each element returns a value or error and later elements still run.
use super::*;
use serde_json::{Value as Json, json};
impl Vm {
    pub fn submit(&mut self, request: &Json) -> Result<Json> {
        if let Some(batch) = request.get("batch") {
            let batch = batch.as_array().ok_or("batch must be an array")?;
            let mut replies = Vec::with_capacity(batch.len());
            for item in batch {
                replies.push(match self.submit(item) {
                    Ok(v) => v,
                    Err(e) => json!({"error":e}),
                });
            }
            return Ok(json!({"results":replies}));
        }
        if let Some(source) = request.get("prepare") {
            let source = source
                .as_str()
                .ok_or("prepare must be a lambda source or name")?;
            let handle = if let Some(arity) = request.get("arity") {
                let arity = arity.as_u64().ok_or("arity must be unsigned")?;
                if arity > 4096 {
                    return Err("prepared arity limit exceeded".into());
                }
                let name = source.chars().map(|c| format!("\\{c}")).collect::<String>();
                let args = (0..arity)
                    .map(|i| format!("vm2-handle-arg-{i}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                self.prepare_source(&format!("(lambda ({args}) ({name} {args}))"))?
            } else {
                self.prepare_source(source)?
            };
            return Ok(json!({"handle":handle.0}));
        }
        if let Some(handle) = request.get("invoke") {
            let handle = handle
                .as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or("invalid prepared handle")?;
            let args = request
                .get("args")
                .and_then(Json::as_array)
                .ok_or("args must be an array")?;
            let args = args
                .iter()
                .map(|v| self.input_value(v, 0))
                .collect::<Result<Vec<_>>>()?;
            let value = self.invoke_prepared_raw(PreparedHandle(handle), &args)?;
            return Ok(json!({"result":self.typed_value(value)?}));
        }
        if let Some(handle) = request.get("ir_handle") {
            let h = handle
                .as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or("invalid prepared handle")?;
            return Ok(json!({"ir":self.prepared_ir(PreparedHandle(h))?}));
        }
        if let Some(source) = request.get("source") {
            let source = source.as_str().ok_or("source must be a string")?;
            if request.get("ir") == Some(&Json::Bool(true)) {
                return Ok(json!({"ir":self.request_source_ir(source)?}));
            }
            let value = self.eval_source_raw(source)?;
            return Ok(json!({"value":self.display(value)?,"result":self.typed_value(value)?}));
        }
        Err("request requires source, prepare, invoke, ir_handle or batch".into())
    }
    fn input_value(&mut self, v: &Json, depth: usize) -> Result<Value> {
        if depth > 100 {
            return Err("argument depth exceeded".into());
        }
        match v {
            Json::Null | Json::Bool(false) => Ok(Value::NIL),
            Json::Bool(true) => Ok(Value::TRUE),
            Json::Number(n) => self.literal(&Form::from_json(Json::Number(n.clone()))?),
            Json::String(s) => self.literal(&Form::String(s.clone())),
            Json::Object(o) if o.len() == 1 && o.contains_key("symbol") => {
                Ok(self.intern(o["symbol"].as_str().ok_or("symbol string required")?))
            }
            Json::Object(o)
                if o.len() == 1 && (o.contains_key("list") || o.contains_key("vector")) =>
            {
                let vector = o.contains_key("vector");
                let a = o[if vector { "vector" } else { "list" }]
                    .as_array()
                    .ok_or("array required")?;
                let values = a
                    .iter()
                    .map(|v| self.input_value(v, depth + 1))
                    .collect::<Result<Vec<_>>>()?;
                Ok(if vector {
                    self.allocate_object(data::Object::Vector(values))
                } else {
                    self.list_value(&values)
                })
            }
            _ => Err("unsupported typed argument".into()),
        }
    }
    pub(crate) fn typed_value(&self, v: Value) -> Result<Json> {
        self.typed_depth(v, 0)
    }
    fn typed_depth(&self, v: Value, depth: usize) -> Result<Json> {
        if depth > 100 {
            return Err("result depth exceeded".into());
        }
        if v == Value::NIL {
            return Ok(json!({"type":"nil"}));
        }
        if v == Value::TRUE {
            return Ok(json!({"type":"boolean","value":true}));
        }
        match v.tag() {
            1 => Ok(json!({"type":"integer","value":v.as_integer()?})),
            7 => Ok(json!({"type":"symbol","value":self.display(v)?})),
            3 => {
                let (a, b) = self.cons_pair(v)?;
                Ok(
                    json!({"type":"cons","car":self.typed_depth(a,depth+1)?,"cdr":self.typed_depth(b,depth+1)?}),
                )
            }
            0 if Self::image_string(v) => Ok(json!({"type":"string","value":self.string_value(v)?})),
            0 => match self.object(v)? {
                data::Object::Float(n) => {
                    Ok(json!({"type":"float","value":n,"bits":format!("{:016x}",n.to_bits())}))
                }
                data::Object::Bignum(n) => Ok(json!({"type":"bignum","value":n.to_string()})),
                data::Object::String(s) => Ok(json!({"type":"string","value":s.as_str()})),
                data::Object::RawString(_) => Err("not-unicode-string: JSON value".into()),
                data::Object::Bytes(b) => Ok(json!({"type":"bytes","value":b})),
                data::Object::BoolVector(b) => Ok(json!({"type":"bool-vector","length":b.len,"bytes":b.bytes})),
                data::Object::Vector(a) => Ok(
                    json!({"type":"vector","value":a.iter().map(|v|self.typed_depth(*v,depth+1)).collect::<Result<Vec<_>>>()?}),
                ),
                data::Object::Record(a) => Ok(json!({"type":"record","value":a.iter().map(|v|self.typed_depth(*v,depth+1)).collect::<Result<Vec<_>>>()?})),
                data::Object::Window(_) => Ok(json!({"type":"window"})),
                data::Object::Frame(_) => Ok(json!({"type":"frame"})),
                data::Object::Terminal(_) => Ok(json!({"type":"terminal"})),
                data::Object::WindowConfiguration(_) => Ok(json!({"type":"window-configuration"})),
                data::Object::Sqlite(_) => Ok(json!({"type":"sqlite"})),
                data::Object::Finalizer(_) => Ok(json!({"type":"finalizer"})),
                data::Object::Process(_) => Ok(json!({"type":"process"})),
                data::Object::Marker(_) => Ok(json!({"type":"marker"})),
                data::Object::Overlay(_) => Ok(json!({"type":"overlay"})),
                data::Object::Buffer(_) => Ok(json!({"type":"buffer"})),
                data::Object::ByteBuffer(_) => Ok(json!({"type":"byte-buffer"})),
                data::Object::SyntaxTable(_) => Ok(json!({"type":"syntax-table"})),
                data::Object::CharTable(_) => Ok(json!({"type":"char-table"})),
                data::Object::SubCharTable(_) => Ok(json!({"type":"sub-char-table"})),
                data::Object::MapKeymapCursor(_) => Err("private traversal cursor".into()),
                data::Object::HashTable(_) => Ok(json!({"type":"hash-table"})),
                data::Object::Obarray(_) => Ok(json!({"type":"obarray"})),
            },
            4 => Ok(json!({"type":"function"})),
            _ => Err("unsupported structured result".into()),
        }
    }
}
