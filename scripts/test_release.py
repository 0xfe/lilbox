#!/usr/bin/env python3
"""Offline installer and release-request tests; never contact GitHub or publish."""
import argparse
import errno
import select
import signal
import time
import hashlib
import io
import json
import os
from pathlib import Path
import pty
import runpy
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest

# Locate real scripts to copy into temporary fixtures; never mutate this checkout.
ROOT = Path(__file__).resolve().parent.parent
# Fixed fake release version, independent of the current Cargo package version.
VERSION = '0.3.0'
# Populated by --binary for tests that exercise the real install/init behavior.
REAL_BINARY = None


def profile_source(name):
    """Match compile-time profile selection of the real native fixture binary."""
    platform = 'macos' if sys.platform == 'darwin' else 'linux'
    return ROOT / 'profiles' / (name if name == 'permissive.toml' else platform + '/' + name)


class ScriptsTest(unittest.TestCase):
    """Run the piped installer against local archives and stubbed download tools."""
    # Create an isolated home, local assets, and fake platform/download commands.
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
                        INIT_LOG=str(self.root / 'init.log'), TERM='xterm-256color')
        self.env.pop('XDG_CONFIG_HOME', None)
        self.env.pop('NO_COLOR', None)
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

    # Install a fixture executable at the front of this test's PATH.
    def tool(self, name, source):
        path = self.bin / name
        path.write_text(source)
        path.chmod(0o755)

    # Run Python stubs with the same interpreter as the test harness.
    def python_tool(self, name, source):
        self.tool(name, f'#!{sys.executable}\n' + source)

    # Build a tiny release archive/checksum, optionally containing invalid executable data.
    def archive(self, target='x86_64-unknown-linux-musl', binary_version=VERSION, symlink=False,
                content=None, release_version=VERSION):
        name = f'lilbox-{release_version}-{target}'
        archive = self.assets / f'{name}.tar.gz'
        with tarfile.open(archive, 'w:gz') as bundle:
            member = tarfile.TarInfo(f'{name}/lilbox')
            member.mode = 0o755
            if symlink:
                member.type = tarfile.SYMTYPE
                member.linkname = '/bin/sh'
                bundle.addfile(member)
            else:
                if content is None:
                    content = f'''#!/bin/sh
case "$1" in
    --version) printf 'lilbox {binary_version}\\n' ;;
    init)
        printf '%s\\n' "$0" >> "$INIT_LOG"
        [ "${{FAIL_INIT:-0}}" = 0 ] || exit 1 ;;
    *) exit 2 ;;
esac
'''.encode()
                member.size = len(content)
                bundle.addfile(member, io.BytesIO(content))
        Path(str(archive) + '.sha256').write_text(
            hashlib.sha256(archive.read_bytes()).hexdigest() + f'  {archive.name}\n')
        return archive

    # Feed the actual installer through stdin to reproduce curl-pipe-shell usage offline.
    def install(self, *args):
        return subprocess.run(['sh', '-s', '--', '--prefix', str(self.prefix), *args],
                              input=(ROOT / 'install.sh').read_bytes(), env=self.env,
                              capture_output=True, timeout=15)

    # Require a nonzero exit and include stdout if the script unexpectedly succeeds.
    def assert_failed(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout)

    # Package the supplied native binary using its actual embedded version.
    def real_archive(self):
        version = subprocess.check_output([REAL_BINARY, '--version'], text=True).strip().removeprefix('lilbox ')
        self.archive(content=REAL_BINARY.read_bytes(), release_version=version)
        return version

    # Capture terminal stdout while keeping installer input piped and stderr separate.
    def terminal_install(self):
        # stdin stays a pipe, exactly as in curl | sh; only stdout is a terminal.
        master, slave = pty.openpty()
        chunks = []

        # Drain the PTY concurrently so a full terminal buffer cannot stall installation.
        def read_output():
            while True:
                try:
                    chunk = os.read(master, 4096)
                except OSError:
                    break  # Linux PTYs report EIO after the final slave closes.
                if not chunk:
                    break
                chunks.append(chunk)

        try:
            process = subprocess.Popen(['sh', '-s', '--', '--prefix', str(self.prefix), '--version', VERSION],
                                       env=self.env, stdin=subprocess.PIPE, stdout=slave, stderr=subprocess.PIPE)
        finally:
            os.close(slave)
        reader = threading.Thread(target=read_output, daemon=True)
        reader.start()
        try:
            _, errors = process.communicate((ROOT / 'install.sh').read_bytes(), timeout=15)
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
            reader.join(timeout=5)
            os.close(master)
        self.assertFalse(reader.is_alive(), 'terminal output reader did not finish')
        self.assertEqual(process.returncode, 0, errors)
        return b''.join(chunks)

    # Verify each platform mapping and replacement install without latest-release discovery.
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
                self.assertEqual((self.root / 'init.log').read_text().splitlines()[-1], str(binary))
                self.assertNotIn(b'\x1b', result.stdout)

    # Resolve a fake latest redirect and install under the disposable home by default.
    def test_latest_and_default_prefix(self):
        self.archive()
        result = subprocess.run(['sh', '-s'], input=(ROOT / 'install.sh').read_bytes(),
                                env=self.env, capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.home / '.local/bin/lilbox').is_file())
        self.assertIn('/releases/latest', self.log.read_text())

    # Progress must reflect ordering and never announce work skipped after a checksum failure.
    def test_progress_follows_work_and_stops_at_failed_verification(self):
        archive = self.archive()
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        stages = ['Checking platform', 'Selected platform:', 'Finding the latest stable release',
                  'Preparing lilbox', 'Downloading ' + archive.name,
                  'Downloading ' + archive.name + '.sha256', 'Verifying the archive SHA-256 checksum',
                  'Extracting the executable', 'Copying the executable', 'Checking the executable version',
                  'Installing ' + str(self.prefix), 'Initializing profiles', 'Installed and ready']
        output = result.stdout.decode()
        positions = [output.index(stage) for stage in stages]
        self.assertEqual(positions, sorted(positions))
        self.assertNotIn('\x1b', output)
        archive.write_bytes(b'corrupt download')
        result = self.install('--version', VERSION)
        self.assert_failed(result)
        self.assertIn(b'Verifying the archive', result.stdout)
        for stage in ('Finding the latest stable release', 'Extracting the executable',
                      'Copying the executable', 'Initializing profiles', 'Installed and ready'):
            self.assertNotIn(stage.encode(), result.stdout)

    # Every pre-replacement failure must keep the old binary and remove staging files.
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
                self.assertFalse((self.root / 'init.log').exists())

    # Reject invalid options and unsupported platforms before invoking the download stub.
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

    # Reject redirects outside the release repository and missing versioned assets.
    def test_invalid_latest_redirect_and_missing_asset(self):
        self.env['LATEST_URL'] = 'https://example.com/releases/tag/v0.3.0'
        self.assert_failed(self.install())
        self.env.pop('LATEST_URL')
        self.assert_failed(self.install('--version', VERSION))
        self.assertFalse((self.prefix / 'bin/lilbox').exists())

    # Refuse a symlink destination without changing the file it points to.
    def test_existing_symlink_is_not_followed(self):
        self.archive()
        self.prefix.joinpath('bin').mkdir(parents=True)
        original = self.root / 'original'
        original.write_text('unchanged')
        self.prefix.joinpath('bin/lilbox').symlink_to(original)
        self.assert_failed(self.install('--version', VERSION))
        self.assertEqual(original.read_text(), 'unchanged')

    # Use real init to fill missing profiles while preserving user content, inodes, and times.
    def test_real_init_fresh_install_and_upgrade_preserve_custom_profiles(self):
        version = self.real_archive()
        # A pre-existing lilbox in PATH must never handle initialization.
        self.tool('lilbox', '#!/bin/sh\nexit 96\n')
        result = self.install('--version', version)
        self.assertEqual(result.returncode, 0, result.stderr)
        config = self.home / '.config/lilbox'
        for name in ('default', 'permissive', 'isolated'):
            self.assertEqual((config / f'{name}.toml').read_bytes(), profile_source(f'{name}.toml').read_bytes())
        custom = config / 'default.toml'
        custom.write_text('# Handwritten profile; must survive upgrades.\n')
        original = custom.stat()
        extra = config / 'work.toml'
        extra.write_text('# Custom named profile.\n')
        (config / 'isolated.toml').unlink()
        result = self.install('--version', version)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(custom.read_text(), '# Handwritten profile; must survive upgrades.\n')
        self.assertEqual((config / f'default.toml.{version}').read_bytes(), profile_source('default.toml').read_bytes())
        self.assertEqual(custom.stat().st_mtime_ns, original.st_mtime_ns)
        self.assertEqual(custom.stat().st_ino, original.st_ino)
        self.assertEqual(extra.read_text(), '# Custom named profile.\n')
        self.assertEqual((config / 'isolated.toml').read_bytes(), profile_source('isolated.toml').read_bytes())
        output = result.stdout.decode()
        for expected in (str(config), str(self.prefix / 'bin/lilbox'), 'existing files preserved',
                         'lilbox check', 'lilbox shell', 'lilbox codex', 'export PATH=', '+-------+'):
            self.assertIn(expected, output)

    # Respect XDG lookup and preserve both valid and dangling profile symlinks.
    def test_real_init_xdg_directory_and_existing_symlinks_are_preserved(self):
        version = self.real_archive()
        xdg = self.root / 'custom config'
        self.env['XDG_CONFIG_HOME'] = str(xdg)
        config = xdg / 'lilbox'
        config.mkdir(parents=True)
        original = self.root / 'external.toml'
        original.write_text('# Existing symlink target.\n')
        (config / 'default.toml').symlink_to(original)
        missing = self.root / 'missing.toml'
        (config / 'isolated.toml').symlink_to(missing)
        result = self.install('--version', version)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((config / 'default.toml').is_symlink())
        self.assertTrue((config / 'isolated.toml').is_symlink())
        self.assertEqual(original.read_text(), '# Existing symlink target.\n')
        self.assertFalse(missing.exists())
        self.assertFalse((self.home / '.config').exists())
        self.assertIn(str(config).encode(), result.stdout)

    # An init failure leaves the installed binary but must report failure and a recovery command.
    def test_real_init_failure_reports_partial_install_and_recovery(self):
        version = self.real_archive()
        blocked = self.root / 'not-a-directory'
        blocked.write_text('preserve this file')
        self.env['XDG_CONFIG_HOME'] = str(blocked)
        result = self.install('--version', version)
        self.assert_failed(result)
        self.assertEqual(blocked.read_text(), 'preserve this file')
        self.assertTrue((self.prefix / 'bin/lilbox').is_file())
        self.assertIn(b'profile initialization failed', result.stderr)
        self.assertIn(b'then retry:', result.stderr)
        self.assertNotIn(b'Installed and ready', result.stdout)
        self.assertEqual(list((self.prefix / 'bin').glob('.lilbox.*')), [])

    # Check terminal color output and both supported ways to disable it.
    def test_terminal_colors_and_opt_out(self):
        self.archive()
        self.assertIn('\x1b[1;36m→\x1b[0m Checking platform'.encode(), self.terminal_install())
        for value in ('', '1'):
            self.env['NO_COLOR'] = value
            self.assertNotIn(b'\x1b', self.terminal_install())
        self.env.pop('NO_COLOR')
        self.env['TERM'] = 'dumb'
        self.assertNotIn(b'\x1b', self.terminal_install())

    # Execute printed PATH guidance against tricky path characters to verify safe quoting.
    def test_path_guidance_is_shell_quoted_and_omitted_when_already_present(self):
        self.archive()
        self.prefix = self.root / "prefix ' with $spaces"
        result = self.install('--version', VERSION)
        self.assertEqual(result.returncode, 0, result.stderr)
        command = next(line.strip() for line in result.stdout.decode().splitlines() if 'export PATH=' in line)
        actual = subprocess.check_output(['sh', '-c', command + '\nprintf "%s" "$PATH"'], env=self.env, text=True)
        self.assertEqual(actual, str(self.prefix / 'bin') + ':' + self.env['PATH'])
        self.env['PATH'] = actual
        result = self.install('--version', VERSION)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn(b'export PATH=', result.stdout)

    # Pinned pre-0.4 executables must not receive flags their init parser rejects.
    def test_legacy_pinned_release_uses_original_init(self):
        self.archive(content=b'#!/bin/sh\ncase "$1" in --version) echo "lilbox 0.3.0" ;; init) [ "$#" = 1 ] ;; *) exit 2 ;; esac\n')
        result = self.install('--version', VERSION)
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.install('--version', VERSION, '--overwrite-profiles')
        self.assert_failed(result)
        self.assertIn(b'requires lilbox 0.4.0', result.stderr)

    # Give the installer a controlling terminal so its /dev/tty prompt is exercised.
    def interactive_install(self, version, reply):
        pid, master = pty.fork()
        if pid == 0:
            os.execve('/bin/sh', ['sh', str(ROOT / 'install.sh'), '--prefix', str(self.prefix),
                                 '--version', version], self.env)
        chunks = bytearray()
        answered = 0
        finished = False
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if not select.select([master], [], [], 0.2)[0]:
                    continue
                try:
                    chunk = os.read(master, 4096)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break
                if not chunk:
                    break
                chunks.extend(chunk)
                while chunks.count(b'[y/N]') > answered:
                    choice = reply[answered] if isinstance(reply, list) else reply
                    os.write(master, choice.encode() + b'\n')
                    answered += 1
            else:
                self.fail('installer terminal prompt timed out')
            _, status = os.waitpid(pid, 0)
            finished = True
            self.assertEqual(os.waitstatus_to_exitcode(status), 0, chunks.decode())
            self.assertTrue(answered, chunks.decode())
            return bytes(chunks)
        finally:
            os.close(master)
            if not finished:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)

    # Both interactive choices must handle every shipped file, including the guide.
    def test_profile_overwrite_prompt_accept_and_decline(self):
        version = self.real_archive()
        config = self.home / '.config/lilbox'
        config.mkdir(parents=True)
        for name in ('default.toml', 'permissive.toml', 'isolated.toml', 'AGENTS.md'):
            (config / name).write_text('user custom file')
        (config / 'work.toml').write_text('custom named profile')
        self.interactive_install(version, 'n')
        for name in ('default.toml', 'permissive.toml', 'isolated.toml', 'AGENTS.md'):
            self.assertEqual((config / name).read_text(), 'user custom file')
            source = (ROOT / 'docs/LILBOX-AGENTS.md') if name == 'AGENTS.md' else profile_source(name)
            self.assertEqual((config / f'{name}.{version}').read_bytes(), source.read_bytes())
        self.interactive_install(version, 'yes')
        for name in ('default.toml', 'permissive.toml', 'isolated.toml', 'AGENTS.md'):
            source = (ROOT / 'docs/LILBOX-AGENTS.md') if name == 'AGENTS.md' else profile_source(name)
            self.assertEqual((config / name).read_bytes(), source.read_bytes())
        self.assertEqual((config / 'work.toml').read_text(), 'custom named profile')

    # One response must affect exactly one file; preserve both neighboring decisions.
    def test_profile_overwrite_prompt_mixed_choices(self):
        version = self.real_archive()
        config = self.home / '.config/lilbox'
        config.mkdir(parents=True)
        names = ('default.toml', 'permissive.toml', 'isolated.toml', 'AGENTS.md')
        for name in names:
            (config / name).write_text('user custom file')
        output = self.interactive_install(version, ['yes', 'n', 'yes', 'n'])
        self.assertEqual(output.count(b'[y/N]'), 4)
        for index, name in enumerate(names):
            if index % 2 == 0:
                self.assertEqual((config / name).read_bytes(), profile_source(name).read_bytes())
                self.assertFalse((config / f'{name}.{version}').exists())
            else:
                self.assertEqual((config / name).read_text(), 'user custom file')
                self.assertTrue((config / f'{name}.{version}').is_file())

    # Explicit overwrite must work without a terminal and leave custom names alone.
    def test_overwrite_profiles_flag_replaces_shipped_files(self):
        version = self.real_archive()
        config = self.home / '.config/lilbox'
        config.mkdir(parents=True)
        (config / 'default.toml').write_text('custom default')
        (config / 'AGENTS.md').write_text('custom guide')
        result = self.install('--version', version, '--overwrite-profiles')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((config / 'default.toml').read_bytes(), profile_source('default.toml').read_bytes())
        self.assertEqual((config / 'AGENTS.md').read_bytes(), (ROOT / 'docs/LILBOX-AGENTS.md').read_bytes())
        self.assertFalse((config / f'default.toml.{version}').exists())




