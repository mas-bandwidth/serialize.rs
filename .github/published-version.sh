#!/bin/sh
# Resolve what the crates.io index actually holds for this crate.
#
# Two jobs need this answer and it has to be the same answer in both: the
# published-version job, which reports the gap between the manifest and the
# index, and the consumer job, which installs the exact version the index
# holds. Asking twice in two places is how two answers come apart, so both
# ask here.
#
# Prints the answer for the log, and writes crate, manifest and published to
# GITHUB_OUTPUT when running under Actions. An empty published means the index
# holds no unyanked version of the crate.
set -eu

crate=$(sed -n 's/^name = "\(.*\)"/\1/p' Cargo.toml | head -1)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
if [ -z "$crate" ] || [ -z "$version" ]; then
  echo "::error::could not read name and version out of Cargo.toml"
  exit 1
fi

# the sparse index path: 1/, 2/, 3/<first letter>/, then <first two>/<next two>/
lower=$(printf '%s' "$crate" | tr '[:upper:]' '[:lower:]')
case ${#lower} in
  1) path="1/$lower" ;;
  2) path="2/$lower" ;;
  3) path="3/$(printf '%s' "$lower" | cut -c1)/$lower" ;;
  *) path="$(printf '%s' "$lower" | cut -c1-2)/$(printf '%s' "$lower" | cut -c3-4)/$lower" ;;
esac

index="${TMPDIR:-/tmp}/crates-io-index.json"
published=""
if curl -sS -f "https://index.crates.io/$path" -o "$index"; then
  published=$(jq -r 'select(.yanked | not) | .vers' "$index" | sort -V | tail -n1)
fi

echo "crate:      $crate"
echo "manifest:   $version"
if [ -z "$published" ]; then
  echo "published:  nothing (the index holds no unyanked version of $crate)"
else
  echo "published:  $published"
fi

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  {
    echo "crate=$crate"
    echo "manifest=$version"
    echo "published=$published"
  } >> "$GITHUB_OUTPUT"
fi
