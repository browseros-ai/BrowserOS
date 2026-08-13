#!/usr/bin/env bash

set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 4 ]; then
  echo "Usage: $0 <pr-url-or-number> <expected-head-sha> [subject] [body]" >&2
  exit 2
fi

pr="$1"
expected_head="$2"
subject="${3:-}"
body="${4:-}"
attempts="${RELEASE_PR_MERGE_ATTEMPTS:-36}"
poll_seconds="${RELEASE_PR_MERGE_POLL_SECONDS:-5}"

if [ -z "${GITHUB_REPOSITORY:-}" ] || [ -z "${GH_TOKEN:-}" ]; then
  echo "GITHUB_REPOSITORY and GH_TOKEN are required" >&2
  exit 2
fi
if [[ ! "$expected_head" =~ ^[0-9a-fA-F]{40}$ ]]; then
  echo "Expected head must be a full commit SHA: $expected_head" >&2
  exit 2
fi
if [[ ! "$attempts" =~ ^[1-9][0-9]*$ ]]; then
  echo "RELEASE_PR_MERGE_ATTEMPTS must be a positive integer" >&2
  exit 2
fi
if [[ ! "$poll_seconds" =~ ^[0-9]+$ ]]; then
  echo "RELEASE_PR_MERGE_POLL_SECONDS must be a non-negative integer" >&2
  exit 2
fi

inspect_pr() {
  gh pr view "$pr" \
    --repo "$GITHUB_REPOSITORY" \
    --json state,mergeStateStatus,headRefOid,isDraft,statusCheckRollup
}

verify_head() {
  local details="$1"
  local actual_head
  actual_head="$(jq -er '.headRefOid' <<<"$details")"
  if [ "$actual_head" != "$expected_head" ]; then
    echo "Release PR head changed: expected $expected_head, found $actual_head" >&2
    exit 1
  fi
}

merge_args=(
  pr merge "$pr"
  --repo "$GITHUB_REPOSITORY"
  --squash
  --delete-branch
  --match-head-commit "$expected_head"
)
if [ -n "$subject" ]; then
  merge_args+=(--subject "$subject")
fi
if [ -n "$body" ]; then
  merge_args+=(--body "$body")
fi

for ((attempt = 1; attempt <= attempts; attempt++)); do
  details="$(inspect_pr)"
  state="$(jq -er '.state' <<<"$details")"
  verify_head "$details"
  if [ "$state" = "MERGED" ]; then
    echo "Release PR merged: $pr"
    exit 0
  fi
  if [ "$state" != "OPEN" ]; then
    echo "Release PR is not open: $pr ($state)" >&2
    exit 1
  fi

  if [ "$(jq -r '.isDraft' <<<"$details")" = "true" ]; then
    echo "Release PR is still a draft: $pr" >&2
    exit 1
  fi

  merge_state="$(jq -er '.mergeStateStatus' <<<"$details")"
  failed_checks="$(jq '[.statusCheckRollup[]? | select(
    (.__typename == "CheckRun" and ((.conclusion // "") | test("^(FAILURE|CANCELLED|TIMED_OUT|ACTION_REQUIRED)$"))) or
    (.__typename == "StatusContext" and ((.state // "") | test("^(FAILURE|ERROR)$")))
  )] | length' <<<"$details")"
  if [ "$merge_state" = "DIRTY" ] || [ "$failed_checks" -gt 0 ]; then
    echo "Release PR cannot merge: state=$merge_state, failed_checks=$failed_checks" >&2
    exit 1
  fi

  if ! gh "${merge_args[@]}"; then
    gh "${merge_args[@]}" --auto || true
  fi

  details="$(inspect_pr)"
  state="$(jq -er '.state' <<<"$details")"
  verify_head "$details"
  if [ "$state" = "MERGED" ]; then
    echo "Release PR merged: $pr"
    exit 0
  fi
  if [ "$attempt" -lt "$attempts" ]; then
    echo "Release PR is not merged yet ($attempt/$attempts)"
    sleep "$poll_seconds"
  fi
done

echo "Release PR did not merge after $attempts attempts: $pr" >&2
exit 1
