#!/usr/bin/env python3
"""perfbench's elisp-benchmarks rows and README tables, without an engine.

Run: python3 -B -m unittest discover -s crates/romacs-vm-gui/harness/perfbench -p 'test_perfbench*.py'
"""
import contextlib
import io
import json
import os
import re
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

import perfbench as pb
import readme


def sample(instructions, seconds, errors=()):
    return {'instructions': instructions, 'seconds': seconds, 'errors': list(errors)}


def member_row(name, gnu, native, romacs, load=1.0):
    made = pb.row(f'elb {name}', 'elb', 'run', gnu, romacs, 3)
    made.update(gnu_native=native, load=load)
    return made


def results(rows, native=True):
    meta = {'started': '20261105T180000Z', 'commit': 'a' * 40, 'fixtures': {'commit': 'b' * 40}, 'runs': 3,
            'max_load': 8.0, 'elb_native': native}
    return {'meta': meta, 'skipped': [], 'rows': pb.with_ratios(rows, None)}


class Members(unittest.TestCase):
    def test_reported_errors_are_found_and_ordinary_output_is_not(self):
        output = ('Window size: 23 x 80\nIn toplevel form:\nelb-smie.el:12:1: Warning: unused\n'
                  "benchmarks/elb-smie.el: Error: Symbol's function definition is void: read-positioning-symbols\n"
                  'Error running: (wrong-type-argument)\n')
        self.assertEqual(pb.ELB_ERROR.findall(output),
                         ["Error: Symbol's function definition is void: read-positioning-symbols",
                          'Error running: (wrong-type-argument)'])
        self.assertEqual(pb.ELB_ERROR.findall('Window size: 23 x 80\nPERFBENCH (:workload elb)\n'), [])

    def test_a_timeout_names_no_directories(self):
        with self.assertRaises(pb.Refused) as refused:
            pb.run(['sh', '-c', 'sleep 5', '/home/Some User/tree/perfbench.el'], None, None, .1)
        self.assertNotIn('Some User', str(refused.exception))
        self.assertIn('perfbench.el', str(refused.exception))

    def test_a_member_is_its_medians_less_the_setup(self):
        got = pb.elb_member([sample(110, 2.0), sample(130, 1.0), sample(120, 3.0)], 100)
        self.assertEqual((got['instructions'], got['wall']), (20, 2.0))

    def test_a_count_not_above_the_setup_has_no_instructions_figure(self):
        got = pb.elb_member([sample(90, 1e-5)], 100)
        self.assertEqual((got['instructions'], got['wall']), (None, 1e-5))
        self.assertIn('-10', pb.elb_note({'gnu_native': got}))

    def test_a_missing_or_erring_member_has_no_figures_and_says_why(self):
        missing = pb.elb_member('no elb-scroll-entry after loading the suite', 100)
        erring = pb.elb_member([sample(110, .1), sample(110, .1, ['x.el: Error: void'])], 100)
        for got in (missing, erring):
            self.assertEqual((got['instructions'], got['wall']), (None, None))
        self.assertIn('Error: void', erring['failed'])
        self.assertEqual(pb.elb_note({'gnu': {'wall': 1}, 'romacs': missing}),
                         'Romacs: no elb-scroll-entry after loading the suite')

    def test_an_engine_refusing_the_suite_leaves_the_others_their_rows(self):
        class Batch:
            def once(self, engine, workload, skip=False, extra=None):
                if engine == 'romacs':
                    raise pb.Refused('romacs: no result from elb-prepare')
                return {'check': ['fibn'], 'instructions': 100}
        gnu, romacs = (pb.elb_column(Batch(), engine, {}, 3) for engine in ('gnu', 'romacs'))
        self.assertEqual((gnu['entries'], gnu['setup']), (['fibn'], 100))
        got = pb.elb_member(pb.elb_samples(None, Batch(), romacs, 'fibn'), romacs.get('setup'))
        self.assertEqual((got['instructions'], got['wall']), (None, None))
        self.assertIn('no result from elb-prepare', got['failed'])

    def test_the_geometric_mean_names_the_members_it_leaves_out(self):
        rows = [member_row('a', {'instructions': 100, 'wall': 1.0}, {'instructions': 50, 'wall': .5},
                           {'instructions': 25, 'wall': 4.0}),
                member_row('b', {'instructions': 400, 'wall': 4.0}, {'instructions': 200, 'wall': 2.0},
                           {'instructions': 100, 'wall': 1.0}, load=9.0),
                member_row('fibn', {'instructions': 100, 'wall': 1.0}, {'instructions': None, 'wall': 1e-5},
                           {'instructions': 100, 'wall': 1.0}),
                member_row('scroll', {'instructions': 9, 'wall': 9.0}, {'instructions': 9, 'wall': 9.0},
                           {'instructions': None, 'wall': None, 'failed': 'no entry'})]
        mean = pb.elb_geomean_row(rows, ['gnu', 'gnu_native', 'romacs', 'neomacs'], 3)
        self.assertAlmostEqual(mean['gnu']['wall'], 4 ** (1 / 3))
        self.assertAlmostEqual(mean['romacs']['wall'], 4 ** (1 / 3))
        self.assertNotIn('gnu_native', mean)  # the median covers that column
        self.assertIn('against GNU .elc', mean['note'])
        self.assertIn('3 of 4', mean['note'])
        self.assertIn('left out: scroll', mean['note'])
        self.assertEqual(mean['load'], 9.0)

    def test_load_refuses_the_native_wall_too(self):
        made = member_row('a', {'wall': 1.0}, {'wall': .5}, {'wall': 2.0}, load=12)
        pb.gate_walls([made], 8)
        self.assertEqual([made[c].get('wall') for c in ('gnu', 'gnu_native', 'romacs')], [None] * 3)
        self.assertEqual(made['gnu_native']['wall_refused'], .5)

    def test_ratios_are_romacs_over_each_gnu(self):
        made = pb.with_ratios([member_row('a', {'wall': 1.0}, {'wall': .5}, {'wall': 2.0})], None)[0]
        self.assertEqual((made['wall_ratio'], made['native_wall_ratio']), (2.0, 4.0))


