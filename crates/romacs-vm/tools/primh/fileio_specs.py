#!/usr/bin/env python3
"""Write the primh specs for insert-file-contents and write-region (lane vm2/fileio).

The specs only DECLARE argument domains and probes; every answer comes from
the pinned GNU oracle through `record.py primh` (45b0f7699df, refused at any
other revision).  Both DEFUNs are C in GNU src/fileio.c at the pin:
`insert-file-contents' fileio.c:4057, `write-region' fileio.c:5461.

FIXTURES.  A case must make its own file: GNU runs every row in a fresh
process (effect=global) and the Rust replay builds a fresh Vm per row, but
neither can share state with the other.  So the FILENAME argument (or a
probe's first form) writes its fixture with `write-region' to a fixed path
under /tmp named after the primitive and the fixture, then returns the path.
GNU evaluates every row twice in one process (byte-compiled, then
interpreted; gnu_driver.el), so each argument expression also resets the
buffer it acts in.  Paths are absolute and fixed, never relative and never
under HOME: GNU's HOME is private per recording and its `default-directory'
is the recorder's, neither of which a replay shares (DIVERGENCES.md
VM2-PATH2).

GNU's -Q image also runs Lisp the C calls (`format-decode',
`after-insert-file-set-coding', `set-auto-coding'); with these fixtures
(no coding cookie, no format) none of them changes a value a row observes,
which the recorded rows themselves show: every refusal VM2 makes is pinned
as a named divergence, every other row must PASS.

Usage: python3 fileio_specs.py MANIFEST   (rewrites fileio-specs/*.json)
"""
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "fileio-specs"


def q(text):
    """A Lisp string literal for TEXT (str: multibyte; bytes: unibyte octets)."""
    if isinstance(text, bytes):
        return '"' + "".join(chr(b) if 32 <= b < 127 and b not in (34, 92) else f"\\{b:03o}" for b in text) + '"'
    return json.dumps(text, ensure_ascii=False).replace("\\u000d", "\\r").replace("\\u0000", "\\000")


def path(prim, name):
    return f"/tmp/romacs-primh-fio-{prim}-{name}"


def fixture(prim, name, content, buffer):
    """An expression that resets BUFFER, writes CONTENT to the fixture and returns its path."""
    p = path(prim, name)
    return (f'(progn (set-buffer (get-buffer-create "{buffer}")) (erase-buffer) (setq buffer-file-name nil) '
            f'(write-region {q(content)} nil "{p}") "{p}")')


IFC = "ifc"
FILES = {
    "ascii": "hi\n",
    "utf8": "λx\nb",
    "empty": "",
    "org": "* Heading\nλ body\n",
}


