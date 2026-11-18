#!/usr/bin/env bash
# Re-vendors the `yaml` package that Playwright's utilsBundle.js bundles, from the npm
# registry, at the version that bundle's LICENSE names (`yaml@2.9.0` for Playwright 1.64.1):
#   ./sync.sh 2.9.0
#
# The tarball must match the registry's own `dist.integrity` before anything is copied. Every
# file is copied byte for byte; nothing here is ever edited by hand. README.md says why these
# files and not the others.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <yaml version>" >&2
  exit 2
fi
version="$1"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The license and the package's ES module build (`browser/dist`), every module of it: its
# index re-exports them all, and the prelude's bundler keeps only what parseDocument reaches.
FILES=(
  LICENSE
  browser/dist/compose/compose-collection.js
  browser/dist/compose/compose-doc.js
  browser/dist/compose/compose-node.js
  browser/dist/compose/compose-scalar.js
  browser/dist/compose/composer.js
  browser/dist/compose/resolve-block-map.js
  browser/dist/compose/resolve-block-scalar.js
  browser/dist/compose/resolve-block-seq.js
  browser/dist/compose/resolve-end.js
  browser/dist/compose/resolve-flow-collection.js
  browser/dist/compose/resolve-flow-scalar.js
  browser/dist/compose/resolve-props.js
  browser/dist/compose/util-contains-newline.js
  browser/dist/compose/util-empty-scalar-position.js
  browser/dist/compose/util-flow-indent-check.js
  browser/dist/compose/util-map-includes.js
  browser/dist/doc/Document.js
  browser/dist/doc/anchors.js
  browser/dist/doc/applyReviver.js
  browser/dist/doc/createNode.js
  browser/dist/doc/directives.js
  browser/dist/errors.js
  browser/dist/index.js
  browser/dist/log.js
  browser/dist/nodes/Alias.js
  browser/dist/nodes/Collection.js
  browser/dist/nodes/Node.js
  browser/dist/nodes/Pair.js
  browser/dist/nodes/Scalar.js
  browser/dist/nodes/YAMLMap.js
  browser/dist/nodes/YAMLSeq.js
  browser/dist/nodes/addPairToJSMap.js
  browser/dist/nodes/identity.js
  browser/dist/nodes/toJS.js
  browser/dist/parse/cst-scalar.js
  browser/dist/parse/cst-stringify.js
  browser/dist/parse/cst-visit.js
  browser/dist/parse/cst.js
  browser/dist/parse/lexer.js
  browser/dist/parse/line-counter.js
  browser/dist/parse/parser.js
  browser/dist/public-api.js
  browser/dist/schema/Schema.js
  browser/dist/schema/common/map.js
  browser/dist/schema/common/null.js
  browser/dist/schema/common/seq.js
  browser/dist/schema/common/string.js
  browser/dist/schema/core/bool.js
  browser/dist/schema/core/float.js
  browser/dist/schema/core/int.js
  browser/dist/schema/core/schema.js
  browser/dist/schema/json/schema.js
  browser/dist/schema/tags.js
  browser/dist/schema/yaml-1.1/binary.js
  browser/dist/schema/yaml-1.1/bool.js
  browser/dist/schema/yaml-1.1/float.js
  browser/dist/schema/yaml-1.1/int.js
  browser/dist/schema/yaml-1.1/merge.js
  browser/dist/schema/yaml-1.1/omap.js
  browser/dist/schema/yaml-1.1/pairs.js
  browser/dist/schema/yaml-1.1/schema.js
  browser/dist/schema/yaml-1.1/set.js
  browser/dist/schema/yaml-1.1/timestamp.js
  browser/dist/stringify/foldFlowLines.js
  browser/dist/stringify/stringify.js
  browser/dist/stringify/stringifyCollection.js
  browser/dist/stringify/stringifyComment.js
  browser/dist/stringify/stringifyDocument.js
  browser/dist/stringify/stringifyNumber.js
  browser/dist/stringify/stringifyPair.js
  browser/dist/stringify/stringifyString.js
  browser/dist/util.js
  browser/dist/visit.js
)

curl -fsSL "https://registry.npmjs.org/yaml/$version" -o "$work/meta.json"
field() { node -e "process.stdout.write(require(process.argv[1]).dist[process.argv[2]])" "$work/meta.json" "$1"; }
tarball="$(field tarball)"
integrity="$(field integrity)"
curl -fsSL "$tarball" -o "$work/yaml.tgz"
actual="sha512-$(openssl dgst -sha512 -binary "$work/yaml.tgz" | openssl base64 -A)"
if [[ "$actual" != "$integrity" ]]; then
  echo "sync.sh: $tarball is $actual, the registry says $integrity" >&2
  exit 1
fi
mkdir -p "$work/package"
tar -xzf "$work/yaml.tgz" -C "$work"
# A file the new release added to browser/dist must be listed above, or the bundle would
# reach a module this tree does not hold.
unlisted="$(cd "$work/package" && find browser/dist -type f | LC_ALL=C sort | while read -r file; do
  printf '%s\n' "${FILES[@]}" | grep -qxF "$file" || echo "$file"
done)"
if [[ -n "$unlisted" ]]; then
  echo "sync.sh: yaml $version ships files FILES does not list:" >&2
  echo "$unlisted" >&2
  exit 1
fi

rm -rf "$here/browser"
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

echo "Vendored yaml $version. Changes against HEAD:"
git -C "$here" diff --stat -- .
git -C "$here" status --short --untracked-files=all -- .
