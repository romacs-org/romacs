//! `libxml-parse-html-region' against pinned GNU (libxml2 2.14): every
//! start tag after every open element (the auto-close table), the head's
//! closers, whitespace after and inside each element (HTML_PARSE_NOBLANKS),
//! and documents for the tokenizer's corners: character references, raw
//! text, comments and bogus markup, implied and repeated html/head/body,
//! end-tag priorities and attributes. Both engines read one case file and
//! answer each case's tree's printed md5; a mismatch shows Romacs's tree.
use romacs_vm::Vm;
use std::path::Path;

const TAGS: &str = "a abbr acronym address b bdo big blockquote button caption center cite code col colgroup dd del dfn dir div dl dt em
fieldset font form h1 h2 h3 h4 h5 h6 hr i ins kbd label legend li listing map menu object ol optgroup option p pre q s samp select small
span strike strong sub sup table tbody td tfoot th thead tr tt u ul var section article nav header footer aside main figure img br input
meta link frameset frame noscript embed area param textarea iframe xmp dialog template canvas video audio source picture svg nobr wbr
marquee basefont title style script base noembed noframes plaintext applet";

const DOCUMENTS: &[&str] = &[
    "<html><body><h1>T</h1><p>a <a href=\"https://x/1\">l</a> and <i>e</i>.</p><ul><li>w <b>b</b></li></ul><table><tr><td>one</td><td>two</td></tr></table></body></html>",
    "Hello <b>world</b>", "<p>a<p>b<div>c</div>", "<ul><li>one<li>two</ul>after", "<table><tr><td>1<td>2<tr><td>3</table>",
    "&amp; &lt; &nbsp; &eacute; &#233; &#x263A; &bogus; &copy &copyx &notit; &amp &AMP; &Amp; &#0; &#x110000; &#128; &#xD800; &#65; &#x41 &#13;&#1;&#127;&#x9F;&#xFDD0;",
    "<!-- top --><p>x</p>", "<!DOCTYPE html><html><head><title>T &amp; t</title><style>p{a:b}</style></head><body><p>z</p></body></html>",
    "<P CLASS=Foo ID='x' data-x=\"1\" disabled>Up</P>", "<br><br/><img src=x alt=\"a b\"><hr>text", "<b>bold <i>both</b> italic</i> plain",
    "<script>if (a<b) x=\"</p>\";</script><p>after</p>", "<p>x</p>  <p>y</p>", "<div> <b>a</b> <i>b</i> </div>", "<p>a</p></body></html><p>late</p>",
    "<textarea>a &amp; <b>b</b></textarea>", "<title>a<b>b</b></title><p>c</p>", "<div><table><tr><td>a</div>b</td></tr></table>",
    "<html>  <head>  </head>  <body>  <p> s </p>  </body>  </html>", "<?xml version=\"1.0\"?><p>pi</p>", "<p id=a id=b>dup</p>",
    "<p foo>x</p>", "<input checked value=1>", "<p>x</p><!--unclosed", "<!x>y", "</ x>y", "</>y", "<![CDATA[x]]>y", "<!-- a -- b -->y",
    "<!--x--!>y", "<!--->y", "<!-->x", "<!--unclosed", "<pre>\nfirst\nsecond</pre>", "<p>CR\r\nLF\rend</p>", "x<html><body>y",
    "<html lang=en><body class=a><body id=b>z", "<p>one</p><head><title>late</title></head><p>two</p>", "<iframe>i<b>x</b></iframe>",
    "<html><head><title>x</title></head>t<body>u</body></html>", "<html>t<head><title>x</title></head><body>u", "<body><p>a</html>b",
    "<head><title>x</title><p>y</p><meta></head>", "<html><frameset><frame></frameset></html>", "<frame>x", "<noframes>a<b>b</b></noframes>",
    "<plaintext>a<b>b</b>", "<p><span>x</p>y</span>z", "<table><tr><td>a</tr>b</td></table>", "<li><div>a</li>b", "<a href=\"x\"title=y>z</a>",
    "<a href=x/>y", "<img src=a.png\"/>", "<p a=\"&quot;q&quot;\" b=\"&#x27;\" c=&lt;>x", "<p class=\"a&amp b\">x", "<a href=\"a?b=1&amp;c=2&copy=3&copy;\">l</a>",
    "<head>text</head>", "<head> <title>t</title> x</head>", " <p>lead</p>",
    "\n<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>T</title>\n</head>\n<body>\n<p>x</p>\n</body>\n</html>\n",
    "<!DOCTYPE HTML PUBLIC \"-//W3C//DTD HTML 4.01//EN\"><body> <p>a</p> <b>b</b> </body>", "<p>x<!--c-->y</p>", "<div id=\"a\"><!-- c --></div>",
    "<table><caption>c</caption><thead><tr><th>h</th></tr></thead><tbody><tr><td>d</td></tr></tbody></table>", "<p>Zoë — “q” 中文 😀</p>",
    "<a<b>x</a<b>", "<p\nclass=\"m\"\n>multi\nline</p>", "<div>unclosed <span>to the end",
];

