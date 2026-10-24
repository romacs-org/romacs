//! Native Custom declaration metadata, without loading the GNU Custom UI.
use super::*;

defsubr! { name = "custom-autoload", gnu = "custom.el:681", effect = MutatesArgs,
fn custom_autoload(cx, symbol: Value, load: Value; optional noset: Value) {
    cx.custom_autoload_metadata(symbol, load, noset)
}}

defsubr! { name = "custom-add-load", gnu = "custom.el:674", effect = MutatesArgs,
fn custom_add_load(cx, symbol: Value, load: Value) {
    cx.custom_load_metadata(symbol, load, None)
}}

#[cfg(test)]
mod tests {
    use crate::Vm;
    #[test]
    fn metadata_keeps_load_order_deduplicates_equal_names_and_preserves_value() {
        let mut vm = Vm::new();
        let result = vm.eval_source_raw("(progn (setq sample-option 42) (custom-autoload 'sample-option \"one\") (custom-autoload 'sample-option (copy-sequence \"one\") t) (custom-autoload 'sample-option \"two\" t) (list sample-option (get 'sample-option 'custom-autoload) (get 'sample-option 'custom-loads) (custom-autoload 'sample-option \"two\")))").unwrap();
        assert_eq!(vm.display(result).unwrap(), "(42 noset (\"two\" \"one\") nil)");
    }
}
