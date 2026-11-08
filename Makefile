# Bundled Lisp packages' ERT suites on a prebuilt host, with the targets and
# variables of GNU's test/Makefile.in.  GNU's own suites run under gnu-tests/.
PYTHON ?= python3
MODULE ?=
SELECTOR ?=
TEST_FILES ?=
EMACS_TEST_TIMEOUT ?=
JOBS ?=
export MODULE SELECTOR TEST_FILES EMACS_TEST_TIMEOUT JOBS

.PHONY: check check-expensive check-all check-list check-runner

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

check-%:
	$(PYTHON) scripts/test-ert --module $*