fn cases() -> Vec<String> {
    let tags: Vec<&str> = TAGS.split_whitespace().collect();
    let mut cases: Vec<String> = DOCUMENTS.iter().map(|s| s.to_string()).collect();
    for old in &tags {
        for new in &tags {
            cases.push(format!("<{old}>a<{new}>b"));
        }
        cases.push(format!("<div><{old}>a</{old}> <span>b</span></div>"));
        cases.push(format!("<div><{old}> <span>x</span></{old}></div>"));
        cases.push(format!("<head><{old}>b"));
    }
    cases
}

/// The md5 of each case's printed tree, one per line; SHOW also prints them.
const PROGRAM: &str = r#"(let ((print-escape-newlines t) (print-length nil) (print-level nil) (out nil))
  (dolist (case (split-string (with-temp-buffer (let ((coding-system-for-read 'utf-8-unix)) (insert-file-contents romacs-cases-file)) (buffer-string)) "\n====\n"))
    (with-temp-buffer
      (insert case)
      (let ((tree (prin1-to-string (condition-case e (libxml-parse-html-region (point-min) (point-max)) (error (list 'signal e))))))
        (setq out (cons (if romacs-show tree (md5 tree)) out)))))
  (encode-coding-string (mapconcat #'identity (nreverse out) "\n") 'utf-8))"#;

fn bootstrapped() -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    let lisp = root.join("lisp/gnu");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('='))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    paths.push(root.join("lisp/romacs"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("lisp/romacs/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

#[test]
fn html_trees_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let cases = cases();
    let file = std::env::temp_dir().join(format!("romacs-libxml-html-{}.txt", std::process::id()));
    std::fs::write(&file, cases.join("\n====\n")).unwrap();
    let setup = |show: bool| format!("(setq romacs-cases-file {:?} romacs-show {})", file.display().to_string(), if show { "t" } else { "nil" });
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", &setup(false), "--eval", &format!("(princ {PROGRAM})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = bootstrapped();
    vm.eval_source(&setup(false)).unwrap();
    let actual = String::from_utf8(vm.eval_source(PROGRAM).unwrap().bytes().unwrap().to_vec()).unwrap();
    let differing: Vec<usize> = expected.lines().zip(actual.lines()).enumerate().filter(|(_, (g, r))| g != r).map(|(i, _)| i).collect();
    let shown = if differing.is_empty() { String::new() } else {
        vm.eval_source(&setup(true)).unwrap();
        let trees = String::from_utf8_lossy(vm.eval_source(PROGRAM).unwrap().bytes().unwrap()).into_owned();
        let trees: Vec<&str> = trees.lines().collect();
        differing.iter().take(20).map(|&i| format!("{:?}\n  Romacs {}", cases[i], trees.get(i).unwrap_or(&"?"))).collect::<Vec<_>>().join("\n")
    };
    std::fs::remove_file(&file).unwrap();
    assert_eq!(expected.lines().count(), cases.len());
    assert_eq!(actual.lines().count(), cases.len());
    assert!(differing.is_empty(), "{} of {} trees differ from GNU's:\n{shown}", differing.len(), cases.len());
}

/// Past libxml2's 256 open elements the parse stops. Each case's tree is
/// summarised without recursion (GNU cannot print one so deep): its depth
/// and the texts of its two innermost elements.
#[test]
fn deep_nesting_stops_as_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let program = r#"(let (out)
  (dolist (n '(253 254 255 2000))
    (with-temp-buffer
      (dotimes (_ n) (insert "<span>"))
      (insert "x<b>y</b>z</span>w")
      (let ((node (libxml-parse-html-region (point-min) (point-max))) (depth 0) (texts nil))
        (while (and (consp node) (symbolp (car node)))
          (setq depth (1+ depth))
          (push (seq-filter #'stringp (nthcdr 2 node)) texts)
          (setq node (seq-find #'consp (nthcdr 2 node))))
        (push (list n depth (car texts) (cadr texts)) out))))
  (nreverse out))"#;
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {program}))")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut vm = bootstrapped();
    let actual = vm.eval_source(&format!("(encode-coding-string (prin1-to-string {program}) (quote utf-8))")).unwrap().bytes().unwrap().to_vec();
    assert_eq!(String::from_utf8(actual).unwrap(), String::from_utf8(output.stdout).unwrap());
}
