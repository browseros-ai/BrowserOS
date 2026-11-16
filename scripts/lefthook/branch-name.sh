#!/usr/bin/env bash
# Called from lefthook.yml. Kept in a file because lefthook on Windows runs
# `run:` as sh -c "<command>" without escaping, so double quotes inside an
# inline script end the argument early and break it.
branch=$(git rev-parse --abbrev-ref HEAD)
if [[ "$branch" == "main" || "$branch" == "master" ]]; then
  exit 0
fi
if [[ ! "$branch" =~ ^(feat|fix|bugfix|hotfix|release|docs|refactor|test|chore|experiment)/[a-z0-9-]+$ ]]; then
  echo "⚠️  Warning: Branch name '$branch' doesn't match recommended format."
  echo "   Use: <type>/<short-description>"
  echo "   Types: feat, fix, bugfix, hotfix, release, docs, refactor, test, chore, experiment"
  echo "   Example: feat/add-auth, fix/login-crash"
  echo ""
  echo "   To rename your branch:"
  echo "     git branch -m <new-name>"
  echo "     git push -u origin <new-name>"
fi
