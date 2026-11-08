PYTHON ?= python3
MODULE ?=
SELECTOR ?= t
TEST_FILES ?=
TEST_TIMEOUT ?= 55
export MODULE SELECTOR TEST_FILES TEST_TIMEOUT

.PHONY: test-elisp test-elisp-list test-elisp-runner

test-elisp:
	$(PYTHON) scripts/test-ert

test-elisp-%:
	$(MAKE) --no-print-directory test-elisp MODULE=$*

test-elisp-list:
	$(PYTHON) scripts/test-ert --list

test-elisp-runner:
	$(PYTHON) -m unittest discover -s scripts/tests -p test_module_ert.py