def ifc_specs():
    filename = [fixture(IFC, n, c, "fio-ifc") for n, c in FILES.items()]
    filename += [f'(progn (set-buffer (get-buffer-create "fio-ifc")) (erase-buffer) "{path(IFC, "missing-never-created")}")',
                 '"/tmp/"', "nil", "'sym", "5"]
    extra = [filename, ["nil", "t"], ["nil", "1", "-1", "1.0"], ["nil", "3", "100"], ["nil"]]

    def f(name, content):
        """A probe fixture: write the file from a string (no buffer is touched,
        so the probe's own `with-temp-buffer' stays current) and return its path."""
        p = path(IFC, "p-" + name)
        return f'(progn (write-region {q(content)} nil "{p}") "{p}")'

    def tb(body, pre=""):
        return f"(with-temp-buffer {pre}{body})"

    probes = []
    # Decoded text, point, the coding name, deactivate-mark: per content shape,
    # empty and non-empty buffer, every supported coding-system-for-read.
    shapes = {"ascii-lf": "hi\n", "ascii": "hi", "utf8-lf": "λ\n", "utf8": "λ", "empty": "", "multi": "a\nλ\nb",
              "crlf": "a\r\nb\r\n", "cr": "a\rb", "nul": "a\000b", "esc": "a\033b", "latin": b"caf\351\n",
              "bom": b"\357\273\277x"}
    for name, content in shapes.items():
        for csr in ["nil", "'undecided", "'utf-8", "'utf-8-unix", "'latin-1", "'no-conversion"]:
            for pre in ["", '(insert "ZZ") (goto-char 2) ']:
                probes.append(tb(f"(let* ((file {f(name, content)}) (coding-system-for-read {csr}) (last-coding-system-used 'sentinel) (deactivate-mark nil) "
                                 f"(r (insert-file-contents file))) "
                                 f"(list (equal (car r) file) (cdr r) (append (buffer-string) nil) (point) (point-min) (point-max) last-coding-system-used deactivate-mark))", pre))
    org = f("org", FILES["org"])
    probes += [
        # VISIT: the buffer visits the file, point stays at 1.
        tb(f"(let* ((file {org}) (r (insert-file-contents file t))) (list (equal (car r) file) (cdr r) (equal buffer-file-name file) (point) (equal (buffer-string) \"* Heading\\nλ body\\n\") last-coding-system-used))"),
        # Visiting a missing file completes the visit, then signals (fileio.c:5016, 5308).
        tb(f"(let ((file \"{path(IFC, 'p-missing-never-created')}\") (last-coding-system-used 'sentinel)) (list (condition-case e (insert-file-contents file t) (error (list (car e) (nth 1 e) (nth 2 e) (equal (nth 3 e) file)))) (equal buffer-file-name file) (buffer-size) last-coding-system-used))"),
        tb(f"(condition-case e (insert-file-contents \"{path(IFC, 'p-missing-never-created')}\") (error (list (car e) (nth 1 e) (nth 2 e))))"),
        tb(f'(progn (insert "q") (condition-case e (insert-file-contents {org} t) (error e)))'),
        tb(f'(condition-case e (insert-file-contents {org} t 1) (error e))'),
        tb(f'(condition-case e (insert-file-contents {org} t nil 3) (error e))'),
        tb('(condition-case e (insert-file-contents "/tmp/") (error (list (car e) (nth 1 e) (nth 2 e))))'),
        # BEG/END are byte offsets; END before BEG inserts nothing; past EOF stops at EOF.
        tb(f"(let ((r (insert-file-contents {org} nil 2 9))) (list (cdr r) (buffer-string) last-coding-system-used))"),
        tb(f"(let ((r (insert-file-contents {org} nil 9 2))) (list (cdr r) (buffer-string) last-coding-system-used))"),
        tb(f"(let ((r (insert-file-contents {org} nil 12 100))) (list (cdr r) (append (buffer-string) nil)))"),
        tb(f"(let ((r (insert-file-contents {org} nil nil 2))) (list (cdr r) (buffer-string)))"),
        tb(f"(let ((r (insert-file-contents {org} nil 1.0 3.0))) (list (cdr r) (buffer-string)))"),
        tb(f"(condition-case e (insert-file-contents {org} nil 1.5) (error e))"),
        tb(f"(condition-case e (insert-file-contents {org} nil nil -2) (error e))"),
        tb(f"(condition-case e (insert-file-contents {org} nil 'x) (error e))"),
        # A byte range that splits λ: GNU decodes the fragment as raw bytes (VM2 refuses).
        tb(f"(let ((r (insert-file-contents {org} nil 10 11))) (list (cdr r) (append (buffer-string) nil) last-coding-system-used))"),
        # Markers at point follow their insertion type; narrowing grows.
        tb(f'(progn (insert "AB") (let ((m (copy-marker 2)) (m2 (copy-marker 2 t))) (goto-char 2) (insert-file-contents {org}) (list (marker-position m) (marker-position m2) (point))))'),
        tb(f'(progn (insert "ABCD") (narrow-to-region 2 3) (goto-char 3) (insert-file-contents {org}) (list (point) (point-min) (point-max) (buffer-size)))'),
        # Inserted text takes no properties from its neighbours (C
        # `put-text-property': `propertize' is not in VM2 yet).
        tb(f"(progn (insert \"AB\") (put-text-property 1 3 'face 'bold) (goto-char 2) (insert-file-contents {org}) (list (get-text-property 1 'face) (get-text-property 2 'face) (get-text-property 18 'face) (get-text-property 19 'face)))"),
        # Read-only: barf-if-buffer-read-only; inhibit-read-only overrides.
        tb(f"(progn (setq buffer-read-only t) (condition-case e (insert-file-contents {org}) (error (list (car e) (eq (nth 1 e) (current-buffer))))))"),
        tb(f"(progn (setq buffer-read-only t) (let ((inhibit-read-only t)) (cdr (insert-file-contents {org}))))"),
        # REPLACE: VM2 refuses.
        tb(f'(progn (insert "old") (list (cdr (insert-file-contents {org} nil nil nil t)) (equal (buffer-string) "* Heading\\nλ body\\n")))'),
        tb(f"(progn (insert \"old\") (cdr (insert-file-contents {org} nil nil nil 'if-regular)))"),
        # Change hooks run for a non-visiting insertion (VM2 refuses).
        tb(f"(let ((log nil)) (set (make-local-variable 'after-change-functions) (list (lambda (&rest a) (setq log (cons a log))))) (insert-file-contents {org}) log)"),
        # A unibyte buffer: VM2 refuses.
        tb(f"(progn (set-buffer-multibyte nil) (cdr (insert-file-contents {f('ascii2', 'ab')})))"),
        # A file name handler table that matches nothing: GNU ignores it (VM2 refuses any).
        tb(f"(let ((file-name-handler-alist '((\"\\\\`/nowhere/\" . ignore)))) (cdr (insert-file-contents {org})))"),
        # The visited name is a fresh string, retained by the buffer across collections.
        tb(f"(let ((file {org})) (insert-file-contents file t) (let ((i 0)) (while (< i 20000) (list i (copy-sequence \"g\")) (setq i (1+ i)))) (list (equal buffer-file-name file) (eq buffer-file-name file) (equal (buffer-string) \"* Heading\\nλ body\\n\")))"),
        # The returned name is the expanded one: `.' and `..' and repeated slashes.
        tb(f"(let ((file {org})) (equal (car (insert-file-contents (concat \"/tmp//./x/../\" (substring file 5)))) file))"),
        # An already-expanded name comes back EQ (Fexpand_file_name returns its
        # argument); a rewritten one keeps the argument's multibyteness.
        # (VM2's reader makes every literal multibyte, so compare, not print.)
        tb(f'(let* ((file {org}) (r (insert-file-contents file))) (list (eq (car r) file) (eq (multibyte-string-p (car r)) (multibyte-string-p file))))'),
        tb(f'(let* ((file (concat "/tmp//./x/../" (substring {org} 5))) (r (insert-file-contents file))) (list (eq (car r) file) (eq (multibyte-string-p (car r)) (multibyte-string-p file)) (car r)))'),
    ]
    return extra, probes