class Memory(unittest.TestCase):
    def test_a_limited_run_cannot_allocate_past_its_limit(self):
        grow = ['python3', '-c', 'x = bytearray(2 << 30); print("allocated")']
        output, _, _ = pb.run(grow, dict(os.environ), None, 60, memory=1 << 30)
        self.assertIn('MemoryError', output)
        self.assertIn('allocated', pb.run(grow, dict(os.environ), None, 60)[0])


    def test_neomacs_compiling_a_workload_file_is_limited_too(self):
        seen = []
        self.addCleanup(setattr, pb, 'run', pb.run)
        pb.run = lambda *a, **k: seen.append(k.get('memory')) or ('', 0, 0)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'm.el'
            source.write_text('')
            ctx = type('Ctx', (), {'work': Path(directory), 'engines': {'neomacs': 'n'}, 'memory': {'neomacs': 7}})
            pb.neomacs_compiled(ctx, type('Batch', (), {'env': {'neomacs': {}}}), source)
        self.assertEqual(seen, [7])


class GnuEngine(unittest.TestCase):
    def fake(self, directory, commit):
        path = Path(directory) / 'emacs'
        path.write_text(f'#!/bin/sh\necho \'noise\'\necho \'("32.0.50" {commit} t)\'\n')
        path.chmod(0o755)
        return str(path)

    def test_the_build_records_what_an_upgrade_would_move(self):
        self.addCleanup(setattr, pb, 'package_of', pb.package_of)
        pb.package_of = lambda _: 'emacs-x 1-1'
        with tempfile.TemporaryDirectory() as directory:
            build = pb.gnu_build(self.fake(directory, '"abc"'))
        self.assertEqual((build['version'], build['commit'], build['native'], build['package']),
                         ('32.0.50', 'abc', True, 'emacs-x 1-1'))
        self.assertEqual(len(build['sha256']), 64)

    def test_a_build_with_no_commit_or_package_is_still_read(self):
        self.addCleanup(setattr, pb, 'package_of', pb.package_of)
        pb.package_of = lambda _: None
        with tempfile.TemporaryDirectory() as directory:
            build = pb.gnu_build(self.fake(directory, 'nil'))
        self.assertIsNone(build['commit'])
        meta = {'gnu_build': build, 'fixtures': {'commit': 'b' * 40}}
        self.assertIn('the installed GNU Emacs 32.0.50, natively compiled', readme.gnu_engine(meta))

    def test_the_header_names_engine_commit_package_and_the_pinned_inputs(self):
        meta = {'gnu_build': {'version': '32.0.50', 'commit': 'c' * 40, 'package': 'emacs-x 1-1', 'native': True},
                'fixtures': {'commit': 'b' * 40}}
        text = readme.gnu_engine(meta)
        for part in ('32.0.50', '`cccccccc`', 'emacs-x 1-1', 'natively compiled', 'inputs are pinned at `bbbbbbbb`'):
            self.assertIn(part, text)

    def test_a_run_from_before_keeps_its_pinned_wording(self):
        self.assertEqual(readme.gnu_engine({'fixtures': {'commit': 'b' * 40}}), 'pinned GNU Emacs `bbbbbbbb`')

    def test_a_missing_package_manager_is_no_failure(self):
        self.addCleanup(setattr, pb.subprocess, 'run', pb.subprocess.run)

        def absent(*_, **__):
            raise FileNotFoundError('pacman')
        pb.subprocess.run = absent
        self.assertIsNone(pb.package_of('/usr/bin/emacs'))


