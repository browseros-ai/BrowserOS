#!/usr/bin/env bash

set -euo pipefail

if [ "$#" -lt 3 ]; then
  echo "Usage: $0 <remote-branch> <commit-message> <updates/path> [updates/path ...]" >&2
  exit 2
fi

branch="$1"
commit_message="$2"
shift 2
snapshot_paths=("$@")

if [ -z "${GITHUB_REPOSITORY:-}" ] || [ -z "${GH_TOKEN:-}" ]; then
  echo "GITHUB_REPOSITORY and GH_TOKEN are required" >&2
  exit 2
fi
if ! git check-ref-format --branch "$branch" >/dev/null; then
  echo "Invalid remote branch: $branch" >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
for snapshot_path in "${snapshot_paths[@]}"; do
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
  if [ ! -f "$repo_root/$snapshot_path" ]; then
    echo "Snapshot does not exist: $snapshot_path" >&2
    exit 2
  fi
done

temp_root="$(mktemp -d)"
worktree="$temp_root/repo"

# Invoked through the EXIT trap.
# shellcheck disable=SC2329
cleanup() {
  git -C "$repo_root" worktree remove --force "$worktree" >/dev/null 2>&1 || true
  rm -rf "$temp_root"
}
trap cleanup EXIT

git -C "$repo_root" fetch origin \
  "refs/heads/$branch:refs/remotes/origin/$branch" --no-tags
git -C "$repo_root" worktree add --detach "$worktree" "origin/$branch"

path_slug="$(basename "${snapshot_paths[0]}" | tr -cs '[:alnum:]' '-')"
run_id="${GITHUB_RUN_ID:-local}"
run_attempt="${GITHUB_RUN_ATTEMPT:-1}"

for attempt in 1 2 3 4 5; do
  git -C "$worktree" fetch origin \
    "refs/heads/$branch:refs/remotes/origin/$branch" --no-tags
  git -C "$worktree" checkout --detach "origin/$branch"
  git -C "$worktree" reset --hard "origin/$branch"
  git -C "$worktree" clean -fd

  for snapshot_path in "${snapshot_paths[@]}"; do
    mkdir -p "$worktree/$(dirname "$snapshot_path")"
    cp "$repo_root/$snapshot_path" "$worktree/$snapshot_path"
  done

  if git -C "$worktree" diff --quiet -- "${snapshot_paths[@]}"; then
    echo "Snapshots already current: ${snapshot_paths[*]}"
    exit 0
  fi

  pr_branch="bot/release-snapshot-${run_id}-${run_attempt}-${path_slug}-${attempt}"
  git -C "$worktree" checkout -B "$pr_branch"
  git -C "$worktree" config user.name "github-actions[bot]"
  git -C "$worktree" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
  git -C "$worktree" add -- "${snapshot_paths[@]}"
  git -C "$worktree" commit -m "$commit_message"
  head_sha="$(git -C "$worktree" rev-parse HEAD)"

  if ! git -C "$worktree" push origin "HEAD:refs/heads/$pr_branch"; then
    echo "Snapshot branch push failed on attempt $attempt" >&2
    continue
  fi

  pr_url=""
  if ! pr_url="$(gh pr create \
    --repo "$GITHUB_REPOSITORY" \
    --base "$branch" \
    --head "$pr_branch" \
    --title "$commit_message" \
    --body "Automated release snapshot update for workflow run ${run_id}.${run_attempt}.")"; then
    git -C "$worktree" push origin --delete "$pr_branch" >/dev/null 2>&1 || true
    echo "Snapshot PR creation failed on attempt $attempt" >&2
    continue
  fi

  merge_sha=""
  for merge_try in 1 2 3 4 5 6; do
    gh pr merge "$pr_url" \
      --repo "$GITHUB_REPOSITORY" \
      --squash \
      --auto \
      --delete-branch \
      --match-head-commit "$head_sha" \
      --subject "$commit_message" \
      --body "Automated release snapshot update." || true
    state="$(gh pr view "$pr_url" \
      --repo "$GITHUB_REPOSITORY" \
      --json state \
      --jq '.state' 2>/dev/null || true)"
    if [ "$state" = "MERGED" ]; then
      merge_sha="$(gh pr view "$pr_url" \
        --repo "$GITHUB_REPOSITORY" \
        --json mergeCommit \
        --jq '.mergeCommit.oid // ""')"
      break
    fi
    echo "Snapshot PR is not merged yet ($merge_try/6)"
    sleep "${SNAPSHOT_MERGE_POLL_SECONDS:-5}"
  done

  if [[ "$merge_sha" =~ ^[0-9a-fA-F]{40}$ ]]; then
    git -C "$worktree" fetch origin \
      "refs/heads/$branch:refs/remotes/origin/$branch" --no-tags
    git -C "$worktree" merge-base --is-ancestor "$merge_sha" "origin/$branch"
    for snapshot_path in "${snapshot_paths[@]}"; do
      merged_path="$temp_root/merged-$(basename "$snapshot_path")"
      git -C "$worktree" show "$merge_sha:$snapshot_path" > "$merged_path"
      cmp "$repo_root/$snapshot_path" "$merged_path"
    done
    echo "Snapshot PR merged at $merge_sha: ${snapshot_paths[*]}"
    exit 0
  fi

  gh pr close "$pr_url" \
    --repo "$GITHUB_REPOSITORY" \
    --delete-branch >/dev/null 2>&1 || true
  echo "Snapshot PR did not merge on attempt $attempt; retrying from current $branch" >&2
done

echo "Snapshot PR failed after five attempts: ${snapshot_paths[*]}" >&2
exit 1
