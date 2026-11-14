#!/usr/bin/env bash
# Acceptance checks for a built Neo SDK image: it boots, drives a real page through `run`, keeps
# its host contract (README.md), and survives the stops and kills a host puts it through.
#
#   accept.sh <image> [--data DIR] [--logs DIR] [--prefix NAME] [--case CHECK] [--scale 1|2]
#
#   --data    bind-mount DIR as /data and leave it there afterwards. Default: a named volume that
#             is removed with everything else.
#   --logs    save each container's log there, for CI artifacts.
#   --prefix  name what it creates NAME-<run>-*. Default: neo-sdk-accept.
#   --case    run only identity, boot and one takeover-scale, takeover-restart, takeover-expiry
#             or takeover-refusal check in a fresh container. Default: the full acceptance suite.
#   --scale   lifecycle check scale, 1 or 2. Default: 2. takeover-scale measures both scales.
#
# Prints one `PASS|FAIL|SKIP <check> <detail>` line per check and exits 1 if any check failed.
# Everything it creates is named <prefix>-<run>-* and removed on exit: the containers, an
# --internal Docker network (no route out, no published ports) and the data volume. An image
# for another architecture than the Docker engine's gets the static identity check only; booting
# it would run under emulation, which proves nothing about that architecture.
#
#   check            what passes
#   identity         claw-sdk marker; server sha256 = provenance.json; the server's compiled-in
#                    source digest = the build's (provenance source_digest); browser recorded; no noVNC
#                    or websockify; python3 present; the CAPTCHA key files are exactly the ones
#                    provenance.json's captchaKeys names (none in a keyless image), non-empty, 0400
#                    neo, with the baked label only when it names any
#   boot             healthy within 90 s with a host group and token, under only the capabilities
#                    README.md documents and no-new-privileges; listeners as README "Ports"
#   mcp              smoke_sdk.py; a `run` with the host's _meta fills a page; a repeat replays
#   host-api         group + token: 200; wrong token: 401; no group, with or without the
#                    token: EACCES at connect
#   downloads        a click that downloads from a page on 127.0.0.1: `download.path()` is the
#                    browser's file; the same run uploads it and a saveAs copy inside the default
#                    downloads folder; hashes agree with bytes served on the host socket;
#                    /tmp remains writable and a second run uploads the original again
#   pdf-downloads    host's exact PDF attachment, four valid PDF cases (attachment, inline viewer,
#                    download attribute, typed Blob), and ZIP attachment; disk and host SHA-256
#   blobs            a 1.5 MB+ full-page PNG and a PDF returned from `run` come back as
#                    {"$neo":"blob"} markers and read back whole at /v1/runs/{id}/blobs/{id};
#                    small bytes stay inline; 300 KiB of text is still result_too_large
#   blank-tab        GET /v1/tabs marks one tab owner "neo"; a script closes every tab it sees
#                    (context.pages()); only Neo's tab is left, the server is the same process,
#                    and the next run passes
#   task-page        20 times each, a task's next run starts on its own tab after a run that ends
#                    on a wait, a wait loop cut by its deadline, or its script's act on another
#                    task's tab and the New Tab (under tabs.scope browser, set for that shape and
#                    put back); two tasks interleaved; once a run without a task closed the tab,
#                    a new tab of its own (accept_task_page.py)
#   frames           10 times each, a task's tab read by a run without a task lists its
#                    cross-site and same-origin frames at the script's first statement and after
#                    a title; a later run of the task that opened it too; a cross-site frame
#                    added mid-run shows after the script's next wait (accept_frames.py)
#   tab-keep         host keeps survive idle sweeps; DELETE and TTL restart the idle clock;
#                    agents can fill, read, and close kept tabs, including task.close()
#   takeover-scale   366x700 phone metrics at 1x/2x, actual RFB 366x700/732x1400; PNG/JPEG
#                    capture density and fixture pixels; owner input, physical keyboard boxes,
#                    refits and restoration of display/window and a different agent viewport
#   takeover-restart opt-in server-only SIGKILL while fitted; same browser PID/start, CDP link
#                    and target; display/window recovery and original viewport outcome recorded
#   takeover-expiry  opt-in 60 s TTL expires itself; original display/window/viewport and input
#   takeover-refusal opt-in Xvnc resize rejection; 503, unchanged original state, input unlocked
#   mcp-remote       9011 is the same MCP over TLS; with NEO_MCP_REMOTE=false (the default) a
#                    sibling container is cut off on 9010 and 9011; with true it is answered
#   settings         PATCH /v1/settings applies and lists an unknown name, a bad value is 400 and
#                    changes nothing; after SIGKILL of the server alone (the browser relaunches it)
#                    GET answers what the host set; settings.json is 0600 neo under /data
#   settings-file    fresh root-written state; recording off before any API write, malformed
#                    files and no token; fixed file replaces saved state on each container start
#   settings-file-ids     a host that runs Neo under another PUID and PGID owns and loads its settings
#   settings-file-link    a state folder that is a symlink keeps its link and loads the fixed file
#   files            downloads-only defaults; all six path reads and file chooser; live PATCH;
#                    symlinks; Neo's files and directory ancestors protected even with / listed;
#                    saveAs and all screenshot path forms refuse before send or mkdir; Neo's home
#                    (HOME: dotfiles, fontconfig, ~/.pki/nssdb, the crash reporter's settings) is
#                    refused by every write and read spelling, a folder listed inside it stays
#                    usable unless it is a dot-folder, and ~/Downloads works by its real path
#   trace-values     on control detects canaries; off omits values from HTTP/events and raw files;
#                    blob survives a server restart, expires at startup, and its download stays
#   restart-browser  neo-quit 9110 quits cleanly, the supervisor restarts Neo, a new run passes
#   stop             docker stop -t 25 takes under 20 s, exits 0, Neo and claw-sdk log the stop
#   persist          a new container on the same /data reads the run and replays its requestId
#   kill-all         SIGKILL of the whole Neo tree (the host's OOM cgroup.kill) and a restart loop:
#                    healthy again, socket recreated, the run readable and replayed
#   captcha-keys     neo.solveCaptcha is listed only with a key: not in a keyless image, whose server
#                    logs that solving is off; listed once the host mounts a key file read-only and
#                    points the variable at it. A keyed image: listed with its own keys, the server
#                    logs solving on with each provider it carries, also under other PUID/PGID;
#                    no file door a script has reads or overwrites a key file even with / listed;
#                    both variables set empty turn solving off
set -uo pipefail

usage() { sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }

here=$(cd -- "$(dirname -- "$0")" && pwd)

image="" data_dir="" logs_dir="" name_prefix=neo-sdk-accept selected_case="" selected_scale=2
while [ $# -gt 0 ]; do
  case "$1" in
    --data) data_dir=${2:?--data needs a directory}; shift 2 ;;
    --logs) logs_dir=${2:?--logs needs a directory}; shift 2 ;;
    --prefix) name_prefix=${2:?--prefix needs a name}; shift 2 ;;
    --case) selected_case=${2:?--case needs a check}; shift 2 ;;
    --scale) selected_scale=${2:?--scale needs a value}; shift 2 ;;
    -h | --help) usage ;;
    -*) echo "accept.sh: unknown option '$1'" >&2; usage ;;
    *) [ -z "$image" ] || usage; image=$1; shift ;;
  esac
done
[ -n "$image" ] || usage
case "$selected_case" in ""|takeover-scale|takeover-restart|takeover-expiry|takeover-refusal) ;; *) usage ;; esac
case "$selected_scale" in 1|2) ;; *) usage ;; esac
docker image inspect "$image" >/dev/null 2>&1 || { echo "accept.sh: no image '$image'" >&2; exit 2; }