class Session(unittest.TestCase):
    def test_a_row_that_fails_costs_that_row_and_leaves_the_session_usable(self):
        sent = []
        session = type('S', (), {'engine': type('E', (), {'keys': staticmethod(sent.append), 'name': 'gnu'}),
                                 'stopped': None, 'ask': staticmethod(lambda *_: True)})

        def fail():
            raise TimeoutError('no result')
        got = pb.session_row(session, 'session x', fail)
        self.assertEqual((got['wall'], got['block']), (None, None))
        self.assertIn('no result', got['failed'])
        self.assertEqual(len(sent), 3)
        self.assertIsNone(session.stopped)

    def test_a_failed_row_runs_its_cleanup(self):
        asked = []
        session = type('S', (), {'engine': type('E', (), {'keys': staticmethod(lambda _: None), 'name': 'gnu'}),
                                 'stopped': None, 'ask': staticmethod(lambda form, *_: asked.append(form))})

        def fail():
            raise TimeoutError('no result')
        pb.session_row(session, 'session M-x shell', fail, 'shell')
        self.assertEqual(asked, ['(perfbench-session-abort (quote shell))'])

    def test_a_dead_engine_stops_the_session_without_losing_the_group(self):
        def dead(*_):
            raise RuntimeError('editor exited')
        session = type('S', (), {'engine': type('E', (), {'keys': staticmethod(lambda _: None), 'name': 'romacs'}),
                                 'stopped': None, 'ask': staticmethod(dead)})
        got = pb.session_row(session, 'session x', dead)
        self.assertIn('editor exited', got['failed'])
        self.assertIn('stopped answering in x', session.stopped)

    def test_a_refusal_and_a_lisp_error_are_a_row_failing_not_the_group(self):
        session = type('S', (), {'engine': type('E', (), {'keys': staticmethod(lambda _: None), 'name': 'gnu'}),
                                 'stopped': None, 'ask': staticmethod(lambda *_: None)})
        for error in (pb.Refused('rounds differ'), pb.parity_live.LispError('gnu', 'form', 'void')):
            def fail():
                raise error
            self.assertIn('not run', pb.session_row(session, 'session x', fail)['failed'])

    def test_rounds_that_made_different_things_are_no_sample(self):
        samples = [{'wall': 1.0, 'block': 1.0, 'instructions': 1}] * 3
        self.assertEqual(pb.session_medians(samples, [2000, 2000, 2000])['check'], 2000)
        with self.assertRaises(pb.Refused):
            pb.session_medians(samples, [0, 0, 2000])

    def test_a_thread_name_cut_mid_character_is_read(self):
        session = pb.Session.__new__(pb.Session)
        session.ask = lambda *_: None
        session.engine = type('E', (), {'process': type('P', (), {'pid': 1})})
        real = pb.subprocess.Popen

        def popen(argv, **kwargs):
            line = rb"import sys,time; sys.stderr.buffer.write(b'caf\xd2-1,12,,instructions:u,5,100.00,,\n'); time.sleep(5)"
            return real(['python3', '-c', line.decode()], **kwargs)
        self.addCleanup(setattr, pb.subprocess, 'Popen', real)
        pb.subprocess.Popen = popen
        instructions, _ = session.counted(lambda: None, 1)
        self.assertEqual(instructions, 12)

    def test_a_failed_act_still_stops_the_counter(self):
        real = pb.subprocess.Popen
        started = []

        def popen(*args, **kwargs):
            started.append(real(['sleep', '30'], stderr=subprocess.PIPE, text=True))
            return started[-1]
        self.addCleanup(setattr, pb.subprocess, 'Popen', real)
        pb.subprocess.Popen = popen
        session = pb.Session.__new__(pb.Session)
        session.ask = lambda *_: None
        session.engine = type('E', (), {'process': type('P', (), {'pid': 1})})

        def fail():
            raise TimeoutError('no result')
        with self.assertRaises(TimeoutError):
            session.counted(fail, 1)
        self.assertIsNotNone(started[0].poll())

    def test_a_session_that_stops_answering_costs_the_rows_after_it_at_once(self):
        def silent(*_):
            raise TimeoutError('no answer')
        session = type('S', (), {'engine': type('E', (), {'keys': staticmethod(lambda _: None), 'name': 'romacs'}),
                                 'stopped': None, 'ask': staticmethod(silent)})
        pb.session_row(session, 'session x', silent)
        self.assertIn('stopped answering in x', session.stopped)
        ran = []
        got = pb.session_row(session, 'session y', lambda: ran.append(1))
        self.assertEqual(ran, [])
        self.assertIn('stopped answering in x', got['failed'])

    def test_load_refuses_block_with_wall(self):
        made = pb.row('session x', 'session', 'run', {'wall': 1.0, 'block': .5}, {'wall': 2.0, 'block': .1}, 3)
        made['load'] = 12
        pb.gate_walls([made], 8)
        self.assertIsNone(made['romacs'].get('block'))
        self.assertEqual(made['romacs']['block_refused'], .1)

    def rows(self):
        made = [pb.row('session vc-dir', 'session', 'run', {'wall': 1.0, 'block': 1.0, 'instructions': 100},
                       {'wall': .5, 'block': .1, 'instructions': 500}, 3)]
        return results(made)

    def test_a_block_ratio_is_given(self):
        self.assertEqual(self.rows()['rows'][0]['block_ratio'], .1)

    def test_a_session_win_is_read_by_wall_and_block_not_instructions(self):
        got = list(readme.wins(self.rows()))
        self.assertEqual((got[0][1], got[0][2], got[0][3], got[0][4]), ('GNU', 5.0, .5, .1))

    def test_a_session_row_slower_on_wall_is_no_win_whatever_its_block(self):
        made = results([pb.row('session vc-dir', 'session', 'run', {'wall': 1.0, 'block': 1.0}, {'wall': 2.0, 'block': .1}, 3)])
        self.assertEqual(list(readme.wins(made)), [])

    def test_its_table_is_its_own(self):
        made = self.rows()
        made['rows'].append(pb.row('startup', 'batch', 'run', {'wall': 1.0}, {'wall': 1.0}, 3))
        pb.with_ratios(made['rows'], None)
        self.assertNotIn('vc-dir', readme.section(made))
        text = readme.session_section(made)
        self.assertIn('| vc-dir | run |', text)
        self.assertNotIn('startup', text)
        self.assertEqual(readme.selector(made['rows'][0]), 'session')

    def test_every_key_row_and_command_has_a_distinct_name(self):
        names = [c[0] for c in pb.SESSION_COMMANDS] + [k[0] for k in pb.SESSION_KEYS]
        self.assertEqual(len(names), len(set(names)))
        self.assertTrue(all(n.startswith('session ') for n in names))


class Source(unittest.TestCase):
    def test_no_function_is_defined_twice(self):
        # A later definition silently replaces an earlier one, and only a live run shows it.
        names = re.findall(r'^def (\w+)\(', Path(pb.__file__).read_text(), re.M)
        self.assertEqual(sorted({n for n in names if names.count(n) > 1}), [])


def lead_results(**overrides):
    def made(name, gnu, romacs):
        return pb.row(name, 'session', 'run', gnu, romacs, 3)
    rows = [made(readme.VC_HISTORY, {'wall': 19.0, 'check': '183574 abc'}, {'wall': 3.0, 'key': .005, 'check': '183574 abc'}),
            made(readme.OPEN_XDISP, {'wall': .65}, {'wall': .77}),
            made(readme.OPEN_XDISP_TS, {'wall': .4}, {'wall': .4}),
            made(readme.OPEN_HEADER_TS, {'wall': 2.0}, {'wall': 2.0}),
            *[made(row, {'wall': 1e-7}, {'wall': 2e-8}) for row in readme.UNCOMPILED.values()],
            made(readme.OPEN_HEADER, {'wall': 2.0}, {'wall': 1.0}),
            made(readme.THEME, {'wall': .05}, {'wall': .06})]
    made_results = results(rows)
    made_results['meta'].update(header_bytes=15_600_000, history_commits=183574)
    for name, change in overrides.items():
        change(made_results)
    return made_results


