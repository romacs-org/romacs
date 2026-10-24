#!/usr/bin/env python3
"""Selection and failure-boundary tests for the opt-in developer loop."""
import contextlib
import importlib.machinery
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

loader = importlib.machinery.SourceFileLoader("dev_loop", str(Path(__file__).with_name("dev-loop")))
spec = importlib.util.spec_from_loader(loader.name, loader)
if spec is None:
    raise RuntimeError("cannot load dev-loop")
loop = importlib.util.module_from_spec(spec)
loader.exec_module(loop)


def package(name="small", targets=None):
    return {"name": name, "id": name,
            "targets": targets if targets is not None else [{"name": name, "kind": ["lib"], "test": True}]}


class SelectionTests(unittest.TestCase):
    def test_new_targets_are_selected_and_slow_targets_never_reach_cargo(self):
        targets = [{"name": "romacs_vm", "kind": ["lib"]}]
        targets += [{"name": n, "kind": ["test"]} for n in [*loop.VM2_SLOW, "new_regression"]]
        flags, names, excluded = loop.select(package("romacs-vm", targets))
        self.assertEqual(names, ["new_regression"])
        self.assertEqual(flags, ["--lib", "--test", "new_regression"])
        self.assertEqual(set(excluded), set(loop.VM2_SLOW))

    def test_stale_exclusion_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "stale"):
            loop.select(package("romacs-vm"))

    def test_heavy_package_has_no_cheap_claim(self):
        with self.assertRaisesRegex(ValueError, "full/GUI/performance"):
            loop.select(package("romacs-gui-tests"))

    def test_no_testable_target_fails(self):
        with self.assertRaisesRegex(ValueError, "no fast-tier"):
            loop.select(package(targets=[{"name": "small", "kind": ["lib"], "test": False}]))


class RunnerTests(unittest.TestCase):
    def invoke(self, fail_at=None, extra_env=None):
        calls = []
        metadata = {"packages": [package()], "workspace_members": ["small"]}
        def output(command, **kwargs):
            if command[0] == "cargo":
                return json.dumps(metadata).encode()
            if "-vV" in command:
                return "host: x86_64-unknown-linux-gnu\n"
            return "/toolchain\n"
        def run(command, **kwargs):
            calls.append((command, kwargs["env"]))
            if fail_at == len(calls):
                raise subprocess.CalledProcessError(100, command)
        stream = io.StringIO()
        with patch.object(loop.sys, "argv", ["dev-loop", "small"]), \
             patch.dict(loop.os.environ, {"PATH": "/bin", "CARGO_TARGET_DIR": "/wrong-slot", **(extra_env or {})}, clear=True), \
             patch.object(loop.subprocess, "check_output", side_effect=output), \
             patch.object(loop.subprocess, "run", side_effect=run), \
             patch.object(loop.shutil, "which", return_value="/bin/cargo-nextest"), \
             patch.object(loop.Path, "is_file", return_value=True), \
             contextlib.redirect_stdout(stream), contextlib.redirect_stderr(stream):
            result = loop.main()
        return result, calls, stream.getvalue()

    def test_config_reaches_both_cargo_and_nextest_and_slot_is_ignored(self):
        result, calls, text = self.invoke()
        self.assertEqual(result, 0)
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0][0][:3], ["cargo", "check", "--config"])
        self.assertEqual(calls[1][0][:4], ["cargo", "nextest", "run", "--config"])
        self.assertIn("--no-tests=fail", calls[1][0])
        for _, env in calls:
            self.assertEqual(env["CARGO_TARGET_DIR"], str(loop.ROOT / "target"))
            self.assertNotIn("RUSTFLAGS", env)
        self.assertIn("PASS (fast tier only)", text)

    def test_check_failure_prevents_tests(self):
        result, calls, text = self.invoke(fail_at=1)
        self.assertEqual(result, 1)
        self.assertEqual(len(calls), 1)
        self.assertIn("FAIL", text)

    def test_test_failure_is_not_success(self):
        result, calls, text = self.invoke(fail_at=2)
        self.assertEqual(result, 1)
        self.assertEqual(len(calls), 2)
        self.assertIn("FAIL", text)

    def test_rustflag_override_is_refused(self):
        result, calls, text = self.invoke(extra_env={"RUSTFLAGS": "-C opt-level=3"})
        self.assertEqual(result, 1)
        self.assertEqual(calls, [])
        self.assertIn("unset rustflag overrides", text)


if __name__ == "__main__":
    unittest.main()