run_id="$(date +%s)-$$"
prefix="$name_prefix-$run_id"
network="$prefix-net"
volume="$prefix-data"
settings_volumes=""
host_gid=2005
secrets=$(mktemp -d)
# The token is mounted read-only for neo (uid PUID) and the test uids, so the folder is 0755.
chmod 0755 "$secrets"
od -An -tx1 -N24 /dev/urandom | tr -d ' \n' > "$secrets/host-token"
printf 'accept-dummy-capsolver-key' > "$secrets/capsolver-api-key"
chmod 0644 "$secrets/host-token" "$secrets/capsolver-api-key"
# Grant only the capabilities README.md says the entrypoint needs. Docker's wider default
# includes CAP_FSETID and would hide a missing capability in a host container.
host_caps=(--cap-drop ALL --cap-add SETUID --cap-add SETGID --cap-add CHOWN --cap-add FOWNER
  --cap-add DAC_OVERRIDE --cap-add KILL --security-opt no-new-privileges)

declare -a results=()
failed=0
result() { # PASS|FAIL|SKIP check detail
  results+=("$1 $2")
  [ "$1" != FAIL ] || failed=1
  printf '%s %s %s\n' "$1" "$2" "$3"
}

remove() { # container: its log goes to --logs first
  if [ -n "$logs_dir" ]; then
    mkdir -p "$logs_dir"
    docker logs "$1" > "$logs_dir/$(docker inspect -f '{{.Name}}' "$1" | tr -d /).log" 2>&1
  fi
  docker rm -f "$1" >/dev/null 2>&1
}

# shellcheck disable=SC2329  # invoked by the EXIT trap
cleanup() {
  local name
  for name in $(docker ps -aq --filter "name=^$prefix-"); do remove "$name"; done
  docker network rm "$network" >/dev/null 2>&1
  [ -n "$data_dir" ] || docker volume rm "$volume" >/dev/null 2>&1
  for name in $settings_volumes; do docker volume rm "$name" >/dev/null 2>&1; done
  rm -rf "$secrets"
}
trap cleanup EXIT

# One field of a JSON object, by a Python expression on `d`; empty when it does not resolve.
jget() { python3 -c 'import json,sys; print(eval(sys.argv[2], {"d": json.loads(sys.argv[1])}))' "$1" "$2" 2>/dev/null; }

data_mount() { if [ -n "$data_dir" ]; then echo "$data_dir:/data"; else echo "$volume:/data"; fi; }

# Starts a container of the image under test on the private network, with the host contract's
# inputs: the token, the socket group, a persistent /data. Extra docker-run arguments go first.
start_neo() { # name [docker run args...] [-- command...]
  local name=$1; shift
  docker run --memory 3g --memory-swap 3g -d --name "$name" --network "$network" --shm-size=1g --stop-timeout 25 \
    --label neo-sdk-accept="$run_id" "${host_caps[@]}" \
    -v "$(data_mount)" -v "$secrets:/run/neo-accept:ro" \
    -e CLAW_HOST_TOKEN_FILE=/run/neo-accept/host-token -e NEO_HOST_GID=$host_gid \
    "$@" >/dev/null
}

wait_healthy() { # name seconds
  local i
  for i in $(seq 1 "$2"); do
    case "$(docker inspect -f '{{.State.Health.Status}}' "$1" 2>/dev/null)" in
      healthy) echo "$i"; return 0 ;;
      "") return 1 ;;
    esac
    [ "$(docker inspect -f '{{.State.Running}}' "$1")" = true ] || return 1
    sleep 1
  done
  return 1
}

wait_ready() { # name seconds: /system/ready through the browser's proxy, from inside
  local i
  for i in $(seq 1 "$2"); do
    docker exec "$1" curl -fsS -o /dev/null http://127.0.0.1:9010/system/ready 2>/dev/null && { echo "$i"; return 0; }
    sleep 1
  done
  return 1
}

install_client() { # name: the probes, beside smoke.py whose MCP client they reuse
  docker exec "$1" mkdir -p /opt/neo-accept
  local file
  for file in "$here/smoke.py" "$here/smoke_sdk.py" "$here/accept_client.py" "$here/accept_files.py" \
    "$here/trace_values.py" "$here/accept_task_page.py" "$here/accept_tab_keep.py" \
    "$here/accept_frames.py" "$here/accept_settings_file.py" "$here/accept_takeover_scale.py" "$here/accept_takeover_lifecycle.py"; do
    docker cp -q "$file" "$1:/opt/neo-accept/" || return 1
  done
}

# A host process is uid 2001 in the socket's group. The agent stand-in is uid 2002.
as_host=(setpriv --reuid 2001 --regid 2001 --groups "$host_gid")
as_host_no_group=(setpriv --reuid 2001 --regid 2001 --clear-groups)
as_agent=(setpriv --reuid 2002 --regid 2002 --clear-groups)
client() { local name=$1; shift; docker exec -w /opt/neo-accept "$name" "$@"; }
run_call() { client "$1" "${as_host[@]}" python3 accept_client.py run http://127.0.0.1:9010/mcp "$2" "$3"; }
host_get() { client "$1" "${as_host[@]}" python3 accept_client.py host /run/neo/host.sock /run/neo-accept/host-token "$2"; }
host_send() { # name method path json
  client "$1" "${as_host[@]}" python3 accept_client.py host-send /run/neo/host.sock /run/neo-accept/host-token "$2" "$3" "$4"
}
server_pid() { # name: the claw-sdk server's pid, by its argv[0] (only this check's container).
  # Not /proc/<pid>/exe: under the dropped capabilities root may not read another uid's.
  docker exec "$1" sh -c 'server=$(neo-server-info path)
    for p in /proc/[0-9]*; do
      [ "$(tr "\0" "\n" < "$p/cmdline" 2>/dev/null | head -1)" = "$server" ] && echo "${p#/proc/}"
    done' | head -1
}
probe() { # target-ip port scheme, from a throwaway sibling on the same network
  docker run --memory 3g --memory-swap 3g --rm -i --name "$prefix-probe" --network "$network" --entrypoint python3 "$image" \
    - probe "$1" "$2" "$3" < "$here/accept_client.py"
}
run_ok() { # json from run_call: status ok and the page really filled
  [ "$(jget "$1" 'd["meta"]["status"]')" = ok ] && [ "$(jget "$1" 'd["value"]["value"]["value"]')" = host ] \
    && [ "$(jget "$1" 'd["value"]["value"]["title"]')" = neo-sdk-accept ]
}
runs_for() { jget "$(host_get "$1" "/v1/runs?requestId=$2")" 'len(d["body"]["items"])'; }
lists_captcha() { # name: whether the run tool offers neo.solveCaptcha
  jget "$(client "$1" python3 accept_client.py describe http://127.0.0.1:9010/mcp)" \
    '"neo.solveCaptcha(page)" in d["description"]'
}

takeover_probe() { # container check scale: run only the probe's owned tab and save its evidence
  local name=$1 check=$2 scale=$3 output status artifact_path lifecycle_case
  artifact_path="/tmp/neo-accept-$check"
  if [ "$check" = takeover-scale ]; then
    output=$(client "$name" "${as_host[@]}" python3 accept_takeover_scale.py \
      http://127.0.0.1:9010/mcp /run/neo/host.sock /run/neo-accept/host-token "$artifact_path" 2>&1)
    status=$?
  else
    case "$check" in
      takeover-restart) lifecycle_case=restart ;;
      takeover-expiry) lifecycle_case=ttl ;;
      takeover-refusal) lifecycle_case=refusal ;;
    esac
    # The collector revalidates the exact recorded server PID before signaling it. Root stays
    # inside this acceptance container; the host group is still needed for the Unix socket.
    output=$(client "$name" setpriv --regid "$host_gid" --clear-groups python3 accept_takeover_lifecycle.py \
      --case "$lifecycle_case" --scale "$scale" http://127.0.0.1:9010/mcp \
      /run/neo/host.sock /run/neo-accept/host-token "$artifact_path" 2>&1)
    status=$?
  fi
  if [ -n "$logs_dir" ]; then
    mkdir -p "$logs_dir/$check"
    printf '%s\n' "$output" > "$logs_dir/$check.json"
    docker cp -q "$name:$artifact_path/." "$logs_dir/$check/" >/dev/null 2>&1 || true
  fi
  if [ "$status" = 0 ] && [ "$(jget "$output" 'd["ok"]')" = True ]; then
    result PASS "$check" "$(jget "$output" 'd["detail"]')"
  else
    result FAIL "$check" "$(jget "$output" '"; ".join(d["problems"])' || printf '%s' "${output:0:500}")"
  fi
}

