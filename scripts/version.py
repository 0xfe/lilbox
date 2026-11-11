#!/usr/bin/env python3
"""Prepare and validate stable releases without committing or pushing user changes."""
import argparse
from datetime import date
from pathlib import Path
import re
import subprocess
import sys

# Anchor subprocesses and metadata edits to this checkout, regardless of caller CWD.
ROOT = Path(__file__).resolve().parent.parent
# Stable SemVer only; component bounds are checked separately before integer use.
VERSION_RE = r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)'
# The complete write set for version preparation; used for validation and rollback.
FILES = ('Cargo.toml', 'Cargo.lock', 'README.md', 'CHANGELOG.md')


class ReleaseError(Exception):
    """An actionable validation or command failure reported without a traceback."""
    pass


def run(*args):
    """Run argv directly in the checkout and surface captured command failures."""
    result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True)
    if result.returncode:
        raise ReleaseError(f"{' '.join(args[:2])} failed: {result.stderr.strip() or result.stdout.strip()}")
    return result.stdout.strip()


def version_tuple(value, allow_v=True):
    """Validate stable syntax and Cargo's u64 bounds, returning sortable integers."""
    canonical = value[1:] if allow_v and value.startswith('v') else value
    if not re.fullmatch(VERSION_RE, canonical):
        raise ReleaseError('Use a stable X.Y.Z version with no leading zeros, prerelease suffix, or build metadata.')
    # Match Cargo/SemVer's unsigned 64-bit version components.
    if any(len(part) > 20 for part in canonical.split('.')):
        raise ReleaseError('Version component exceeds Cargo\'s u64 limit.')
    numbers = tuple(map(int, canonical.split('.')))
    if any(number > 2**64 - 1 for number in numbers):
        raise ReleaseError('Version component exceeds Cargo\'s u64 limit.')
    return numbers


def normalize(value):
    """Remove an optional tag prefix and return canonical X.Y.Z text."""
    return '.'.join(map(str, version_tuple(value)))


def table(text, header, name=None):
    """Locate one literal TOML version and its offsets without reformatting the file."""
    pattern = r'(?ms)^' + re.escape(header) + r'\s*\n.*?(?=^\[|\Z)'
    matches = [match for match in re.finditer(pattern, text)
               if name is None or re.search(r'^name\s*=\s*"' + re.escape(name) + r'"\s*$', match[0], re.M)]
    if len(matches) != 1:
        raise ReleaseError(f'Expected exactly one {header} table' + (f' for {name}' if name else ''))
    block = matches[0]
    versions = list(re.finditer(r'^version\s*=\s*"([^"\n]+)"[^\n]*$', block[0], re.M))
    if len(versions) != 1:
        raise ReleaseError(f'Expected one literal version in {header}')
    value = versions[0]
    return value[1], (block.start() + value.start(1), block.start() + value.end(1))


def sources():
    """Read the intended write set and require agreement between Cargo versions."""
    data = {}
    for name in FILES:
        path = ROOT / name
        if path.is_symlink() or not path.is_file():
            raise ReleaseError(f'{name} must be a regular file, not a symlink.')
        data[name] = path.read_text()
    current, _ = table(data['Cargo.toml'], '[package]')
    version_tuple(current, allow_v=False)
    locked, _ = table(data['Cargo.lock'], '[[package]]', 'lilbox')
    if locked != current:
        raise ReleaseError('Cargo.lock version differs from Cargo.toml; refresh the lockfile first.')
    return current, data


def clean_tree():
    """Refuse preparation/publication while tracked or untracked user changes exist."""
    if run('git', 'status', '--porcelain'):
        raise ReleaseError('Commit or stash your changes first; version preparation requires a clean working tree.')


def history():
    """Collect local/remote tags and GitHub reservations, including draft releases."""
    # Fail closed if either remote tags or GitHub releases cannot be read.
    origin = run('git', 'remote', 'get-url', 'origin')
    repository = run('gh', 'repo', 'view', origin, '--json', 'nameWithOwner', '--jq', '.nameWithOwner')
    tags = {}
    local = run('git', 'for-each-ref', '--format=%(refname:strip=2) %(objectname) %(*objectname)', 'refs/tags')
    for line in local.splitlines():
        fields = line.split()
        tags[fields[0]] = fields[-1]
    remote = run('git', 'ls-remote', '--tags', 'origin')
    for line in remote.splitlines():
        sha, ref = line.split()
        name = ref.removeprefix('refs/tags/')
        if name.endswith('^{}'):
            tags[name[:-3]] = sha
        else:
            tags.setdefault(name, sha)
    releases = run('gh', 'api', '--paginate', f'repos/{repository}/releases', '--jq', '.[].tag_name')
    reserved = {}
    for name in list(tags) + releases.splitlines():
        try:
            reserved[name] = version_tuple(name)
        except ReleaseError:
            continue  # Non-stable/unrelated tags do not order stable releases.
    return repository, tags, reserved


def available(version, reserved):
    """Require a version above every known stable tag or release, not just HEAD's."""
    target = version_tuple(version)
    if reserved:
        latest = max(reserved, key=reserved.get)
        if target <= reserved[latest]:
            raise ReleaseError(f'Version {version} must be newer than existing tag/release {latest}; versions are never overwritten.')


def readme_version(text, current, target):
    """Update required version references and validate any optional banner example."""
    patterns = [
        ('version heading', r'(Version \*\*)([^*\s]+)(\*\*)', True),
        ('pinned installer example', r'(--version )(\S+)', True),
        ('startup banner example', r'(?m)^(  lilbox )([0-9]\S*)$', False),
    ]
    for label, pattern, required in patterns:
        matches = list(re.finditer(pattern, text))
        if not matches and not required:
            continue
        if len(matches) != 1 or matches[0][2] != current:
            raise ReleaseError(f'README {label} must contain exactly one version reference matching {current}.')
        # Replace only the captured version; historical prose is not release metadata.
        match = matches[0]
        text = text[:match.start(2)] + target + text[match.end(2):]
    return text


