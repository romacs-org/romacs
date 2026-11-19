#!/usr/bin/env python3
"""A startup that fails holds its screen until the window is closed: exit_live.py's
startup-failed and startup-failed-server workflows, as a check runs them
(tools/lib/impact.py maps the startup and exit paths here).

usage: startup_failed_live.py --binary BIN --output DIR [--workflow NAME]...
"""
import sys

import exit_live

WORKFLOWS = ('startup-failed', 'startup-failed-server')

if __name__ == '__main__':
    sys.exit(exit_live.main(default=list(WORKFLOWS)))