class Lead(unittest.TestCase):
    def test_every_name_in_the_template_is_filled_and_none_is_left(self):
        text = readme.lead_section(lead_results())
        self.assertNotIn('{{', text)
        for part in ('3 s', '19 s', '183,574', '5 ms', '650 ms', '770 ms', '16 MB', '50 ms', '60 ms'):
            self.assertIn(part, text)

    def test_the_template_keeps_the_names_of_the_draft(self):
        names = set(re.findall(r'\{\{([^{}]+)\}\}', readme.LEAD.read_text()))
        self.assertEqual(names, set(readme.lead_values(lead_results())))

    def test_a_value_the_run_lacks_stops_it_and_says_which(self):
        def refuse(made):
            made['rows'][1]['romacs']['wall_refused'] = made['rows'][1]['romacs'].pop('wall')
        made = lead_results(x=refuse)
        self.assertEqual(readme.lead_missing(made), ['open-xdisp-romacs'])
        with self.assertRaises(SystemExit) as stopped:
            readme.lead_section(made)
        self.assertIn('open-xdisp-romacs', str(stopped.exception))

    def test_a_refused_value_the_lead_does_not_quote_is_no_failure(self):
        def refuse(made):
            row = pb.row('session vc-dir', 'session', 'run', {'wall_refused': 1.0}, {'wall_refused': 1.0}, 3)
            made['rows'].append(row)
            # Each of these rows' other metrics is quoted by nothing either.
            made['rows'][0]['romacs']['block_refused'] = 1.0
            made['rows'][3]['gnu']['first_refused'] = 1.0
        self.assertEqual(readme.lead_missing(lead_results(x=refuse)), [])

    def test_the_history_row_is_upstream_masters_not_the_pins(self):
        self.assertNotEqual(pb.UPSTREAM, pb.PIN)
        self.assertEqual(len(pb.UPSTREAM), 40)

    def test_a_name_the_filler_does_not_know_is_a_failure_not_a_blank(self):
        self.assertEqual(readme.lead_missing(lead_results(), 'a {{no-such-number}} b'), ['no-such-number'])

    def test_a_log_that_differs_from_gnus_gives_no_commit_count(self):
        def partial(made):
            made['rows'][0]['romacs']['check'] = '2000 def'
        self.assertIn('vc-history-commits', readme.lead_missing(lead_results(x=partial)))

    def test_a_log_short_of_gits_count_is_no_commit_count_though_both_engines_agree(self):
        def short(made):
            made['meta']['history_commits'] = 194171
        self.assertIn('vc-history-commits', readme.lead_missing(lead_results(x=short)))

    def test_a_run_with_session_selected_checks_the_lead_and_quick_does_not(self):
        args = lambda **k: type('Args', (), {'only': None, 'quick': False, **k})  # noqa: E731
        self.assertTrue(pb.selected(args(), 'session', 'session', False))
        self.assertTrue(pb.selected(args(only=['session']), 'session', 'session', False))
        self.assertFalse(pb.selected(args(quick=True), 'session', 'session', False))
        self.assertFalse(pb.selected(args(only=['elb']), 'session', 'session', False))
        self.assertEqual(len(readme.lead_missing(results([]))), len(readme.lead_values(results([]))))

    def test_a_failed_lead_leaves_the_readme_alone(self):
        def refuse(made):
            made['rows'][0]['gnu'].pop('wall')
        with tempfile.TemporaryDirectory() as directory:
            path, data = Path(directory) / 'README.md', Path(directory) / 'results.json'
            original = ('<!-- perfbench-lead-begin -->\nOLD\n<!-- perfbench-lead-end -->\n<!-- perfbench-session-begin -->\nOLD\n'
                        '<!-- perfbench-session-end -->\n')
            path.write_text(original)
            data.write_text(json.dumps(lead_results(x=refuse)))
            with self.assertRaises(SystemExit):
                readme.main([str(data), str(path)])
            self.assertEqual(path.read_text(), original)

    def test_a_run_that_measured_none_of_it_keeps_the_block(self):
        made = results([pb.row('startup', 'batch', 'run', {'wall': 1.0}, {'wall': 1.0}, 3)])
        self.assertFalse(readme.lead_measured(made))
        self.assertTrue(readme.lead_measured(lead_results()))

    def test_load_refuses_first_shown_and_key_with_the_wall(self):
        made = pb.row(readme.VC_HISTORY, 'session', 'run', {'wall': 1.0}, {'wall': 1.0, 'first': .1, 'key': .005}, 3)
        made['load'] = 12
        pb.gate_walls([made], 8)
        self.assertEqual((made['romacs'].get('first'), made['romacs'].get('key')), (None, None))

    def test_a_session_note_gives_first_shown_and_median_key(self):
        made = pb.row(readme.VC_HISTORY, 'session', 'run', {'first': .2, 'key': .02}, {'first': .1, 'key': .005}, 3)
        self.assertEqual(readme.session_extras(made), 'first shown GNU 200 ms, Romacs 100 ms; median key GNU 20 ms, Romacs 5 ms')
        self.assertEqual(readme.session_extras(pb.row('r', 'session', 'run', {}, {}, 3)), '')
        moded = pb.row('r', 'session', 'run', {'detail': 'c-mode'}, {'detail': 'c-ts-mode'}, 3)
        self.assertEqual(readme.session_extras(moded), 'mode GNU c-mode, Romacs c-ts-mode')

    def test_the_generated_header_is_the_same_every_time_and_about_its_size(self):
        text = pb.generated_header(1200)
        self.assertEqual(text, pb.generated_header(1200))
        self.assertEqual(text.count('\n'), 1200)
        self.assertIn('#define REG_000000_CNTL__FIELD_01_MASK', text)
        self.assertGreater(len(pb.generated_header(pb.HEADER_LINES)), 13_000_000)

    def test_each_engine_has_themes_enough_for_its_rows(self):
        for cycle, visits in pb.SESSION_THEMES.values():
            self.assertGreaterEqual(len(cycle), 2)
            self.assertGreaterEqual(len(visits), 3)
            self.assertEqual(set(cycle) & set(visits), set())


