"""Failure cases for the complete tree-sitter query record comparison."""
from contextlib import redirect_stdout
from copy import deepcopy
import importlib.machinery
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "treesit-query-diff"
LOADER = importlib.machinery.SourceFileLoader("treesit_query_diff", str(SCRIPT))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
DIFF = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(DIFF)
import corpus
from corpus import MODE_MINIMUM, freeze_cases, sha256
from records import capture, probe_expectations


def manifest_fixture():
    cases = freeze_cases([dict(language=language, text="{}", queries=["(_) @n"],
                               origins=["shipped.el"], shipped=1)
                          for language in sorted(MODE_MINIMUM)])
    return dict(format=1, c_revision="a" * 40, cases=cases)


def write_manifest(path, manifest):
    content = (DIFF.canonical(manifest) + "\n").encode()
    path.write_bytes(content)
    return sha256(content)


def command(*args):
    return subprocess.run([sys.executable, "-B", SCRIPT, *args], capture_output=True, text=True)


class CorpusDigestTest(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.path = Path(scratch.name) / "corpus.json"
        self.manifest = manifest_fixture()
        self.expected = write_manifest(self.path, self.manifest)
        self.manifest["c_editor_sha256"] = "1" * 64
        self.compare_args = ("compare", "--manifest", self.path,
                             "--reference", self.path, "--candidate", self.path)

    def test_editor_fingerprint_does_not_change_identity(self):
        for fingerprint in ("1" * 64, "2" * 64):
            with self.subTest(fingerprint=fingerprint):
                write_manifest(self.path, dict(self.manifest, c_editor_sha256=fingerprint))
                self.assertEqual(DIFF.load_manifest(self.path, self.expected), self.manifest["cases"])

    def test_changed_query_digest_fails(self):
        self.manifest["cases"][0]["query_sha256"][0] = "0" * 64
        write_manifest(self.path, self.manifest)
        with self.assertRaisesRegex(ValueError, "frozen manifest digest changed"):
            DIFF.load_manifest(self.path, self.expected)

    def test_substituted_manifest_and_sidecar_fail(self):
        case = self.manifest["cases"][0]
        case["queries"][0] = "_ @other"
        freeze_cases([case])
        raw_digest = write_manifest(self.path, self.manifest)
        self.path.with_suffix(".sha256").write_text(raw_digest + "\n")
        with self.assertRaisesRegex(ValueError, "frozen manifest digest changed"):
            DIFF.load_manifest(self.path, self.expected)
        result = command(*self.compare_args, "--corpus-digest", self.expected)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("frozen manifest digest changed", result.stderr)

    def test_missing_digest_fails_for_both_commands(self):
        for args in (("run", "--out", self.path.parent / "run", "--c-reference", "HEAD"),
                     self.compare_args):
            with self.subTest(command=args[0]):
                result = command(*args)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("the following arguments are required: --corpus-digest", result.stderr)

    def test_missing_c_reference_fails(self):
        result = command("run", "--out", self.path.parent / "run", "--corpus-digest", self.expected)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("the following arguments are required: --c-reference", result.stderr)

    def test_reference_without_c_query_engine_fails(self):
        with patch.object(DIFF, "archive", return_value=self.path.parent):
            with self.assertRaisesRegex(ValueError, r"C reference .* has no .*query\.c"):
                DIFF.archive_reference("a" * 40, self.path.parent / "reference")

    def test_bundled_grammar_without_sample_fails(self):
        root = self.path.parent
        bundle = root / "crates/romacs-grammars/src/lib.rs"
        bundle.parent.mkdir(parents=True)
        bundle.write_text('"kotlin" tree_sitter_kotlin_ng::LANGUAGE, "https://example.org/kotlin" '
                          f'"{self.manifest["c_revision"]}";\n')
        with self.assertRaisesRegex(ValueError, "no corpus sample for bundled grammar kotlin"):
            corpus.grammar_samples(root, {})

    @unittest.skipUnless(os.environ.get("ROMACS_QUERY_CORPUS_FIXTURES"),
                         "set ROMACS_QUERY_CORPUS_FIXTURES to the typing-modes artifact directory")
    def test_both_extracted_manifests_verify_against_pinned_digest(self):
        root = Path(os.environ["ROMACS_QUERY_CORPUS_FIXTURES"])
        expected = "357a891a6b1c7921f74927894e79a79a0d80eb37139e47fc1ad46a6f3cc1f293"
        for name in ("q-verify/recheck/production-equivalent", "ts-query/hardened"):
            with self.subTest(extraction=name):
                self.assertEqual(len(DIFF.load_manifest(root / name / "corpus.json", expected)),
                                 16)


class CReferenceQueryTest(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.native = self.write(DIFF.NATIVE, """use tree_sitter::ffi;
pub(crate) use ffi::{TSNode};
pub(crate) struct Query(*mut ffi::TSQuery);
impl Query {
    fn new() {}
    fn capture_name() {}
    fn pattern_count() {}
    fn predicates() {}
    /// Every match of the query under NODE
    fn matches() {}
}
""")
        self.bindings = self.write("crates/vendor/tree-sitter/binding_rust/bindings.rs",
                                   "    pub fn ts_query_capture_count(self_: *const TSQuery) -> u32;\n")
        self.engine = self.write(DIFF.C_ENGINE, "uint32_t ts_query_capture_count(const TSQuery *self) {\n")

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def install(self):
        DIFF.add_reference_query_tests(self.root, DIFF.ROOT / DIFF.SUPPORT)
        return self.native.read_text()

    def test_adds_accessor_once(self):
        installed = self.install()
        self.assertIn("#[cfg(test)]\n    fn capture_count(&self) -> u32", installed)
        self.assertIn('#[path = "query_oracle_tests.rs"]\nmod query_oracle;', installed)
        self.assertEqual(installed, self.install())

    def test_keeps_existing_accessor(self):
        text = self.native.read_text().replace("    fn capture_name",
                                              "    fn capture_count() {}\n    fn capture_name")
        self.native.write_text(text)
        self.engine.unlink()
        self.bindings.unlink()
        self.assertEqual(self.install().count("fn capture_count("), 1)

    def test_accessor_requires_the_c_symbol_and_rust_binding(self):
        for path in (self.engine, self.bindings):
            with self.subTest(path=path.name):
                original = path.read_text()
                path.write_text("/* capture count removed */\n")
                with self.assertRaisesRegex(ValueError, f"ts_query_capture_count.*{path.name}"):
                    self.install()
                path.write_text(original)

    def test_missing_wrapper_method_and_anchor_are_reported(self):
        original = self.native.read_text()
        for old, message in (
            ("fn matches()", "missing C reference method Query::matches"),
            ("/// Every match of the query under NODE", "C reference anchor.*Every match"),
        ):
            with self.subTest(missing=old):
                self.native.write_text(original.replace(old, ""))
                with self.assertRaisesRegex(ValueError, message):
                    self.install()


def fixture():
    case = dict(language="json", queries=["(_) @n"] * 8, origins=["shipped.el"] * 8,
                shipped=8, states=[[1, 0, None, [[0, 10]], "hash"],
                                    [1, 1, [0, 0, "x"], [[0, 10]], "hash2"]])
    rows = [["grammar", "json", 8, 14]]
    rows += [["compile", i, ["ok", {"patterns": 1, "captures": [[110]], "predicates": [[]]}]]
             for i in range(8)]
    matches = [[0, [capture(0, 1, 10, 0, 1), capture(0, 2, 10, 1, 2)]]]
    for seed, index, change, ranges, digest in case["states"]:
        rows.append(["state", seed, index, change, digest])
        rows += [["run", i, 0, ranges[0], deepcopy(matches)] for i in range(8)]
    return case, rows


def probes(rust):
    return [["probe", name, index, new if rust else old]
            for name, rows in probe_expectations() for index, (old, new) in enumerate(rows)]


class QueryDiffTest(unittest.TestCase):
    def compare(self, case, old, new):
        with tempfile.TemporaryDirectory() as directory:
            before, after = (Path(directory) / name for name in ("before", "after"))
            for path, rows in ((before, old), (after, new)):
                path.write_text("".join(json.dumps(row) + "\n" for row in rows))
            with redirect_stdout(io.StringIO()):
                DIFF.compare(before, after, [case], True)

    def test_each_manifest_query_requires_one_compile_and_one_run_per_edit(self):
        case, records = fixture()
        self.compare(case, records + probes(False), records + probes(True))
        for label, cut, duplicate in (
            ("missing compile", lambda row: row[:2] != ["compile", 2], None),
            ("missing query and runs", lambda row: row[:2] not in (["compile", 2], ["run", 2]), None),
            ("duplicate compile", lambda row: True, 2),
            ("missing run", lambda row: row is not records[-2], None),
            ("duplicate run", lambda row: True, len(records) - 1),
            ("missing edit", lambda row: row is not records[18], None),
            ("out of manifest", lambda row: True, None),
        ):
            with self.subTest(label=label):
                altered = [row for row in records if cut(row)]
                if duplicate is not None:
                    altered.insert(duplicate, records[duplicate])
                if label == "out of manifest":
                    altered.insert(5, ["compile", 9, ["error", 0, "Syntax error at"]])
                with self.assertRaisesRegex(ValueError, "expected"):
                    self.compare(case, altered + probes(False), altered + probes(True))

    def test_dropped_extra_reordered_and_wrong_capture(self):
        case, old = fixture()
        for action in ("drop", "extra", "swap", "wrong kind"):
            with self.subTest(action=action):
                new = deepcopy(old)
                captures = new[-1][-1][0][1]
                if action == "drop":
                    captures.pop()
                elif action == "extra":
                    captures.append(captures[0])
                elif action == "swap":
                    captures.reverse()
                else:
                    captures[0][2] = 8
                with self.assertRaisesRegex(ValueError, "seed 1 edit 1 range 0.*shipped.el"):
                    self.compare(case, old + probes(False), new + probes(True))

    def test_known_exceptions_require_exact_nodes(self):
        case, records = fixture()
        for label, arm in (("wildcard-negated-field", False),
                           ("fourth-capture", True), ("wide-step-index", True)):
            with self.subTest(label=label):
                c = probes(False)
                rust = probes(True)
                row = next(row for row in (rust if arm else c) if row[1] == label and row[-1])
                row[-1][0][1][0][4] = [999, 1001]
                with self.assertRaisesRegex(ValueError, f"probe {label} case"):
                    self.compare(case, records + c, records + rust)

    def test_extra_record_after_last_probe_fails(self):
        case, records = fixture()
        with self.assertRaisesRegex(ValueError, "unexpected extra record"):
            self.compare(case, records + probes(False), records + probes(True) + [["other"]])


if __name__ == "__main__":
    unittest.main()
