//! `libxml-parse-xml-region' against pinned GNU (libxml2 2.14 under xml.c's
//! options): well-formedness, blank nodes (NOBLANKS), namespaces, entities
//! and the internal subset, comments and processing instructions at top
//! level, and what real consumers read (a DOCTYPE'd SVG, D-Bus
//! introspection, Atom with and without a prefix). Both engines read one
//! case file and print each case's tree.
//!
//! Known difference: make_dom interns a non-ASCII name undecoded, so GNU's
//! symbol has a unibyte name, and Romacs's is the name; those cases only
//! compare whether the document parses.
use romacs_vm::Vm;
use std::path::Path;

const CASES: &[&str] = &[
    "<a/>",
    "<a>x</a>",
    "<a b='1' c=\"2\">t<d/>u</a>",
    "",
    "   ",
    "not xml",
    "<a>",
    "<a></b>",
    "<a><b></a></b>",
    "<a/><b/>",
    "<a/>junk",
    "junk<a/>",
    "<!-- c --><a/>",
    "<a/><!-- c -->",
    "<!-- 1 --><a/><!-- 2 -->",
    "<!-- only -->",
    "<a><!-- in --></a>",
    "<a>\n  <b/>\n  <c> x </c>\n</a>",
    "<a> </a>",
    "<a>  <!-- x -->  </a>",
    "<a> t <b/> </a>",
    "<a>&lt;&gt;&amp;&apos;&quot;</a>",
    "<a>&#65;&#x3bb;</a>",
    "<a>&nope;</a>",
    "<a b='&lt;&#65;'/>",
    "<a b='1' b='2'/>",
    "<a><![CDATA[x<y]]></a>",
    "<a>x<![CDATA[y]]>z</a>",
    "<a><![CDATA[]]></a>",
    "<a><![CDATA[  ]]></a>",
    "<?xml version='1.0'?><a/>",
    "<?xml version='1.0' encoding='ISO-8859-1'?><a>é</a>",
    "<?xml version='1.0' encoding='UTF-8'?><a>λ</a>",
    "<?pi data?><a/>",
    "<a><?pi data?></a>",
    "<a/><?pi data?>",
    "<!DOCTYPE a><a/>",
    "<!DOCTYPE a [<!ENTITY e 'v'>]><a>&e;</a>",
    "<!DOCTYPE a [<!ENTITY e 'v'>]><a>x&e;y</a>",
    "<!DOCTYPE a [<!ENTITY e '<b/>'>]><a>&e;</a>",
    "<!DOCTYPE a [<!ENTITY e 'v'>]><a b='&e;'/>",
    "<!DOCTYPE a SYSTEM 'x.dtd'><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA 'dflt'>]><a/>",
    "<x:a xmlns:x='u'/>",
    "<x:a xmlns:x='u' x:b='1'/>",
    "<x:a/>",
    "<a xmlns='u'><b/></a>",
    "<a xmlns='u' xmlns:y='v' y:q='1'><y:b/></a>",
    "<feed xmlns='http://www.w3.org/2005/Atom'><title>t</title></feed>",
    "<atom:feed xmlns:atom='http://www.w3.org/2005/Atom'><atom:title>t</atom:title></atom:feed>",
    "<a:b:c xmlns:a='u'/>",
    "<λ β='γ'/>",
    "<a b=1/>",
    "<a b/>",
    "<a b='<'/>",
    "<1a/>",
    "<a>]]></a>",
    "<a>\r\nx\ry</a>",
    "<a b='x\ny\tz'/>",
    "<a>&#0;</a>",
    "<a>&#xD800;</a>",
    "<a>\u{1}</a>",
    "<a>--</a>",
    "<!-- a -- b --><a/>",
    "<a><b>1</b> <b>2</b></a>",
    "<a> <b/>t</a>",
    "<a>t <b/> <c/></a>",
    "<a>   <![CDATA[x]]>   </a>",
    "<a b=''/>",
    "<a b='x&lt;y'/>",
    "<!DOCTYPE a [<!ENTITY e 'v'>]><a b='x&e;y'/>",
    "<!DOCTYPE a [<!ENTITY e ''>]><a b='&e;'/>",
    "<a b='&#10;c&#9;d'/>",
    "<a b='  x  '/>",
    "<a xml:lang='en'/>",
    "<a z:q='1'/>",
    "<xml:a/>",
    "<a xmlns:x='u'><x:b/></a>",
    "<a><x:b xmlns:x='u'/><x:c/></a>",
    "<a x:b='1' y:b='2' xmlns:x='u' xmlns:y='u'/>",
    "<a x:b='1' b='2' xmlns:x='u'/>",
    "<x:a xmlns:x='u'></y:a>",
    "<x:a xmlns:x='u' xmlns:y='u'></y:a>",
    "<a xmlns:x=''/>",
    "<a xmlns=''/>",
    "<a xmlns:xmlns='u'/>",
    "<xmlns:a/>",
    "<:a/>",
    "<a:/>",
    "<a: xmlns:a='u'/>",
    "<a :b='1'/>",
    "<a b:='1'/>",
    "<a b:c:d='1' xmlns:b='u'/>",
    "<a/><?xml version='1.0'?>",
    "  <?xml version='1.0'?><a/>",
    "<?xml?><a/>",
    "<?xml version='2.0'?><a/>",
    "<?xml version='1.0' standalone='yes'?><a/>",
    "<?xml version='1.0' standalone='maybe'?><a/>",
    "<?xml version=1.0?><a/>",
    "<?xml version='1.0'  ?><a/>",
    "<?xml encoding='utf-8'?><a/>",
    "<?XML version='1.0'?><a/>",
    "<?xml-stylesheet href='x'?><a/>",
    "<?a?><b/>",
    "<? a?><b/>",
    "<a><?xml x?></a>",
    "<a><?x ?></a>",
    "<a><?x?y?></a>",
    "<!DOCTYPE a [<!ENTITY e '<b>'>]><a>&e;</a>",
    "<!DOCTYPE a [<!ENTITY e '<b/>'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e '&e;'>]><a>&e;</a>",
    "<!DOCTYPE a [<!ENTITY e '&f;'><!ENTITY f 'v'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e '&nope;'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e '&nope;'>]><a>&e;</a>",
    "<!DOCTYPE a [<!ENTITY e '&#60;'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e '&#38;#60;'>]><a b='&e;'/>",
    "<!DOCTYPE a SYSTEM 'x.dtd'><a>&e;</a>",
    "<!DOCTYPE a SYSTEM 'x.dtd'><a b='&e;'/>",
    "<!DOCTYPE a PUBLIC 'p' 'x.dtd'><a/>",
    "<!DOCTYPE a PUBLIC 'p'><a/>",
    "<!DOCTYPE a [<!ENTITY % p '<!ENTITY e \"v\">'> %p;]><a>&e;</a>",
    "<!DOCTYPE a [<!ENTITY % p 'x'> %p;]><a/>",
    "<!DOCTYPE a [%q;]><a>&e;</a>",
    "<!DOCTYPE a [<!ENTITY e SYSTEM 'x'>]><a>&e;</a>",
    "<!DOCTYPE a [<!ENTITY e SYSTEM 'x'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e SYSTEM 'x' NDATA n><!NOTATION n SYSTEM 'y'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ELEMENT a (b)><!ELEMENT b EMPTY>]><a> <b/> </a>",
    "<!DOCTYPE a [<!ELEMENT a (#PCDATA|b)*>]><a> <b/> </a>",
    "<!DOCTYPE a [<!ELEMENT a ANY>]><a> <b/> </a>",
    "<!DOCTYPE a [<!ATTLIST a b NMTOKENS #IMPLIED>]><a b='  x   y  '/>",
    "<!DOCTYPE b><a/>",
    "<!DOCTYPE a><!DOCTYPE a><a/>",
    "<a/><!DOCTYPE a>",
    "<!DOCTYPE a [<!-- c --><?pi x?>]><a/>",
    "<!DOCTYPE a [ junk ]><a/>",
    "<!-- c --><!DOCTYPE a><a/>",
    "<!DOCTYPE a [<!ENTITY lt '&#38;#60;'>]><a>&lt;</a>",
    "<!DOCTYPE a [<!ENTITY e 'v'><!ENTITY e 'w'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY amp 'x'>]><a>&amp;</a>",
    "<a xml:space='preserve'> <b/> </a>",
    "<a xml:space='default'> <b/> </a>",
    "<a xml:space='preserve'><c> <b/> </c></a>",
    "<a xml:space='preserve'><c xml:space='default'> <b/> </c></a>",
    "<a xml:space='bogus'> <b/> </a>",
    "<a>t<b> <c/> </b> <d/></a>",
    "<a><b/> <c/> t</a>",
    "<a> &#32; <b/></a>",
    "<a>&#32;<b/></a>",
    "<a> <b/> &#32;</a>",
    "<a> \n</a>",
    "<a>\n<!-- c -->\n<b/>\n</a>",
    "<a>\n<?p i?>\n<b/>\n</a>",
    "<!DOCTYPE a [<!ENTITY e 'v'>]><a> &e; <b/> </a>",
    "<a> \r\n<b/>\r\n</a>",
    "<a>\r<b/></a>",
    "<a><b/>\r</a>",
    "<a>&#x110000;</a>",
    "<a>&#xFFFE;</a>",
    "<a>&#xFFFF;</a>",
    "<a>&#xE000;</a>",
    "<a>&#1;</a>",
    "<a>&#9;</a>",
    "<a>&#xD;</a>",
    "<a>&#;</a>",
    "<a>&#x;</a>",
    "<a>&;</a>",
    "<a>&amp</a>",
    "<a>&</a>",
    "<a>a&b</a>",
    "<a b='&'/>",
    "<a b='&amp'/>",
    "<a b='&nope;'/>",
    "<a b=\"'\"/>",
    "<a b='\"'/>",
    "<a  b = '1'  />",
    "<a b='1'c='2'/>",
    "<a\n/>",
    "</a>",
    "<a/ >",
    "< a/>",
    "<a></a >",
    "<a></ a>",
    "<a>\u{7f}</a>",
    "<a>\u{fffe}</a>",
    "<!-- --->",
    "<!-- - --><a/>",
    "<!---><a/>",
    "<!----><a/>",
    "<a><!----></a>",
    "<![CDATA[x]]><a/>",
    "<a/><![CDATA[x]]>",
    "<a><![CDATA[]]]></a>",
    "<a><![CDATA[]]]]></a>",
    "<a><![CDATA[x]]</a>",
    "<a><![cdata[x]]></a>",
    "\u{feff}<a/>",
    "\n<a/>\n",
    "<a/>\n\n",
    "<a>é</a>",
    "<é/>",
    "<a\u{301}/>",
    "<a·b/>",
    "<\u{301}/>",
    "<a×/>",
    "<⁰/>",
    "<a.b-c_d1/>",
    "<-a/>",
    "<.a/>",
    "<?xml version='1.foo'?><a/>",
    "<?xml version='1.'?><a/>",
    "<?xml version='1.0.1'?><a/>",
    "<?xml version='1.5'?><a/>",
    "<?xml version='1.10'?><a/>",
    "<?xml version='1'?><a/>",
    "<?xml version='1.0a'?><a/>",
    "<a xmlns:p='http://www.w3.org/2000/xmlns/'><p:b/></a>",
    "<a xmlns:p='http://www.w3.org/XML/1998/namespace'><p:b/></a>",
    "<a xmlns='http://www.w3.org/2000/xmlns/'><b/></a>",
    "<a xmlns:xml='http://www.w3.org/XML/1998/namespace'><xml:b/></a>",
    "<a xmlns:xml='u'><xml:b/></a>",
    "<a xmlns:xmlns='http://www.w3.org/2000/xmlns/'/>",
    "<!DOCTYPE a [<!ATTLIST a b () #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b (x) #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b (x|) #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b (x||y) #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b (1x|y.z) #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b ( x | y ) #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b (x y) #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b NOTATION () #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b NOTATION (1x) #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b NOTATION (x|y) #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b NOTATION(x) #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA \"<\">]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA '&lt;'>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA '&nope;'>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA '&'>]><a/>",
    "<!DOCTYPE a [<!ENTITY e 'v'><!ATTLIST a b CDATA '&e;'>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA '&e;'><!ENTITY e 'v'>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA #FIXED '<'>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA #FIXED'v'>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA'v'>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b ID 'v'>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b(x)'x'>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA #IMPLIED c CDATA #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a b CDATA #IMPLIEDc CDATA #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a>]><a/>",
    "<!DOCTYPE a [<!ATTLIST a >]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (#PCDATA|b)>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (#PCDATA|b)*>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (#PCDATA)>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (#PCDATA)*>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (#PCDATA)+>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (#PCDATA)?>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (#PCDATA|b)+>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (#PCDATA,b)*>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (b|#PCDATA)*>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (b,(#PCDATA))>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a ()>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (b?)*>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (b)(c)>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a b>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a EMPTYx>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a ANY x>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a(b)>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a EMPTY><!ELEMENT a ANY>]><a> <b/> </a>",
    "<!DOCTYPE a PUBLIC 'p<' 'x'><a/>",
    "<!DOCTYPE a PUBLIC \"p'q\" 'x'><a/>",
    "<!DOCTYPE a PUBLIC 'pλ' 'x'><a/>",
    "<!DOCTYPE a [<!NOTATION n PUBLIC 'p{'>]><a/>",
    "<!DOCTYPE a SYSTEM'x'><a/>",
    "<!DOCTYPE a PUBLIC'p' 'x'><a/>",
    "<!DOCTYPE a PUBLIC 'p''x'><a/>",
    "<!DOCTYPE a[]><a/>",
    "<!DOCTYPE a []><a/>",
    "<!DOCTYPEa><a/>",
    "<!DOCTYPE><a/>",
    "<!DOCTYPE a [] x><a/>",
    "<!DOCTYPE a SYSTEM 'x' [<!ENTITY e 'v'>]><a>&e;</a>",
    "<a><![CDATA[x\r\ny\rz]]></a>",
    "<a><!-- x\r\ny\rz --></a>",
    "<a b='x\ry'/>",
    "<!DOCTYPE a [<!ENTITY e 'x\r\ny'>]><a b='&e;'/>",
    "<?p x\ry?><a/>",
    "<a>x\r\n</a>",
    "<a>\r\n\r\n</a>",
    "<a>&#13;\n</a>",
    "<!DOCTYPE a PUBLIC 'x\ry' 'urn:x'><a/>",
    "<!DOCTYPE a PUBLIC 'x\r\ny' 'urn:x'><a/>",
    "<!DOCTYPE a [<!ENTITY e 'x\ry'>]><a b='&e;'/>",
    "<!-- a\rb --><a/>",
    "<!DOCTYPE a [<!ENTITY e '%x;'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e '%x;'>]><a>&e;</a>",
    "<!DOCTYPE a [<!ENTITY % x 'v'><!ENTITY e '%x;'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e '100%'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e '%x'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e 'p%x;q'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e '%x;'>]><a>&z;</a>",
    "<a xmlns:x=''><x:b/></a>",
    "<a><b/>t<c/> <d/></a>",
    "<a><b/>&#65;<c/> <d/></a>",
    "<a xml:space='preserve'>t<b> <c/> </b></a>",
    "<!DOCTYPE a [<!ELEMENT a (b>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (b,c|d)>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (#PCDATA,b)>]><a/>",
    "<!DOCTYPE a [<!ELEMENT a (b?,(c|d)*)+>]><a> <b/> </a>",
    "<!DOCTYPE a [<!ATTLIST a b (x|y) 'x'>]><a b=' x '/>",
    "<!DOCTYPE a [<!ATTLIST a b ID #REQUIRED c CDATA #FIXED 'v' d NOTATION (n) #IMPLIED>]><a b=' i ' c=' v '/>",
    "<!DOCTYPE a [<!ATTLIST a b BOGUS #IMPLIED>]><a/>",
    "<!DOCTYPE a [<!NOTATION n PUBLIC 'p'><!NOTATION m PUBLIC 'p' 's'><!NOTATION o SYSTEM 's'>]><a/>",
    "<!DOCTYPE a [<!NOTATION n>]><a/>",
    "<!DOCTYPE a [<!ENTITY e '%x;'>]><a/>",
    "<!DOCTYPE a [<!ENTITY e '&;'>]><a/>",
    "<!DOCTYPE a [<!ENTITY % p SYSTEM 'x'> %p;]><a>&e;</a>",
    "<?xml version='1.0' standalone='yes'?><!DOCTYPE a SYSTEM 'x'><a>&e;</a>",
    "<?xml version='1.1'?><a/>",
    "<?xml version='1.0'encoding='utf-8'?><a/>",
    "<?xml version='1.0' encoding='1x'?><a/>",
    "<?xml version='1.0' encoding='bogus-enc'?><a>x</a>",
    "<?xml version='1.0' encoding=\"UTF-16\"?><a>x</a>",
    "<a b='1' xmlns:c='u' c:d='2' e='3'/>",
    "<a>λ<b c='λ'>é</b></a>",
    "<a b='x\r\ny\rz'/>",
    "<a>]]</a>",
    "<a>]]&gt;</a>",
    "<a>x]]>y</a>",
    "<a><![CDATA[ a ]]> b</a>",
    "<!DOCTYPE a [<!ENTITY e '<b>x</b>t'>]><a>&e;&e;</a>",
    "<!DOCTYPE a [<!ENTITY e 'x'><!ENTITY f '&e;y'>]><a>&f;</a>",
    "<!DOCTYPE a [<!ENTITY e '&f;'><!ENTITY f '&e;'>]><a>&e;</a>",
    "<!DOCTYPE a [<!ENTITY e '&f;'><!ENTITY f '&e;'>]><a b='&e;'/>",
    "<!DOCTYPE a [<!ENTITY e SYSTEM 'x' NDATA n>]><a>&e;</a>",
    "<!DOCTYPE svg PUBLIC '-//W3C//DTD SVG 1.1//EN' 'http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd'><svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><title>t</title>\n  <rect x='0'/>\n</svg>",
    "<!DOCTYPE svg [<!ENTITY ns_svg 'http://www.w3.org/2000/svg'>]><svg xmlns='&ns_svg;' xmlns:x='&ns_svg;'><x:g/></svg>",
    "<!DOCTYPE node PUBLIC '-//freedesktop//DTD D-BUS Object Introspection 1.0//EN' 'http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd'>\n<node>\n <interface name='a.b'>\n  <method name='M'><arg name='x' type='s' direction='in'/></method>\n </interface>\n</node>\n",
    "<?xml version='1.0' encoding='utf-8'?><feed xmlns='http://www.w3.org/2005/Atom' xml:lang='en'><title type='html'>&lt;b&gt;t&lt;/b&gt;</title><entry><content type='xhtml'><div xmlns='http://www.w3.org/1999/xhtml'><p>x</p></div></content></entry></feed>",
];