class Markdown(unittest.TestCase):
    def test_a_table_head_has_a_rule_as_wide_as_itself(self):
        self.assertEqual(readme.header('| a | b |'), ['| a | b |', '| --- | --- |'])

    def test_every_table_row_is_as_wide_as_its_head(self):
        rows = [pb.row('startup', 'batch', 'run', {'wall': 1.0}, {'wall': 1.0}, 3, 'a | b'),
                pb.row('session x', 'session', 'run', {'wall': 1.0}, {'wall': 1.0}, 3, 'c | d'),
                member_row('fibn', {'wall': 1.0}, {'wall': 1.0}, {'wall': 1.0})]
        rows[2]['note'] = 'e | f'
        made = results(rows)
        for text in (readme.section(made), readme.session_section(made), readme.elb_section(made)):
            table = [re.split(r'(?<!\\)\|', line) for line in text.splitlines() if line.startswith('|')]
            self.assertGreater(len(table), 2)
            self.assertEqual({len(line) for line in table}, {len(table[0])}, text)

    def test_no_org_markup_is_left_in_what_it_writes(self):
        made = lead_results()
        made['meta'].update(gnu_build={'version': '32', 'commit': 'c' * 40, 'native': True},
                            install={'files': 100, 'not_compiled': 3})
        for text in (readme.section(made), readme.session_section(made), readme.elb_section(made), readme.headlines(made),
                     readme.lead_section(made)):
            self.assertNotRegex(text, r'(?m)~[0-9a-f]{8}~|^\|-$|^#\+|(?<![\w`])=[-.a-z]+=')

    def test_a_block_marker_is_a_comment_a_page_does_not_show(self):
        for begin, end in readme.BLOCKS.values():
            self.assertRegex(begin, r'^<!-- perfbench-[a-z]+-begin -->$')
            self.assertRegex(end, r'^<!-- perfbench-[a-z]+-end -->$')

    def test_the_lead_keeps_the_hero_clip_under_its_title(self):
        lines = [line for line in readme.LEAD.read_text().splitlines() if line]
        self.assertEqual(lines[:2], ['# Romacs', '<!-- hero-clip -->'])


class Ratio(unittest.TestCase):
    def test_a_count_swallowed_by_its_floor_has_no_ratio(self):
        self.assertEqual(pb.ratio(1, 2), .5)
        for romacs, gnu in ((-1, 2), (1, -2), (-1, -2), (0, 2), (1, 0), (None, 2), (1, None)):
            self.assertIsNone(pb.ratio(romacs, gnu))


class Medians(unittest.TestCase):
    def rows(self):
        made = [member_row(name, {'instructions': 100, 'wall': 1.0}, {'instructions': native, 'wall': 1.0},
                           {'instructions': 200, 'wall': 2.0}) for name, native in (('a', 100), ('b', 50), ('c', 1))]
        return made + [pb.elb_median_row(made, 3)]

    def test_a_near_zero_member_does_not_decide_the_native_summary(self):
        rows = pb.with_ratios(self.rows(), None)
        summary = rows[-1]
        self.assertEqual(summary['native_instructions_ratio'], 4.0)  # of 2, 4 and 200
        self.assertEqual(summary['instructions_ratio'], 2.0)
        self.assertIn('median', summary['note'])
        self.assertIn('GNU native 3', summary['note'])

    def test_a_ratio_no_member_has_has_no_median(self):
        self.assertIsNone(pb.with_ratios(self.rows(), None)[-1]['neomacs_instructions_ratio'])


class Engine(unittest.TestCase):
    def test_a_failed_start_leaves_nothing_running(self):
        class Failing(pb.parity_live.Engine):
            def launch(self, tools):
                self.processes.append(subprocess.Popen(['sleep', '60']))
                raise TimeoutError('no init')
        with tempfile.TemporaryDirectory() as directory:
            engine = Failing('neomacs', [], Path(directory), None, {}, 1)
            with self.assertRaises(TimeoutError):
                engine.start('tools')
        self.assertIsNotNone(engine.processes[0].poll())


