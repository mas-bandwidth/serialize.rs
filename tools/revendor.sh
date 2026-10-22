#!/bin/sh
# tools/revendor.sh <tag>: copy the vendored format files from mas-bandwidth/serialize at a
# release tag, verbatim, then show what changed. The spec-sync CI job diffs the same four
# paths against upstream and fails on drift.
#
#   STANDARD.md            the wire format, every rule citing its vectors
#   IMPLEMENTING.md        the practice STANDARD.md links
#   conformance/           the shared corpus, run by tests/conformance.rs
#   conformance-controls/  the negative controls and their .expected lists
#
# A changed corpus may need runner work: a new OPERATION needs a dispatch arm, and the runner
# fails on a vector it cannot drive rather than skipping it.
set -eu
if [ "$#" -ne 1 ]; then
    echo "usage: tools/revendor.sh <tag>   (for example v2.0.0)" >&2
    exit 2
fi
tag="$1"
root="$(cd "$(dirname "$0")/.." && pwd)"
upstream="$(mktemp -d)"
trap 'rm -rf "$upstream"' EXIT
git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$tag" https://github.com/mas-bandwidth/serialize.git "$upstream"
echo "upstream: mas-bandwidth/serialize $tag @ $(git -C "$upstream" rev-parse HEAD)"
cp "$upstream/STANDARD.md" "$upstream/IMPLEMENTING.md" "$root/"
rm -rf "$root/conformance" "$root/conformance-controls"
cp -R "$upstream/conformance" "$upstream/conformance-controls" "$root/"
git -C "$root" status --short -- STANDARD.md IMPLEMENTING.md conformance conformance-controls
