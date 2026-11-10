#!/usr/bin/env python3
"""Validate the static site's local assets before publishing; standard library only."""
import hashlib
from html.parser import HTMLParser
from pathlib import Path
import re
import sys
from urllib.parse import urlsplit


class Page(HTMLParser):
    def __init__(self):
        super().__init__()
        self.references = []
        self.canonical = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == 'script' and 'src' in attrs:
            raise ValueError('Keep JavaScript embedded in index.html.')
        for attribute in ('src', 'href'):
            value = attrs.get(attribute, '')
            if value and not urlsplit(value).scheme and not value.startswith('#'):
                self.references.append(value)
        if tag == 'link' and attrs.get('rel') == 'canonical':
            self.canonical.append(attrs.get('href'))


def validate(root):
    index = root / 'index.html'
    if index.is_symlink():
        raise ValueError('index.html must not be a symlink.')
    html = index.read_text(encoding='utf-8')
    page = Page()
    page.feed(html)
    if page.canonical != ['https://mo.town/lilbox/']:
        raise ValueError('Expected the public mo.town/lilbox/ canonical URL.')
    if 'curl -fsSL https://mo.town/lilbox/install.sh | sh' not in html:
        raise ValueError('Missing the public installation command.')
    assets = root / 'assets'
    if assets.is_symlink() or not assets.is_dir():
        raise ValueError('Expected a regular assets directory.')
    files = list(assets.iterdir())
    if not files:
        raise ValueError('The site needs its icon.')
    for path in files:
        match = re.fullmatch(r'[a-z0-9-]+-([0-9a-f]{12})\.(png|webp)', path.name)
        if path.is_symlink() or not path.is_file() or not match:
            raise ValueError(f'Expected a fingerprinted PNG/WebP file: {path.name}')
        digest = hashlib.sha256(path.read_bytes()).hexdigest()[:12]
        if match[1] != digest:
            raise ValueError(f'Asset changed: rename {path.name} with fingerprint {digest} and update index.html.')
    for reference in page.references:
        if not re.fullmatch(r'assets/[a-z0-9-]+\.(png|webp)', reference):
            raise ValueError(f'Unexpected local reference: {reference}')
        if not (root / reference).is_file():
            raise ValueError(f'Missing asset: {reference}')
    if not page.references:
        raise ValueError('Missing the local icon reference.')
    print('Website HTML, public URLs, and asset fingerprints validated.')


if __name__ == '__main__':
    try:
        validate(Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent / 'www')
    except (OSError, ValueError) as error:
        sys.exit(f'lilbox website: {error}')