fn lisp_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' | '\\' => { out.push('\\'); out.push(c); }
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

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

const RUN: &str = r#"(with-temp-buffer
  (let ((coding-system-for-read 'utf-8)) (insert-file-contents FILE))
  (let ((cases (read (current-buffer))) (out nil))
    (dolist (case cases)
      (setq out (cons (with-temp-buffer
                        (insert case)
                        (let ((tree (condition-case e (libxml-parse-xml-region (point-min) (point-max)) (error (list 'signal e)))))
                          (if (string-match-p "\\`[[:ascii:]]*\\'" (symbol-name (or (car-safe tree) 'x)))
                              (prin1-to-string tree)
                            "non-ASCII name")))
                      out)))
    (append (encode-coding-string (mapconcat #'identity (nreverse out) "\036") 'utf-8) nil)))"#;

#[test]
fn xml_regions_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("romacs-libxml-xml-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("cases.el");
    let list = CASES.iter().map(|case| lisp_string(case)).collect::<Vec<_>>().join("\n");
    std::fs::write(&file, format!("(\n{list}\n)\n")).unwrap();
    let form = RUN.replace("FILE", &format!("{:?}", file.to_str().unwrap()));
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval"])
        .arg(format!("(prin1 {form})")).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let bytes = |codes: &str| String::from_utf8_lossy(&codes.trim_matches(['(', ')']).split_whitespace().map(|n| n.parse::<u8>().unwrap()).collect::<Vec<_>>()).into_owned();
    let expected = bytes(&String::from_utf8(output.stdout).unwrap());
    let mut vm = bootstrapped();
    let actual = bytes(&vm.eval_source(&form).and_then(|v| v.display()).unwrap());
    let (expected, actual): (Vec<_>, Vec<_>) = (expected.split('\u{1e}').collect(), actual.split('\u{1e}').collect());
    assert_eq!(expected.len(), CASES.len());
    assert_eq!(actual.len(), CASES.len());
    let failures: Vec<String> = CASES.iter().zip(expected.iter().zip(&actual)).filter(|(_, (e, a))| e != a)
        .map(|(case, (e, a))| format!("{case:?}\n  GNU    {e}\n  Romacs {a}")).collect();
    assert!(failures.is_empty(), "{} of {} differ:\n{}", failures.len(), CASES.len(), failures.join("\n"));
}

/// The arguments, the region, and where a region's text comes from.
const FORMS: &str = r#"
(with-temp-buffer (insert "<!-- 1 --><a/><!-- 2 -->") (list (libxml-parse-xml-region (point-min) (point-max) nil t) (libxml-parse-xml-region nil nil "http://base/" nil)))
(with-temp-buffer (insert "<a><!-- in --></a>") (libxml-parse-xml-region (point-min) (point-max) nil t))
(with-temp-buffer (insert "<a/>") (condition-case e (libxml-parse-xml-region 1 5 3) (error e)))
(with-temp-buffer (insert "xx<a/>yy") (libxml-parse-xml-region 3 7))
(with-temp-buffer (insert "<a>λ</a>") (libxml-parse-xml-region))
(with-temp-buffer (insert "<a/>") (narrow-to-region 2 3) (libxml-parse-xml-region))
(with-temp-buffer (insert "<a/>") (libxml-parse-xml-region 5 1))
(with-temp-buffer (insert "<a/>") (condition-case e (libxml-parse-xml-region 1 99) (error (car e))))
(with-temp-buffer (insert "<a/>") (libxml-parse-xml-region (copy-marker 1) (copy-marker 5)))
(with-temp-buffer (insert "<a/>") (condition-case e (libxml-parse-xml-region "a" 5) (error e)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "<a>\316\273</a>") (libxml-parse-xml-region))
(with-temp-buffer (set-buffer-multibyte nil) (insert "<a>\351</a>") (libxml-parse-xml-region))
(with-temp-buffer (insert "<a>" (decode-coding-string "\351" 'utf-8) "</a>") (libxml-parse-xml-region))
(libxml-available-p)
(mapcar (lambda (n) (with-temp-buffer (dotimes (_ n) (insert "<a>")) (dotimes (_ n) (insert "</a>")) (null (libxml-parse-xml-region)))) '(255 256 257 300))
(mapcar (lambda (n) (null (with-temp-buffer (insert "<!DOCTYPE a [<!ENTITY % p0 '<!-- x -->'>" (mapconcat (lambda (i) (format "<!ENTITY %% p%d '&#37;p%d;'>" i (1- i))) (number-sequence 1 n) "") (format "%%p%d;" n) "]><a/>") (libxml-parse-xml-region)))) '(18 19 500))
(mapcar (lambda (n) (null (with-temp-buffer (insert "<!DOCTYPE a [<!ENTITY e0 'v'>" (mapconcat (lambda (i) (format "<!ENTITY e%d '&e%d;'>" i (1- i))) (number-sequence 1 n) "") (format "]><a>&e%d;</a>" n)) (libxml-parse-xml-region)))) '(18 19 10000))
(mapcar (lambda (n) (null (with-temp-buffer (insert "<!DOCTYPE a [<!ENTITY e0 'v'>" (mapconcat (lambda (i) (format "<!ENTITY e%d '&e%d;'>" i (1- i))) (number-sequence 1 n) "") (format "]><a b='&e%d;'/>" n)) (libxml-parse-xml-region)))) '(18 19 10000))
(mapcar (lambda (n) (null (with-temp-buffer (insert "<!DOCTYPE a [<!ELEMENT a " (make-string n ?\() "b" (make-string n ?\)) ">]><a/>") (libxml-parse-xml-region)))) '(256 257 100000))
(mapcar (lambda (n) (null (with-temp-buffer (insert "<!DOCTYPE a [<!ENTITY % p0 '<!-- xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx -->'>" (mapconcat (lambda (i) (format "<!ENTITY %% p%d '&#37;p%d;&#37;p%d;'>" i (1- i) (1- i))) (number-sequence 1 n) "") (format "%%p%d;" n) "]><a/>") (libxml-parse-xml-region)))) '(12 13 14 18 40))
(mapcar (lambda (n) (null (with-temp-buffer (insert "<!DOCTYPE a [<!ENTITY e0 '" (make-string 50 ?x) "'>" (mapconcat (lambda (i) (format "<!ENTITY e%d '&e%d;&e%d;'>" i (1- i) (1- i))) (number-sequence 1 n) "") (format "]><a>&e%d;</a>" n)) (libxml-parse-xml-region)))) '(12 14 18 40))
(mapcar (lambda (n) (null (with-temp-buffer (insert "<!DOCTYPE a [<!ENTITY e0 '" (make-string 50 ?x) "'>" (mapconcat (lambda (i) (format "<!ENTITY e%d '&e%d;&e%d;'>" i (1- i) (1- i))) (number-sequence 1 n) "") (format "]><a b='&e%d;'/>" n)) (libxml-parse-xml-region)))) '(12 14 18 40))
(mapcar (lambda (k) (null (with-temp-buffer (insert "<!DOCTYPE a [<!ENTITY e '" (make-string 1000 ?x) "'>]><a>" (mapconcat (lambda (_) "&e;") (number-sequence 1 k) "") "</a>") (libxml-parse-xml-region)))) '(100 900 1000 5000))
(mapcar (lambda (k) (null (with-temp-buffer (insert "<!DOCTYPE a [<!ENTITY e '" (make-string 1000 ?x) "'>]><a " (mapconcat (lambda (i) (format "b%d='&e;'" i)) (number-sequence 1 k) " ") "/>") (libxml-parse-xml-region)))) '(100 900 1000 5000))
(let ((entity (concat "<!DOCTYPE a [<!ENTITY e '" (make-string 1000 ?x) "'>]>")) (refs (mapconcat (lambda (_) "&e;") (number-sequence 1 981) "")) (padding (make-string 300000 ?y))) (list (null (with-temp-buffer (insert entity "<a>" refs padding "</a>") (libxml-parse-xml-region))) (null (with-temp-buffer (insert entity "<a>" padding refs "</a>") (libxml-parse-xml-region)))))
(mapcar (lambda (s) (with-temp-buffer (insert s) (null (libxml-parse-xml-region)))) '("<é/>" "<λ β='γ'/>" "<aá/>" "<a·b/>" "<⁰/>" "<a×/>"))
"#;

#[test]
fn xml_region_forms_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = bootstrapped();
    let mut failures = Vec::new();
    for form in FORMS.lines().filter(|line| !line.is_empty()) {
        let source = format!("(append (encode-coding-string (prin1-to-string {form}) 'utf-8) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            failures.push(format!("{form}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
