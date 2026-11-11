#!/usr/bin/env bash
#
# Runs every example script in this folder through `dev.sh run` against the
# running claw-sdk container and checks what came back, in the MCP reply and in
# Neo's own trace over the host API. Prints ok or FAIL per step and exits 1 if
# any step failed.
#
#   ../../dev.sh up --server claw-sdk && ./check.sh
#
# Rerunnable: the lock and takeover steps use fresh task ids each time, so an
# earlier run's lock or tab never decides this one. The takeover step releases its
# lock and closes its example tab when it is done.

set -uo pipefail # not -e: a failed step is reported and the rest still run

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DEV=$HERE/../../dev.sh
GATEWAY=http://127.0.0.1:${NEO_GATEWAY_PORT:-16090}
STAMP=$(date +%Y%m%d-%H%M%S)
failed=0

command -v jq >/dev/null || { echo "check.sh: needs jq" >&2; exit 1; }

# Read from the CLI record `dev.sh run --json` prints (design shape: requestId,
# taskId, code, timeoutMs, timing, response, transportError).
DEFS='
def meta: .response._meta["com.browseros.neo/run"] // {};
def value: .response.structuredContent.value;
def outcome: if .transportError then "transport error: \(.transportError)"
  else "run \(meta.id) \(meta.status)"
    + (if meta.error then " \(meta.error.owner)/\(meta.error.code) delivery=\(meta.error.delivery)" else "" end)
  end;
'

# One example; prints its CLI record. A refused run exits 1, which is an
# outcome the caller checks, not a reason to stop.
run() {
  local script=$1
  shift
  "$DEV" run -f "$HERE/$script" --json "$@" || true
}

# expect <label> <json> <jq predicate> <jq detail>
expect() {
  local verdict detail
  verdict=$(jq -r "$DEFS $3" <<<"$2" 2>/dev/null)
  detail=$(jq -r "$DEFS $4" <<<"$2" 2>/dev/null)
  if [ "$verdict" = true ]; then
    echo "ok   $1 (${detail:-?})"
  else
    echo "FAIL $1 (${detail:-no reply: ${2:0:200}})"
    failed=$((failed + 1))
  fi
}

# The tab a run's operations targeted, from Neo's trace of that run.
tab_of() {
  local id
  id=$(jq -r "$DEFS meta.id // empty" <<<"$1")
  [ -n "$id" ] && "$DEV" host GET "/v1/runs/$id" | jq -r '.tabs[0] // empty'
}

# Neo's own record of a run (operations included), once its screenshot work is done.
trace_of() {
  local id
  id=$(jq -r "$DEFS meta.id // empty" <<<"$1")
  [ -n "$id" ] && "$DEV" host GET "/v1/runs/$id?wait=10"
}

reads() {
  local rec
  rec=$(run goto-title.js --task check-goto)
  expect "(a) goto https://example.com, read the title" "$rec" \
    'value.title == "Example Domain"' 'outcome + " title=\(value.title)"'

  rec=$(run aria-ref.js --task check-aria)
  expect "(b) ariaSnapshot({mode:'ai'}), then getByRef on a printed ref" "$rec" \
    'value.byRef == "Save order" and value.byAriaRef == "Save order" and value.frameByRef == "Frame action"' \
    'outcome + " ref=\(value.ref) frameRef=\(value.frameRef)"'

  rec=$(run frame-read.js --task check-frame)
  expect "(c) read inside an iframe (frameLocator, page.frames())" "$rec" \
    'value.button == "Frame action" and value.text == "Inside the frame" and value.fromFrames == "Inside the frame"' \
    'outcome + " button=\(value.button) frames=\(value.frames)"'
}

lock() {
  local task=check-lock-$STAMP rec tab lock_id trace op
  rec=$(run goto-title.js --task "$task")
  tab=$(tab_of "$rec")
  expect "(d) task $task opens a page" "$rec" "meta.status == \"ok\" and \"$tab\" != \"\"" 'outcome + " tab='"$tab"'"'
  [ -n "$tab" ] || return

  rec=$("$DEV" host POST /v1/locks "{\"tabId\":$tab,\"ttlSeconds\":60}")
  lock_id=$(jq -r '.id // empty' <<<"$rec" 2>/dev/null)
  expect "(d) POST /v1/locks on tab $tab" "$rec" '.id != null and .releasedAt == null' '"lock \(.id) settled=\(.settled)"'
  [ -n "$lock_id" ] || return

  rec=$(run task-goto.js --task "$task")
  expect "(d) the task's page.goto is refused while locked" "$rec" \
    'meta.status == "error" and meta.error.owner == "host" and meta.error.code == "owner_has_control"' 'outcome'
  # Delivery as Neo's trace recorded it: what a host audits after the fact.
  op=$(jq -r "$DEFS meta.error.operationId // empty" <<<"$rec")
  trace=$("$DEV" host GET "/v1/runs/$(jq -r "$DEFS meta.id" <<<"$rec")")
  expect "(d) Neo's trace has the refused op as not_sent" "$trace" \
    "[.operations[] | select(.id == \"$op\") | .delivery] == [\"not_sent\"]" \
    '"op '"$op"' \(.error.owner)/\(.error.code)"'

  rec=$("$DEV" host DELETE "/v1/locks/$lock_id")
  expect "(d) DELETE /v1/locks/<id>" "$rec" '.releasedAt != null' '"lock \(.id) released"'

  rec=$(run task-goto.js --task "$task")
  expect "(d) the same run works once the lock is gone" "$rec" \
    'meta.status == "ok" and (value.url | test("from=task-goto"))' 'outcome + " url=\(value.url)"'
}

