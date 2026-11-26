#!/usr/bin/env bash
# The zero-conflict fuzzer's site run (design 10.3): every site of a URL list, one tab at a time,
# through neo-snapshot-apc's live test, in a fresh headless browser (`tools/test-browser.mjs`:
# throwaway profile, mock keychain). It runs on this machine only: each site's captures and
# oracle are real pages' text, so they go to a private folder outside every git work tree and
# never to the remote pool, which uploads packages/browseros-agent. README.md beside it says more.
#
#   run.sh [--sites FILE] [--out DIR] [--browser PATH]
#
#   --sites FILE    the URL list (default: sites.txt beside this script)
#   --out DIR       the private folder (default: $HOME/.neo-zero-conflict/<UTC timestamp>)
#   --browser PATH  the browser binary, as NEO_E2E_BROWSER (default: the launcher's own default)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
agent="$(cd "$here/../../../.." && pwd -P)"
test_name=site_lines_reach_their_elements

usage() {
  sed -n '8,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
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

# Refused before anything is written: inside the worktree that holds this script, or inside the
# main checkout that worktree belongs to (a worktree can sit inside it, as .wt/ does here).
top="$(git -C "$here" rev-parse --show-toplevel)"
common="$(git -C "$here" rev-parse --path-format=absolute --git-common-dir)"
for tree in "$(cd "$top" && pwd -P)" "$(cd "$(dirname "$common")" && pwd -P)"; do
  case "$out/" in
    "$tree"/*) die "the private folder $out is inside the git work tree $tree; real pages' captures never go there" ;;
  esac
done

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
# with lines times nodes. Optimized, Wikipedia's 10,000 lines check in 6 s instead of 22 s.
exec node tools/test-browser.mjs -- cargo test --release -p neo-snapshot-apc \
  --test zero_conflict_live -- --ignored --exact "$test_name" --nocapture
