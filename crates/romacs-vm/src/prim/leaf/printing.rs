use super::*;
impl Leaf<'_> {
    pub(crate) fn format_message(&mut self, template: Value, arguments: &[Value]) -> std::result::Result<Value, Fault> {
        self.string_view(template)?;
        let variable = self.intern("text-quoting-style");
        let value = self.variable_value(variable)?.unwrap_or(Value::NIL);
        // GNU doc.c default_to_grave_quoting_style and Ftext_quoting_style.
        // Non-nil explicit styles bypass both the capability flag and table.
        let style = if value == Value::NIL {
            let flag = self.intern("internal--text-quoting-flag");
            if self.variable_value(flag)?.unwrap_or(Value::NIL) == Value::NIL {
                "grave"
            } else {
                let variable = self.intern("standard-display-table");
                let table = self.variable_value(variable)?.unwrap_or(Value::NIL);
                // disptab.h: DISP_TABLE_P requires purpose AND all 18 slots
                // (six ordinary slots plus twelve box-drawing slots).
                let purpose = self.intern("display-table");
                let display_table = match self.vm.object_if(table) {
                    Some(data::Object::CharTable(t)) => t.extras.len() == 18
                        && t.purpose == purpose,
                    _ => false,
                };
                let grave = if display_table {
                    // U+2018 uses char_table_ref, including defaults/parents.
                    let vector = self.vm.char_table_get(table, 0x2018).map_err(Fault::Error)?;
                    matches!(self.vm.object_if(vector), Some(data::Object::Vector(v))
                        if v.len() == 1 && v[0].integer_value() == Some(96))
                } else {
                    false
                };
                if grave { "grave" } else { "curve" }
            }
        } else {
            if value == self.intern("grave") {
                "grave"
            } else if value == self.intern("straight") {
                "straight"
            } else {
                "curve"
            }
        };
        let mut values = Vec::with_capacity(arguments.len() + 1);
        values.push(template);
        values.extend_from_slice(arguments);
        self.vm.format_values(&values, Some(style)).map_err(Fault::Error)
    }

    pub(crate) fn print_text(&mut self, object: Value, quoted: bool, overrides: Value) -> std::result::Result<Value, Fault> {
        let text = self.vm.print_render(object, quoted, overrides, false).map_err(Fault::Error)?;
        Ok(self.string(&text))
    }
}
