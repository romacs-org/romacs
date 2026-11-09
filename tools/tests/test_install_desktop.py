"""Checkout desktop installation, destination protection and Exec quoting."""
import contextlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("installer", Path(__file__).with_name("install-desktop.py"))
assert spec is not None and spec.loader is not None
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class DesktopInstallTests(unittest.TestCase):
    def test_install_generates_every_exec_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            prefix = root / 'prefix with spaces $value "quote \\slash'
            applications = root / "applications"
            with contextlib.redirect_stdout(io.StringIO()):
                installer.install(prefix, applications)
            launcher = prefix / "bin/romacs"
            desktop = applications / "romacs.desktop"
            original = desktop.read_bytes()
            self.assertEqual(launcher.resolve(), Path(__file__).resolve().with_name("romacs"))
            entries = [line for line in original.decode().splitlines() if line.startswith("Exec=")]
            command = "Exec=" + installer.exec_quote(launcher)
            self.assertEqual(entries, [command + " %F", command + " --gnu-lisp", command + " --trace-latency"])
            with self.assertRaises(FileExistsError):
                installer.install(prefix, applications)
            self.assertEqual(desktop.read_bytes(), original)

    def test_invalid_exec_prefix_is_refused_before_writing(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ("bad\nline", "bad\rline", "bad%field"):
                with self.subTest(name=name), self.assertRaises(ValueError):
                    installer.install(root / name, root / "applications")
                self.assertFalse((root / name).exists())
            with self.assertRaises(ValueError):
                installer.exec_quote("bad\x00path")


if __name__ == "__main__":
    unittest.main()
