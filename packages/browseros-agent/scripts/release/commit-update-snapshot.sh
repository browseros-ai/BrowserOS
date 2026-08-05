#!/usr/bin/env bash

set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "Usage: $0 <updates/path> <remote-branch> <commit-message>" >&2
  exit 2
fi

snapshot_path="$1"
branch="$2"
commit_message="$3"

case "$snapshot_path" in
  updates/*) ;;
  *)
    echo "Snapshot path must be under updates/: $snapshot_path" >&2
    exit 2
    ;;
esac

case "/$snapshot_path/" in
  */../*|*/./*)
    echo "Snapshot path must not contain relative traversal: $snapshot_path" >&2
    exit 2
    ;;
esac

repo_root="$(git rev-parse --show-toplevel)"
source_path="$repo_root/$snapshot_path"
if [ ! -f "$source_path" ]; then
  echo "Snapshot does not exist: $snapshot_path" >&2
  exit 2
fi

temp_root="$(mktemp -d)"
worktree="$temp_root/repo"

cleanup() {
  git -C "$repo_root" worktree remove --force "$worktree" >/dev/null 2>&1 || true
  rm -rf "$temp_root"
}
trap cleanup EXIT

git -C "$repo_root" fetch origin "$branch" --no-tags
git -C "$repo_root" worktree add --detach "$worktree" "origin/$branch"
mkdir -p "$worktree/$(dirname "$snapshot_path")"
cp "$source_path" "$worktree/$snapshot_path"

if git -C "$worktree" diff --quiet -- "$snapshot_path"; then
  echo "Snapshot already current: $snapshot_path"
  exit 0
fi

git -C "$worktree" config user.name "github-actions[bot]"
git -C "$worktree" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git -C "$worktree" add -- "$snapshot_path"
git -C "$worktree" commit -m "$commit_message"

for attempt in 1 2 3 4 5; do
  if git -C "$worktree" push origin "HEAD:$branch"; then
    echo "Snapshot committed: $snapshot_path"
    exit 0
  fi
  if [ "$attempt" -eq 5 ]; then
    echo "Snapshot push failed after $attempt attempts: $snapshot_path" >&2
    exit 1
  fi
  echo "Snapshot push rejected; retrying against the latest $branch ($attempt/5)"
  git -C "$worktree" fetch origin "$branch" --no-tags
  if ! git -C "$worktree" rebase "origin/$branch"; then
    git -C "$worktree" rebase --abort || true
    echo "Snapshot commit conflicts with the latest $branch: $snapshot_path" >&2
    exit 1
  fi
done
