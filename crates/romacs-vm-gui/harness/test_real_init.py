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
        (self.dir / 'init.el').write_text('(bench-block "hermes" (setq hermes-instances \'(("l" . "@HERMES_URL@"))))\n')

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
        with mock.patch.dict(os.environ, {real_init.CONFIG_ENV: str(self.config(hermes_url='http://h:1'))}):
            self.assertEqual(real_init.hermes_url(real_init.load(None)), 'http://h:1')

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

    def test_an_init_that_may_start_hermes_is_refused(self):
        for setting in ("(setq hermes-dashboard-transport-start-mode 'spawn)",
                        "(setopt hermes-dashboard-transport-start-mode 'auto)",
                        "(list :start-mode 'spawn)"):
            (self.dir / 'init.el').write_text(setting)
            with self.assertRaises(ValueError, msg=setting):
                real_init.profile_init(real_init.load(self.config()))
        (self.dir / 'init.el').write_text("(setq hermes-dashboard-transport-start-mode 'remote)")
        real_init.profile_init(real_init.load(self.config()))

    def test_init_session_attaches_hermes_around_the_blocks(self):
        config = real_init.load(self.config(hermes_url='http://h:1'))
        source = real_init.init_source('romacs', self.dir, self.dir / 'port', 'tok', 'init', config)
        blocks = source.index('(bench-block "hermes"')
        self.assertIn('"http://h:1"', source)
        self.assertLess(source.index(real_init.HERMES_REMOTE), blocks)
        self.assertGreater(source.rindex("(setq hermes-dashboard-transport-start-mode 'remote)"), blocks)

    def test_stock_session_loads_no_profile(self):
        source = real_init.init_source('gnu', None, self.dir / 'port', 'tok', 'stock', {})
        self.assertNotIn('bench-block "', source)
        self.assertNotIn('hermes-dashboard-transport-start-mode', source)


if __name__ == '__main__':
    unittest.main()