class DependencyAuditTest(unittest.TestCase):
    """Exercise the advisory gate offline with API-shaped fixtures."""
    def test_active_withdrawn_and_incomplete_results(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location('dependency_audit', ROOT / 'scripts/audit-dependencies.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        lock = '\n'.join(['version = 4', '[[package]]', 'name = "example"',
                           'version = "1.0.0"', 'source = "registry+https://github.com/rust-lang/crates.io-index"'])
        def response(path, payload=None):
            if path == 'querybatch':
                return {'results': [{'vulns': [{'id': 'ACTIVE'}, {'id': 'WITHDRAWN'}]}]}
            return {'id': path.split('/')[-1], 'withdrawn': '2026-01-01' if path.endswith('WITHDRAWN') else None}
        self.assertEqual(module.audit(lock, response), [('example', '1.0.0', 'ACTIVE')])
        with self.assertRaises(ValueError):
            module.audit(lock, lambda *args: {'results': []})
        with self.assertRaises(ValueError):
            module.audit(lock, lambda *args: {'results': [{'next_page_token': 'more'}]})
        with self.assertRaises(ValueError):
            module.packages('version = 4')


class InstallerUploadTest(unittest.TestCase):
    """Exercise publication with a fake gcloud; never read credentials or upload."""
    # Copy the publisher and installer, replacing gcloud with local storage and call logging.
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='lilbox-upload-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo with spaces'
        (self.repo / 'scripts').mkdir(parents=True)
        self.script = self.repo / 'scripts/release-installer.sh'
        self.script.write_bytes((ROOT / 'scripts/release-installer.sh').read_bytes())
        self.source = self.repo / 'install.sh'
        self.source.write_bytes((ROOT / 'install.sh').read_bytes())
        self.bin = self.root / 'tools'
        self.bin.mkdir()
        self.log = self.root / 'cloud.jsonl'
        self.remote = self.root / 'uploaded.sh'
        self.env = dict(os.environ, HOME=str(self.root), TMPDIR=str(self.root),
                        CLOUDSDK_CONFIG=str(self.root / 'cloud-config'), PATH=f'{self.bin}:/usr/bin:/bin',
                        CLOUD_LOG=str(self.log), REMOTE_OBJECT=str(self.remote), INSTALL_SOURCE=str(self.source))
        gcloud = self.bin / 'gcloud'
        gcloud.write_text(f'#!{sys.executable}\n' + '''
import json, os, pathlib, sys
args = sys.argv[1:]
with open(os.environ['CLOUD_LOG'], 'a') as log:
    log.write(json.dumps(args) + '\\n')
if os.environ.get('FAIL_GCLOUD') == args[1]:
    sys.exit('simulated cloud failure')
remote = pathlib.Path(os.environ['REMOTE_OBJECT'])
if args[:2] == ['storage', 'cp']:
    if os.environ.get('EDIT_SOURCE'):
        pathlib.Path(os.environ['INSTALL_SOURCE']).write_text('# Edited during upload\\n')
    remote.write_bytes(pathlib.Path(args[2]).read_bytes())
elif args[:2] == ['storage', 'cat']:
    sys.stdout.buffer.write(b'mismatched readback' if os.environ.get('CORRUPT_READBACK') else remote.read_bytes())
else:
    sys.exit('unexpected cloud command')
''')
        gcloud.chmod(0o755)

    # Invoke the copied publisher from outside its checkout with the fake cloud environment.
    def upload(self, *args):
        return subprocess.run(['bash', str(self.script), *args], cwd=self.root, env=self.env,
                              capture_output=True, text=True, timeout=15)

    # Publish only the staged installer with exact metadata, even if its source changes.
    def test_exact_object_metadata_snapshot_and_readback(self):
        original = self.source.read_bytes()
        self.env['EDIT_SOURCE'] = '1'
        result = self.upload()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0][:2], ['storage', 'cp'])
        staged = Path(calls[0][2])
        self.assertNotEqual(staged, self.source)
        self.assertEqual(calls[0][3:], ['gs://muthanna.com/lilbox/install.sh', '--predefined-acl=publicRead',
                                      '--cache-control=public,max-age=300,must-revalidate', '--content-type=text/plain'])
        self.assertEqual(calls[1], ['storage', 'cat', 'gs://muthanna.com/lilbox/install.sh'])
        self.assertEqual(self.remote.read_bytes(), original)
        self.assertIn('Published https://mo.town/lilbox/install.sh', result.stdout)
        self.assertFalse(staged.parent.exists())

    # Upload/readback failures must return nonzero, clean staging, and omit success text.
    def test_cloud_failures_and_readback_mismatch_never_report_success(self):
        for setting, value in [('FAIL_GCLOUD', 'cp'), ('FAIL_GCLOUD', 'cat'), ('CORRUPT_READBACK', '1')]:
            with self.subTest(setting=setting, value=value):
                self.env[setting] = value
                result = self.upload()
                self.env.pop(setting)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn('Published ', result.stdout)
                self.assertEqual(list(self.root.glob('lilbox-installer-upload.*')), [])
        self.assertIn('does not match', result.stderr)

    # Shell syntax validation must reject a broken installer before any cloud operation.
    def test_invalid_script_fails_before_cloud_access(self):
        self.source.write_text('#!/bin/sh\nif then\n')
        result = self.upload()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.log.exists())
        self.assertEqual(list(self.root.glob('lilbox-installer-upload.*')), [])

    # Help and argument validation must remain free of publication side effects.
    def test_help_and_bad_arguments_do_not_publish(self):
        result = self.upload('--help')
        self.assertEqual(result.returncode, 0)
        self.assertIn('https://mo.town/lilbox/install.sh', result.stdout)
        self.assertNotEqual(self.upload('--clean').returncode, 0)
        self.assertFalse(self.log.exists())


