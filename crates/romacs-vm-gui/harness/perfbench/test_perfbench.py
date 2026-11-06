#!/usr/bin/env python3
"""perfbench's elisp-benchmarks rows and README tables, without an engine.

Run: python3 -B -m unittest discover -s crates/romacs-vm-gui/harness/perfbench -p 'test_perfbench*.py'
"""
import contextlib
import io
import json
import os
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
        self.assertEqual(len(rows), len(pb.MICRO) - 1)
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
        self.assertIn('~aaaaaaaa~', text)
        self.assertIn("--neomacs NEOMACS --only 'keys'", text)
        self.assertEqual(readme.kept_wins(f'# perfbench-headlines-begin\n{text}\n# perfbench-headlines-end\n'),
                         {('how-many xdisp.c', 'GNU'), ('key C-f', 'neomacs')})

    def test_a_lost_headline_is_said(self):
        with tempfile.TemporaryDirectory() as directory:
            path, data = Path(directory) / 'README.org', Path(directory) / 'results.json'
            path.write_text('# perfbench-table-begin\n# perfbench-table-end\n'
                            '# perfbench-headlines-begin\n| old row | GNU | 0.10x | - | ~a~ | =x= |\n'
                            '# perfbench-headlines-end\n# perfbench-elb-begin\n# perfbench-elb-end\n')
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
        self.assertIn('~cccccccc~', text)
        self.assertIn('| 0.50x |', text)

    def test_a_block_the_run_did_not_measure_is_kept(self):
        with tempfile.TemporaryDirectory() as directory:
            path, data = Path(directory) / 'README.org', Path(directory) / 'results.json'
            path.write_text('# perfbench-table-begin\nOLD EDITOR\n# perfbench-table-end\n'
                            '# perfbench-elb-begin\nOLD ELB\n# perfbench-elb-end\n')
            data.write_text(json.dumps(results([member_row('fibn', {'wall': 2.0}, {'wall': 1.0}, {'wall': 1.0})])))
            readme.main([str(data), str(path)])
            text = path.read_text()
        self.assertIn('OLD EDITOR', text)
        self.assertNotIn('OLD ELB', text)
        self.assertIn('| fibn |', text)


if __name__ == '__main__':
    unittest.main()
