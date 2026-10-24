use romacs_vm::Vm;
use serde_json::{json, Value};

fn setup() -> Vm {
    let mut vm = Vm::new();
    vm.eval_source(include_str!("../../../rlisp/romacs-notmuch.el")).unwrap();
    vm
}

#[test]
fn entry_commands_delegate_without_loading_a_ui() {
    let mut vm = setup();
    check(&mut vm, r#"(progn
      (defun romacs-mail-search (account query) (list (plist-get account :name) query))
      (defun romacs-mail-compose (account to subject) (list (plist-get account :name) to subject))
      (defun romacs-mail-reply (all-p) (list 'reply all-p))
      (defun romacs-mail-tree-current-query () 'tree)
      (and (equal (notmuch) '("notmuch" "tag:inbox"))
           (equal (notmuch-search "subject:λ") '("notmuch" "subject:λ"))
           (equal (notmuch-mua-new-mail "to@x" "Topic") '("notmuch" "to@x" "Topic"))
           (equal (notmuch-show-reply) '(reply t))
           (equal (notmuch-show-reply-sender) '(reply nil))
           (equal (romacs-notmuch-reply) '(reply nil))
           (eq (notmuch-tree-from-show-current-query) 'tree)
           (not (null (interactive-form 'notmuch-show-reply)))))"#);
}

#[test]
fn malformed_data_and_depth_limits_refuse_instead_of_truncating() {
    let mut vm = setup();
    check(&mut vm, r#"(and
      (condition-case nil (progn (romacs-notmuch--boolean 'wrong) nil) (error t))
      (condition-case nil (progn (romacs-notmuch--trees 'account '((nil nil)) nil 65 nil) nil) (error t))
      (condition-case nil (progn (romacs-notmuch--parts nil 33) nil) (error t))
      (condition-case nil (progn (romacs-notmuch--parts '((("id" . 1) ("content-type" . "text/plain") ("content" . 42))) 0) nil) (error t))
      (condition-case nil (progn (romacs-notmuch--message 'account "a@x" '(("id" . "a@x") ("headers" . (("From" . "A"))))) nil) (error t)))"#);
}

fn check(vm: &mut Vm, source: &str) {
    let value = vm.eval_source(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    assert_eq!(value.display().unwrap(), "t", "{source}");
}

fn data(vm: &mut Vm, name: &str, value: Value) {
    vm.eval_source(&format!("(setq {name} (romacs-json-read {}))", serde_json::to_string(&value.to_string()).unwrap())).unwrap();
}

fn message(id: &str, matched: bool, excluded: bool) -> Value {
    json!({"id":id,"match":matched,"excluded":excluded,"timestamp":123,
        "date_relative":"Today","headers":{"From":"Alice <a@x>","To":"Me <me@x>","Date":"Saturday","Subject":"Topic"},
        "tags":["inbox"],"body":[{"id":1,"content-type":"text/plain","content":"Body λ"}]})
}

#[test]
fn account_captures_command_and_query_boundaries() {
    let mut vm = setup();
    check(&mut vm, r#"(progn
      (setq romacs-notmuch-command (list "notmuch" "--config=first"))
      (setq account (romacs-notmuch-account))
      (setcar (cdr romacs-notmuch-command) "--config=second")
      (setq request (funcall (plist-get account :message) "a\" b@x"))
      (and (equal (plist-get account :key) '(notmuch ("notmuch" "--config=first")))
           (equal (car request) '("notmuch" "--config=first" "show" "--format=json" "--format-version=5" "--part=0" "--exclude=false" "--body=true" "--decrypt=false" "--" "id:\"a\"\" b@x\""))
           (equal (car (funcall (plist-get account :search) "--odd OR tag:inbox" 10 20 'oldest))
                  '("notmuch" "--config=first" "search" "--format=json" "--format-version=5" "--output=summary" "--sort=oldest-first" "--offset=10" "--limit=20" "--" "--odd OR tag:inbox"))))"#);
    check(&mut vm, r#"(and
      (condition-case nil (progn (romacs-notmuch-id-term "bad\nID") nil) (error t))
      (condition-case nil (progn (funcall (plist-get account :tree) "abc OR *" nil) nil) (error t))
      (condition-case nil (progn (funcall (plist-get account :search) "*" -1 20 'newest) nil) (error t)))"#);
}

#[test]
fn tree_preserves_forest_siblings_matches_and_exclusion() {
    let mut vm = setup();
    let a = message("a@x", false, false);
    let b = message("b@x", false, false);
    let d = message("d@x", true, false);
    let c = message("c@x", false, true);
    let e = message("e@x", true, false);
    data(&mut vm, "forest", json!([[[a,[[b,[[d,[]]]],[c,[]]]],[e,[]]],[[message("z@x",true,false),[]]]]));
    check(&mut vm, r#"(progn
      (setq account (romacs-notmuch-account))
      (setq request (funcall (plist-get account :tree) "abc" "subject:Needle"))
      (setq rows (funcall (cdr request) forest))
      (and (equal (car request) '("notmuch" "show" "--format=json" "--format-version=5" "--body=false" "--entire-thread=true" "--exclude=true" "--decrypt=false" "--" "thread:abc and (subject:Needle)"))
           (equal (romacs-notmuch--map (lambda (r) (cdr (plist-get r :key))) rows) '("a@x" "b@x" "d@x" "c@x" "e@x" "z@x"))
           (equal (romacs-notmuch--map (lambda (r) (plist-get r :depth)) rows) '(0 1 2 1 0 0))
           (equal (romacs-notmuch--map (lambda (r) (plist-get r :lasts)) rows) '((nil) (nil nil) (nil nil t) (nil t) (t) (t)))
           (equal (romacs-notmuch--map (lambda (r) (plist-get r :match)) rows) '(nil nil t nil t t))
           (equal (romacs-notmuch--map (lambda (r) (plist-get r :excluded)) rows) '(nil nil nil t nil nil))
           (equal (romacs-notmuch--map (lambda (r) (plist-get r :thread-start)) rows) '(t nil nil nil nil t))))"#);
}

#[test]
fn null_parent_keeps_child_depth_and_invalid_tuples_fail() {
    let mut vm = setup();
    data(&mut vm, "forest", json!([[[null,[[message("child@x",true,false),[]]]]]]));
    check(&mut vm, r#"(progn
      (setq request (funcall (plist-get (romacs-notmuch-account) :tree) "abc" nil))
      (setq rows (funcall (cdr request) forest))
      (and (= (length rows) 1) (= (plist-get (car rows) :depth) 1)
           (null (plist-get (car rows) :parent-id))
           (condition-case nil (progn (funcall (cdr request) '(((nil nil extra)))) nil) (error t))))"#);
}

#[test]
fn exact_message_accepts_object_not_forest_and_keeps_mime_metadata() {
    let mut vm = setup();
    let mut msg = message("d@x", true, false);
    msg["body"] = json!([{"id":1,"content-type":"multipart/alternative","content":[
        {"id":2,"content-type":"text/html","content":"<b>unsafe</b>"},
        {"id":3,"content-type":"text/plain","content":"Exact child"}]},
        {"id":4,"content-type":"application/pdf","content-disposition":"attachment","filename":"report.pdf","content-length":42}]);
    data(&mut vm, "mail", msg);
    check(&mut vm, r#"(progn
      (setq request (funcall (plist-get (romacs-notmuch-account) :message) "d@x"))
      (setq normalized (funcall (cdr request) mail))
      (and (equal (cdr (plist-get normalized :key)) "d@x")
           (null (plist-get (car (plist-get (car (plist-get normalized :parts)) :children)) :content))
           (= (plist-get (nth 1 (plist-get normalized :parts)) :size) 42)
           (condition-case nil (progn (funcall (cdr request) (list (list (list mail nil)))) nil) (error t))
           (condition-case nil (progn (funcall (cdr (funcall (plist-get (romacs-notmuch-account) :message) "other@x")) mail) nil) (error t))))"#);
}

#[test]
fn reply_uses_authoritative_headers_and_original_plain_alternative() {
    let mut vm = setup();
    let mut original = message("d@x", true, false);
    original["body"] = json!([{"id":1,"content-type":"multipart/alternative","content":[
        {"id":2,"content-type":"text/html","content":"Do not quote"},
        {"id":3,"content-type":"text/plain","content":"Selected body λ"}]}]);
    data(&mut vm,"reply",json!({"reply-headers":{"From":"Alias <alias@x>","To":"List <list@x>","Cc":"Bob <bob@x>","Subject":"Re: Topic","In-reply-to":"<d@x>","References":"<a@x> <b@x> <d@x>"},"original":original}));
    check(&mut vm, r#"(progn
      (setq account (romacs-notmuch-account))
      (setq all (funcall (plist-get account :reply) "d@x" t))
      (setq sender (funcall (plist-get account :reply) "d@x" nil))
      (setq envelope (funcall (cdr all) reply))
      (and (member "--reply-to=all" (car all)) (member "--reply-to=sender" (car sender))
           (equal (cdr (assoc "From" (plist-get envelope :headers))) "Alias <alias@x>")
           (equal (cdr (assoc "To" (plist-get envelope :headers))) "List <list@x>")
           (equal (cdr (assoc "Cc" (plist-get envelope :headers))) "Bob <bob@x>")
           (equal (plist-get envelope :in-reply-to) "<d@x>")
           (equal (plist-get envelope :references) "<a@x> <b@x> <d@x>")
           (equal (plist-get envelope :quote-text) "Selected body λ")
           (condition-case nil (progn (funcall (cdr (funcall (plist-get account :reply) "a@x" t)) reply) nil) (error t))))"#);
}

#[test]
fn search_normalizes_author_groups_and_rejects_missing_identity() {
    let mut vm = setup();
    data(&mut vm,"summaries",json!([{"thread":"abc","timestamp":123,"date_relative":"Today","matched":1,"total":2,"authors":"Dan| Alice, Bob","subject":"Topic","tags":["inbox"]}]));
    check(&mut vm, r#"(progn
      (setq request (funcall (plist-get (romacs-notmuch-account) :search) "*" 0 50 'newest))
      (setq row (car (funcall (cdr request) summaries)))
      (and (equal (plist-get row :authors-matched) "Dan")
           (equal (plist-get row :authors-other) "Alice, Bob")
           (null (funcall (cdr request) nil))
           (condition-case nil (progn (funcall (cdr request) '((("subject" . "bad")))) nil) (error t))))"#);
}
