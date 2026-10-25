use super::*;

impl Vm {
    pub(super) fn oclosure_argument(&mut self, b: &mut Builder, form: &Form) -> Result<usize> {
        let candidate = match form {
            Form::List(parts) if matches!(parts.first(), Some(Form::Sym(s)) if s == "function") && parts.len() == 2 => &parts[1],
            _ => form,
        };
        if let Form::List(parts) = candidate
            && matches!(parts.first(), Some(Form::Sym(s)) if s == "lambda")
            && parts.len() >= 3
        {
            return self.closure(b, &parts[1], &parts[2..], false, true);
        }
        self.expr(b, form)
    }

    pub(super) fn lambda_with_metadata(
        &mut self, b: &mut Builder, args: &[Form], body: &[Form], exposed: bool,
    ) -> Result<usize> {
        let saved = self.keep_captures;
        self.keep_captures |= exposed;
        let result = self.lambda(b, args, body);
        self.keep_captures = saved;
        let result = result?;
        if exposed { self.expose_closure(b, args, body, result) } else { Ok(result) }
    }

    pub(super) fn expose_closure(
        &mut self, b: &mut Builder, args: &[Form], body: &[Form], result: usize,
    ) -> Result<usize> {
        let mut names: Vec<_> = b.env.keys().cloned().collect();
        names.sort();
        let mut order: Vec<_> = b.env.iter().collect();
        // The executable ABI sorts names; GNU's exposed environment is newest first.
        order.sort_by_key(|(name, binding)| (std::cmp::Reverse(binding.reg), *name));
        let descriptors = order.into_iter().map(|(name, _)| {
            let index = names.binary_search(name).unwrap();
            let symbol = match name {
                Name::Source(name) => Form::Sym(name.clone()),
                Name::Generated(name) => Form::Generated(*name),
            };
            Form::Dotted(vec![symbol], Box::new(Form::Int(index as i64)))
        }).collect();
        let body = if body.len() > 1 && self.form_is_string(&body[0]) { &body[1..] } else { body };
        let mut registers = vec![result];
        for form in [Form::List(args.to_vec()), Form::List(body.to_vec()), Form::List(descriptors), Form::Sym("nil".into()), Form::Sym("t".into())] {
            let value = self.literal(&form)?;
            let register = b.reg();
            b.emit(Op::Const(register, value));
            registers.push(register);
        }
        Ok(self.closure_native_call(b, Primitive::ClosureMetadata, &registers))
    }
}
