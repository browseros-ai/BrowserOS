#!/usr/bin/env bash
# Check a pool-run.yml request and resolve its `ref` to one commit of this repository.
#
# The pool runs only code from branches and tags of this repository, which only members
# with write access can push. A pull request's ref (refs/pull/N/head) or a bare commit
# id can name a fork's code, because GitHub serves pull-request commits from the base
# repository. So only an exact branch or tag name is accepted. This runs on a
# GitHub-hosted runner, before any pool slot is requested.
#
# Inputs (environment): POOL_REF, POOL_PRESET, POOL_COMMAND, POOL_REMOTE (the
# repository's URL). Output: `sha=<commit>` appended to $GITHUB_OUTPUT.
set -euo pipefail
ref=${POOL_REF-}
preset=${POOL_PRESET-}
command=${POOL_COMMAND-}
remote=${POOL_REMOTE:?POOL_REMOTE names the repository}
refuse() {
  echo "::error::$*" >&2
  exit 1
}

case "$preset" in
  command) [ -n "$command" ] || refuse 'the command preset needs a command' ;;
  sdk-gate | live-suites)
    [ -z "$command" ] || refuse "the $preset preset takes no command; choose the command preset to run your own" ;;
  *) refuse "unknown preset: $preset" ;;
esac

case "$ref" in
  '' | refs/* | -* | *[[:space:]]*) refuse "ref must be a branch or tag name of this repository, got: $ref" ;;
esac
git check-ref-format "refs/heads/$ref" || refuse "not a valid branch or tag name: $ref"

listing=$(git ls-remote "$remote" "refs/heads/$ref" "refs/tags/$ref" "refs/tags/$ref^{}") \
  || refuse "could not list $remote"
# ls-remote matches its patterns against the end of each name; keep exact names only.
# An annotated tag's commit is listed as <tag>^{} (only when asked for), and is the one to run.
exact() { awk -v name="$1" '$2 == name { print $1 }' <<<"$listing"; }
branch=$(exact "refs/heads/$ref")
tag=$(exact "refs/tags/$ref^{}")
tag=${tag:-$(exact "refs/tags/$ref")}
if [ -n "$branch" ] && [ -n "$tag" ]; then
  refuse "$ref names both a branch and a tag; dispatch the one you mean under another name"
fi
sha=${branch:-$tag}
[ -n "$sha" ] || refuse "no branch or tag named $ref in this repository"
[[ $sha =~ ^[0-9a-f]{40}$ ]] || refuse "unexpected object id for $ref: $sha"
echo "sha=$sha" >>"${GITHUB_OUTPUT:-/dev/stdout}"
echo "$ref resolves to $sha"
