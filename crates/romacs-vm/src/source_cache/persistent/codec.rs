//! Explicit portable codec; avoids procedural-macro compiler dependencies in
//! the developer build. Decoding is fallible; executable bounds are checked by
//! valid_artifact before installation.
use super::*;
use serde_json::{Value as J, json};
fn num(v: &J) -> Option<usize> {
    usize::try_from(v.as_u64()?).ok()
}
pub(in crate::source_cache) fn form_json(f: &Form) -> J {
    match f {
        // Compiler temporaries and heap objects are never portable source literals.
        Form::Generated(_) | Form::Object(_) => J::Null,
        Form::Int(n) => json!(["i", n]),
        Form::Float(n) => json!(["f", n.to_bits()]),
        Form::Sym(s) => json!(["s", s]),
        Form::String(s) => json!(["t", s]),
        Form::Bytes(bytes) => json!(["b", bytes]),
        Form::List(a) => json!(["l", a.iter().map(form_json).collect::<Vec<_>>()]),
        Form::Vector(a) => json!(["v", a.iter().map(form_json).collect::<Vec<_>>()]),
        Form::Dotted(a, t) => json!([
            "d",
            a.iter().map(form_json).collect::<Vec<_>>(),
            form_json(t)
        ]),
    }
}
fn form(v: &J, depth: usize) -> Option<Form> {
    if depth > 256 {
        return None;
    }
    let a = v.as_array()?;
    let kind = a.first()?.as_str()?;
    if a.len() != if kind == "d" { 3 } else { 2 } {
        return None;
    }
    Some(match kind {
        "i" => Form::Int(a[1].as_i64()?),
        "f" => Form::Float(f64::from_bits(a[1].as_u64()?)),
        "s" => Form::Sym(a[1].as_str()?.into()),
        "t" => Form::String(a[1].as_str()?.into()),
        "b" => Form::Bytes(
            a[1].as_array()?
                .iter()
                .map(|v| u8::try_from(v.as_u64()?).ok())
                .collect::<Option<Vec<_>>>()?,
        ),
        "l" | "v" | "d" => {
            let values = a[1]
                .as_array()?
                .iter()
                .map(|v| form(v, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            match kind {
                "l" => Form::List(values),
                "v" => Form::Vector(values),
                _ => Form::Dotted(values, Box::new(form(&a[2], depth + 1)?)),
            }
        }
        _ => return None,
    })
}
fn op_json(op: Op) -> J {
    crate::code_image::encode_op(op)
}
fn op(v: &J) -> Option<Op> {
    crate::code_image::decode_op(v).ok()
}
fn function_json(f: &Function) -> J {
    json!([
        f.regs,
        f.captures,
        f.arity,
        f.required,
        f.rest,
        f.sites,
        f.code.iter().map(|op| op_json(*op)).collect::<Vec<_>>(),
        f.dynamic
    ])
}
fn function(v: &J) -> Option<Function> {
    let a = v.as_array()?;
    // An eighth element, the dialect, is absent from older entries.
    if a.len() != 7 && a.len() != 8 {
        return None;
    }
    let sites = a[5]
        .as_array()?
        .iter()
        .map(|s| s.as_array()?.iter().map(num).collect::<Option<Vec<_>>>())
        .collect::<Option<Vec<_>>>()?;
    Some(Function {
        capture_mutations: Vec::new(),
        mapped_code: None,
        regs: num(&a[0])?,
        captures: num(&a[1])?,
        arity: num(&a[2])?,
        required: num(&a[3])?,
        rest: a[4].as_bool()?,
        sites,
        lazy: None,
        dynamic: a.get(7).and_then(J::as_bool),
        code: a[6]
            .as_array()?
            .iter()
            .map(op)
            .collect::<Option<Vec<_>>>()?,
    })
}
impl Artifact {
    pub(super) fn encode(&self) -> String {
        json!({"compiler":self.compiler,"source_key":self.source_key,"symbols":self.symbols,"specials":self.specials.iter().map(|(s,k)|json!([s,*k as u8])).collect::<Vec<_>>(),"functions":self.functions.iter().map(function_json).collect::<Vec<_>>(),"constants":self.constants.iter().map(|(f,pc,v)|json!([f,pc,form_json(v)])).collect::<Vec<_>>(),"entry":self.entry,"caches":self.caches}).to_string()
    }
    pub(super) fn decode(data: &str) -> Option<Self> {
        let v: J = serde_json::from_str(data).ok()?;
        let specials = v["specials"]
            .as_array()?
            .iter()
            .map(|a| {
                let kind = match a.get(1)?.as_u64()? {
                    0 => SpecialKind::Lexical,
                    1 => SpecialKind::FileLocal,
                    2 => SpecialKind::Global,
                    _ => return None,
                };
                Some((a.get(0)?.as_str()?.into(), kind))
            })
            .collect::<Option<Vec<_>>>()?;
        let constants = v["constants"]
            .as_array()?
            .iter()
            .map(|a| Some((num(a.get(0)?)?, num(a.get(1)?)?, form(a.get(2)?, 0)?)))
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            compiler: v["compiler"].as_str()?.into(),
            source_key: v["source_key"].as_str()?.into(),
            symbols: v["symbols"]
                .as_array()?
                .iter()
                .map(|v| Some(v.as_str()?.into()))
                .collect::<Option<Vec<_>>>()?,
            specials,
            constants,
            entry: num(&v["entry"])?,
            caches: num(&v["caches"])?,
            functions: v["functions"]
                .as_array()?
                .iter()
                .map(function)
                .collect::<Option<Vec<_>>>()?,
        })
    }
}
