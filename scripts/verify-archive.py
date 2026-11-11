#!/usr/bin/env python3
"""Verify a release archive and exercise its executable on the matching host."""
import hashlib
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile


def verify(archive):
    """Check checksum/layout, then run the packaged CLI with disposable configs."""
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    assert Path(str(archive) + '.sha256').read_text() == f'{digest}  {archive.name}\n'
    name = archive.name.removesuffix('.tar.gz')
    version = name.split('-')[1]
    with tempfile.TemporaryDirectory(prefix='lilbox-archive-') as tmp:
        root = Path(tmp).resolve()
        with tarfile.open(archive) as bundle:
            for member in bundle.getmembers():
                parts = Path(member.name).parts
                assert parts and parts[0] == name and '..' not in parts
                assert member.isfile() or member.isdir(), 'Links and special files are not allowed'
            # Extract only the constrained regular files/directories, including
            # on Python versions predating tarfile's data filter.
            for member in bundle.getmembers():
                destination = root / member.name
                if member.isdir():
                    destination.mkdir(parents=True, exist_ok=True)
                else:
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_bytes(bundle.extractfile(member).read())
                    destination.chmod(member.mode & 0o777)
        package = root / name
        for entry in ('lilbox', 'README.md', 'LICENSE', 'CHANGELOG.md', 'install.sh',
                      'profiles/default.toml', 'profiles/isolated.toml',
                      'examples/README.md', 'docs/design.md', 'docs/profiles.md',
                      'docs/development.md', 'docs/security.md'):
            assert (package / entry).is_file(), f'Missing {entry}'
        binary = package / 'lilbox'
        for flag in ('-v', '-V', '--version'):
            result = subprocess.run([binary, flag], env={}, capture_output=True, check=True)
            assert result.stdout == f'lilbox {version}\n'.encode() and not result.stderr
        # These are real sandbox launches and require a matching, permitted host.
        # Isolate init and scratch creation from the developer's actual home.
        home, work, config = (root / part for part in ('home', 'work', 'config'))
        for path in (home, work, config):
            path.mkdir()
        env = {'HOME': str(home), 'PATH': '/usr/bin:/bin', 'TERM': 'dumb'}
        command = [str(binary), '--config-dir', str(config)]
        subprocess.run(command + ['init'], env=env, cwd=work, check=True, capture_output=True)
        for profile in ('default', 'isolated'):
            result = subprocess.run(command + ['-p', profile, 'exec', '/bin/sh', '-c', 'printf archive-ok'],
                                    env=env, cwd=work, capture_output=True, check=True)
            assert result.stdout == b'archive-ok'
            assert f'lilbox {version}'.encode() in result.stderr
            assert f'profile  {profile}'.encode() in result.stderr
            assert b'\x1b' not in result.stderr
    print(f'Verified checksum, contents, version, and sandbox launches: {archive.name}')


if __name__ == '__main__':
    if len(sys.argv) < 2:
        sys.exit('Usage: verify-archive.py ARCHIVE.tar.gz [...]')
    for argument in sys.argv[1:]:
        verify(Path(argument))
