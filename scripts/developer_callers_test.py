#!/usr/bin/env python3
"""Exercise retained mutation-tool command calls without applying mutations."""
import ast
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
CALLERS = {
    "crates/romacs-vm-gui/harness/prompt_mutants.py":
        ["cargo", "test", "-p", "romacs-vm", "--test", "host_input"],
    "crates/romacs-vm/tools/reverse_mutants.py":
        ["cargo", "test", "-p", "romacs-vm", "--test", "reverse",
         "reverse_matches_pinned_gnu", "--", "--nocapture"],
    "crates/romacs-vm/tools/primh/window_mutants.py":
        ["cargo", "test", "-p", "romacs-vm", "--test", "window_prims",
         "--", "selected_case", "--exact"],
    "crates/romacs-vm/tools/primh/bitwise/mutate.py":
        ["cargo", "test", "-p", "romacs-vm", "--test", "bitwise",
         "bitwise_matches_pinned_gnu", "--", "--nocapture"],
    "crates/romacs-vm/evidence/head-order/mutate.py":
        ["cargo", "test", "-p", "romacs-vm", "--test", "head_order", "--", "--nocapture"],
    "crates/romacs-vm/evidence/file-local-special/mutants.py":
        ["cargo", "test", "-p", "romacs-vm", "--test", "file_local_special"],
}


class DeveloperCallers(unittest.TestCase):
    def test_real_command_calls_reach_cargo_and_preserve_its_result(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cargo = root / "cargo"
            cargo.write_text("#!/usr/bin/env python3\nimport json,os,sys\n"
                             "print(json.dumps(dict(args=sys.argv[1:],cwd=os.getcwd(),"
                             "home=os.environ['HOME'],target=os.environ['CARGO_TARGET_DIR'])))\n"
                             "sys.exit(int(os.environ['TEST_CARGO_STATUS']))\n")
            cargo.chmod(0o755)
            for relative, expected in CALLERS.items():
                tree = ast.parse((ROOT / relative).read_text())
                calls = [node for node in ast.walk(tree) if isinstance(node, ast.Call)
                         and isinstance(node.func, ast.Attribute) and node.func.attr == "run"
                         and isinstance(node.func.value, ast.Name) and node.func.value.id == "subprocess"]
                self.assertEqual(len(calls), 1, relative)
                expression = compile(ast.Expression(calls[0]), relative, "eval")
                namespace = {"ROOT": root, "root": root, "subprocess": subprocess,
                             "binary": "window_prims", "test": "selected_case"}
                if relative.endswith("prompt_mutants.py"):
                    namespace["test"] = expected
                if isinstance(calls[0].args[0], ast.Name) and calls[0].args[0].id == "RUN":
                    value = next(node.value for node in tree.body if isinstance(node, ast.Assign)
                                 and any(isinstance(target, ast.Name) and target.id == "RUN"
                                         for target in node.targets))
                    namespace["RUN"] = eval(compile(ast.Expression(value), relative, "eval"), namespace)
                for status in (0, 7):
                    with self.subTest(caller=relative, status=status), patch.dict(os.environ, {
                            "PATH": str(root) + os.pathsep + os.environ["PATH"],
                            "HOME": str(root / "owner"), "CARGO_TARGET_DIR": str(root / "target"),
                            "TEST_CARGO_STATUS": str(status)}):
                        result = eval(expression, namespace)
                    self.assertEqual(result.returncode, status)
                    record = json.loads(result.stdout)
                    self.assertEqual(record["args"], expected[1:])
                    self.assertEqual(record["cwd"], str(root))
                    self.assertEqual(record["home"], str(root / "owner"))
                    self.assertEqual(record["target"], str(root / "target"))

    def test_face_selector_validation_works_without_lane_environment(self):
        environment = {key: value for key, value in os.environ.items()
                       if key not in ("CARGO_TARGET_DIR", "TRYBUILD")}
        with tempfile.TemporaryDirectory() as directory:
            logs = Path(directory) / "uncreated"
            result = subprocess.run([
                os.sys.executable, str(ROOT / "crates/romacs-face/tests/mutants.py"),
                "--apply", "--logs", str(logs), "--only", "no-such-mutant-control"],
                env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn("unknown mutant", result.stderr)
            self.assertNotIn("lane-env", result.stderr)
            self.assertFalse(logs.exists())


if __name__ == "__main__":
    unittest.main()
