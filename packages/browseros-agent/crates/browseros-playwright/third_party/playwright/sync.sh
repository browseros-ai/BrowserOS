#!/usr/bin/env bash
# Re-vendors Playwright's page engine from a local Playwright clone at a release tag:
#   ./sync.sh ~/code/playwright v1.65.0
#
# Files are read with `git show <commit>:<path>`, so the clone's checkout is never touched and
# a sparse or shallow clone works as long as it has the tag. Every file is copied byte for byte;
# nothing here is ever edited by hand. README.md says why each file is on the list and what is
# deliberately left out.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <playwright-checkout> <tag>" >&2
  exit 2
fi
checkout="$1"
tag="$2"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# The runtime import closure of packages/injected/src/injectedScript.ts (type-only imports are
# erased by the bundler, so their files are not needed), minus highlight.ts, highlight.css and
# consoleApi.ts, which the engine build replaces with stubs. Then urlMatch.ts for the agent
# runtime, and the US keyboard table and macOS editing commands for the Rust keyboard port. An
# upstream import that is not on this list makes the engine build fail, so a missed file cannot
# ship silently.
FILES=(
  LICENSE
  NOTICE
  packages/injected/src/ariaSnapshot.ts
  packages/injected/src/ariaSnapshotDistiller.ts
  packages/injected/src/domUtils.ts
  packages/injected/src/injectedScript.ts
  packages/injected/src/layoutSelectorUtils.ts
  packages/injected/src/roleSelectorEngine.ts
  packages/injected/src/roleUtils.ts
  packages/injected/src/selectorEvaluator.ts
  packages/injected/src/selectorGenerator.ts
  packages/injected/src/selectorUtils.ts
  packages/injected/src/utilityScript.ts
  packages/injected/src/xpathSelectorEngine.ts
  packages/isomorphic/ariaSnapshot.ts
  packages/isomorphic/ariaSnapshotRenderer.ts
  packages/isomorphic/cssParser.ts
  packages/isomorphic/cssTokenizer.ts
  packages/isomorphic/locatorGenerators.ts
  packages/isomorphic/locatorUtils.ts
  packages/isomorphic/selectorParser.ts
  packages/isomorphic/stringUtils.ts
  packages/isomorphic/urlMatch.ts
  packages/isomorphic/utilityScriptSerializers.ts
  packages/isomorphic/yaml.ts
  packages/playwright-core/src/server/macEditingCommands.ts
  packages/playwright-core/src/server/usKeyboardLayout.ts
)

commit="$(git -C "$checkout" rev-parse --verify "${tag}^{commit}")"
for file in "${FILES[@]}"; do
  mkdir -p "$here/$(dirname "$file")"
  git -C "$checkout" show "${commit}:${file}" >"$here/$file"
done
printf '%s\n%s\n' "$tag" "$commit" >"$here/VERSION"
# The manifest tests/engine_sources.rs checks the vendored bytes against, in
# `shasum -a 256 -c` format, so a hand edit after this sync fails the crate's tests.
if command -v sha256sum >/dev/null; then
  (cd "$here" && sha256sum "${FILES[@]}") >"$here/SHA256SUMS"
else
  (cd "$here" && shasum -a 256 "${FILES[@]}") >"$here/SHA256SUMS"
fi

echo "Vendored Playwright $tag ($commit). Changes against HEAD:"
git -C "$here" diff --stat -- .
git -C "$here" status --short --untracked-files=all -- .