class Neomacs(unittest.TestCase):
    def test_its_subprocess_failures_stay_its_own(self):
        ctx = pb.Context.__new__(pb.Context)
        ctx.log = lambda *_: None
        for error in (subprocess.TimeoutExpired('x', 1), subprocess.CalledProcessError(1, 'x'), pb.Refused('x')):
            def fail():
                raise error
            self.assertIn('not run', ctx.attempt('neomacs', fail)['failed'])
            with self.assertRaises(type(error)):
                ctx.attempt('romacs', fail)

    def test_its_failed_compile_keeps_the_other_engines_micro_rows(self):
        class Ctx(pb.Context):
            def __init__(self, directory):
                self.quick, self.runs, self.cache, self.store = False, 1, Path(directory), None
                self.gnu_build = {'sha256': 'f' * 64}
                self.engines = {'gnu': 'g', 'romacs': 'r', 'neomacs': 'n'}
                self.compared = tuple(self.engines)
                self.log = lambda *_: None

        class Batch:
            env = {'gnu': {}}
            once = staticmethod(lambda engine, *_, **__: {'instructions': 10 ** 7, 'seconds': 1.0, 'loaded': [],
                                                          'compiled': set(), 'published': 0})
        self.addCleanup(setattr, pb, 'run', pb.run)
        self.addCleanup(setattr, pb, 'neomacs_compiled', pb.neomacs_compiled)
        pb.run = lambda *_, **__: ('', 0, 0)

        def refused(*_):
            raise pb.Refused('compile timed out')
        pb.neomacs_compiled = refused
        with tempfile.TemporaryDirectory() as directory:
            rows = pb.micro_rows(Ctx(directory), Batch)
        self.assertEqual(len(rows), len(pb.MICRO) - 1 + len(pb.UNCOMPILED))
        self.assertIsNotNone(rows[0]['gnu']['instructions'])
        self.assertIn('not run: compile timed out', rows[0]['neomacs']['failed'])

    def test_a_row_it_cannot_run_says_why_and_keeps_its_row(self):
        made = pb.row('r', 'batch', 'run', {'wall': 1.0, 'check': 3}, {'wall': 1.0, 'check': 3}, 3, 'n')
        pb.with_neomacs(made, {'gnu': made['gnu'], 'neomacs': {'instructions': None, 'wall': None, 'failed': 'not run: x'}})
        self.assertIn('neomacs: not run: x', made['note'])
        self.assertIsNone(pb.with_ratios([made], None)[0]['neomacs_wall_ratio'])

    def test_a_different_result_is_said(self):
        made = pb.row('r', 'batch', 'run', {'check': 3}, {'check': 3}, 3, 'n')
        pb.with_neomacs(made, {'gnu': made['gnu'], 'neomacs': {'check': 4}})
        self.assertIn('neomacs result differs: 4', made['note'])

    def test_the_same_result_says_nothing_and_no_run_changes_nothing(self):
        made = pb.row('r', 'batch', 'run', {'check': 3}, {'check': 3}, 3, 'n')
        pb.with_neomacs(made, {'gnu': made['gnu'], 'neomacs': {'check': 3}})
        self.assertEqual(made['note'], 'n')
        other = pb.row('r', 'batch', 'run', {}, {}, 3, 'n')
        self.assertNotIn('neomacs', pb.with_neomacs(other, {'gnu': {}}))

    def test_the_build_names_the_program_a_wrapper_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            program = Path(directory) / '.neomacs-wrapped'
            program.write_text('#!/bin/sh\necho Neomacs 1.2\n')
            program.chmod(0o755)
            wrapper = Path(directory) / 'neomacs'
            wrapper.write_text(f'#!/bin/sh\nexec -a "$0" "{program}" "$@"\n')
            wrapper.chmod(0o755)
            args = type('Args', (), {'neomacs': str(wrapper), 'neomacs_commit': 'abc'})
            build = pb.neomacs_build(args)
        self.assertEqual((build['program'], build['version'], build['commit']), (str(program), 'Neomacs 1.2', 'abc'))


class World(unittest.TestCase):
    def batch(self, boots):
        batch = pb.Batch.__new__(pb.Batch)
        batch.ctx, batch.env, batch.world_ceiling = None, {'romacs': {}}, 100
        self.addCleanup(setattr, pb, 'boot_instructions', pb.boot_instructions)
        self.addCleanup(setattr, pb, 'wait_for_world', pb.wait_for_world)
        pb.boot_instructions = lambda *_: boots
        self.waited = []
        pb.wait_for_world = lambda *a: self.waited.append(a)
        return batch

    def test_a_run_that_left_the_world_stale_is_refused_after_it_returns(self):
        with self.assertRaises(pb.Refused):
            self.batch(10 ** 6).check_world('romacs')
        self.assertEqual(len(self.waited), 1)

    def test_a_held_world_and_other_engines_pass(self):
        self.batch(50).check_world('romacs')
        self.batch(10 ** 6).check_world('neomacs')
        self.assertEqual(self.waited, [])


class Batch(unittest.TestCase):
    def workload(self, gnu, romacs):
        batch = pb.Batch.__new__(pb.Batch)
        ctx = type('Ctx', (), {'runs': 3, 'compared': ('gnu', 'romacs'), 'attempt': staticmethod(lambda _, f: f()),
                               'romacs_note': staticmethod(lambda _: 'Romacs: source')})
        batch.ctx = ctx
        batch.figures = lambda engine, _: {'gnu': gnu, 'romacs': romacs}[engine]
        return batch.workload('r', 'w')

    def test_a_count_of_zero_or_less_has_no_figure_and_says_so(self):
        for count in (0, -5):
            made = self.workload({'instructions': 100, 'wall': 1.0, 'check': 1, 'samples': []},
                                 {'instructions': count, 'wall': 1.0, 'check': 1, 'samples': []})
            self.assertIsNone(made['romacs']['instructions'])
            self.assertIn('not above the setup run', made['note'])


class FirstUse(unittest.TestCase):
    def test_what_a_first_use_compiled_is_said_and_is_outside_the_figures(self):
        batch = pb.Batch.__new__(pb.Batch)
        batch.ctx = type('Ctx', (), {'runs': 1, 'store': Path('/s')})
        made = []

        def once(engine, workload, skip=False, extra=None):
            made.append(skip)
            first = len(made) == 1
            return {'instructions': 10 ** 9 if first else 100 if not skip else 40, 'seconds': 1.0, 'check': 1,
                    'published': 25 if first else 0, 'loaded': [], 'compiled': set()}
        batch.once = once
        figures = batch.figures('romacs', 'eshell-commands')
        self.assertEqual((figures['instructions'], figures['first_use']), (60, 25))
        made.clear()
        self.assertEqual(batch.figures('gnu', 'eshell-commands')['first_use'], 0)
        self.assertEqual(len(made), 2)


class Ert(unittest.TestCase):
    def test_a_refused_file_costs_its_result_once_not_the_row(self):
        calls = []

        def refused(argv, *rest):
            calls.append(argv)
            raise pb.Refused('timed out')
        self.addCleanup(setattr, pb, 'run', pb.run)
        pb.run = refused

        class Ctx:
            fixtures, engines, env, memory = Path('/nonexistent'), {'neomacs': 'n'}, {}, {}
            scratch = staticmethod(lambda suffix: Path('/nonexistent/x'))
            store_marks = staticmethod(dict)
            log = staticmethod(lambda *_: None)

        class Batch:
            env = {'neomacs': {}}
        hung = set()
        first = pb.ert_pass(Ctx, Batch, 'neomacs', hung)
        second = pb.ert_pass(Ctx, Batch, 'neomacs', hung)
        self.assertEqual(first['failed'], pb.ERT_FILES)
        self.assertEqual(second['failed'], pb.ERT_FILES)
        self.assertEqual(len(calls), len(pb.ERT_FILES))


