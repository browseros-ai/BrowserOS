#!/usr/bin/env bash
# The runner's job-started hook on pool machines: it runs after GitHub assigns a job and
# before any of the job's steps, and a nonzero exit fails the job without running them.
#
# The organisation's runner group already admits only the allowed workflow. This check
# holds on the machine itself, even if that setting were changed: a fork's pull request,
# any other event, or any other workflow file or branch never runs a step here. The
# runner sets the GITHUB_* values from the job GitHub assigned. A workflow cannot
# override them, and no step of the job has run yet. runner.sh supplies the allowed values.
set -u
refuse() {
  echo "pool runner: refused $1; only ${POOL_RUNNER_WORKFLOWS:-?} on workflow_dispatch may run here" >&2
  exit 1
}
[ -n "${POOL_RUNNER_REPOSITORY:-}" ] && [ -n "${POOL_RUNNER_WORKFLOWS:-}" ] || refuse 'a job (no policy)'
[ "${GITHUB_REPOSITORY:-}" = "$POOL_RUNNER_REPOSITORY" ] || refuse "repository ${GITHUB_REPOSITORY:-unset}"
[ "${GITHUB_EVENT_NAME:-}" = workflow_dispatch ] || refuse "event ${GITHUB_EVENT_NAME:-unset}"
for allowed in $POOL_RUNNER_WORKFLOWS; do
  if [ "${GITHUB_WORKFLOW_REF:-}" = "$allowed" ]; then
    echo "pool runner: $GITHUB_WORKFLOW_REF, dispatched by ${GITHUB_TRIGGERING_ACTOR:-${GITHUB_ACTOR:-unknown}}"
    exit 0
  fi
done
refuse "workflow ${GITHUB_WORKFLOW_REF:-unset}"
