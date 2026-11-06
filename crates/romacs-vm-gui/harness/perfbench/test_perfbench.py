#!/usr/bin/env python3
"""perfbench's elisp-benchmarks rows and README tables, without an engine.

Run: python3 -B -m unittest discover -s crates/romacs-vm-gui/harness/perfbench -p 'test_perfbench*.py'
"""
import json
import os
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
        mean = pb.elb_geomean_row(rows, ['gnu', 'gnu_native', 'romacs'], 3)
        self.assertAlmostEqual(mean['gnu']['wall'], 4 ** (1 / 3))
        self.assertAlmostEqual(mean['romacs']['wall'], 4 ** (1 / 3))
        self.assertAlmostEqual(mean['gnu_native']['wall'], 1e-5 ** (1 / 3))
        self.assertIsNone(mean['gnu_native']['instructions'])
        self.assertIn('3 of 4', mean['note'])
        self.assertIn('left out: scroll', mean['note'])
        self.assertIn('no mean of GNU native instructions', mean['note'])
        self.assertEqual(mean['load'], 9.0)

    def test_load_refuses_the_native_wall_too(self):
        made = member_row('a', {'wall': 1.0}, {'wall': .5}, {'wall': 2.0}, load=12)
        pb.gate_walls([made], 8)
        self.assertEqual([made[c].get('wall') for c in ('gnu', 'gnu_native', 'romacs')], [None] * 3)
        self.assertEqual(made['gnu_native']['wall_refused'], .5)

    def test_ratios_are_romacs_over_each_gnu(self):
        made = pb.with_ratios([member_row('a', {'wall': 1.0}, {'wall': .5}, {'wall': 2.0})], None)[0]
        self.assertEqual((made['wall_ratio'], made['native_wall_ratio']), (2.0, 4.0))


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