# Each startup case owns fresh persistent data. Root installs fixtures before Neo's real
# entrypoint; the fixed input is a read-only mount, and no settings call repairs a start.
settings_file_case() { # result-label probe-mode [uid]
  local mode=$2 uid=${3:-1000} name="$prefix-$1" own_volume="$prefix-$1-data"
  local first='{}' second='{}' ready=0 volumes_ready=1 target_volume="$prefix-$1-state"
  local -a input_flags state_flags
  input_flags=(-e SETTINGS_FILE_CASE="$mode")
  state_flags=(-v "$own_volume:/data")
  if [ "$mode" = fixed ] || [ "$mode" = linked-ids ]; then
    input_flags+=(-v "$secrets/start-settings.json:/etc/neo/settings.json:ro")
  fi
  settings_volumes="$settings_volumes $own_volume"
  docker volume create "$own_volume" >/dev/null || volumes_ready=0
  if [ "$mode" = linked-ids ]; then
    settings_volumes="$settings_volumes $target_volume"
    docker volume create "$target_volume" >/dev/null || volumes_ready=0
    state_flags+=(-v "$target_volume:/state-volume")
  fi
  if [ "$volumes_ready" = 1 ] \
      && docker run --memory 3g --memory-swap 3g -d --name "$name" --network "$network" --shm-size=1g \
      --stop-timeout 25 --label neo-sdk-accept="$run_id" "${host_caps[@]}" \
      "${state_flags[@]}" -v "$secrets:/run/neo-accept:ro" "${input_flags[@]}" \
      -e PUID="$uid" -e PGID="$uid" -e NEO_HOST_GID="$host_gid" \
      -e CLAW_HOST_TOKEN_FILE=/tmp/settings-host-token --entrypoint bash "$image" -c '
        set -e
        cp /run/neo-accept/host-token /tmp/settings-host-token
        if [ "$SETTINGS_FILE_CASE" = linked-ids ] && [ ! -L /data/browserclaw ]; then
          mkdir -p /state-volume/private-state
          chown "$PUID:$PGID" /state-volume/private-state
          chmod 700 /state-volume/private-state
          ln -s /state-volume/private-state /data/browserclaw
        elif [ ! -e /data/browserclaw/claw-sdk/settings.json ]; then
          mkdir -p /data/browserclaw/claw-sdk
          if [ "$SETTINGS_FILE_CASE" = fixed ]; then
            printf %s '\''{"tabs":{"idleCloseSeconds":900},"trace":{"values":"on"}}'\'' > /data/browserclaw/claw-sdk/settings.json
          else
            cp /run/neo-accept/start-settings.json /data/browserclaw/claw-sdk/settings.json
          fi
        fi
        if [ "$SETTINGS_FILE_CASE" = linked-ids ]; then
          # Keep failed startup volumes inspectable for a bounded time. The two checks
          # report their own filesystem facts even when the host API never becomes ready.
          /usr/local/bin/neo-entrypoint || true
          exec sleep 240
        fi
        exec /usr/local/bin/neo-entrypoint' >/dev/null; then
    wait_healthy "$name" 90 >/dev/null && ready=1
    if [ "$mode" = linked-ids ] && install_client "$name"; then
      first=$(client "$name" setpriv --regid "$host_gid" --clear-groups python3 accept_settings_file.py linked-ids "$ready" 2>&1)
    elif [ "$ready" = 1 ] && install_client "$name"; then
      first=$(client "$name" setpriv --regid "$host_gid" --clear-groups python3 accept_settings_file.py "$mode" 2>&1)
    else
      first='{"ok":false,"checks":[{"name":"fresh container started healthy","ok":false}]}'
    fi
    if [ "$mode" = fixed ] && [ "$ready" = 1 ]; then
      if docker restart "$name" >/dev/null && wait_healthy "$name" 90 >/dev/null; then
        second=$(client "$name" setpriv --regid "$host_gid" --clear-groups python3 accept_settings_file.py fixed-restarted 2>&1)
      else
        second='{"ok":false,"checks":[{"name":"container restarted healthy","ok":false}]}'
      fi
    fi
  else
    first='{"ok":false,"checks":[{"name":"fresh container started healthy","ok":false}]}'
  fi
  if [ "$mode" = linked-ids ]; then
    settings_file_report=$first
  else
    settings_file_report=$(merge_settings_file_reports "$first" "$second")
  fi
  remove "$name"
}

# Even a failed docker exec yields a valid JSON artifact with the preceding observations.
merge_settings_file_reports() {
  python3 - "$@" <<'PY'
import json, sys
checks = []
for raw in sys.argv[1:]:
    try:
        result = json.loads(raw)
        checks.extend(result.get('checks', []))
        if result and not result.get('checks'):
            checks.append({'name': 'probe supplied observations', 'ok': False})
    except (ValueError, AttributeError) as error:
        checks.append({'name': 'probe emitted JSON', 'ok': False, 'error': str(error)})
problems = [row['name'] for row in checks if not row['ok']]
print(json.dumps({'ok': bool(checks) and not problems, 'checks': checks, 'problems': problems,
                  'detail': f'{len(checks)} startup settings assertions; {len(problems)} failed'}))
PY
}

report_settings_file() { # label JSON
  local label=$1 report=$2
  if [ -n "$logs_dir" ]; then mkdir -p "$logs_dir"; printf '%s\n' "$report" > "$logs_dir/$label.json"; fi
  if [ "$(jget "$report" 'd["ok"]')" = True ]; then
    result PASS "$label" "$(jget "$report" 'd["detail"]')"
  else
    result FAIL "$label" "$(jget "$report" 'd["detail"] + "; " + "; ".join(d["problems"])')"
  fi
}

