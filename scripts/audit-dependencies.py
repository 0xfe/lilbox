#!/usr/bin/env python3
"""Fail closed on active OSV advisories for every registry package in Cargo.lock.

Uses Python 3.9+ standard libraries. No blanket ignore flag or cached success;
unavailable/invalid advisory responses fail the check and release verification.
"""
import json
from pathlib import Path
import re
import sys
import urllib.request


def packages(lock):
    """Read the three plain Cargo-generated strings needed for registry queries."""
    result = []
    for block in lock.split('[[package]]')[1:]:
        fields = dict(re.findall(r'^(name|version|source) = "([^"\n]+)"$', block, re.M))
        if fields.get('source', '').startswith('registry+'):
            if not fields.get('name') or not fields.get('version'):
                raise ValueError('Registry entry is missing name/version')
            result.append({'package': {'name': fields['name'], 'ecosystem': 'crates.io'},
                           'version': fields['version']})
    if not result:
        raise ValueError('No registry packages found in Cargo.lock')
    return result


def request(path, payload=None):
    """Read one HTTPS API response with bounded waiting; errors propagate."""
    data = None if payload is None else json.dumps(payload).encode()
    req = urllib.request.Request('https://api.osv.dev/v1/' + path, data=data,
                                 headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.load(response)


def audit(lock, fetch=request):
    """Check all locked platforms, ignoring only explicitly withdrawn advisories."""
    queries = packages(lock)
    results = fetch('querybatch', {'queries': queries})['results']
    if len(results) != len(queries):
        raise ValueError('Incomplete OSV response')
    findings = set()
    for query, result in zip(queries, results):
        if result.get('next_page_token'):
            raise ValueError('Incomplete paginated OSV response')
        for vulnerability in result.get('vulns', []):
            identifier = vulnerability['id']
            if not re.fullmatch(r'[A-Za-z0-9_-]+', identifier):
                raise ValueError('Invalid advisory identifier')
            detail = fetch('vulns/' + identifier)
            if detail.get('id') != identifier:
                raise ValueError('Advisory identity mismatch')
            if not detail.get('withdrawn'):
                findings.add((query['package']['name'], query['version'], identifier))
    return sorted(findings)


def main():
    """Print package/advisory identifiers without dependency source credentials."""
    root = Path(__file__).resolve().parent.parent
    findings = audit((root / 'Cargo.lock').read_text())
    for name, version, identifier in findings:
        print(f'{name} {version}: {identifier}', file=sys.stderr)
    if findings:
        return 1
    print('No active OSV advisories in Cargo.lock.')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except Exception as error:
        sys.exit(f'Dependency advisory check failed: {error}')
