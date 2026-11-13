#!/usr/bin/env bash
# Acceptance checks for a built Neo SDK image: it boots, drives a real page through `run`, keeps
# its host contract (README.md), and survives the stops and kills a host puts it through.
#
#   accept.sh <image> [--data DIR] [--logs DIR] [--prefix NAME]
#
#   --data    bind-mount DIR as /data and leave it there afterwards. Default: a named volume that
#             is removed with everything else.
#   --logs    save each container's log there, for CI artifacts.
#   --prefix  name what it creates NAME-<run>-*. Default: neo-sdk-accept.
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
#                    or websockify; python3 present; CAPTCHA keys present only when baked
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
#   mcp-remote       9011 is the same MCP over TLS; with NEO_MCP_REMOTE=false (the default) a
#                    sibling container is cut off on 9010 and 9011; with true it is answered
#   settings         PATCH /v1/settings applies and lists an unknown name, a bad value is 400 and
#                    changes nothing; after SIGKILL of the server alone (the browser relaunches it)
#                    GET answers what the host set; settings.json is 0600 neo under /data
#   files            downloads-only defaults; all six path reads and file chooser; live PATCH;
#                    symlinks; Neo's files and directory ancestors protected even with / listed;
#                    saveAs and all screenshot path forms refuse before send or mkdir
#   trace-values     on control detects canaries; off omits values from HTTP/events and raw files;
#                    blob survives a server restart, expires at startup, and its download stays
#   restart-browser  neo-quit 9110 quits cleanly, the supervisor restarts Neo, a new run passes
#   stop             docker stop -t 25 takes under 20 s, exits 0, Neo and claw-sdk log the stop
#   persist          a new container on the same /data reads the run and replays its requestId
#   kill-all         SIGKILL of the whole Neo tree (zuzu's OOM cgroup.kill) and a restart loop:
#                    healthy again, socket recreated, the run readable and replayed
#   captcha-keys     neo.solveCaptcha is listed only with a key: none in a keyless image, and
#                    listed once the host mounts a key file read-only and points the variable at it
set -uo pipefail

usage() { sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }

here=$(cd -- "$(dirname -- "$0")" && pwd)

image="" data_dir="" logs_dir="" name_prefix=neo-sdk-accept
while [ $# -gt 0 ]; do
  case "$1" in
    --data) data_dir=${2:?--data needs a directory}; shift 2 ;;
    --logs) logs_dir=${2:?--logs needs a directory}; shift 2 ;;
    --prefix) name_prefix=${2:?--prefix needs a name}; shift 2 ;;
    -h | --help) usage ;;
    -*) echo "accept.sh: unknown option '$1'" >&2; usage ;;
    *) [ -z "$image" ] || usage; image=$1; shift ;;
  esac
done
[ -n "$image" ] || usage
docker image inspect "$image" >/dev/null 2>&1 || { echo "accept.sh: no image '$image'" >&2; exit 2; }

run_id="$(date +%s)-$$"
prefix="$name_prefix-$run_id"
network="$prefix-net"
volume="$prefix-data"
host_gid=2005
secrets=$(mktemp -d)
# The token is mounted read-only for neo (uid PUID) and the test uids, so the folder is 0755.
chmod 0755 "$secrets"
od -An -tx1 -N24 /dev/urandom | tr -d ' \n' > "$secrets/host-token"
printf 'accept-dummy-capsolver-key' > "$secrets/capsolver-api-key"
chmod 0644 "$secrets/host-token" "$secrets/capsolver-api-key"
# The capabilities README.md says the entrypoint needs, and no more: zuzu's cells grant a set
# this small (no CAP_FSETID among them), so Docker's wider default would hide a missing one.
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
  for file in "$here/smoke.py" "$here/smoke_sdk.py" "$here/accept_client.py" "$here/accept_files.py" "$here/trace_values.py"; do
    docker cp -q "$file" "$1:/opt/neo-accept/" || return 1
  done
}