class Table(unittest.TestCase):
    def test_the_saved_table_keeps_the_members_apart_and_says_how_romacs_ran(self):
        rows = [pb.row('startup -Q --batch', 'batch', 'run', {'wall': .05}, {'wall': .1}, 3),
                member_row('fibn', {'wall': 2.0}, {'wall': 1.0}, {'wall': 1.0})]
        made = results(rows)
        made['meta'].update(romacs='r', romacs_sha256='0' * 64, runtime='t', gnu='g', quick=False, load_start=1.0,
                            load_end=1.0, previous=None, romacs_lisp='compiled store')
        text = pb.table(made)
        self.assertLess(text.index('startup -Q'), text.index('GNU ELPA elisp-benchmarks'))
        self.assertLess(text.index('GNU ELPA elisp-benchmarks'), text.index('| elb fibn'))
        self.assertIn('Romacs compiled store.', text)


class Store(unittest.TestCase):
    def test_a_load_no_artifact_ran_is_named(self):
        samples = [{'loaded': ['/r/gnu-lisp/org.el', '/r/gnu-lisp/ox.el'], 'compiled': {'/r/gnu-lisp/org.el'}, 'published': 0},
                   {'loaded': ['/r/gnu-lisp/org.el', '/r/gnu-lisp/ox.el'],
                    'compiled': {'/r/gnu-lisp/org.el', '/r/gnu-lisp/ox.el'}, 'published': 0}]
        self.assertEqual(pb.store_note(samples), 'Romacs: compiled store, 1 of 2 loads compiled, read as source: ox.el')

    def test_every_load_compiled_and_no_load_at_all_say_so(self):
        self.assertEqual(pb.store_note([{'loaded': ['/r/a.el'], 'compiled': {'/r/a.el'}, 'published': 0}]),
                         'Romacs: compiled store, 1 of 1 loads compiled')
        self.assertEqual(pb.store_note([{'loaded': None, 'compiled': set(), 'published': 0}]),
                         'Romacs: compiled store, 0 of 0 loads compiled')

    def test_a_compile_during_a_row_is_said(self):
        note = pb.store_note([{'loaded': ['/r/a.el'], 'compiled': {'/r/a.el'}, 'published': 2}])
        self.assertIn('2 artifacts compiled during this row', note)

    def test_a_long_list_of_source_reads_is_cut_with_its_count(self):
        note = pb.store_note([{'loaded': [f'/r/{n}.el' for n in range(9)], 'compiled': set(), 'published': 0}])
        self.assertIn('and 3 more', note)

    def test_the_loaded_line_is_read_from_ert_output(self):
        self.assertEqual(pb.loaded_line('Ran 3 tests\nPERFBENCH-LOADED ("/r/a.el")\n'), ['/r/a.el'])
        self.assertEqual(pb.loaded_line('PERFBENCH-LOADED nil\n'), [])
        self.assertIsNone(pb.loaded_line('Ran 3 tests\n'))

    def test_the_harness_own_lisp_is_no_load(self):
        loaded = [str(pb.HARNESS / 'parity-probe.el'), '/h/.config/romacs/init.el', '/r/a.el']
        self.assertEqual(pb.compiled_loads([{'loaded': loaded, 'compiled': {'/r/a.el'}, 'published': 0}]),
                         ({'/r/a.el'}, set()))

    def test_the_use_log_counts_from_its_mark(self):
        with tempfile.TemporaryDirectory() as directory:
            store = Path(directory)
            (store / 'gen').mkdir()
            log = store / 'gen/use.log'
            log.write_text('a/1\t/r/old.el\n')
            marks = pb.store_marks(store)
            with log.open('a') as out:
                out.write('a/2\t/r/new.el\n')
            (store / 'gen/key').mkdir()
            (store / 'gen/key/x.rlc').write_text('')
            self.assertEqual(pb.store_since(store, marks), {'compiled': {'/r/new.el'}, 'published': 1})
            log.write_text('a/3\t/r/x.el\n')  # compacted, shorter than its mark: no proof, and a compile ran
            self.assertEqual(pb.store_since(store, {'logs': {log: 10 ** 6}, 'at': time.time_ns() + 10 ** 12}),
                             {'compiled': set(), 'published': 1})

    def test_a_compile_the_store_then_pruned_still_counts(self):
        with tempfile.TemporaryDirectory() as directory:
            store = Path(directory)
            (store / 'gen/old').mkdir(parents=True)
            (store / 'gen/old/a.rlc').write_text('')
            os.utime(store / 'gen/old/a.rlc', ns=(1, 1))
            (store / 'gen/use.log').write_text('')
            marks = pb.store_marks(store)
            (store / 'gen/old/a.rlc').unlink()  # one removed
            (store / 'gen/new').mkdir()
            (store / 'gen/new/b.rlc').write_text('')  # one published: no net growth
            self.assertEqual(pb.store_since(store, marks)['published'], 1)

    def test_a_run_with_no_load_report_is_said_not_counted_as_none(self):
        note = pb.store_note([{'loaded': [], 'compiled': set(), 'published': 0, 'unreported': True},
                              {'loaded': ['/r/a.el'], 'compiled': {'/r/a.el'}, 'published': 0}])
        self.assertIn('loads not checked in 1 of 2 runs', note)


