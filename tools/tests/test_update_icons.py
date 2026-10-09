"""Checks for the boundaries required by automatic icon commits."""

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
spec = importlib.util.spec_from_file_location('update_icons', TOOLS / 'update-icons.py')
# The executable retains its hyphenated filename for just/workflow compatibility.
icons = importlib.util.module_from_spec(spec)
spec.loader.exec_module(icons)
sys.modules['update_icons'] = icons
spec = importlib.util.spec_from_file_location('schema', TOOLS / 'check-upstream-schema.py')
schema = importlib.util.module_from_spec(spec)
spec.loader.exec_module(schema)
SVG = b'<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>'


class IconUpdateTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.icon_dir = self.root / 'data/icons/providers'
        self.icon_dir.mkdir(parents=True)
        self.source = self.root / 'src/icons.rs'
        self.source.parent.mkdir()
        self.source.write_text('unchanged code\n' + icons.render_table(['old']) + '\n')
        (self.icon_dir / 'old.svg').write_bytes(SVG)
        self.constants = patch.multiple(icons, ROOT=self.root, ICON_DIR=self.icon_dir, ICONS_RS=self.source)
        self.constants.start()
        self.addCleanup(self.constants.stop)
        self.git('init', '-q')
        self.git('config', 'user.name', 'Test')
        self.git('config', 'user.email', 'test@example.com')
        self.git('add', '.')
        self.git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'baseline')

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, stderr=subprocess.STDOUT)

    def test_failed_download_preserves_all_local_files(self):
        def fetch(url):
            if url == 'good':
                return SVG
            raise OSError('network failed')
        with patch.object(icons, 'fetch', side_effect=fetch):
            with self.assertRaises(OSError):
                icons.sync({'new': 'good', 'another': 'bad'})
        self.assertEqual((self.icon_dir / 'old.svg').read_bytes(), SVG)
        self.assertFalse((self.icon_dir / 'new.svg').exists())

    def test_listing_and_downloads_share_one_commit(self):
        commit = 'a' * 40
        listing = [{'name': 'ProviderIcon-new.svg', 'download_url': 'https://unexpected.example/icon.svg'}]
        with patch.object(icons, 'fetch', return_value=json.dumps(listing).encode()) as fetch:
            urls = icons.upstream_icons(commit)
        self.assertIn('ref=' + commit, fetch.call_args.args[0])
        self.assertEqual(urls['new'], f'https://raw.githubusercontent.com/{icons.REPO}/{commit}/{icons.RESOURCE_PATH}/ProviderIcon-new.svg')

    def test_valid_update_removes_obsolete_icons_and_only_changes_generated_code(self):
        with patch.object(icons, 'fetch', return_value=SVG):
            self.assertEqual(icons.sync({'new': 'unused'}), (['new'], [], ['old']))
        icons.regenerate(['new'])
        self.git('add', '.')
        icons.validate_staged()
        self.assertFalse((self.icon_dir / 'old.svg').exists())
        self.assertTrue(self.source.read_text().startswith('unchanged code\n'))

    def test_rejects_code_outside_table_and_unrelated_staged_files(self):
        self.source.write_text(self.source.read_text().replace('unchanged code', 'changed code'))
        self.git('add', '.')
        with self.assertRaisesRegex(ValueError, 'outside the generated table'):
            icons.validate_staged()
        self.git('reset', '--hard', '-q', 'HEAD')
        (self.root / 'unexpected.txt').write_text('unexpected')
        self.git('add', '.')
        with self.assertRaisesRegex(ValueError, 'unexpected staged file'):
            icons.validate_staged()

    def test_rejects_invalid_or_active_svg_before_writing(self):
        for content in (b'<html/>', b'<svg><script/></svg>', b'<svg><image href="https://example.com/icon"/></svg>'):
            with patch.object(icons, 'fetch', return_value=content):
                with self.assertRaises(ValueError):
                    icons.sync({'new': 'unused'})
            self.assertEqual((self.icon_dir / 'old.svg').read_bytes(), SVG)
            self.assertFalse((self.icon_dir / 'new.svg').exists())

    def test_rejects_staged_symlink(self):
        path = self.icon_dir / 'old.svg'
        path.unlink()
        path.symlink_to(self.source)
        self.git('add', '.')
        with self.assertRaises(ValueError):
            icons.validate_staged()


class SchemaTests(unittest.TestCase):
    def test_detects_changed_and_removed_sources(self):
        baseline = {'files': {'same.swift': 'a', 'changed.swift': 'b', 'removed.swift': 'c'}}
        self.assertEqual(schema.changed_files(baseline, {'same.swift': 'a', 'changed.swift': 'new'}),
                         ['changed.swift', 'removed.swift'])
        self.assertEqual(schema.changed_files(baseline, baseline['files']), [])


if __name__ == '__main__':
    unittest.main()
