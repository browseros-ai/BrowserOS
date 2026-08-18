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

details=""
state=""
is_draft=""
merge_state=""
failed_checks=""

read_inspection() {
  local phase="$1"
  local actual_head
  local parsed
  if ! details="$(inspect_pr)"; then
    echo "Release PR inspection failed $phase: $pr" >&2
    return 1
  fi
  if ! parsed="$(jq -er '
    if ((.state | type) == "string")
      and ((.headRefOid | type) == "string")
      and ((.isDraft | type) == "boolean")
      and ((.mergeStateStatus | type) == "string")
    then [
      .state,
      .headRefOid,
      (.isDraft | tostring),
      .mergeStateStatus,
      ([.statusCheckRollup[]? | select(
        (.__typename == "CheckRun" and ((.conclusion // "") | test("^(FAILURE|CANCELLED|TIMED_OUT|ACTION_REQUIRED)$"))) or
        (.__typename == "StatusContext" and ((.state // "") | test("^(FAILURE|ERROR)$")))
      )] | length | tostring)
    ] | @tsv
    else error("missing pull request fields")
    end
  ' <<<"$details")"; then
    echo "Release PR inspection was not parseable $phase: $pr" >&2
    return 1
  fi
  IFS=$'\t' read -r state actual_head is_draft merge_state failed_checks <<<"$parsed"
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
  if ! read_inspection "before merge"; then
    if [ "$attempt" -lt "$attempts" ]; then
      echo "Release PR is not ready yet ($attempt/$attempts)"
      sleep "$poll_seconds"
      continue
    fi
    break
  fi
  if [ "$state" = "MERGED" ]; then
    echo "Release PR merged: $pr"
    exit 0
  fi
  if [ "$state" != "OPEN" ]; then
    echo "Release PR is not open: $pr ($state)" >&2
    exit 1
  fi

  if [ "$is_draft" = "true" ]; then
    echo "Release PR is still a draft: $pr" >&2
    exit 1
  fi

  if [ "$merge_state" = "DIRTY" ] || [ "$failed_checks" -gt 0 ]; then
    echo "Release PR cannot merge: state=$merge_state, failed_checks=$failed_checks" >&2
    exit 1
  fi

  if ! gh "${merge_args[@]}"; then
    gh "${merge_args[@]}" --auto || true
  fi

  if ! read_inspection "after merge"; then
    if [ "$attempt" -lt "$attempts" ]; then
      echo "Release PR is not ready yet ($attempt/$attempts)"
      sleep "$poll_seconds"
      continue
    fi
    break
  fi
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
