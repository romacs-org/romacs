use romacs_vm::Vm;
fn printed(source: &str) -> String {
 let mut vm=Vm::new(); vm.set_collection_floor(1);
 let value=vm.eval_source(&format!("(prin1-to-string (condition-case e {source} (error e)))")).expect("evaluation");
 serde_json::from_str::<String>(&value.display().expect("display")).expect("printer text")
}

// GNU: (4194303 1114111 t nil nil)
#[test]
fn p1a_range() { assert_eq!(printed("(list (max-char) (max-char t) (characterp 4194303) (characterp 4194304) (characterp -1))"), "(4194303 1114111 t nil nil)"); }

// GNU: (5 4 5 2)
#[test]
fn p1a_widths() { assert_eq!(printed("(list (string-bytes (char-to-string 4194175)) (string-bytes (char-to-string 1114112)) (string-bytes (char-to-string 2097152)) (string-bytes (char-to-string 4194303)))"), "(5 4 5 2)"); }

// GNU: (t 1 t 4194303 "\377")
#[test]
fn p1a_raw() { assert_eq!(printed("(let ((s (char-to-string 4194303))) (list (stringp s) (length s) (multibyte-string-p s) (aref s 0) s))"), "(t 1 t 4194303 \"\\377\")"); }

// GNU: (4194303 "\377" 4194303 -1 255 "\273")
#[test]
fn p1a_conversions() { assert_eq!(printed("(list (aref (string-to-multibyte (unibyte-string 255)) 0) (string-to-unibyte (string-to-multibyte (unibyte-string 255))) (unibyte-char-to-multibyte 255) (multibyte-char-to-unibyte 955) (multibyte-char-to-unibyte 4194303) (string-make-unibyte \"λ\"))"), "(4194303 \"\\377\" 4194303 -1 255 \"\\273\")"); }

// GNU: (4194303 2 "\377")
#[test]
fn p1a_as_multibyte() { assert_eq!(printed("(let ((s (string-as-multibyte (unibyte-string 255)))) (list (aref s 0) (string-bytes s) (string-as-unibyte s)))"), "(4194303 2 \"\\377\")"); }

// GNU: (nil t t t)
#[test]
fn p1a_equal() { assert_eq!(printed("(list (equal (unibyte-string 255) (string-to-multibyte (unibyte-string 255))) (equal \"abc\" (string-to-multibyte \"abc\")) (string-equal \"abc\" (string-to-multibyte \"abc\")) (= (sxhash \"abc\") (sxhash (string-to-multibyte \"abc\"))))"), "(nil t t t)"); }

// GNU: (3 4194303 2 1114112 t nil)
#[test]
fn p1a_construct() { assert_eq!(printed("(let ((s (string 97 955 4194303)) (m (make-string 2 1114112))) (list (length s) (aref s 2) (length m) (aref m 0) (char-equal 97 97) (char-equal 97 98)))"), "(3 4194303 2 1114112 t nil)"); }

// GNU: (error "Attempt to store non-byte value into unibyte string")
#[test]
fn p1a_aset_unibyte() { assert_eq!(printed("(condition-case e (aset (unibyte-string 97) 0 955) (error e))"), "(error \"Attempt to store non-byte value into unibyte string\")"); }

// GNU: (wrong-type-argument characterp 4194304)
#[test]
fn p1a_aset_range() { assert_eq!(printed("(condition-case e (aset (copy-sequence \"ab\") 0 4194304) (error e))"), "(wrong-type-argument characterp 4194304)"); }

// GNU: (error "Cannot convert character at index 0 to unibyte")
#[test]
fn p1a_to_unibyte_error() { assert_eq!(printed("(condition-case e (string-to-unibyte \"λ\") (error e))"), "(error \"Cannot convert character at index 0 to unibyte\")"); }

// GNU: "\273\273"
#[test]
fn p1a_fill() { assert_eq!(printed("(fillarray (unibyte-string 97 98) 955)"), "\"\\273\\273\""); }
