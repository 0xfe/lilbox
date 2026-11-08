#!/usr/bin/env bash
set -euo pipefail
CDPATH='' cd -- "$(dirname "$0")/.."

command -v gh >/dev/null || { echo 'Install GitHub CLI and run gh auth login first.' >&2; exit 1; }
[[ -z $(git status --porcelain) ]] || { echo 'Commit all changes before requesting a release.' >&2; exit 1; }
[[ $(git symbolic-ref --short HEAD) == main ]] || { echo 'Request releases from main.' >&2; exit 1; }
commit=$(git rev-parse HEAD)
remote_commit=$(git ls-remote --exit-code origin refs/heads/main | cut -f1)
[[ "$commit" == "$remote_commit" ]] || { echo 'Push main and synchronize with origin/main before releasing.' >&2; exit 1; }
package_id=$(cargo pkgid --offline)
version=${package_id##*[#@]}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Releases require a stable X.Y.Z version.' >&2; exit 1; }
existing_tag=$(git ls-remote origin "refs/tags/v$version")
[[ -z "$existing_tag" ]] || { echo "Tag v$version already exists; bump the version first." >&2; exit 1; }
origin_url=$(git remote get-url origin)
repository=$(gh repo view "$origin_url" --json nameWithOwner --jq .nameWithOwner)
gh workflow run release.yml --repo "$repository" --ref main \
    -f "commit=$commit" -f "version=$version"
echo "Requested v$version from $commit. Publication follows successful verification of all four targets."
echo "Follow progress: gh run list --repo $repository --workflow release.yml"
