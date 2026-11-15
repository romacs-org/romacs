"""real_init.py: the profile config and the safety it enforces."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import real_init


class RealInitTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix='real-init-'))
        (self.dir / 'init.el').write_text('(bench-block "tally" (setq tally-width 4))\n')

    def config(self, **keys):
        path = self.dir / 'profile.json'
        path.write_text(json.dumps({'init': 'init.el', **keys}))
        return path

    def test_paths_resolve_beside_the_config(self):
        config = real_init.load(self.config(projects=['p'], sources=[['lisp', '*.el']], gnosis_db='db/gnosis.db'))
        self.assertEqual(config['init'], (self.dir / 'init.el').resolve())
        self.assertEqual(config['projects'], [(self.dir / 'p').resolve()])
        self.assertEqual(config['sources'], [((self.dir / 'lisp').resolve(), '*.el')])
        self.assertEqual(config['gnosis_db'], (self.dir / 'db/gnosis.db').resolve())

    def test_no_config_is_empty_and_the_init_profile_needs_one(self):
        with mock.patch.dict(os.environ, {real_init.CONFIG_ENV: ''}):
            self.assertEqual(real_init.load(None), {})
        with self.assertRaises(ValueError):
            real_init.profile_init({})

    def test_environment_names_the_config(self):
        with mock.patch.dict(os.environ, {real_init.CONFIG_ENV: str(self.config(org='notes.org'))}):
            self.assertEqual(real_init.load(None)['org'], (self.dir / 'notes.org').resolve())

    def test_live_gnosis_databases_are_refused(self):
        live = self.dir / 'live/gnosis.db'
        config = real_init.load(self.config(live_gnosis_db=['live/gnosis.db']))
        with self.assertRaises(ValueError):
            real_init.refuse_live_gnosis(live, config)
        with self.assertRaises(ValueError):
            real_init.refuse_live_gnosis(Path.home() / '.config/emacs/gnosis/gnosis.db', {})
        with mock.patch.dict(os.environ, {'XDG_CONFIG_HOME': str(self.dir / 'xdg')}):
            with self.assertRaises(ValueError):
                real_init.refuse_live_gnosis(self.dir / 'xdg/emacs/gnosis/gnosis.db', {})
        copy = self.dir / 'copy/gnosis.db'
        self.assertEqual(real_init.refuse_live_gnosis(copy, config), copy.resolve())

    def test_init_session_runs_the_blocks_before_the_endpoint(self):
        config = real_init.load(self.config())
        source = real_init.init_source('romacs', self.dir, self.dir / 'port', 'tok', 'init', config)
        self.assertLess(source.index('(bench-block "tally"'), source.index('romacs-mcp-enable-session-tools'))

    def test_stock_session_loads_no_profile(self):
        source = real_init.init_source('gnu', None, self.dir / 'port', 'tok', 'stock', {})
        self.assertNotIn('bench-block "', source)


if __name__ == '__main__':
    unittest.main()
