"""Known-answer tests for the tier P runner (scripts/probe, scripts/probes.py).

Every expected value here is known independently of the code under test: a
fake probe's own return value, a rule whose answer follows from arithmetic,
or an instruction/allocation count fixed by construction.
None is a captured output.
"""
import io
import json
import subprocess
import unittest
from pathlib import Path

import probes

HERE = Path(__file__).resolve().parent
# The admission screen as shipped before 6d2fca81 corrected it.
SHIPPED_SCREEN_COMMIT = "5e76b328"


def fake(name, value, answer=None, unit="units"):
    def run():
        if isinstance(value, Exception):
            raise value
        return value
    return probes.Probe(name, unit, answer, f"{name} doc", run)


def run(names, registry, as_json=False):
    out = io.StringIO()
    status = probes.run(names, registry, as_json=as_json, out=out)
    return status, out.getvalue()


class RunnerTests(unittest.TestCase):
    def test_prints_value_unit_and_known_answer_match(self):
        status, out = run([], {"a": fake("a", 7, answer=7, unit="widgets"), "b": fake("b", 5)})
        self.assertEqual(status, 0)
        lines = out.splitlines()
        self.assertRegex(lines[0], r"^a +7 widgets  = known answer  \(\d+\.\d\ds\)$")
        self.assertRegex(lines[1], r"^b +5 units  \(\d+\.\d\ds\)$")
        self.assertRegex(lines[2], r"^2 probes in \d+\.\d\ds \(build and registry 0\.00s\)$")

    def test_total_includes_setup_time(self):
        ticks = iter([0.0, 0.0, 1.0, 1.0])
        out = io.StringIO()
        probes.run([], {"a": fake("a", 1)}, out=out, clock=lambda: next(ticks), setup_seconds=2.5)
        self.assertEqual(out.getvalue().splitlines()[-1], "1 probes in 3.50s (build and registry 2.50s)")

    def test_missed_known_answer_exits_one_and_names_the_answer(self):
        status, out = run([], {"a": fake("a", 8, answer=7), "b": fake("b", 0, answer=0)})
        self.assertEqual(status, 1)
        self.assertIn("8 units  MISMATCH: known answer 7", out)
        self.assertIn("0 units  = known answer", out)

    def test_zero_is_a_value_not_a_missing_answer(self):
        status, out = run([], {"a": fake("a", 1, answer=0)})
        self.assertEqual(status, 1)
        self.assertIn("MISMATCH: known answer 0", out)

    def test_errors_and_non_integers_exit_two_without_stopping_other_probes(self):
        registry = {"boom": fake("boom", RuntimeError("no counter")), "float": fake("float", 1.5),
                    "bool": fake("bool", True, answer=1), "ok": fake("ok", 3, answer=3)}
        status, out = run([], registry)
        self.assertEqual(status, 2)
        self.assertIn("ok", out)
        self.assertNotIn("boom ", out)
        self.assertNotIn("float ", out)
        self.assertNotIn("bool ", out)   # True is not the integer 1

    def test_error_outranks_mismatch(self):
        status, _ = run([], {"a": fake("a", 2, answer=1), "b": fake("b", ValueError("x"))})
        self.assertEqual(status, 2)

    def test_runs_only_named_probes_in_the_order_named(self):
        registry = {name: fake(name, i) for i, name in enumerate("abc")}
        status, out = run(["c", "a"], registry)
        self.assertEqual(status, 0)
        self.assertEqual([line.split()[0] for line in out.splitlines()[:2]], ["c", "a"])
        self.assertNotIn("\nb ", out)

    def test_unknown_probe_exits_two_and_runs_nothing(self):
        ran = []
        registry = {"a": probes.Probe("a", "u", None, "", lambda: ran.append(1) or 1)}
        self.assertEqual(run(["a", "nope"], registry)[0], 2)
        self.assertEqual(ran, [])

    def test_json_mode_writes_one_json_object_only(self):
        status, out = run([], {"a": fake("a", 7, answer=7), "b": fake("b", 2, answer=3)}, as_json=True)
        self.assertEqual(status, 1)
        document = json.loads(out)
        self.assertEqual([(p["name"], p["value"], p["answer"], p["matches"]) for p in document["probes"]],
                         [("a", 7, 7, True), ("b", 2, 3, False)])
        self.assertGreaterEqual(document["seconds"], 0)

    def test_instruction_known_answer_is_resolution_bounded_without_sample_selection(self):
        for value in (964, 999, 1000, 1001, 1036):
            self.assertEqual(run([], {"nop": fake("nop", value, 1000, "instructions:u")})[0], 0)
        for value in (0, 1, 963, 1037, 100_000):
            self.assertEqual(run([], {"nop": fake("nop", value, 1000, "instructions:u")})[0], 1)

    def test_qualification_actually_disqualifies_excess_variation(self):
        for spread, expected in ((36, 0), (37, 2)):
            values = iter([1000] * 49 + [1000 + spread])
            entry = probes.Probe("nop", "instructions:u", None, "", lambda: next(values))
            out = io.StringIO()
            self.assertEqual(probes.qualify([], {"nop": entry}, 50, out=out), expected)
            row, = json.loads(out.getvalue())["qualification"]
            self.assertEqual(row["qualified"], expected == 0)
            self.assertEqual(len(row["values"]), 50)
            self.assertNotIn("value", row)

    def test_qualification_never_hides_an_unavailable_reading(self):
        values = iter([1000] * 49 + [RuntimeError("unscheduled")])
        def sample():
            value = next(values)
            if isinstance(value, Exception):
                raise value
            return value
        entry = probes.Probe("nop", "instructions:u", 1000, "", sample)
        out = io.StringIO()
        self.assertEqual(probes.qualify([], {"nop": entry}, 50, out=out), 2)
        row, = json.loads(out.getvalue())["qualification"]
        self.assertFalse(row["qualified"])
        self.assertEqual(row["errors"], ["unscheduled"])
        self.assertEqual(len(row["values"]), 49)

    def test_qualification_does_not_admit_a_constant_broken_counter(self):
        entry = fake("nop", 100_000, 1000, "instructions:u")
        self.assertEqual(probes.qualify([], {"nop": entry}, 50, out=io.StringIO()), 1)

    def test_comparison_differences_within_resolution_are_not_evidence(self):
        for delta in (-37, -36, -1, 0, 1, 36, 37):
            out = io.StringIO()
            probes.run([], {"a": fake("a", 1000 + delta, unit="instructions:u")},
                       as_json=True, out=out, baseline={"a": {"value": 1000, "unit": "instructions:u"}})
            row, = json.loads(out.getvalue())["probes"]
            self.assertEqual(row["delta"], delta)
            self.assertEqual(row["evidence"], abs(delta) > 36)

    def test_registration_is_one_decorator_and_refuses_duplicates(self):
        try:
            @probe_decorator("probe-test-answer", "things", answer=42)
            def answer():
                """The answer."""
                return 42
            entry = probes.PYTHON_PROBES["probe-test-answer"]
            self.assertEqual((entry.unit, entry.answer, entry.doc, entry.run()), ("things", 42, "The answer.", 42))
            with self.assertRaisesRegex(ValueError, "duplicate"):
                probe_decorator("probe-test-answer", "things")(answer)
        finally:
            probes.PYTHON_PROBES.pop("probe-test-answer", None)