# --- identity --------------------------------------------------------------------------------
image_arch=$(docker image inspect -f '{{.Architecture}}' "$image")
engine_arch=$(docker version -f '{{.Server.Arch}}')
keys_label=$(docker image inspect -f '{{index .Config.Labels "com.browseros.neo-sdk.captcha-keys"}}' "$image")
facts=$(docker run --memory 3g --memory-swap 3g --rm --network none --name "$prefix-identity" --entrypoint sh "$image" -c '
  p=/usr/share/neo-sdk/provenance.json
  echo "kind=$(neo-server-info kind)"
  echo "sha=$(sha256sum "$(neo-server-info path)" | cut -d" " -f1)"
  echo "provenance_sha=$(jq -r .server.sha256 $p)"
  echo "built_from=$(LC_ALL=C grep -aoE "browseros-claw-sdk-source=[0-9a-f]{64}" "$(neo-server-info path)" | head -1 | cut -d= -f2)"
  echo "sources=$(jq -r ".source_digest // empty" $p)"
  echo "marker=$(jq -r .server.version $p)"
  echo "browser=$(jq -r .browser.version $p)/$(dpkg-query -W -f "\${Version}" browserclaw)"
  echo "provenance_deb=$(jq -r .browser.deb $p)"
  echo "revision=$(jq -r .revision $p)"
  [ -e /usr/share/novnc ] && echo novnc=present || echo novnc=absent
  command -v websockify >/dev/null && echo websockify=present || echo websockify=absent
  command -v python3 >/dev/null && echo python3=present || echo python3=absent
  echo "keys=$(find /home/neo/.config/browseros/captcha -type f 2>/dev/null | wc -l)"
  echo "key_modes=$(find /home/neo/.config/browseros/captcha -type f -exec stat -c "%a:%U" {} + 2>/dev/null | sort -u | tr "\n" " ")"
  echo "key_names=$(find /home/neo/.config/browseros/captcha -type f -printf "%f\n" 2>/dev/null | sort | tr "\n" ",")"
  echo "key_empty=$(find /home/neo/.config/browseros/captcha -type f -empty 2>/dev/null | wc -l)"
  echo "captcha_keys=$(jq -r "if has(\"captchaKeys\") then (.captchaKeys | join(\",\")) else \"unrecorded\" end" $p)"
  echo "notices=$(ls /usr/share/doc/neo-sdk/playwright 2>/dev/null | tr "\n" " ")"' 2>&1)
fact() { printf '%s\n' "$facts" | sed -n "s/^$1=//p"; }
problems=()
[ "$(fact kind)" = claw-sdk ] || problems+=("server kind '$(fact kind)'")
[ -n "$(fact sha)" ] && [ "$(fact sha)" = "$(fact provenance_sha)" ] || problems+=("server sha256 differs from provenance.json")
[ -n "$(fact sources)" ] && [ "$(fact built_from)" = "$(fact sources)" ] || problems+=("server compiled from sources '$(fact built_from)', not the build's '$(fact sources)'")
[ "$(fact browser)" = "$(fact browser | cut -d/ -f1)/$(fact provenance_deb)" ] || problems+=("deb $(fact browser) not in provenance")
[ "$(fact novnc)/$(fact websockify)" = absent/absent ] || problems+=("viewer installed")
[ "$(fact python3)" = present ] || problems+=("no python3 for neo-quit")
case "$(fact notices)" in *LICENSE*NOTICE*THIRD_PARTY_NOTICES.md*) ;; *) problems+=("Playwright notices missing") ;; esac
# The image says which CAPTCHA keys it carries (provenance.json captchaKeys, names only); the key
# files must be exactly those. An image built before captchaKeys is judged by its label alone.
captcha_keys=$(fact captcha_keys)
if [ "$captcha_keys" = unrecorded ]; then
  if [ "$keys_label" = baked ]; then
    [ "$(fact keys)" -ge 1 ] && [ "$(fact key_modes)" = "400:neo " ] || problems+=("baked keys: $(fact keys) files, modes $(fact key_modes)")
  else
    [ "$(fact keys)" = 0 ] || problems+=("keyless image holds $(fact keys) key files")
  fi
