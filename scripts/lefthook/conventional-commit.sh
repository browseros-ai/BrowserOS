#!/usr/bin/env bash
# Called from lefthook.yml. Kept in a file because lefthook on Windows runs
# `run:` as sh -c "<command>" without escaping, so double quotes inside an
# inline script end the argument early and break it.
msg=$(head -1 "$1")
if [[ ! "$msg" =~ ^(feat|fix|docs|style|refactor|perf|test|chore|ci|build|revert)(\(.+\))?\!?:\ .+ ]]; then
  echo "Commit message must follow Conventional Commits format:"
  echo "  <type>(<optional scope>): <description>"
  echo "  Types: feat, fix, docs, style, refactor, perf, test, chore, ci, build, revert"
  echo ""
  echo "Examples:"
  echo "  feat(auth): add OAuth2 support"
  echo "  fix: resolve null pointer exception"
  exit 1
fi