probe_decorator = probes.probe


class RustProbeTests(unittest.TestCase):
    """The Rust counters, through the real build and subprocess boundary."""

    @classmethod
    def setUpClass(cls):
        cls.binary = probes.rust_binary()
        cls.registry = probes.rust_probes(cls.binary)

    def value(self, name):
        return self.registry[name].run()

    def test_lists_every_probe_with_its_known_answer(self):
        self.assertEqual({name: entry.answer for name, entry in self.registry.items()},
                         {"counter-nop-1000": 1000, "counter-alloc-3": 3, "text-find-newline-instructions": None,
                          "text-chat-stream-allocs": None, "mem-live-conses": 1000})

    def test_known_answers_check_instruction_sanity_within_resolution_and_other_counts_exactly(self):
        self.assertTrue(probes.matches_answer(self.registry["counter-nop-1000"], self.value("counter-nop-1000")))
        self.assertEqual(self.value("counter-alloc-3"), 3)
        self.assertEqual(self.value("mem-live-conses"), 1000)

    def test_primitive_allocation_count_is_deterministic_across_processes(self):
        values = {self.value("text-chat-stream-allocs") for _ in range(3)}
        self.assertEqual(len(values), 1, values)
        self.assertGreater(values.pop(), 0)

    def test_instruction_probes_disqualify_variation_beyond_resolution(self):
        for entry in self.registry.values():
            if entry.unit == "instructions:u":
                values = [entry.run() for _ in range(50)]
                self.assertTrue(probes.qualifies(entry, values), (entry.name, values))

    def test_competing_inherited_pmu_events_refuse_a_number(self):
        import ctypes
        import os
        import platform
        import struct
        syscall = {"x86_64": 298, "aarch64": 241}[platform.machine()]
        libc = ctypes.CDLL(None, use_errno=True)
        fds = []
        try:
            for _ in range(32):
                attr = ctypes.create_string_buffer(struct.pack(
                    "IIQQQQQIIQ", 0, 64, 1, 0, 0, 0,
                    (1 << 1) | (1 << 5) | (1 << 6), 0, 0, 0))
                fd = libc.syscall(syscall, ctypes.byref(attr), 0, -1, -1, 0)
                if fd < 0:
                    error = ctypes.get_errno()
                    raise OSError(error, os.strerror(error))
                fds.append(fd)
            done = subprocess.run([str(HERE / "probe"), "--json", "text-find-newline-instructions"],
                                  capture_output=True, text=True, pass_fds=fds)
            self.assertEqual(done.returncode, 2, (done.stdout, done.stderr))
            result, = json.loads(done.stdout)["probes"]
            self.assertIn("missing/multiplexed instruction counter", result["error"])
            self.assertNotIn("value", result)
        finally:
            for fd in fds:
                os.close(fd)

    def test_unknown_rust_probe_exits_two(self):
        done = subprocess.run([self.binary, "no-such-probe"], capture_output=True, text=True)
        self.assertEqual(done.returncode, 2)
        self.assertIn("unknown probe", done.stderr)


class CommandLineTests(unittest.TestCase):
    def cli(self, *args):
        return subprocess.run([str(HERE / "probe"), *args], capture_output=True, text=True)

    def test_list_shows_python_and_rust_probes(self):
        done = self.cli("--list")
        self.assertEqual(done.returncode, 0, done.stderr)
        for name in ("counter-nop-1000", "mem-live-conses"):
            self.assertRegex(done.stdout, rf"(?m)^{name} ")

    def test_unknown_probe_is_a_usage_error(self):
        done = self.cli("no-such-probe")
        self.assertEqual(done.returncode, 2)
        self.assertIn("unknown probe", done.stderr)

    def test_json_run_of_named_probes(self):
        done = self.cli("--json", "counter-alloc-3", "mem-live-conses")
        self.assertEqual(done.returncode, 0, done.stderr)
        document = json.loads(done.stdout)
        self.assertEqual([(p["name"], p["value"], p["matches"]) for p in document["probes"]],
                         [("counter-alloc-3", 3, True), ("mem-live-conses", 1000, True)])


if __name__ == "__main__":
    unittest.main()
