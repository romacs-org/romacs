# The gates.  Rust: gate-small (what changed, and the structural rules), gate-full.
# Lisp: check-MODULE (one package or library), check (all).
#
# The check targets run bundled Lisp packages' ERT suites on a prebuilt host, with
# the targets and variables of GNU's test/Makefile.in.  GNU's own suites run under
# gnu-tests/.
PYTHON ?= python3
BASE ?=
GATE_ARGS ?= $(if $(ROMACS_GNU_ORACLE),--oracle $(ROMACS_GNU_ORACLE))
MODULE ?=
SELECTOR ?=
TEST_FILES ?=
EMACS_TEST_TIMEOUT ?=
JOBS ?=
export MODULE SELECTOR TEST_FILES EMACS_TEST_TIMEOUT JOBS

.PHONY: gate-small gate-full check check-expensive check-all check-list check-runner

gate-small:
	$(PYTHON) scripts/gate $(GATE_ARGS) changed $(BASE)

gate-full:
	$(PYTHON) scripts/gate $(GATE_ARGS) full

check:
	$(PYTHON) scripts/test-ert

check-expensive:
	$(PYTHON) scripts/test-ert --selector '(not (tag :unstable))'

check-all:
	$(PYTHON) scripts/test-ert --selector t

check-list:
	$(PYTHON) scripts/test-ert --list

check-runner:
	$(PYTHON) -m unittest discover -s scripts/tests -p test_module_ert.py
	$(PYTHON) -m unittest discover -s scripts/tests -p test_gate_changed.py

check-%:
	$(PYTHON) scripts/test-ert --module $*
