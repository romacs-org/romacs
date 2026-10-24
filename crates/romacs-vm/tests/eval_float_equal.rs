use romacs_vm::Vm;
fn answer(vm:&mut Vm,source:&str)->Result<String,String>{vm.eval_source(source).and_then(|v|v.display())}
#[test]
fn equal_signed_zero_uses_bits_in_lists_without_changing_numeric_equal() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(list (equal 0.0 -0.0) (equal (list 0.0) (list -0.0)) (= 0.0 -0.0))"),Ok("(nil nil t)".into()));
}
#[test]
fn equal_nan_bits_include_sign_in_scalars_and_vectors() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(list (equal 0.0e+NaN 0.0e+NaN) (equal 0.0e+NaN -0.0e+NaN) (equal [0.0e+NaN] [0.0e+NaN]) (equal [0.0e+NaN] [-0.0e+NaN]))"),Ok("(t nil t nil)".into()));
    assert_eq!(answer(&mut vm,"(let ((h (make-hash-table :test 'eql))) (puthash 0.0 'positive h) (puthash -0.0 'negative h) (puthash 0.0e+NaN 'nan h) (list (gethash 0.0 h) (gethash -0.0 h) (gethash 0.0e+NaN h) (gethash -0.0e+NaN h)))"),Ok("(positive negative nan nil)".into()));
}