WR = "wr"


def wr_specs():
    def buf(text, value):
        return f'(progn (set-buffer (get-buffer-create "fio-wr")) (erase-buffer) (setq buffer-file-name nil) (insert {q(text)}) {value})'
    start = [buf("héllo\nwörld\n", "nil"), buf("héllo\nwörld\n", "2"), buf("héllo", '"strλ\\n"'), buf("x", "'sym")]
    # (An out-of-range END prints the buffer in its signal data, and VM2's
    # printer writes `#<buffer>' without the name: probes cover that case
    # with the data's numbers instead.)
    end = ["nil", "4", "'sym"]
    filename = [f'"{path(WR, "dom")}"', "nil", f'"{path(WR, "no-such-dir")}/x"']
    extra = [start, end, filename, ["nil", "t"], ["nil", "t"], ["nil"], ["nil"]]

    def back(p):
        """Read P back exactly (utf-8-unix: no EOL conversion) as a list of characters
        (`append', a C function: `string-to-list' is subr.el Lisp)."""
        return f'(let ((coding-system-for-read (quote utf-8-unix))) (with-temp-buffer (insert-file-contents "{p}") (append (buffer-string) nil)))'

    def tb(body):
        return f"(with-temp-buffer {body})"

    P = lambda n: path(WR, n)
    probes = [
        # START nil writes the whole buffer although narrowed; narrowing survives.
        tb(f'(progn (insert "héllo\\nwörld\\n") (narrow-to-region 3 5) (list (write-region nil nil "{P("narrow")}") (point-min) (point-max) last-coding-system-used {back(P("narrow"))}))'),
        tb(f'(progn (insert "héllo\\nwörld\\n") (list (write-region 5 2 "{P("rev")}") {back(P("rev"))}))'),
        tb(f'(progn (insert "héllo\\nwörld\\n") (list (write-region (copy-marker 2) (copy-marker 4) "{P("marker")}") {back(P("marker"))}))'),
        tb(f'(progn (insert "abc") (condition-case e (write-region 10 2 "{P("oob")}") (error (list (car e) (nth 2 e) (nth 3 e)))))'),
        tb(f'(progn (insert "abc") (narrow-to-region 2 3) (condition-case e (write-region 1 3 "{P("oob2")}") (error (list (car e) (nth 2 e) (nth 3 e)))))'),
        tb(f'(progn (insert "abc") (condition-case e (write-region 2 nil "{P("endnil")}") (error e)))'),
        tb(f'(progn (insert "abc") (condition-case e (write-region 1.5 2 "{P("float")}") (error e)))'),
        # Strings: multibyte text as UTF-8, unibyte octets as themselves.
        f'(list (write-region "λ\\n" nil "{P("mb")}") {back(P("mb"))})',
        f'(list (write-region "\\316\\273!" nil "{P("ub")}") {back(P("ub"))})',
        f'(list (write-region "a\\r\\nb" nil "{P("cr")}") {back(P("cr"))})',
        f'(list (write-region "" nil "{P("empty")}") {back(P("empty"))})',
        # APPEND: t appends, a number seeks (and zero-fills past the end), negative signals.
        f'(progn (write-region "ab\\n" nil "{P("app")}") (write-region "cd\\n" nil "{P("app")}" t) {back(P("app"))})',
        f'(progn (write-region "abcd" nil "{P("off")}") (write-region "X" nil "{P("off")}" 1) {back(P("off"))})',
        f'(progn (write-region "abcd" nil "{P("offf")}") (write-region "X" nil "{P("offf")}" 2.0) {back(P("offf"))})',
        f'(progn (write-region "ab" nil "{P("past")}") (write-region "X" nil "{P("past")}" 4) {back(P("past"))})',
        f'(condition-case e (write-region "X" nil "{P("neg")}" -1) (error e))',
        f'(progn (write-region "ab" nil "{P("app0")}") (write-region "X" nil "{P("app0")}" 0) {back(P("app0"))})',
        # VISIT: t and a string set buffer-file-name; another value is quiet.
        tb(f'(progn (insert "v") (list (write-region nil nil "{P("visit")}" nil t) (equal buffer-file-name "{P("visit")}")))'),
        tb(f'(progn (insert "v") (list (write-region nil nil "{P("vs")}" nil "{P("vs-other-never-created")}") (equal buffer-file-name "{P("vs-other-never-created")}") {back(P("vs"))} (condition-case e (insert-file-contents "{P("vs-other-never-created")}") (file-missing (quote missing)))))'),
        tb(f'(progn (insert "v") (list (write-region nil nil "{P("quiet")}" nil (quote quiet)) buffer-file-name))'),
        # LOCKNAME must be a string; VM2 creates no lock file either way.
        f'(condition-case e (write-region "x" nil "{P("lock")}" nil nil (quote foo)) (error e))',
        f'(list (write-region "x" nil "{P("lock2")}" nil nil "{P("lock2-name")}") {back(P("lock2"))})',
        # MUSTBENEW: excl refuses an existing file. (Non-nil non-excl asks the
        # user, which a batch recording cannot answer; VM2 refuses it.)
        f'(progn (write-region "x" nil "{P("excl")}") (condition-case e (write-region "y" nil "{P("excl")}" nil nil nil (quote excl)) (error (list (car e) (nth 1 e) (equal (nth 2 e) "{P("excl")}")))))',
        # Errors.
        f'(condition-case e (write-region "x" nil "{P("no-such-dir")}/x") (error (list (car e) (nth 1 e) (nth 2 e))))',
        '(condition-case e (write-region "x" nil "/tmp/") (error (list (car e) (nth 1 e) (nth 2 e))))',
        f'(condition-case e (write-region "x" nil 5) (error e))',
        # Coding: utf-8 and utf-8-unix write the same octets; others VM2 refuses.
        f'(let ((coding-system-for-write (quote utf-8))) (list (write-region "λ\\n" nil "{P("c8")}") last-coding-system-used {back(P("c8"))}))',
        f'(let ((coding-system-for-write (quote utf-8-unix))) (list (write-region "λ\\n" nil "{P("c8u")}") last-coding-system-used {back(P("c8u"))}))',
        f'(let ((coding-system-for-write (quote latin-1))) (list (write-region "é\\n" nil "{P("cl1")}") last-coding-system-used))',
        f'(let ((coding-system-for-write (quote utf-8-dos))) (list (write-region "a\\nb" nil "{P("cdos")}") last-coding-system-used))',
        tb(f'(progn (set (make-local-variable (quote buffer-file-coding-system)) (quote utf-8)) (insert "λ") (list (write-region nil nil "{P("bf8")}") last-coding-system-used {back(P("bf8"))}))'),
        # Annotations and a unibyte buffer: VM2 refuses.
        f'(let ((write-region-annotate-functions (list (lambda (s e) nil)))) (write-region "x" nil "{P("ann")}"))',
        tb(f'(progn (set-buffer-multibyte nil) (insert "ab") (write-region nil nil "{P("uni")}"))'),
        # A file name handler table that matches nothing: GNU ignores it (VM2 refuses any).
        f'(let ((file-name-handler-alist (quote (("\\\\`/nowhere/" . ignore))))) (write-region "x" nil "{P("fnh")}"))',
        # Round trip: what write-region writes, insert-file-contents reads back.
        tb(f'(progn (insert "* Heading\\nλ body\\n") (write-region nil nil "{P("rt")}") (with-temp-buffer (insert-file-contents "{P("rt")}" t) (list (equal (buffer-string) "* Heading\\nλ body\\n") (equal buffer-file-name "{P("rt")}"))))'),
    ]
    return extra, probes


def spec(name, arity, gnu_def, rust, extra, probes):
    return dict(name=name, arity=arity, effect="global", gnu_files=["fileio.c"], gnu_def=gnu_def,
                rust=rust, args=["none"] * len(extra), extra={str(i): v for i, v in enumerate(extra)},
                probes=probes)


def main():
    manifest = {m["name"]: m for m in json.loads(Path(sys.argv[1]).read_text())}
    OUT.mkdir(exist_ok=True)
    out = []
    for name, arity, gen in [("insert-file-contents", [1, 5], ifc_specs), ("write-region", [3, 7], wr_specs)]:
        m = manifest[name]
        assert [m["min"], m["max"]] == arity and m["gnu"].startswith("fileio.c:"), m
        extra, probes = gen()
        out.append(spec(name, arity, m["gnu"], m["rust"], extra, probes))
    for s in out:
        (OUT / f"{s['name']}.json").write_text(json.dumps(s, indent=1, ensure_ascii=False) + "\n")
        print(s["name"], "written", len(s["probes"]), "probes")


if __name__ == "__main__":
    main()