class Readme(unittest.TestCase):
    ROWS = staticmethod(lambda: [pb.row('startup -Q --batch', 'batch', 'run', {'wall': .05}, {'wall': .1}, 3),
                                 member_row('fibn', {'wall': 2.0}, {'wall': 1.0}, {'wall': 1.0})])

    def test_the_editor_table_has_no_elb_rows_and_the_elb_table_only_them(self):
        made = results(self.ROWS())
        self.assertNotIn('fibn', readme.section(made))
        elb = readme.elb_section(made)
        self.assertNotIn('startup', elb)
        self.assertIn("GNU's own benchmark suite", elb)
        self.assertIn('| fibn | 2 s | 1 s | 1 s | 0.50x | 1.00x |', elb)

    def test_the_header_says_how_romacs_ran(self):
        made = results(self.ROWS())
        self.assertIn('from source', readme.section(made))
        self.assertIn('not how an installation runs', readme.elb_section(made))
        made['meta']['install'] = {'files': 100, 'not_compiled': 3}
        for text in (readme.section(made), readme.elb_section(made)):
            self.assertIn('97 of the 100 files', text)
            self.assertIn('--compile-lisp-tree', text)
            self.assertNotIn('not how an installation runs', text)

    def test_without_native_compilation_the_column_is_absent_and_said_so(self):
        elb = readme.elb_section(results(self.ROWS(), native=False))
        self.assertNotIn('GNU native |', elb)
        self.assertIn('built without it', elb)

    def test_notes_carry_no_directories_and_no_empty_separator(self):
        self.assertEqual(readme.public('refused: timed out after 900s: -Q --batch -l /home/someone/tree/perfbench.el'),
                         'refused: timed out after 900s: -Q --batch -l perfbench.el')
        self.assertEqual(readme.public('wall refused: load 9.0 > 8.0; '), 'wall refused: load 9.0 > 8.0')
        self.assertEqual(readme.public('GNU 927/929 expected, Romacs 890/929 expected'),
                         'GNU 927/929 expected, Romacs 890/929 expected')

    def test_a_headline_needs_every_metric_measured_to_clear_the_margin(self):
        made = results([member_row('win', {'instructions': 100, 'wall': 1.0}, {'instructions': 10, 'wall': .1},
                                   {'instructions': 50, 'wall': .5}),
                        member_row('split', {'instructions': 100, 'wall': 1.0}, {'instructions': 100, 'wall': 1.0},
                                   {'instructions': 50, 'wall': 1.5}),
                        member_row('close', {'instructions': 100, 'wall': 1.0}, {'instructions': 100, 'wall': 1.0},
                                   {'instructions': 90, 'wall': .9}),
                        member_row('no-wall', {'instructions': 100}, {'instructions': 100}, {'instructions': 50})])
        found = {(m['row'], name) for m, name, *_ in readme.wins(made)}
        self.assertEqual(found, {('elb win', 'GNU'), ('elb no-wall', 'GNU'), ('elb no-wall', 'GNU native')})

    def test_two_negative_counts_make_no_win_of_their_ratio(self):
        made = results([pb.row('key C-f', 'gui', 'key', {'instructions': -100, 'wall': 1.0},
                               {'instructions': -10, 'wall': .5}, 3)])
        self.assertEqual(list(readme.wins(made)), [])

    def test_a_negative_ratio_and_a_summary_are_no_wins(self):
        made = results([member_row('noise', {'instructions': 100}, {'instructions': 100}, {'instructions': -5}),
                        member_row('geometric mean', {'instructions': 100}, {'instructions': 100}, {'instructions': 5})])
        self.assertEqual(list(readme.wins(made)), [])

    def test_headlines_carry_build_and_rerun_command(self):
        rows = [pb.row('how-many xdisp.c', 'batch', 'run', {'instructions': 100}, {'instructions': 10}, 3),
                pb.row('key C-f', 'gui', 'key', {'instructions': 100}, {'instructions': 200}, 3)]
        rows[1]['neomacs'] = {'instructions': 1000}
        text = readme.headlines(results(rows))
        self.assertIn("--only 'how-many xdisp.c'", text)
        self.assertIn('`aaaaaaaa`', text)
        self.assertIn("--neomacs NEOMACS --only 'keys'", text)
        self.assertEqual(readme.kept_wins(f'<!-- perfbench-headlines-begin -->\n{text}\n<!-- perfbench-headlines-end -->\n'),
                         {('how-many xdisp.c', 'GNU'), ('key C-f', 'neomacs')})

    def test_a_lost_headline_is_said(self):
        with tempfile.TemporaryDirectory() as directory:
            path, data = Path(directory) / 'README.md', Path(directory) / 'results.json'
            path.write_text('<!-- perfbench-table-begin -->\n<!-- perfbench-table-end -->\n'
                            '<!-- perfbench-headlines-begin -->\n| old row | GNU | 0.10x | - | ~a~ | =x= |\n'
                            '<!-- perfbench-headlines-end -->\n<!-- perfbench-elb-begin -->\n<!-- perfbench-elb-end -->\n')
            data.write_text(json.dumps(results(self.ROWS())))
            with contextlib.redirect_stderr(io.StringIO()) as err:
                readme.main([str(data), str(path)])
        self.assertIn('no longer a headline: old row against GNU', err.getvalue())

    def test_neomacs_has_its_columns_only_when_it_ran(self):
        rows = self.ROWS()
        self.assertNotIn('neomacs', readme.section(results(rows)))
        rows[0]['neomacs'] = {'wall': .2}
        made = results(rows)
        made['meta']['neomacs'] = {'version': 'Neomacs 1.2', 'commit': 'c' * 40}
        text = readme.section(made)
        self.assertIn('| neomacs | Romacs/GNU', text)
        self.assertIn('`cccccccc`', text)
        self.assertIn('| 0.50x |', text)

    def test_a_block_the_run_did_not_measure_is_kept(self):
        with tempfile.TemporaryDirectory() as directory:
            path, data = Path(directory) / 'README.md', Path(directory) / 'results.json'
            path.write_text('<!-- perfbench-table-begin -->\nOLD EDITOR\n<!-- perfbench-table-end -->\n'
                            '<!-- perfbench-elb-begin -->\nOLD ELB\n<!-- perfbench-elb-end -->\n')
            data.write_text(json.dumps(results([member_row('fibn', {'wall': 2.0}, {'wall': 1.0}, {'wall': 1.0})])))
            readme.main([str(data), str(path)])
            text = path.read_text()
        self.assertIn('OLD EDITOR', text)
        self.assertNotIn('OLD ELB', text)
        self.assertIn('| fibn |', text)


if __name__ == '__main__':
    unittest.main()
