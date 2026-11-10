#!/usr/bin/env python3
"""Offline website deployment tests: fake gcloud, no credentials or live writes."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class UploadTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='lilbox-www-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo with spaces'
        shutil.copytree(ROOT / 'www', self.repo / 'www')
        (self.repo / 'scripts').mkdir()
        for name in ('upload.sh', 'check-www.py'):
            shutil.copyfile(ROOT / 'scripts' / name, self.repo / 'scripts' / name)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        (self.bin / 'python3').symlink_to(sys.executable)
        self.log = self.root / 'calls.jsonl'
        self.remote = self.root / 'remote.html'
        self.env = dict(os.environ, HOME=str(self.root), TMPDIR=str(self.root),
                        CLOUDSDK_CONFIG=str(self.root / 'cloud'), PATH=f'{self.bin}:/usr/bin:/bin',
                        CLOUD_LOG=str(self.log), REMOTE_INDEX=str(self.remote),
                        LOCAL_INDEX=str(self.repo / 'www/index.html'))
        gcloud = self.bin / 'gcloud'
        gcloud.write_text(f'#!{sys.executable}\n' + '''
import json, os, pathlib, sys
args = sys.argv[1:]
with open(os.environ['CLOUD_LOG'], 'a') as log:
    log.write(json.dumps(args) + '\\n')
if os.environ.get('FAIL_CLOUD') == args[1]:
    sys.exit('simulated upload failure')
if args[:2] == ['storage', 'rsync']:
    if os.environ.get('EDIT_SOURCE'):
        pathlib.Path(os.environ['LOCAL_INDEX']).write_text('edited during upload')
elif args[:2] == ['storage', 'cp']:
    pathlib.Path(os.environ['REMOTE_INDEX']).write_bytes(pathlib.Path(args[2]).read_bytes())
elif args[:2] == ['storage', 'cat']:
    content = pathlib.Path(os.environ['REMOTE_INDEX']).read_bytes()
    sys.stdout.buffer.write(b'wrong page' if os.environ.get('CORRUPT_READBACK') else content)
else:
    sys.exit('unexpected cloud operation')
''')
        gcloud.chmod(0o755)

    def upload(self):
        return subprocess.run(['bash', str(self.repo / 'scripts/upload.sh')], cwd=self.root,
                              env=self.env, capture_output=True, text=True, timeout=15)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_assets_before_index_and_only_website_objects(self):
        original = (self.repo / 'www/index.html').read_bytes()
        self.env['EDIT_SOURCE'] = '1'
        result = self.upload()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual(len(calls), 3)
        self.assertEqual(calls[0][:2], ['storage', 'rsync'])
        self.assertEqual(calls[0][3:], ['gs://muthanna.com/lilbox/assets/', '--recursive', '--checksums-only',
                                      '--predefined-acl=publicRead', '--cache-control=public,max-age=86400,immutable'])
        self.assertEqual(calls[1][:2], ['storage', 'cp'])
        self.assertEqual(calls[1][3:], ['gs://muthanna.com/lilbox/index.html', '--predefined-acl=publicRead',
                                      '--cache-control=public,max-age=300,must-revalidate', '--content-type=text/html'])
        self.assertEqual(calls[2], ['storage', 'cat', 'gs://muthanna.com/lilbox/index.html'])
        self.assertEqual(self.remote.read_bytes(), original)
        self.assertNotIn('install.sh', json.dumps(calls))
        self.assertNotIn('README.md', json.dumps(calls))
        self.assertFalse(Path(calls[1][2]).parent.exists())
        self.assertIn('Published https://mo.town/lilbox/', result.stdout)

    def test_failed_assets_never_publish_page(self):
        self.env['FAIL_CLOUD'] = 'rsync'
        result = self.upload()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.calls()), 1)
        self.assertFalse(self.remote.exists())
        self.assertNotIn('Published ', result.stdout)
        self.assertEqual(list(self.root.glob('lilbox-site-upload.*')), [])

    def test_publish_and_readback_failures_are_errors(self):
        for key, value in [('FAIL_CLOUD', 'cp'), ('FAIL_CLOUD', 'cat'), ('CORRUPT_READBACK', '1')]:
            with self.subTest(key=key, value=value):
                self.env[key] = value
                result = self.upload()
                self.env.pop(key)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn('Published ', result.stdout)
                self.assertEqual(list(self.root.glob('lilbox-site-upload.*')), [])

    def test_changed_or_missing_icon_fails_before_upload(self):
        icon = next((self.repo / 'www/assets').iterdir())
        icon.write_bytes(b'changed without a new fingerprint')
        self.assertNotEqual(self.upload().returncode, 0)
        self.assertFalse(self.log.exists())
        icon.unlink()
        self.assertNotEqual(self.upload().returncode, 0)
        self.assertFalse(self.log.exists())


if __name__ == '__main__':
    unittest.main()