class CheckoutMetadataTest(unittest.TestCase):
    """Catch drift in the real checkout without querying GitHub or dispatching releases."""

    # Fixture-only tests cannot detect a README layout change in the actual repository.
    def test_current_release_metadata_is_consistent(self):
        version_script = runpy.run_path(str(ROOT / 'scripts/version.py'))
        current, data = version_script['sources']()
        version_script['check_sources'](current, current, data)


class VersionFlowTest(unittest.TestCase):
    """Use real temporary Git repositories; only GitHub API/dispatch is stubbed."""
    # Create real local Git history/remotes and stub only GitHub history and dispatch.
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='lilbox-version-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.origin = self.root / 'origin.git'
        self.bin = self.root / 'tools'
        self.bin.mkdir()
        (self.bin / 'python3').symlink_to(sys.executable)
        self.log = self.root / 'dispatch.jsonl'
        self.env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
        self.env.update(HOME=str(self.root), GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null',
                        PATH=f'{self.bin}:/usr/bin:/bin', DISPATCH_LOG=str(self.log))
        self.git('init', '-b', 'main')
        self.git('config', 'user.name', 'Release Test')
        self.git('config', 'user.email', 'release@example.invalid')
        subprocess.run(['git', 'init', '--bare', str(self.origin)], env=self.env, capture_output=True, check=True)
        self.git('remote', 'add', 'origin', str(self.origin))
        (self.repo / 'scripts').mkdir()
        for name in ('version.py', 'request-release.sh'):
            (self.repo / 'scripts' / name).write_bytes((ROOT / 'scripts' / name).read_bytes())
        self.metadata('0.3.0')
        self.commit('Initial version metadata')
        self.git('tag', 'v0.2.0')
        (self.repo / 'change.txt').write_text('a real change\n')
        self.commit('Improve the sandbox launcher')
        self.git('push', '-u', 'origin', 'main', '--tags')
        gh = self.bin / 'gh'
        gh.write_text(f'#!{sys.executable}\n' + '''
import json, os, sys
args = sys.argv[1:]
if args[:2] == ['repo', 'view']:
    print('0xfe/lilbox')
elif args[0] == 'api':
    if os.environ.get('HISTORY_FAIL'):
        sys.exit('GitHub unavailable')
    print(os.environ.get('RELEASE_TAGS', ''))
elif args[:2] == ['workflow', 'run']:
    with open(os.environ['DISPATCH_LOG'], 'a') as log:
        log.write(json.dumps(args) + '\\n')
else:
    sys.exit('unexpected gh call: ' + repr(args))
''')
        gh.chmod(0o755)

    # Run Git inside the fixture with global user configuration disabled.
    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.repo, env=self.env, stderr=subprocess.PIPE).decode().strip()

    # Commit fixture changes so clean-tree checks exercise real Git state.
    def commit(self, message):
        self.git('add', '.')
        self.git('commit', '-m', message)

    # Write minimal version files, including a dependency version that must not change.
    def metadata(self, version):
        (self.repo / 'Cargo.toml').write_text(f'[package]\nname = "lilbox"\nversion = "{version}"\n\n[dependencies]\nanyhow = "1"\n')
        (self.repo / 'Cargo.lock').write_text(f'version = 4\n\n[[package]]\nname = "dependency"\nversion = "0.3.0"\n\n[[package]]\nname = "lilbox"\nversion = "{version}"\ndependencies = ["dependency"]\n')
        (self.repo / 'README.md').write_text(f'Version **{version}** supports macOS and Linux.\ninstall.sh --version {version}\n\n  lilbox {version}\n')
        (self.repo / 'CHANGELOG.md').write_text(f'# Changelog\n\n## {version} — 2026-11-08\n\n- Previous notes.\n\n## 0.2.0 — 2026-10-01\n\n- Historical notes.\n')

    # Call version preparation/checking or the release wrapper inside the fixture.
    def invoke(self, action, version=None):
        command = [sys.executable, str(self.repo / 'scripts/version.py'), action]
        if action == 'release':
            command = ['bash', str(self.repo / 'scripts/request-release.sh')]
        if version is not None:
            command.append(version)
        return subprocess.run(command, cwd=self.repo, env=self.env, capture_output=True, text=True, timeout=15)

    # Capture exact metadata bytes to detect partial writes after rejected operations.
    def snapshot(self):
        return {name: (self.repo / name).read_bytes() for name in ('Cargo.toml', 'Cargo.lock', 'README.md', 'CHANGELOG.md')}

    # Require rejection without metadata edits or a workflow dispatch.
    def assert_rejected(self, result, before):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.snapshot(), before)
        self.assertFalse(self.log.exists())

    # A default bump changes only intended metadata and leaves local/remote HEAD untouched.
    def test_default_patch_bump_updates_only_version_files_without_commit_or_push(self):
        head = self.git('rev-parse', 'HEAD')
        result = self.invoke('bump')
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in ('Cargo.toml', 'Cargo.lock', 'README.md', 'CHANGELOG.md'):
            self.assertIn('0.3.1', (self.repo / name).read_text())
        self.assertIn('name = "dependency"\nversion = "0.3.0"', (self.repo / 'Cargo.lock').read_text())
        self.assertIn('- Improve the sandbox launcher', (self.repo / 'CHANGELOG.md').read_text())
        self.assertIn('## 0.3.0 — 2026-11-08\n\n- Previous notes.', (self.repo / 'CHANGELOG.md').read_text())
        self.assertEqual(self.git('rev-parse', 'HEAD'), head)
        self.assertEqual(self.git('ls-remote', 'origin', 'refs/heads/main').split()[0], head)
        self.assertEqual(set(self.git('diff', '--name-only').splitlines()), set(self.snapshot()))
        self.assertFalse(self.log.exists())

    # Promote handwritten unreleased notes under the requested canonical version heading.
    def test_explicit_version_normalizes_v_and_preserves_unreleased_notes(self):
        path = self.repo / 'CHANGELOG.md'
        path.write_text(path.read_text().replace('# Changelog\n', '# Changelog\n\n## Unreleased\n\n- Handwritten change.\n', 1))
        self.commit('Add unreleased notes')
        result = self.invoke('bump', 'v0.4.0')
        self.assertEqual(result.returncode, 0, result.stderr)
        notes = path.read_text()
        self.assertNotIn('## Unreleased', notes)
        self.assertIn('\n\n- Handwritten change.\n', notes)
        self.assertIn('## 0.4.0 — ', notes)

    # Compare version components numerically across a multi-digit minor transition.
    def test_numeric_ordering_and_minor_rollover(self):
        self.metadata('0.9.9')
        self.commit('Prepare current version')
        result = self.invoke('bump', '0.10.0')
        self.assertEqual(result.returncode, 0, result.stderr)

    # Reject malformed or non-increasing versions without edits or shell interpretation.
    def test_invalid_equal_older_and_overflow_versions_leave_files_untouched(self):
        before = self.snapshot()
        for version in ('0.3.0', '0.2.9', '01.2.3', '1.02.3', '1.2.03', '1.2', '1.2.3.4',
                        '1.2.3-rc.1', '1.2.3+build', ' 1.2.3', '1.2.3\n', 'v', 'vv1.2.3',
                        '1.2.$(touch BAD)', '18446744073709551616.0.0'):
            with self.subTest(version=version):
                self.assert_rejected(self.invoke('bump', version), before)
        self.assertFalse((self.repo / 'BAD').exists())

    # All tag forms and GitHub release history reserve versions against reuse.
    def test_local_remote_annotated_and_github_release_reservations(self):
        before = self.snapshot()
        self.git('tag', '0.3.1')
        self.assert_rejected(self.invoke('bump'), before)
        self.git('tag', '-d', '0.3.1')
        self.git('tag', '-a', 'v0.3.1', '-m', 'Reserved version')
        self.git('push', 'origin', 'v0.3.1')
        self.git('tag', '-d', 'v0.3.1')
        self.assert_rejected(self.invoke('bump'), before)
        self.env['RELEASE_TAGS'] = 'v0.5.0\nv0.4.0'
        self.assert_rejected(self.invoke('bump', '0.4.1'), before)
        self.assert_rejected(self.invoke('bump', '0.5.0'), before)

    # Unavailable history or user changes must stop preparation before metadata mutation.
    def test_remote_failures_and_dirty_tree_fail_before_writes(self):
        before = self.snapshot()
        self.env['HISTORY_FAIL'] = '1'
        self.assert_rejected(self.invoke('bump'), before)
        self.env.pop('HISTORY_FAIL')
        self.git('remote', 'set-url', 'origin', str(self.root / 'missing.git'))
        self.assert_rejected(self.invoke('bump'), before)
        self.git('remote', 'set-url', 'origin', str(self.origin))
        (self.repo / 'untracked').write_text('user work')
        self.assert_rejected(self.invoke('bump'), before)
        self.assertEqual((self.repo / 'untracked').read_text(), 'user work')

    # A stale README reference must abort the entire proposed version update.
    def test_inconsistent_metadata_fails_without_partial_bump(self):
        readme = self.repo / 'README.md'
        readme.write_text(readme.read_text().replace('Version **0.3.0**', 'Version **0.1.0**'))
        self.commit('Inconsistent docs fixture')
        self.assert_rejected(self.invoke('bump'), self.snapshot())

    # Removing a presentation-only banner must not block version preparation or release.
    def test_readme_without_banner_can_bump_and_release(self):
        readme = self.repo / 'README.md'
        readme.write_text(readme.read_text().replace('  lilbox 0.3.0\n', ''))
        self.commit('Remove startup banner example')
        result = self.invoke('bump')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn('  lilbox ', readme.read_text())
        self.commit('Prepare next patch')
        self.git('push', 'origin', 'main')
        result = self.invoke('release')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('version=0.3.1', self.log.read_text())

    # Optional banners still need to match, and a duplicate cannot hide a stale version.
    def test_stale_or_duplicate_banner_is_rejected(self):
        readme = self.repo / 'README.md'
        original = readme.read_text()
        for contents in (original.replace('  lilbox 0.3.0', '  lilbox 0.2.0'),
                         original + '  lilbox 0.2.0\n'):
            with self.subTest(contents=contents):
                readme.write_text(contents)
                self.commit('Invalid banner fixture')
                result = self.invoke('bump')
                self.assert_rejected(result, self.snapshot())
                self.assertIn('README startup banner example', result.stderr)

    # Releasing the prepared version dispatches precisely the pushed HEAD and version.
    def test_release_current_dispatches_exact_commit_without_mutation(self):
        before = self.snapshot()
        result = self.invoke('release')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.snapshot(), before)
        args = json.loads(self.log.read_text())
        self.assertEqual(args, ['workflow', 'run', 'release.yml', '--repo', '0xfe/lilbox', '--ref', 'main',
                                '-f', 'commit=' + self.git('rev-parse', 'HEAD'), '-f', 'version=0.3.0'])

    # A newer version requires preparation, manual commit, and push before dispatch.
    def test_release_new_version_stops_for_manual_commit_and_push(self):
        result = self.invoke('release', 'v0.4.0')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('commit and push', result.stdout)
        self.assertFalse(self.log.exists())
        self.assert_rejected(self.invoke('release', '0.4.0'), self.snapshot())
        self.commit('Prepare v0.4.0')
        self.assert_rejected(self.invoke('release', '0.4.0'), self.snapshot())
        self.git('push', 'origin', 'main')
        result = self.invoke('release', '0.4.0')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('version=0.4.0', self.log.read_text())

    # Reject publication from the wrong branch or with a stale/reserved version.
    def test_release_rejects_feature_branch_older_versions_and_existing_releases(self):
        before = self.snapshot()
        self.assert_rejected(self.invoke('release', '0.2.9'), before)
        self.git('switch', '-c', 'feature')
        self.assert_rejected(self.invoke('release'), before)
        self.git('switch', 'main')
        self.env['RELEASE_TAGS'] = 'v0.3.0'
        self.assert_rejected(self.invoke('release'), before)

    # Revalidate exact version text and fresh reservations at the workflow boundary.
    def test_workflow_check_rejects_noncanonical_mismatched_or_newly_reserved_versions(self):
        before = self.snapshot()
        result = self.invoke('check', '0.3.0')
        self.assertEqual(result.returncode, 0, result.stderr)
        for version in ('v0.3.0', '0.4.0', '00.3.0'):
            self.assert_rejected(self.invoke('check', version), before)
        self.env['RELEASE_TAGS'] = 'v0.3.1'
        self.assert_rejected(self.invoke('check', '0.3.0'), before)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True, help='Built native lilbox executable for installer integration tests')
    options, test_args = parser.parse_known_args()
    REAL_BINARY = options.binary.resolve(strict=True)
    unittest.main(argv=[sys.argv[0], *test_args])
