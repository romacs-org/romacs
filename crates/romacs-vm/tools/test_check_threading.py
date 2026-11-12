"""check_threading.py sees the typed handlers' entries and rejects one whose
dispatch is a call, which would keep a native frame per instruction."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_threading  # noqa: E402


def function(name, *lines):
    return f"\n0000000000001000 <romacs_vm::threaded::{name}>:\n" + "\n".join(
        f"  1000:\t00 00\t{line}" for line in lines)


TAIL = ("movzbl 0x7c(%rdx),%eax", "mov    (%rsi,%rcx,8),%rcx", "sub    $0xffffffffffffff80,%rdx", "jmp    *%rax")
CALLED = ("movzbl 0x7c(%rdx),%eax", "call   *%rax", "ret")


class ScanTest(unittest.TestCase):
    def test_a_typed_wrapper_that_tail_jumps_passes(self):
        records, violations = check_threading.scan(function("audited::t_mov", *TAIL))
        self.assertEqual(records, [dict(handler="audited::t_mov", calls=0, indirect_jumps=1)])
        self.assertEqual(violations, [])

    def test_a_typed_wrapper_with_a_non_tail_dispatch_fails(self):
        _, violations = check_threading.scan(function("audited::t_mov", *CALLED))
        self.assertEqual([(v["handler"], v["instruction"].split("\t")[-1]) for v in violations],
                         [("audited::t_mov", "call   *%rax")])

    def test_a_call_into_the_engine_fails_and_a_linkage_stub_passes(self):
        text = (function("audited::legacy", "call   1234 <romacs_vm::threaded::sealed_call>")
                + function("mul", "call   *0x10(%rip)", "jmp    *%rax"))
        _, violations = check_threading.scan(text)
        self.assertEqual([v["handler"] for v in violations], ["audited::legacy"])

    def test_other_crates_and_the_driver_are_not_handlers(self):
        text = (function("execute", *CALLED)
                + "\n0000000000002000 <romacs_vm::gc::collect>:\n  2000:\t00\tcall   *%rax")
        self.assertEqual(check_threading.scan(text), ([], []))

    def test_a_missing_typed_handler_fails(self):
        report = check_threading.check(function("audited::t_mov", *TAIL))
        self.assertFalse(report["passed"])
        self.assertIn("audited::t_jump", report["violations"][-1]["missing"])


if __name__ == "__main__":
    unittest.main()