def release_notes(tags, repository, version):
    """Draft notes from commits after the highest stable tag reachable from HEAD."""
    bases = []
    for name, sha in tags.items():
        try:
            number = version_tuple(name)
        except ReleaseError:
            continue
        ancestor = subprocess.run(['git', 'merge-base', '--is-ancestor', sha, 'HEAD'],
                                  cwd=ROOT, capture_output=True)
        if ancestor.returncode == 0:
            bases.append((number, sha))
    revision = max(bases)[1] + '..HEAD' if bases else 'HEAD'
    subjects = run('git', 'log', '--no-merges', '--format=%s', revision).splitlines()
    if subjects:
        return '\n'.join('- ' + subject for subject in subjects) + '\n'
    return f'- See the [release notes](https://github.com/{repository}/releases/tag/v{version}) for details.\n'


def bump(version, current, data, repository, tags):
    """Prepare all version edits with rollback on write failure; never commit/push."""
    if version_tuple(version) <= version_tuple(current):
        raise ReleaseError(f'New version must be greater than the current version {current}.')
    updated = dict(data)
    for name, header, package in [('Cargo.toml', '[package]', None), ('Cargo.lock', '[[package]]', 'lilbox')]:
        _, (start, end) = table(data[name], header, package)
        updated[name] = data[name][:start] + version + data[name][end:]
    updated['README.md'] = readme_version(data['README.md'], current, version)
    changelog = data['CHANGELOG.md']
    if re.search(r'^##\s+v?' + re.escape(version) + r'(?:\s|$)', changelog, re.M):
        raise ReleaseError(f'CHANGELOG.md already contains version {version}.')
    heading = f'## {version} — {date.today().isoformat()}'
    unreleased = re.search(r'^## (?:\[Unreleased\]|Unreleased)[ \t]*$', changelog, re.M)
    if unreleased:
        updated['CHANGELOG.md'] = changelog[:unreleased.start()] + heading + changelog[unreleased.end():]
    else:
        first_entry = re.search(r'^## ', changelog, re.M)
        if not first_entry:
            raise ReleaseError('CHANGELOG.md must contain a release heading.')
        at = first_entry.start()
        updated['CHANGELOG.md'] = changelog[:at] + heading + '\n\n' + release_notes(tags, repository, version) + '\n' + changelog[at:]
    # Construct and validate every edit before writing. Restore originals on I/O failure.
    written = []
    try:
        for name in FILES:
            written.append(name)
            (ROOT / name).write_text(updated[name])
    except OSError:
        for name in written:
            (ROOT / name).write_text(data[name])
        raise
    print(f'Prepared {current} -> {version}: ' + ', '.join(FILES))
    print('Review the changelog, run just verify, then commit and push these changes manually.')
    print(f'Afterwards, run: just release {version}')


def check_sources(version, current, data):
    """Require Cargo, README examples, and dated release notes to match the request."""
    if version != current:
        raise ReleaseError(f'Requested {version}, but Cargo.toml contains {current}.')
    readme_version(data['README.md'], current, current)
    if not re.search(r'^## ' + re.escape(version) + r' — \d{4}-\d{2}-\d{2}\s*$', data['CHANGELOG.md'], re.M):
        raise ReleaseError(f'Missing dated CHANGELOG.md entry for {version}.')


def main():
    """Prepare a newer version, validate CI inputs, or dispatch the pushed version."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['bump', 'release', 'check'])
    parser.add_argument('version', nargs='?')
    args = parser.parse_args()
    current, data = sources()
    requested = args.version or None
    if args.action == 'bump' and requested is None:
        major, minor, patch = version_tuple(current)
        requested = f'{major}.{minor}.{patch + 1}'
    version = normalize(requested) if requested else current
    if args.action == 'check':
        version_tuple(args.version or '', allow_v=False)
        check_sources(version, current, data)
    else:
        clean_tree()
        if args.action == 'release' and run('git', 'symbolic-ref', '--short', 'HEAD') != 'main':
            raise ReleaseError('Request releases from main.')
        if version_tuple(version) < version_tuple(current):
            raise ReleaseError(f'Version cannot be lower than current version {current}.')
        if args.action == 'bump' and version == current:
            raise ReleaseError(f'New version must be greater than current version {current}.')
    # Recheck reservations for every action, including workflow validation: a tag
    # or draft may have appeared since local preparation.
    repository, tags, reserved = history()
    available(version, reserved)
    if args.action == 'bump' or (args.action == 'release' and version != current):
        bump(version, current, data, repository, tags)
        return
    check_sources(version, current, data)
    if args.action == 'check':
        print(f'Validated release {version}.')
        return
    # Bind publication to the reviewed, pushed main commit rather than allowing
    # the workflow to silently build a different revision.
    commit = run('git', 'rev-parse', 'HEAD')
    remote = run('git', 'ls-remote', '--exit-code', 'origin', 'refs/heads/main').split()[0]
    if commit != remote:
        raise ReleaseError('Push main and synchronize with origin/main before releasing.')
    run('gh', 'workflow', 'run', 'release.yml', '--repo', repository, '--ref', 'main',
        '-f', f'commit={commit}', '-f', f'version={version}')
    print(f'Requested v{version} from {commit}. Publication follows successful verification of all four targets.')
    print(f'Follow progress: gh run list --repo {repository} --workflow release.yml')


if __name__ == '__main__':
    try:
        main()
    except (ReleaseError, OSError, ValueError) as error:
        sys.exit(f'lilbox release: {error}')
