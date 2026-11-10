#!/usr/bin/env bash
# Re-vendors the `mime` package that Playwright's utilsBundle.js bundles, from the npm
# registry, at the version that bundle's LICENSE names (`mime@4.1.0` for Playwright 1.64.1):
#   ./sync.sh 4.1.0
#
# The tarball must match the registry's own `dist.integrity` before anything is copied. Every
# file is copied byte for byte; nothing here is ever edited by hand. README.md says why each
# file is on the list.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <mime version>" >&2
  exit 2
fi
version="$1"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The class with `getType`, the instance Playwright's bundle builds (`index.ts`), its two
# tables, and the license.
FILES=(
  LICENSE
  src/Mime.ts
  src/index.ts
  types/other.ts
  types/standard.ts
)

curl -fsSL "https://registry.npmjs.org/mime/$version" -o "$work/meta.json"
field() { node -e "process.stdout.write(require(process.argv[1]).dist[process.argv[2]])" "$work/meta.json" "$1"; }
tarball="$(field tarball)"
integrity="$(field integrity)"
curl -fsSL "$tarball" -o "$work/mime.tgz"
actual="sha512-$(openssl dgst -sha512 -binary "$work/mime.tgz" | openssl base64 -A)"
if [[ "$actual" != "$integrity" ]]; then
  echo "sync.sh: $tarball is $actual, the registry says $integrity" >&2
  exit 1
fi
mkdir -p "$work/package"
tar -xzf "$work/mime.tgz" -C "$work"

for file in "${FILES[@]}"; do
  mkdir -p "$here/$(dirname "$file")"
  cp "$work/package/$file" "$here/$file"
done
printf '%s\n%s\n' "$version" "$integrity" >"$here/VERSION"
# The manifest tests/engine_sources.rs checks the vendored bytes against, in
# `shasum -a 256 -c` format, so a hand edit after this sync fails the crate's tests.
if command -v sha256sum >/dev/null; then
  (cd "$here" && sha256sum "${FILES[@]}") >"$here/SHA256SUMS"
else
  (cd "$here" && shasum -a 256 "${FILES[@]}") >"$here/SHA256SUMS"
fi

echo "Vendored mime $version. Changes against HEAD:"
git -C "$here" diff --stat -- .
git -C "$here" status --short --untracked-files=all -- .
