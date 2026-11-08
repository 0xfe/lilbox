#!/usr/bin/env python3
"""Offline installer and release-request tests; never contact GitHub or publish."""
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
VERSION = '0.3.0'
SHA = 'a' * 40


class ScriptsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='lilbox-release-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / 'tools'
        self.bin.mkdir()
        self.assets = self.root / 'assets'
        self.assets.mkdir()
        self.home = self.root / 'home'
        self.home.mkdir()
        self.prefix = self.root / 'prefix with spaces'
        self.log = self.root / 'calls.jsonl'
        self.env = dict(os.environ, HOME=str(self.home), PATH=f'{self.bin}:/usr/bin:/bin',
                        TMPDIR=str(self.root), FIXTURES=str(self.assets), CALL_LOG=str(self.log),
                        FAKE_OS='Linux', FAKE_ARCH='x86_64', FAKE_VERSION=VERSION,
                        FAKE_SHA=SHA, FAKE_BRANCH='main')
        # Independent of the shell's stdin: exercise the actual curl | sh pattern.
        self.tool('uname', '#!/bin/sh\ncase "$1" in -s) echo "$FAKE_OS" ;; -m) echo "$FAKE_ARCH" ;; esac\n')
        self.python_tool('curl', '''
import json, os, pathlib, sys
args = sys.argv[1:]
with open(os.environ['CALL_LOG'], 'a') as log:
    log.write(json.dumps(args) + '\\n')
if os.environ.get('FAIL_DOWNLOAD'):
    sys.exit(22)
if '-w' in args:
    print(os.environ.get('LATEST_URL', 'https://github.com/0xfe/lilbox/releases/tag/v0.3.0'), end='')
else:
    url = next(arg for arg in args if arg.startswith('https://'))
    source = pathlib.Path(os.environ['FIXTURES']) / url.rsplit('/', 1)[-1]
    destination = pathlib.Path(args[args.index('-o') + 1])
    if not source.is_file():
        sys.exit(22)
    destination.write_bytes(source.read_bytes())
''')

    def tool(self, name, source):
        path = self.bin / name
        path.write_text(source)
        path.chmod(0o755)

    def python_tool(self, name, source):
        self.tool(name, f'#!{sys.executable}\n' + source)

    def archive(self, target='x86_64-unknown-linux-musl', binary_version=VERSION, symlink=False):
        name = f'lilbox-{VERSION}-{target}'
        archive = self.assets / f'{name}.tar.gz'
        with tarfile.open(archive, 'w:gz') as bundle:
            member = tarfile.TarInfo(f'{name}/lilbox')
            member.mode = 0o755
            if symlink:
                member.type = tarfile.SYMTYPE
                member.linkname = '/bin/sh'
                bundle.addfile(member)
            else:
                content = f'#!/bin/sh\nprintf "lilbox {binary_version}\\n"\n'.encode()
                member.size = len(content)
                bundle.addfile(member, io.BytesIO(content))
        Path(str(archive) + '.sha256').write_text(
            hashlib.sha256(archive.read_bytes()).hexdigest() + f'  {archive.name}\n')
        return archive

    def install(self, *args):
        return subprocess.run(['sh', '-s', '--', '--prefix', str(self.prefix), *args],
                              input=(ROOT / 'install.sh').read_bytes(), env=self.env,
                              capture_output=True, timeout=15)

    def assert_failed(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout)

    def test_platforms_pinned_version_and_upgrade(self):
        for os_name, arch, target in [
            ('Darwin', 'arm64', 'aarch64-apple-darwin'),
            ('Darwin', 'x86_64', 'x86_64-apple-darwin'),
            ('Linux', 'aarch64', 'aarch64-unknown-linux-musl'),
            ('Linux', 'x86_64', 'x86_64-unknown-linux-musl'),
        ]:
            with self.subTest(os=os_name, arch=arch):
                self.env.update(FAKE_OS=os_name, FAKE_ARCH=arch)
                self.archive(target)
                result = self.install('--version', f'v{VERSION}')
                self.assertEqual(result.returncode, 0, result.stderr)
                binary = self.prefix / 'bin/lilbox'
                self.assertEqual(binary.stat().st_mode & 0o777, 0o755)
                self.assertEqual(subprocess.check_output([binary, '--version']), b'lilbox 0.3.0\n')
                calls = self.log.read_text()
                self.assertIn(f'{target}.tar.gz', calls)
                self.assertNotIn('/releases/latest', calls)

    def test_latest_and_default_prefix(self):
        self.archive()
        result = subprocess.run(['sh', '-s'], input=(ROOT / 'install.sh').read_bytes(),
                                env=self.env, capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.home / '.local/bin/lilbox').is_file())
        self.assertIn('/releases/latest', self.log.read_text())

    def test_failures_preserve_existing_binary_and_clean_up(self):
        destination = self.prefix / 'bin/lilbox'
        destination.parent.mkdir(parents=True)
        for failure in ('checksum', 'checksum_name', 'download', 'version', 'symlink'):
            with self.subTest(failure=failure):
                destination.write_text('keep existing binary')
                archive = self.archive(binary_version='9.9.9' if failure == 'version' else VERSION,
                                       symlink=failure == 'symlink')
                if failure == 'checksum':
                    archive.write_bytes(b'corrupt download')
                if failure == 'checksum_name':
                    Path(str(archive) + '.sha256').write_text('0' * 64 + '  wrong.tar.gz\n')
                if failure == 'download':
                    self.env['FAIL_DOWNLOAD'] = '1'
                result = self.install('--version', VERSION)
                self.env.pop('FAIL_DOWNLOAD', None)
                self.assert_failed(result)
                self.assertEqual(destination.read_text(), 'keep existing binary')
                self.assertEqual(list(self.root.glob('lilbox-install.*')), [])
                self.assertEqual(list(destination.parent.glob('.lilbox.*')), [])

    def test_bad_inputs_fail_before_download(self):
        for args in [('--prefix', 'relative'), ('--version', '../../escape'), ('--unknown',), ('--version',)]:
            with self.subTest(args=args):
                self.assert_failed(self.install(*args))
        for key, value in [('FAKE_OS', 'FreeBSD'), ('FAKE_ARCH', 'riscv64')]:
            prior = self.env[key]
            self.env[key] = value
            self.assert_failed(self.install('--version', VERSION))
            self.env[key] = prior
        self.assertFalse(self.log.exists())

    def test_invalid_latest_redirect_and_missing_asset(self):
        self.env['LATEST_URL'] = 'https://example.com/releases/tag/v0.3.0'
        self.assert_failed(self.install())
        self.env.pop('LATEST_URL')
        self.assert_failed(self.install('--version', VERSION))
        self.assertFalse((self.prefix / 'bin/lilbox').exists())

    def test_existing_symlink_is_not_followed(self):
        self.archive()
        self.prefix.joinpath('bin').mkdir(parents=True)
        original = self.root / 'original'
        original.write_text('unchanged')
        self.prefix.joinpath('bin/lilbox').symlink_to(original)
        self.assert_failed(self.install('--version', VERSION))
        self.assertEqual(original.read_text(), 'unchanged')

    def release_tools(self):
        self.python_tool('git', '''
import os, sys
args = sys.argv[1:]
if args[0] == 'status': print(os.environ.get('FAKE_DIRTY', ''), end='')
elif args[0] == 'symbolic-ref': print(os.environ['FAKE_BRANCH'])
elif args[0] == 'rev-parse': print(os.environ['FAKE_SHA'])
elif args[0] == 'ls-remote':
    if 'refs/heads/main' in args:
        print(os.environ.get('FAKE_REMOTE', os.environ['FAKE_SHA']) + '\\trefs/heads/main')
    elif os.environ.get('FAKE_TAG'):
        print(os.environ['FAKE_SHA'] + '\\trefs/tags/v0.3.0')
elif args[0] == 'remote': print('git@github.com:0xfe/lilbox.git')
else: sys.exit(1)
''')
        self.tool('cargo', '#!/bin/sh\necho "path+file:///repo/lilbox#${FAKE_VERSION}"\n')
        self.python_tool('gh', '''
import json, os, sys
args = sys.argv[1:]
if args[:2] == ['repo', 'view']: print('0xfe/lilbox')
elif args[:2] == ['workflow', 'run']:
    with open(os.environ['CALL_LOG'], 'a') as log: log.write(json.dumps(args) + '\\n')
else: sys.exit(1)
''')

    def request_release(self):
        return subprocess.run(['bash', str(ROOT / 'scripts/request-release.sh')],
                              env=self.env, capture_output=True, timeout=15)

    def test_release_request_pins_commit_and_version(self):
        self.release_tools()
        result = self.request_release()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(self.log.read_text()),
                         ['workflow', 'run', 'release.yml', '--repo', '0xfe/lilbox', '--ref', 'main',
                          '-f', f'commit={SHA}', '-f', f'version={VERSION}'])

    def test_release_request_rejects_unready_sources(self):
        self.release_tools()
        for key, value in [('FAKE_DIRTY', ' M README.md'), ('FAKE_BRANCH', 'feature'),
                           ('FAKE_REMOTE', 'b' * 40), ('FAKE_TAG', 'exists'),
                           ('FAKE_VERSION', '0.3.0-rc.1')]:
            with self.subTest(key=key):
                previous = self.env.get(key)
                self.env[key] = value
                self.assert_failed(self.request_release())
                if previous is None:
                    self.env.pop(key)
                else:
                    self.env[key] = previous
        self.assertFalse(self.log.exists())


if __name__ == '__main__':
    unittest.main()