takeover() {
  local task=check-takeover-$STAMP rec tab id
  rec=$(run goto-title.js --task "$task")
  tab=$(tab_of "$rec")
  expect "(e) task $task opens a page" "$rec" "meta.status == \"ok\" and \"$tab\" != \"\"" 'outcome + " tab='"$tab"'"'
  [ -n "$tab" ] || return

  rec=$("$DEV" host POST /v1/takeovers "{\"tabId\":$tab}")
  id=$(jq -r '.id // empty' <<<"$rec" 2>/dev/null)
  expect "(e) POST /v1/takeovers on tab $tab" "$rec" \
    '.state == "active" and (.display | startswith("/v1/display?mode=control&release="))' '"takeover \(.id) display=\(.display)"'
  [ -n "$id" ] || return
  echo "     owner's control display, live until the release below:"
  echo "     $GATEWAY/?mode=control&release=$id"

  rec=$(run task-goto.js --task "$task")
  expect "(e) a run on the task is refused while the takeover is active" "$rec" \
    'meta.status == "error" and meta.error.owner == "host" and meta.error.delivery == "not_sent"' 'outcome'

  rec=$("$DEV" host POST "/v1/takeovers/$id/release")
  expect "(e) POST /v1/takeovers/<id>/release" "$rec" '.state == "released"' '"state=\(.state) reason=\(.releaseReason)"'

  rec=$(run task-goto.js --task "$task")
  expect "(e) the run works again after release" "$rec" 'meta.status == "ok"' 'outcome + " url=\(value.url)"'

  rec=$(run close-page.js --task "$task")
  expect "(e) close the takeover's tab" "$rec" 'meta.status == "ok"' 'outcome'
}

# Playwright actions and expect: each script fills or types, clicks or presses, and asserts with
# expect(...).toHaveText, and the writes it sent all came back done.
actions() {
  local rec
  rec=$(run form-actions.js --task check-actions)
  expect "(g) fill a form, click Send, expect toHaveText" "$rec" \
    'meta.status == "ok" and meta.effects == "done" and value.shown == "Thanks, Ada Lovelace (ada@example.com)"' \
    'outcome + " shown=\(value.shown)"'
  expect "(g) Neo's trace has the fills and the click, delivered" "$(trace_of "$rec")" \
    '([.operations[] | select(.name == "locator.fill" or .name == "locator.click") | .delivery] | length >= 3 and all(. == "done"))' \
    '[.operations[] | select(.parentId == null) | .name] | join(", ")'

  rec=$(run clear-field.js --task check-actions)
  expect "(g) locator.clear empties a field" "$rec" 'meta.status == "ok" and value == ""' 'outcome'

  rec=$(run set-checked.js --task check-actions)
  expect "(g) locator.setChecked(true) checks a box" "$rec" 'meta.status == "ok" and value == true' 'outcome'

  rec=$(run keyboard-submit.js --task check-actions)
  expect "(g) keyboard.type then Enter, expect toHaveText" "$rec" \
    'meta.status == "ok" and value.shown == "Results for claw sdk"' 'outcome + " shown=\(value.shown)"'

  rec=$(run upload-file.js --task check-actions)
  expect "(g) setInputFiles uploads a local path and an in-memory payload" "$rec" \
    'meta.status == "ok" and (value.uploaded | startswith("os-release: "))
      and value.payload == "note.txt: hello from Neo"' \
    'outcome + " uploaded=\(value.uploaded) payload=\(value.payload)"'
  expect "(g) Neo's trace records both uploads as done" "$(trace_of "$rec")" \
    '[.operations[] | select(.name == "locator.setInputFiles") | .delivery] == ["done", "done"]' \
    '[.operations[] | select(.name == "locator.setInputFiles") | .delivery] | join(", ")'

}

probes() {
  local rec
  rec=$(run probe-evaluate.js --task check-probes)
  expect "(f) page.evaluate returns the main-world title" "$rec" \
    'meta.status == "ok" and value == "Evaluate"' 'outcome'

  rec=$(run probe-file-url.js --task check-probes)
  expect "(f) goto file:///etc/os-release loads the local file" "$rec" \
    'meta.status == "ok" and (value.content | contains("PRETTY_NAME="))' 'outcome'

  local script
  # The SDK refuses these itself, before any operation, so the error carries no delivery and
  # the run sends nothing.
  for script in probe-bring-to-front.js probe-network.js; do
    rec=$(run "$script" --task check-probes)
    expect "(f) ${script%.js} is refused by the SDK" "$rec" \
      'meta.status == "error" and meta.error.owner == "script" and meta.effects != "unknown"' \
      'outcome + ": " + meta.error.message'
  done

  rec=$(run wait-for-timeout.js --task check-probes)
  expect "(f) page.waitForTimeout(1500) waits" "$rec" \
    'meta.status == "ok" and value.waitedMs >= 1500' 'outcome + " waited=\(value.waitedMs)ms"'

  # `run` takes a script and a timeout, nothing else: a run picks its tab through
  # _meta's taskId, not a tab parameter.
  local fields
  fields=$("$DEV" tools run | sed -n '/^{/,$p' | jq -c '[.properties | keys[]] + [.additionalProperties]')
  if [ "$fields" = '["code","timeout",false]' ]; then
    echo "ok   (f) run's input is {code, timeout} only, no tab parameter ($fields)"
  else
    echo "FAIL (f) run's input is {code, timeout} only, no tab parameter (got ${fields:-nothing})"
    failed=$((failed + 1))
  fi
}

reads
lock
takeover
actions
probes
echo
echo "Trace UI: $GATEWAY/trace"
if [ "$failed" -gt 0 ]; then
  echo "$failed step(s) failed"
  exit 1
fi
echo "all steps ok"
