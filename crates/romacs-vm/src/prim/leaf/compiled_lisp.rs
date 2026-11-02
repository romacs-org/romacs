//! Background compiles of compiled Lisp (`crate::compiled_lisp`): what
//! rlisp/romacs-precompile.el's idle worker asks.
use super::*;

impl Leaf<'_> {
    fn file_name_string(&mut self, name: &std::ffi::OsStr) -> Value {
        self.vm.make_lisp_string(name.as_encoded_bytes().to_vec(), true)
    }

    pub(crate) fn compiled_lisp_candidates(&mut self) -> crate::prim::Out {
        let files = self.vm.precompile_candidates();
        let names: Vec<Value> = files.iter().map(|f| self.file_name_string(f.as_os_str())).collect();
        Ok(self.vm.list_value(&names))
    }

    pub(crate) fn compiled_lisp_compile_command(&mut self, file: Value) -> crate::prim::Out {
        let file = self.string_text(file)?;
        let Some((program, args, environment)) = self.vm.precompile_command(std::path::Path::new(&file)).map_err(Fault::Error)? else {
            return Ok(Value::NIL);
        };
        let program = self.file_name_string(&program);
        let args: Vec<Value> = args.iter().map(|a| self.file_name_string(a)).collect();
        let environment: Vec<Value> = environment.iter().map(|(name, value)| {
            let mut entry = name.clone();
            entry.push("=");
            entry.push(value);
            self.file_name_string(&entry)
        }).collect();
        let (args, environment) = (self.vm.list_value(&args), self.vm.list_value(&environment));
        Ok(self.vm.list_value(&[program, args, environment]))
    }
}
