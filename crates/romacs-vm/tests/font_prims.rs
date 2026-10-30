//! font.c's font objects on the initial terminal, against pinned GNU in
//! batch (`emacs -Q --batch'): specs, their properties and names, the
//! errors, and no font to list or open. The GUI's display and its fonts are
//! romacs-vm-gui's (fonts.rs).
use romacs_vm::Vm;

fn printed(vm: &mut Vm, source: &str) -> String {
    let source = format!("(condition-case e (prin1-to-string {source}) (error (prin1-to-string (list 'signal e))))");
    let value = vm.eval_source(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
    value.display().unwrap()
}

#[test]
fn font_specs_on_the_initial_terminal_are_gnus() {
    let mut vm = Vm::new();
    let cases = [
        ("(list (framep (selected-frame)) (window-system) (terminal-live-p nil) (frame-initial-p))", "(t nil t t)"),
        ("(let ((s (font-spec :family \"Foo\" :weight 'bold :size 12))) (list (fontp s) (fontp s 'font-spec) (fontp s 'font-entity) (fontp \"x\") (font-get s :family) (font-get s :weight) (font-get s :size) (font-xlfd-name s)))",
         r#"(t t nil nil Foo bold 12 "-*-Foo-bold-*-*-*-12-*-*-*-*-*-*-*")"#),
        ("(let ((s (font-spec :name \"Foo-12:weight=demibold:slant=italic\"))) (list (font-get s :family) (font-get s :size) (font-get s :weight) (font-get s :slant) (font-get s :name) (font-xlfd-name s)))",
         r#"(Foo 12.0 demibold italic "Foo-12:weight=demibold:slant=italic" "-*-Foo-demibold-italic-*-*-*-120-*-*-*-*-*-*")"#),
        ("(let ((s (font-spec :family \"A\"))) (list (font-put s :slant 'italic) (font-get s :slant) (font-put s :zz 3) (font-get s :zz) (font-put s :aa 1) (font-get s :aa)))",
         "(italic italic 3 3 1 1)"),
        ("(font-spec :weight 'nope)", r#"(signal (error "invalid font property" (:weight . nope)))"#),
        ("(font-spec :family)", r#"(signal (error "No value for key ‘:family’"))"#),
        ("(font-get (font-spec :name \"-*-x-*\") :family)", "x"),
        ("(fontp 1 'foo)", "(signal (wrong-type-argument font-extra-type foo))"),
        ("(font-get 1 :family)", "(signal (wrong-type-argument font 1))"),
        ("(list (find-font (font-spec :family \"Foo\")) (list-fonts (font-spec)) (internal-char-font nil 955) (font-family-list))", "(nil nil nil nil)"),
        ("(font-xlfd-name (font-spec :name \"-GNU -FreeSans-semibold-italic-normal-*-*-*-*-*-*-0-iso10646-1\") t)",
         r#""-GNU -FreeSans-semibold-italic-normal-*-0-iso10646-1""#),
        ("(font-get (font-spec :name \"Foo:light:roman\") :slant)", "roman"),
        ("(font-get (font-spec :spacing 'm :registry \"ISO10646-1\") :registry)", "iso10646-1"),
    ];
    let failures: Vec<String> = cases.iter().filter_map(|(source, gnu)| {
        let romacs = printed(&mut vm, source);
        let romacs: String = serde_json::from_str(&romacs).unwrap_or(romacs);
        (romacs != *gnu).then(|| format!("{source}\n  romacs {romacs}\n  gnu    {gnu}"))
    }).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