# The host as zuzu runs it: uid 2001 in the socket's group. The agent stand-in is uid 2002.
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
  [ "$(jget "$1" 'd["meta"]["status"]')" = ok ] && [ "$(jget "$1" 'd["value"]["value"]["value"]')" = zuzu ] \
    && [ "$(jget "$1" 'd["value"]["value"]["title"]')" = neo-sdk-accept ]
}
runs_for() { jget "$(host_get "$1" "/v1/runs?requestId=$2")" 'len(d["body"]["items"])'; }
lists_captcha() { # name: whether the run tool offers neo.solveCaptcha
  jget "$(client "$1" python3 accept_client.py describe http://127.0.0.1:9010/mcp)" \
    '"neo.solveCaptcha(page)" in d["description"]'
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
if [ "$keys_label" = baked ]; then
  [ "$(fact keys)" -ge 1 ] && [ "$(fact key_modes)" = "400:neo " ] || problems+=("baked keys: $(fact keys) files, modes $(fact key_modes)")
else
  [ "$(fact keys)" = 0 ] || problems+=("keyless image holds $(fact keys) key files")
fi
detail="$image_arch, browser $(fact browser), server $(fact marker) $(fact sha | cut -c1-12), rev $(fact revision | cut -c1-9), keys ${keys_label:-none}"
if [ ${#problems[@]} -eq 0 ]; then result PASS identity "$detail"; else result FAIL identity "${problems[*]}"; fi

if [ "$image_arch" != "$engine_arch" ]; then
  for check in boot mcp host-api downloads pdf-downloads blobs settings files trace-values mcp-remote restart-browser stop persist kill-all captcha-keys; do
    result SKIP "$check" "$image_arch image on a $engine_arch engine: emulated, not evidence"
  done
  printf '\n'; printf '%s\n' "${results[@]}"
  exit "$failed"
fi

docker network create --internal --label neo-sdk-accept="$run_id" "$network" >/dev/null || { echo "accept.sh: no network" >&2; exit 1; }
[ -n "$data_dir" ] || docker volume create --label neo-sdk-accept="$run_id" "$volume" >/dev/null

# --- boot ------------------------------------------------------------------------------------
main="$prefix-main"
start_neo "$main" -e CLAW_LOG=neo_server=debug,neo_playwright=debug,neo_trace=debug,neo_http=debug,neo_takeover=debug "$image"
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

  # --- downloads -----------------------------------------------------------------------------
  # zuzu's neo_download -> neo_upload and `import`, as the host uid with the socket's group.
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
  # zuzu's full-page screenshot and PDF, too large for a reply, read back on the host socket.
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
  # Compare only the keys this check set; added settings keep their defaults.
  if [ "$(jget "$set_reply" 'd["status"]')" = 200 ] && [ "$(jget "$set_reply" 'd["body"]["unknown"]')" = "['futureUnknown']" ] \
      && [ "$(jget "$refused" 'd["status"]')" = 400 ] && [ -n "$old_pid" ] && [ -n "$new_pid" ] && [ "$new_pid" != "$old_pid" ] \
      && [ "$(jget "$read_back" 'd["body"]["settings"]["tabs"]["idleCloseSeconds"]')" = 300 ] \
      && [ "$(jget "$read_back" 'd["body"]["settings"]["trace"]["retentionSeconds"]')" = 86400 ] \
      && [ "$(jget "$read_back" 'd["body"]["settings"]["files"]["readDirs"]')" = "['/etc', '/data/downloads']" ] \
      && [ "$file_mode" = "600 neo" ]; then
    result PASS settings "set 300 s / 1 day / files.readDirs=[/etc,/data/downloads], futureUnknown listed unknown, 5 s refused; server $old_pid killed, $new_pid relaunched, GET answers what the host set; settings.json $file_mode"
  else
    result FAIL settings "set=${set_reply:0:200}; refused=${refused:0:160}; pid $old_pid -> $new_pid; read=${read_back:0:200}; file=$file_mode"
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
  for _ in $(seq 1 60); do docker logs "$main" 2>&1 | grep -q 'Neo exited; restarting' && { restarted=1; break; }; sleep 1; done
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
  # zuzu restarts the entrypoint itself and its OOM path kills the whole cgroup at once
  # (cell/bin/entrypoint.sh). PID 1 here is tini running that restart loop.
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
    for _ in $(seq 1 30); do docker logs "$looped" 2>&1 | grep -q '^\[accept\] entrypoint exited 137' && { restarted=1; break; }; sleep 1; done
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
  want_main=False
  [ "$keys_label" != baked ] || want_main=True
  if [ "$main_captcha" = "$want_main" ] && [ "$keyed_captcha" = True ]; then
    result PASS captcha-keys "listed: image's own keys $main_captcha, a host's read-only key file $keyed_captcha"
  else
    result FAIL captcha-keys "image's own keys listed=$main_captcha (want $want_main); host key file listed=${keyed_captcha:-not booted}"
  fi
else
  for check in mcp host-api downloads pdf-downloads blobs settings files trace-values mcp-remote restart-browser stop persist kill-all captcha-keys; do
    result FAIL "$check" "not run: the image did not boot"
  done
fi

printf '\n'
printf '%s\n' "${results[@]}"
exit "$failed"