else
  expected_files=$(for name in ${captcha_keys//,/ }; do
    case $name in (capsolver) echo capsolver-api-key ;; (2captcha) echo 2captcha-api-key ;; (*) echo "unknown-$name" ;; esac
  done | sort | tr '\n' ,)
  [ "$(fact key_names)" = "$expected_files" ] || problems+=("key files '$(fact key_names)' are not captchaKeys '$captcha_keys'")
  if [ -n "$captcha_keys" ]; then
    [ "$keys_label" = baked ] || problems+=("captchaKeys '$captcha_keys' without the baked label")
    [ "$(fact key_empty)" = 0 ] && [ "$(fact key_modes)" = "400:neo " ] \
      || problems+=("key files: $(fact key_empty) empty, modes $(fact key_modes)")
  else
    [ "$keys_label" != baked ] || problems+=("baked label but captchaKeys is empty")
  fi
fi
detail="$image_arch, browser $(fact browser), server $(fact marker) $(fact sha | cut -c1-12), rev $(fact revision | cut -c1-9), keys ${captcha_keys:-none}"
if [ ${#problems[@]} -eq 0 ]; then result PASS identity "$detail"; else result FAIL identity "${problems[*]}"; fi

if [ "$image_arch" != "$engine_arch" ]; then
  if [ -n "$selected_case" ]; then
    result SKIP "$selected_case" "$image_arch image on a $engine_arch engine: emulated, not evidence"
  else
    for check in boot mcp host-api downloads pdf-downloads blobs frames tab-keep takeover-scale settings settings-file settings-file-ids settings-file-link files trace-values mcp-remote restart-browser stop persist kill-all captcha-keys; do
      result SKIP "$check" "$image_arch image on a $engine_arch engine: emulated, not evidence"
    done
  fi
  printf '\n'; printf '%s\n' "${results[@]}"
  exit "$failed"
fi

docker network create --internal --label neo-sdk-accept="$run_id" "$network" >/dev/null || { echo "accept.sh: no network" >&2; exit 1; }
[ -n "$data_dir" ] || docker volume create --label neo-sdk-accept="$run_id" "$volume" >/dev/null

if [ -n "$selected_case" ]; then
  selected="$prefix-$selected_case"
  if [ "$selected_case" = takeover-refusal ]; then
    # This Xvnc parameter is startup-only. A launcher mounted only in this disposable
    # container makes the real display refuse resizes without changing the built image.
    cat > "$secrets/Xvnc" <<'XVNC'
#!/bin/sh
exec /usr/bin/Xtigervnc "$@" -AcceptSetDesktopSize=0
XVNC
    chmod 0755 "$secrets/Xvnc"
    start_neo "$selected" -v "$secrets/Xvnc:/usr/local/bin/Xvnc:ro" -e CLAW_LOG=trace "$image"
  else
    start_neo "$selected" -e CLAW_LOG=trace "$image"
  fi
  if seconds=$(wait_healthy "$selected" 90) && install_client "$selected"; then
    result PASS boot "fresh container healthy after ${seconds}s"
    takeover_probe "$selected" "$selected_case" "$selected_scale"
  else
    result FAIL "$selected_case" "fresh container not healthy within 90 seconds or probe installation failed"
  fi
  printf '\n'; printf '%s\n' "${results[@]}"
  exit "$failed"
fi

printf %s '{"tabs":{"idleCloseSeconds":300},"trace":{"values":"off","screenshots":"off","retentionSeconds":3600},"futureGroup":true}' > "$secrets/start-settings.json"
chmod 0644 "$secrets/start-settings.json"

# --- boot ------------------------------------------------------------------------------------
main="$prefix-main"
# Exercise the host's most verbose filter so the canary catches dependency payload logs
# as well as Neo's own call notes.
start_neo "$main" -e CLAW_LOG=trace "$image"
if seconds=$(wait_healthy "$main" 90) && install_client "$main"; then
  listening=$(jget "$(client "$main" python3 accept_client.py listeners)" '" ".join(a for a in d["listening"] if not a.startswith("127.0.0.11:"))')
  socket=$(docker exec "$main" stat -c '%a %U %g' /run/neo /run/neo/host.sock 2>&1 | tr '\n' ' ')
  want="0.0.0.0:9010 0.0.0.0:9011 127.0.0.1:5900 127.0.0.1:9110 127.0.0.1:9210"
  problems=()
  for address in $want; do case " $listening " in *" $address "*) ;; *) problems+=("not listening on $address") ;; esac; done
  for address in $listening; do
    case "$address" in 127.0.0.1:* | ::1:*) ;; 0.0.0.0:9010 | 0.0.0.0:9011) ;; *) problems+=("unexpected listener $address") ;; esac
  done
  case " $listening " in *":6080 "*) problems+=("6080 is served") ;; esac
  [ "$socket" = "2750 neo $host_gid 660 neo $host_gid " ] || problems+=("socket dir and socket: $socket")
  if [ ${#problems[@]} -eq 0 ]; then result PASS boot "healthy after ${seconds}s; $listening; /run/neo $socket"
  else result FAIL boot "${problems[*]}"; fi
  booted=1
else
  result FAIL boot "not healthy within 90 s: $(docker logs "$main" 2>&1 | tail -3 | tr '\n' ' ')"
  booted=0
fi

if [ "$booted" = 1 ]; then
  main_captcha=$(lists_captcha "$main")

  settings_file_case settings-file-saved saved
  saved_settings=$settings_file_report
  settings_file_case settings-file-fixed fixed
  report_settings_file settings-file "$(merge_settings_file_reports "$saved_settings" "$settings_file_report")"
  settings_file_case settings-file-linked-ids linked-ids 2307
  for check in ids link; do
    check_report=$(python3 - "$settings_file_report" "$check" <<'PY'
import json, sys
try:
    report = json.loads(sys.argv[1])[sys.argv[2]]
except (ValueError, KeyError, TypeError):
    report = {'ok': False, 'checks': [{'name': 'startup facts available', 'ok': False}],
              'problems': ['startup facts unavailable'], 'detail': 'not ready; before and after-PATCH facts unavailable',
              'before': None, 'afterPatch': None}
print(json.dumps(report))
PY
    )
    report_settings_file "settings-file-$check" "$check_report"
  done

  # --- mcp -----------------------------------------------------------------------------------
  task="accept-$run_id" request="req-$run_id"
  smoke=$(client "$main" python3 smoke_sdk.py --mcp-port 9010 2>&1); smoke_status=$?
  first=$(run_call "$main" "$task" "$request" 2>&1)
  again=$(run_call "$main" "$task" "$request" 2>&1)
  run_id_first=$(jget "$first" 'd["meta"]["id"]')
  if [ "$smoke_status" = 0 ] && [ "$(jget "$first" 'd["tools"]')" = "['run']" ] && run_ok "$first" \
      && [ -n "$run_id_first" ] && [ "$(jget "$again" 'd["meta"]["id"]')" = "$run_id_first" ]; then
    result PASS mcp "smoke_sdk ok; run $run_id_first filled the page; repeat of $request replayed it"
  else
    result FAIL mcp "smoke=$smoke_status $(printf '%s' "$smoke" | tail -1); first=${first:0:300}; again=${again:0:200}"
  fi

  # --- host-api ------------------------------------------------------------------------------
  docker exec "$main" sh -c 'printf wrong-token-wrong-token-wrong-token-xx > /tmp/wrong-token && chmod 0644 /tmp/wrong-token'
  path="/v1/runs/$run_id_first"
  good=$(host_get "$main" "$path")
  wrong=$(client "$main" "${as_host[@]}" python3 accept_client.py host /run/neo/host.sock /tmp/wrong-token "$path")
  no_group=$(client "$main" "${as_host_no_group[@]}" python3 accept_client.py host /run/neo/host.sock /run/neo-accept/host-token "$path")
  agent=$(client "$main" "${as_agent[@]}" python3 accept_client.py host /run/neo/host.sock /run/neo-accept/host-token "$path")
  if [ "$(jget "$good" 'd["status"]')" = 200 ] && [ "$(jget "$good" 'd["body"]["requestId"]')" = "$request" ] \
      && [ "$(jget "$good" 'd["body"]["taskId"]')" = "$task" ] && [ "$(jget "$wrong" 'd["status"]')" = 401 ] \
      && [ "$(jget "$no_group" 'd["errno"]')" = 13 ] && [ "$(jget "$agent" 'd["errno"]')" = 13 ]; then
    result PASS host-api "group+token 200 ($(jget "$good" 'd["body"]["operationCount"]') ops), wrong token 401, no group EACCES, agent uid with token EACCES"
  else
    result FAIL host-api "good=${good:0:200}; wrong=${wrong:0:120}; no_group=$no_group; agent=$agent"
  fi

  # The probe's tab owns the fit and its original viewport. Release and close it before any
  # later acceptance case uses the shared display; preserve the actual encoded images in logs.
  takeover_probe "$main" takeover-scale 2

  # --- downloads -----------------------------------------------------------------------------
  # Download, upload and import, as the host uid with the socket's group.
  downloaded=$(client "$main" "${as_host[@]}" python3 accept_client.py downloads \
    http://127.0.0.1:9010/mcp /run/neo/host.sock /run/neo-accept/host-token 2>&1)
  if [ -n "$logs_dir" ]; then mkdir -p "$logs_dir"; printf '%s\n' "$downloaded" > "$logs_dir/downloads.json"; fi
  download_run=$(jget "$downloaded" 'd["run"]')
  if [ "$(jget "$downloaded" 'd["ok"]')" = True ]; then
    result PASS downloads "$(jget "$downloaded" 'd["detail"]')"
  else
    result FAIL downloads "$(jget "$downloaded" '"; ".join(d["problems"])' || printf '%s' "${downloaded:0:300}")"
  fi

  # The collector reads neo's 0700 /data as root. Its socket peer still needs the host gid.
  pdf_downloaded=$(client "$main" setpriv --regid "$host_gid" --clear-groups python3 accept_client.py pdf-downloads \
    http://127.0.0.1:9010/mcp /run/neo/host.sock /run/neo-accept/host-token 2>&1)
  if [ -n "$logs_dir" ]; then
    mkdir -p "$logs_dir"
    printf '%s\n' "$pdf_downloaded" > "$logs_dir/pdf-downloads.json"
  fi
  if [ "$(jget "$pdf_downloaded" 'd["ok"]')" = True ]; then
    result PASS pdf-downloads "$(jget "$pdf_downloaded" 'd["detail"]')"
  else
    result FAIL pdf-downloads "$(jget "$pdf_downloaded" '"; ".join(d["problems"])' || printf '%s' "${pdf_downloaded:0:300}")"
  fi

  # --- blobs ---------------------------------------------------------------------------------
  # Full-page screenshot and PDF, too large for a reply, read back on the host socket.
  blobbed=$(client "$main" "${as_host[@]}" python3 accept_client.py blobs \
    http://127.0.0.1:9010/mcp /run/neo/host.sock /run/neo-accept/host-token 2>&1)
  blob_run=$(jget "$blobbed" 'd["run"]') blob_id=$(jget "$blobbed" 'd["blob"]')
  if [ "$(jget "$blobbed" 'd["ok"]')" = True ]; then
    result PASS blobs "$(jget "$blobbed" 'd["detail"]')"
  else
    result FAIL blobs "$(jget "$blobbed" '"; ".join(d["problems"])' || printf '%s' "${blobbed:0:300}")"
  fi

  # --- blank-tab -----------------------------------------------------------------------------
  # Neo keeps one blank tab of its own: a script that closes every tab it can see leaves the
  # browser and the server running, Neo's tab still listed, and the next run works.
  neo_tabs() { jget "$1" '" ".join(str(i["tabId"]) for i in d["body"]["items"] if i.get("owner") == "neo")'; }
  all_tabs() { jget "$1" '" ".join(str(i["tabId"]) for i in d["body"]["items"])'; }
  blank_pid=$(server_pid "$main")
  tabs_before=$(host_get "$main" /v1/tabs)
  neo_before=$(neo_tabs "$tabs_before")
  closed=$(client "$main" "${as_host[@]}" python3 accept_client.py close-all http://127.0.0.1:9010/mcp 2>&1)
  tabs_after=""
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    tabs_after=$(host_get "$main" /v1/tabs)
    [ "$(all_tabs "$tabs_after")" = "$neo_before" ] && break
    sleep 0.5
  done
  blank_run=$(run_call "$main" "blank-$run_id" "blank-req-$run_id" 2>&1)
  if [ -n "$neo_before" ] && [ "${neo_before// /}" = "$neo_before" ] \
      && [ "$(jget "$closed" 'd["meta"]["status"]')" = ok ] \
      && [ "$(all_tabs "$tabs_after")" = "$neo_before" ] && [ "$(neo_tabs "$tabs_after")" = "$neo_before" ] \
      && run_ok "$blank_run" && [ -n "$blank_pid" ] && [ "$(server_pid "$main")" = "$blank_pid" ]; then
    result PASS blank-tab "one tab of Neo's ($neo_before); a script closed the $(jget "$closed" 'd["value"]["value"]["closed"]') it saw; only Neo's left, server $blank_pid alive, the next run passed"
  else
    result FAIL blank-tab "neo before=[$neo_before] closed=${closed:0:200} after=[$(all_tabs "$tabs_after")] neo after=[$(neo_tabs "$tabs_after")] pid $blank_pid->$(server_pid "$main") run=${blank_run:0:200}"
  fi

  # --- task-page -----------------------------------------------------------------------------
  # Which tab a task's next run starts on: its own, whatever the last run ended on (a wait, a
  # wait loop cut by the deadline, the script's own act on another task's tab under the browser
  # tab scope), and a new one once a script without a task closed it (accept_task_page.py).
  task_page=$(client "$main" "${as_host[@]}" python3 accept_task_page.py http://127.0.0.1:9010/mcp \
    /run/neo/host.sock /run/neo-accept/host-token 20 2>&1)
  if [ -n "$logs_dir" ]; then mkdir -p "$logs_dir"; printf '%s\n' "$task_page" > "$logs_dir/task-page.json"; fi
  if [ "$(jget "$task_page" 'd["ok"]')" = True ]; then
    result PASS task-page "$(jget "$task_page" 'd["detail"]')"
  else
    result FAIL task-page "$(jget "$task_page" 'd["detail"] + "; " + "; ".join(d["problems"][:3])' || printf '%s' "${task_page:0:300}")"
  fi

  # --- frames --------------------------------------------------------------------------------
  # Every page a script reaches lists its whole frame tree, cross-site children included, from
  # its first statement: a task's tab read by a run without a task, the shape a host's observer
  # reads (accept_frames.py).
  frames=$(client "$main" "${as_host[@]}" python3 accept_frames.py http://127.0.0.1:9010/mcp 10 2>&1)
  if [ -n "$logs_dir" ]; then mkdir -p "$logs_dir"; printf '%s\n' "$frames" > "$logs_dir/frames.json"; fi
  if [ "$(jget "$frames" 'd["ok"]')" = True ]; then
    result PASS frames "$(jget "$frames" 'd["detail"]')"
  else
    result FAIL frames "$(jget "$frames" 'd["detail"] + "; " + "; ".join(d["problems"][:3])' || printf '%s' "${frames:0:300}")"
  fi

  tab_keep=$(client "$main" "${as_host[@]}" python3 accept_tab_keep.py \
    http://127.0.0.1:9010/mcp /run/neo/host.sock /run/neo-accept/host-token 2>&1)
  if [ -n "$logs_dir" ]; then printf '%s\n' "$tab_keep" > "$logs_dir/tab-keep.json"; fi
  if [ "$(jget "$tab_keep" 'd["ok"]')" = True ]; then
    result PASS tab-keep "$(jget "$tab_keep" 'd["detail"]')"
  else
    result FAIL tab-keep "$(jget "$tab_keep" 'd["detail"]' || printf '%s' "${tab_keep:0:300}")"
  fi

  # --- mcp-remote, loopback-only half ----------------------------------------------------------
  tls=$(docker exec "$main" curl -sk -o /dev/null -w '%{http_code}' https://127.0.0.1:9011/system/ready)
  main_ip=$(docker inspect -f "{{(index .NetworkSettings.Networks \"$network\").IPAddress}}" "$main")
  remote_http=$(probe "$main_ip" 9010 http) remote_tls=$(probe "$main_ip" 9011 https)
  remote_off_ok=0
  if [ "$tls" = 200 ] && [ "$(jget "$remote_http" 'd["http"]')" = closed ] && [ "$(jget "$remote_tls" 'd["http"]')" = closed ]; then
    remote_off_ok=1
  fi

  # --- settings ------------------------------------------------------------------------------
  # The browser relaunches a killed server by itself and no host notices, so what the host set
  # must come back from /data before the server's first run. Only the server is killed.
  set_reply=$(host_send "$main" PATCH /v1/settings \
    '{"tabs":{"idleCloseSeconds":300},"trace":{"retentionSeconds":86400},"files":{"readDirs":["/etc","/data/downloads"]},"futureUnknown":true}')
  refused=$(host_send "$main" PATCH /v1/settings '{"tabs":{"idleCloseSeconds":5}}')
  restart_keep_tab=$(jget "$(host_get "$main" /v1/tabs)" 'next(t["tabId"] for t in d["body"]["items"] if t.get("owner") == "neo")')
  restart_keep=$(host_send "$main" PUT "/v1/tabs/$restart_keep_tab/keep" '{}')
  old_pid=$(server_pid "$main") new_pid="" read_back='{}'
  [ -n "$old_pid" ] && docker exec "$main" kill -KILL "$old_pid"
  for _ in $(seq 1 60); do
    new_pid=$(server_pid "$main")
    [ -n "$new_pid" ] && [ "$new_pid" != "$old_pid" ] && break
    sleep 1
  done
  for _ in $(seq 1 60); do
    read_back=$(host_get "$main" /v1/settings)
    [ "$(jget "$read_back" 'd["status"]')" = 200 ] && break
    sleep 1
  done
  file_mode=$(docker exec "$main" stat -c '%a %U' /data/browserclaw/claw-sdk/settings.json 2>&1)
  restart_keep_gone=$(jget "$(host_get "$main" /v1/tabs)" "any(t['tabId'] == $restart_keep_tab and 'keep' not in t for t in d['body']['items'])")
  # Compare only the keys this check set; added settings keep their defaults.
  if [ "$(jget "$set_reply" 'd["status"]')" = 200 ] && [ "$(jget "$set_reply" 'd["body"]["unknown"]')" = "['futureUnknown']" ] \
      && [ "$(jget "$refused" 'd["status"]')" = 400 ] && [ -n "$old_pid" ] && [ -n "$new_pid" ] && [ "$new_pid" != "$old_pid" ] \
      && [ "$(jget "$read_back" 'd["body"]["settings"]["tabs"]["idleCloseSeconds"]')" = 300 ] \
      && [ "$(jget "$read_back" 'd["body"]["settings"]["trace"]["retentionSeconds"]')" = 86400 ] \
      && [ "$(jget "$read_back" 'd["body"]["settings"]["files"]["readDirs"]')" = "['/etc', '/data/downloads']" ] \
      && [ "$(jget "$restart_keep" 'd["status"]')" = 200 ] && [ "$restart_keep_gone" = True ] \
      && [ "$file_mode" = "600 neo" ]; then
    result PASS settings "set 300 s / 1 day / files.readDirs=[/etc,/data/downloads], futureUnknown listed unknown, 5 s refused; server $old_pid killed, $new_pid relaunched, GET answers what the host set; settings.json $file_mode; tab keep ended across restart"
  else
    result FAIL settings "set=${set_reply:0:200}; refused=${refused:0:160}; pid $old_pid -> $new_pid; read=${read_back:0:200}; file=$file_mode; keep reset=$restart_keep_gone"
  fi
  host_send "$main" PUT /v1/settings '{}' >/dev/null

  # Root reads Neo's private files and rechecks the exact server PID before each restart.
  # The host primary gid still enforces the socket's peer rule. The probe restores trace
  # settings so the existing persistence checks retain their normal on-mode artifacts.
  trace_values=$(client "$main" setpriv --regid "$host_gid" --clear-groups python3 trace_values.py \
    http://127.0.0.1:9010/mcp /run/neo/host.sock /run/neo-accept/host-token 2>&1)
  if [ -n "$logs_dir" ]; then
    mkdir -p "$logs_dir"
    printf '%s\n' "$trace_values" > "$logs_dir/trace-values.json"
  fi
  if [ "$(jget "$trace_values" 'd["ok"]')" = True ]; then
    result PASS trace-values "$(jget "$trace_values" 'd["detail"]')"
  else
    result FAIL trace-values "$(jget "$trace_values" '"; ".join(d["problems"])' || printf '%s' "${trace_values:0:300}")"
  fi

  # --- restart-browser -----------------------------------------------------------------------
  docker exec "$main" neo-quit 9110; quit_status=$?
  restarted=0
  # grep reads the whole log, not -q: under pipefail an early -q match kills `docker logs`
  # with SIGPIPE while it is still writing, and the pipeline then fails on a long log.
  for _ in $(seq 1 60); do docker logs "$main" 2>&1 | grep -F 'Neo exited; restarting' >/dev/null && { restarted=1; break; }; sleep 1; done
  if [ "$quit_status" = 0 ] && [ "$restarted" = 1 ] && back=$(wait_ready "$main" 90) \
      && after=$(run_call "$main" "$task" "req-$run_id-after-quit" 2>&1) && run_ok "$after"; then
    result PASS restart-browser "neo-quit exit 0; Neo restarted and ready again after ${back}s; new run ok"
  else
    result FAIL restart-browser "quit=$quit_status restarted=$restarted after=${after:-none}"
  fi

  # --- stop ----------------------------------------------------------------------------------
  started=$(python3 -c 'import time; print(time.time())')
  docker stop -t 25 "$main" >/dev/null
  took=$(python3 -c "import time; print(round(time.time() - $started, 2))")
  code=$(docker inspect -f '{{.State.ExitCode}}' "$main")
  logs=$(docker logs "$main" 2>&1)
  if python3 -c "import sys; sys.exit(not $took < 20)" && [ "$code" = 0 ] \
      && grep -q '^\[neo\] Stopping Neo' <<<"$logs" && grep -q 'SIGTERM: stopping' <<<"$logs"; then
    result PASS stop "${took}s, exit 0, Neo quit and claw-sdk logged SIGTERM"
  else
    result FAIL stop "${took}s, exit $code, $(grep -cE 'Stopping Neo|SIGTERM: stopping' <<<"$logs") of 2 stop lines"
  fi

  # --- persist -------------------------------------------------------------------------------
  second="$prefix-persist"
  start_neo "$second" "$image"
  if wait_healthy "$second" 90 >/dev/null && install_client "$second"; then
    record=$(host_get "$second" "/v1/runs/$run_id_first")
    replay=$(run_call "$second" "$task" "$request" 2>&1)
    count=$(runs_for "$second" "$request")
    kept=$(jget "$(host_get "$second" "/v1/runs/${download_run:-none}/downloads/0")" 'd["status"]')
    kept_blob=$(jget "$(host_get "$second" "/v1/runs/${blob_run:-none}/blobs/${blob_id:-b0}")" 'd["status"]')
    if [ "$(jget "$record" 'd["status"]')" = 200 ] && [ "$(jget "$replay" 'd["meta"]["id"]')" = "$run_id_first" ] && [ "$count" = 1 ] \
        && [ "$kept" = 200 ] && [ "$kept_blob" = 200 ]; then
      result PASS persist "new container on the same /data: run readable, $request replayed as $run_id_first, 1 run, its download and blob still served"
    else
      result FAIL persist "record=${record:0:120}; replay=${replay:0:160}; runs=$count; download=$kept; blob=$kept_blob"
    fi
  else
    result FAIL persist "second container not healthy: $(docker logs "$second" 2>&1 | tail -3 | tr '\n' ' ')"
  fi
  remove "$second"

  # --- kill-all ------------------------------------------------------------------------------
  # Model a host that kills the whole cgroup on OOM and restarts the entrypoint itself.
  # PID 1 here is tini running that restart loop.
  looped="$prefix-kill-all"
  start_neo "$looped" --entrypoint /usr/bin/tini "$image" -- bash -c \
    'while :; do /usr/local/bin/neo-entrypoint; echo "[accept] entrypoint exited $?"; sleep 1; done'
  if wait_healthy "$looped" 90 >/dev/null && install_client "$looped"; then
    # Every process but tini, the loop (tini's child running `bash -c`) and this exec.
    docker exec "$looped" python3 -c '
import os, signal
def stat(pid):
    try:
        return open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()[1], open(f"/proc/{pid}/cmdline", "rb").read()
    except OSError:
        return None, b""
pids = [int(p) for p in os.listdir("/proc") if p.isdigit()]
keep = {1, os.getpid()} | {p for p in pids if stat(p)[0] == "1" and stat(p)[1].startswith(b"bash\0-c\0")}
for pid in pids:
    if pid not in keep and stat(pid)[0] not in (None, "0"):
        try: os.kill(pid, signal.SIGKILL)
        except OSError: pass
'
    restarted=0
    for _ in $(seq 1 30); do docker logs "$looped" 2>&1 | grep '^\[accept\] entrypoint exited 137' >/dev/null && { restarted=1; break; }; sleep 1; done
    ready=0
    for _ in $(seq 1 90); do [ "$(docker logs "$looped" 2>&1 | grep -c '^\[neo\] Ready')" -ge 2 ] && { ready=1; break; }; sleep 1; done
    healthy=$(wait_healthy "$looped" 30 >/dev/null && echo yes || echo no)
    record=$(host_get "$looped" "/v1/runs/$run_id_first")
    replay=$(run_call "$looped" "$task" "$request" 2>&1)
    count=$(runs_for "$looped" "$request")
    if [ "$restarted" = 1 ] && [ "$ready" = 1 ] && [ "$healthy" = yes ] && [ "$(jget "$record" 'd["status"]')" = 200 ] \
        && [ "$(jget "$replay" 'd["meta"]["id"]')" = "$run_id_first" ] && [ "$count" = 1 ]; then
      result PASS kill-all "SIGKILL of the tree, entrypoint restarted, healthy, socket answers, $request replayed as $run_id_first"
    else
      result FAIL kill-all "restarted=$restarted ready=$ready healthy=$healthy record=${record:0:100} replay=${replay:0:120} runs=$count"
    fi
  else
    result FAIL kill-all "restart-loop container not healthy"
  fi
  remove "$looped"

  # --- mcp-remote, remote-on half --------------------------------------------------------------
  open="$prefix-remote" open_http='{}' open_tls='{}'
  docker run --memory 3g --memory-swap 3g -d --name "$open" --network "$network" --shm-size=1g --tmpfs /data:rw,size=1g \
    --label neo-sdk-accept="$run_id" "${host_caps[@]}" -e NEO_MCP_REMOTE=true "$image" >/dev/null
  if wait_healthy "$open" 90 >/dev/null; then
    open_ip=$(docker inspect -f "{{(index .NetworkSettings.Networks \"$network\").IPAddress}}" "$open")
    open_http=$(probe "$open_ip" 9010 http) open_tls=$(probe "$open_ip" 9011 https)
  fi
  if [ "$remote_off_ok" = 1 ] && [ "$(jget "$open_http" 'd["http"]')" = "HTTP/1.1 200 OK" ] \
      && [ "$(jget "$open_tls" 'd["http"]')" = "HTTP/1.1 200 OK" ]; then
    result PASS mcp-remote "9011 TLS answers inside; sibling cut off on 9010/9011 by default, answered with NEO_MCP_REMOTE=true"
  else
    result FAIL mcp-remote "tls=$tls off: $remote_http $remote_tls; on: $open_http $open_tls"
  fi
  remove "$open"

  # --- captcha-keys --------------------------------------------------------------------------
  # Mounted outside /home/neo: the entrypoint re-owns that tree, which a read-only mount refuses.
  keyed="$prefix-keys" keyed_captcha=""
  docker run --memory 3g --memory-swap 3g -d --name "$keyed" --network "$network" --shm-size=1g --tmpfs /data:rw,size=1g \
    --label neo-sdk-accept="$run_id" "${host_caps[@]}" \
    -v "$secrets:/run/neo-accept:ro" \
    -e CLAW_HOST_TOKEN_FILE=/run/neo-accept/host-token -e NEO_HOST_GID=$host_gid \
    -v "$secrets/capsolver-api-key:/run/neo-keys/capsolver-api-key:ro" \
    -e BROWSEROS_CAPSOLVER_API_KEY_FILE=/run/neo-keys/capsolver-api-key \
    -e BROWSEROS_2CAPTCHA_API_KEY_FILE=/tmp/neo-accept-missing-key "$image" >/dev/null
  if wait_healthy "$keyed" 90 >/dev/null && install_client "$keyed"; then
    docker exec "$keyed" ln -s /tmp/neo-accept-future-key /tmp/neo-accept-missing-key
    keyed_captcha=$(lists_captcha "$keyed")
    # Root owns fixtures and compares saved bytes; the socket still requires its host group.
    filed=$(docker exec --user "0:$host_gid" -w /opt/neo-accept "$keyed" python3 accept_files.py \
      http://127.0.0.1:9010/mcp /run/neo/host.sock /run/neo-accept/host-token 2>&1)
    if [ "$(jget "$filed" 'd["ok"]')" = True ]; then
      result PASS files "$(jget "$filed" 'd["detail"]')"
    else
      result FAIL files "$(jget "$filed" '"; ".join(d["problems"])' || printf '%s' "${filed:0:300}")"
    fi
    if [ -n "$logs_dir" ]; then mkdir -p "$logs_dir"; printf '%s\n' "$filed" > "$logs_dir/files.json"; fi
  else
    result FAIL files "keyed container not healthy or client install failed"
  fi
  # The server's own word at its start (app.rs), from the key files it read: names of providers
  # and booleans only, never a key. A keyless image says solving is off.
  want_main=False
  if [ "$captcha_keys" = unrecorded ]; then [ "$keys_label" != baked ] || want_main=True
  elif [ -n "$captcha_keys" ]; then want_main=True; fi
  captcha_problems=()
  [ "$main_captcha" = "$want_main" ] || captcha_problems+=("image's own keys listed=$main_captcha (want $want_main)")
  [ "$keyed_captcha" = True ] || captcha_problems+=("host key file listed=${keyed_captcha:-not booted}")
  # The whole log first: under pipefail a reader that stops early (head) can kill `docker logs`.
  main_log=$(docker logs "$main" 2>&1)
  started_line=$(grep -F 'neo.solveCaptcha is ' <<<"$main_log" | sed -n 1p)
  # The providers to look for by name: none for an image built before captchaKeys, which says
  # only that it is keyed (its label); the on/off checks still hold for it.
  providers=""
  [ "$captcha_keys" = unrecorded ] || providers=${captcha_keys//,/ }
  if [ "$want_main" = True ]; then
    grep -qF 'neo.solveCaptcha is on' <<<"$started_line" || captcha_problems+=("server did not log solving on")
    for name in $providers; do
      flag=$([ "$name" = 2captcha ] && echo twocaptcha || echo "$name")
      grep -qE "\b$flag=true\b" <<<"$started_line" || captcha_problems+=("server did not read the $name key")
    done
    # The image's own keys under a host's other ids (the entrypoint re-owns Neo's home, the
    # keys' folder, to them): the server still reads every key, and no file door a script has
    # reads or overwrites one, even with / listed.
    # Started as the host-key container above is: an expendable tmpfs /data (start_neo's shared
    # volume would clash), the host's token and group, the image last.
    scratch_neo() { # name [docker run args...]
      local name=$1; shift
      docker run --memory 3g --memory-swap 3g -d --name "$name" --network "$network" --shm-size=1g \
        --tmpfs /data:rw,size=1g --label neo-sdk-accept="$run_id" "${host_caps[@]}" \
        -v "$secrets:/run/neo-accept:ro" -e CLAW_HOST_TOKEN_FILE=/run/neo-accept/host-token \
        -e NEO_HOST_GID=$host_gid "$@" "$image" >/dev/null
    }
    own="$prefix-own-keys" own_keys="" own_captcha=""
    scratch_neo "$own" -e PUID=2345 -e PGID=2345
    if wait_healthy "$own" 90 >/dev/null && install_client "$own"; then
      own_captcha=$(lists_captcha "$own")
      own_log=$(docker logs "$own" 2>&1)
      own_line=$(grep -F 'neo.solveCaptcha is ' <<<"$own_log" | sed -n 1p)
      own_keys=$(docker exec --user "0:$host_gid" -w /opt/neo-accept "$own" python3 accept_files.py \
        http://127.0.0.1:9010/mcp /run/neo/host.sock /run/neo-accept/host-token --image-keys 2>&1)
    fi
    remove "$own"
    [ "$own_captcha" = True ] || captcha_problems+=("under PUID/PGID 2345 listed=${own_captcha:-not booted}")
    for name in $providers; do
      flag=$([ "$name" = 2captcha ] && echo twocaptcha || echo "$name")
      grep -qE "\b$flag=true\b" <<<"${own_line:-}" || captcha_problems+=("under PUID/PGID 2345 the server did not read the $name key")
    done
    [ "$(jget "$own_keys" 'd["ok"]')" = True ] \
      || captcha_problems+=("own key files: $(jget "$own_keys" '"; ".join(d["problems"])' || printf '%s' "${own_keys:0:200}")")
    # A host turns solving off by setting both variables empty.
    off="$prefix-keys-off" off_captcha=""
    scratch_neo "$off" -e BROWSEROS_CAPSOLVER_API_KEY_FILE= -e BROWSEROS_2CAPTCHA_API_KEY_FILE=
    if wait_healthy "$off" 90 >/dev/null && install_client "$off"; then off_captcha=$(lists_captcha "$off"); fi
    remove "$off"
    [ "$off_captcha" = False ] || captcha_problems+=("empty key variables listed=${off_captcha:-not booted} (want False)")
  else
    grep -qF 'neo.solveCaptcha is off' <<<"$started_line" || captcha_problems+=("server did not log solving off")
  fi
  if [ ${#captcha_problems[@]} -eq 0 ]; then
    detail="listed: image's own keys $main_captcha, a host's read-only key file $keyed_captcha"
    [ "$want_main" != True ] || detail+="; server read ${providers:-its keys}, under PUID/PGID 2345 too; every script file door refused on own keys; empty variables turn it off"
    result PASS captcha-keys "$detail"
  else
    result FAIL captcha-keys "${captcha_problems[*]}"
  fi
else
  for check in settings-file settings-file-ids settings-file-link; do
    report_settings_file "$check" '{"ok":false,"checks":[{"name":"startup cases ran","ok":false}],"problems":["not run: the image did not boot"],"detail":"not run: the image did not boot; no startup-case volumes were created","before":null,"afterPatch":null}'
  done
  for check in mcp host-api downloads pdf-downloads blobs frames tab-keep takeover-scale settings files trace-values mcp-remote restart-browser stop persist kill-all captcha-keys; do
    result FAIL "$check" "not run: the image did not boot"
  done
fi

printf '\n'
printf '%s\n' "${results[@]}"
exit "$failed"
