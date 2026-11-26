#!/usr/bin/env bash
# The zero-conflict fuzzer's site run (design 10.3): every site of a URL list, one tab at a time,
# through neo-snapshot-apc's live test, in a fresh headless browser (`tools/test-browser.mjs`:
# throwaway profile, mock keychain). It runs on this machine only: each site's captures and
# oracle are real pages' text, so they go to a private folder outside every git work tree and
# never to the remote pool, which uploads packages/browseros-agent. README.md beside it says more.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
agent="$(cd "$here/../../../.." && pwd -P)"
test_name=site_lines_reach_their_elements

usage() {
  cat <<'EOF'
run.sh [--sites FILE] [--out DIR] [--browser PATH]

  --sites FILE    the URL list (default: sites.txt beside this script)
  --out DIR       the private folder (default: $HOME/.neo-zero-conflict/<UTC timestamp>)
  --browser PATH  the browser binary, as NEO_E2E_BROWSER (default: the launcher's own default)
EOF
}

die() {
  echo "run.sh: $*" >&2
  exit 2
}

# An absolute, symlink-resolved form of $1, which need not exist yet: its deepest existing
# ancestor resolved, the rest appended. A `..` in the part that does not exist would escape that
# resolution, so it is refused.
absolute() {
  local path="$1" rest=""
  case "$path" in
    /*) ;;
    *) path="$PWD/$path" ;;
  esac
  while [ ! -d "$path" ]; do
    rest="/$(basename "$path")$rest"
    path="$(dirname "$path")"
  done
  case "$rest/" in
    */../* | */./*) die "$1: name the folder without . or .. below its existing part" ;;
  esac
  printf '%s%s\n' "$(cd "$path" && pwd -P)" "$rest"
}

sites="$here/sites.txt"
out=""
browser=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --sites)
      [ "$#" -ge 2 ] || die "--sites needs a file"
      sites="$2"
      shift 2
      ;;
    --out)
      [ "$#" -ge 2 ] || die "--out needs a folder"
      out="$2"
      shift 2
      ;;
    --browser)
      [ "$#" -ge 2 ] || die "--browser needs a path"
      browser="$2"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      die "unknown argument: $1"
      ;;
  esac
done

[ -f "$sites" ] || die "no site list at $sites"
sites="$(absolute "$sites")"
if [ -z "$out" ]; then
  out="$HOME/.neo-zero-conflict/$(date -u +%Y%m%dT%H%M%SZ)"
fi
out="$(absolute "$out")"

# Refused before anything is written: inside any work tree of this repository, the main checkout
# or a linked worktree wherever it sits, since the remote pool uploads packages/browseros-agent
# from whichever one it is run in. The live test refuses the same folders.
trees="$(git -C "$here" worktree list --porcelain)"
while IFS= read -r line; do
  tree="${line#worktree }"
  [ "$tree" != "$line" ] && [ -d "$tree" ] || continue
  tree="$(cd "$tree" && pwd -P)"
  case "$out/" in
    "$tree"/*) die "the private folder $out is inside the git work tree $tree; real pages' captures never go there" ;;
  esac
done <<<"$trees"

mkdir -p "$out"
chmod 700 "$out"
echo "run.sh: sites $sites" >&2
echo "run.sh: private folder $out" >&2

export NEO_ZERO_CONFLICT_SITES="$sites"
export NEO_ZERO_CONFLICT_PRIVATE="$out"
if [ -n "$browser" ]; then
  export NEO_E2E_BROWSER="$browser"
fi

cd "$agent"
# --release: every printed line runs its own query over its whole document, so the check grows
# with lines times nodes.
exec node tools/test-browser.mjs -- cargo test --release -p neo-snapshot-apc \
  --test zero_conflict_live -- --ignored --exact "$test_name" --nocapture
