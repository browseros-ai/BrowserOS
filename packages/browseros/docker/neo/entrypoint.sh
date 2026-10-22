#!/usr/bin/env bash
# Container supervisor for the BrowserOS neo image. Runs as root under tini.
#
#   Xvnc :1 (X server + VNC on loopback :5900) ◀── draws ── Neo ── starts ──▶ claw-server
#     │                                                      └─ MCP proxy :9010 ◀── agents
#     └── websockify :6080 (noVNC page + WebSocket) ◀── browser tab
#   openbox keeps Neo maximized and follows viewer-driven screen resizes.
#
# Only Neo is restarted here, because users can close it. If Xvnc, openbox or
# websockify exits, the container exits and Docker's restart policy takes over.
set -euo pipefail

readonly PROFILE=/data/profile
readonly DOWNLOADS=/data/downloads
# claw-server's database, recordings, skills and agent settings. It defaults to
# ~/.browserclaw, which is not on the volume and would vanish with the container.
readonly CLAW_DATA=/data/browserclaw
readonly NEO_HOME=/home/neo
readonly XDG_DIR=/tmp/neo-xdg
readonly CONFIGURED=/tmp/neo-configured
readonly MCP_URL=http://127.0.0.1:9010
# Neo otherwise picks CDP and claw-server ports at launch (default, or the next
# free one) and can land elsewhere after a restart. Nothing else in the
# container competes for them, so pin them; neo-quit relies on the CDP port.
readonly CDP_PORT=9110
readonly SERVER_PORT=9210

: "${NEO_SCREEN:=1600x1000}" "${NEO_SANDBOX:=false}" "${NEO_ARGS:=}"
: "${PUID:=1000}" "${PGID:=1000}"
# claw-server's mode (README). hosted hides the MCP tools that assume an agent
# harness on this machine or a person at the cockpit: skills, human help and
# name_session, which taskName replaces.
: "${BROWSERCLAW_MODE:=hosted}"

is_true() {
  case "${1,,}" in
    true | 1 | yes | on) return 0 ;;
    false | 0 | no | off) return 1 ;;
    *) die "expected true or false, got '$1'" ;;
  esac
}

log() { printf '[neo] %s\n' "$*"; }
die() { log "$*" >&2; exit 1; }

# Starts "$@" as the neo user in the background. The subshell execs, so $! is
# the program's own pid (setpriv, env and Neo's launcher all exec in turn) and
# signals reach it directly; a plain backgrounded function would leave $! on
# a wrapper shell that dies on SIGTERM while the program keeps running.
spawn_as_neo() {
  (exec setpriv --reuid=neo --regid=neo --init-groups \
    env HOME="$NEO_HOME" USER=neo DISPLAY=:1 XDG_RUNTIME_DIR="$XDG_DIR" LANG=C.UTF-8 \
      BROWSERCLAW_DIR="$CLAW_DATA" BROWSERCLAW_MODE="$BROWSERCLAW_MODE" "$@") &
}

# Bind-mounted host folders are owned by the host user; running Neo as that
# uid/gid is what lets it write the profile without a recursive chown.
apply_ids() {
  [[ "$PUID" =~ ^[0-9]+$ && "$PGID" =~ ^[0-9]+$ ]] || die "PUID and PGID must be numeric"
  [ "$(id -g neo)" = "$PGID" ] || groupmod -o -g "$PGID" neo
  [ "$(id -u neo)" = "$PUID" ] || usermod -o -u "$PUID" neo
  chown -R neo:neo "$NEO_HOME"
}

prepare_data() {
  mkdir -p "$PROFILE" "$DOWNLOADS" "$CLAW_DATA" "$XDG_DIR" "$NEO_HOME/.config/gtk-3.0" "$NEO_HOME/.config/gtk-4.0"
  chown neo:neo /data "$PROFILE" "$DOWNLOADS" "$CLAW_DATA" "$XDG_DIR"
  chmod 700 "$XDG_DIR"
  ln -sfn "$DOWNLOADS" "$NEO_HOME/Downloads"

  # There is no taskbar to restore a minimized window from, so hide Neo's
  # minimize button. Chromium takes its frame-button layout from GTK.
  for gtk in gtk-3.0 gtk-4.0; do
    printf '[Settings]\ngtk-decoration-layout=:maximize,close\n' > "$NEO_HOME/.config/$gtk/settings.ini"
  done
  chown -R neo:neo "$NEO_HOME/.config"

  # Neo's MCP proxy drops connections that are not from loopback unless this
  # Local State pref is set, and Docker's port forwarding arrives from the
  # bridge address. Merge it: the file holds the profile's other browser-wide
  # settings and must never be replaced.
  local state="$PROFILE/Local State" tmp
  tmp=$(mktemp)
  if [ -s "$state" ]; then
    jq '.browseros.server.allow_remote_in_mcp = true' "$state" > "$tmp" \
      || die "$state is not valid JSON; fix or remove it"
  else
    echo '{"browseros":{"server":{"allow_remote_in_mcp":true}}}' > "$tmp"
  fi
  mv "$tmp" "$state"
  chown neo:neo "$state"

  # Chromium's profile lock outlives the container that held it. No Neo can be
  # running against this profile yet, so a lock here is stale. (One container
  # per data folder; two would corrupt the profile.)
  rm -f "$PROFILE"/SingletonLock "$PROFILE"/SingletonSocket "$PROFILE"/SingletonCookie
}

start_display() {
  # X servers expect a root-owned, sticky socket directory.
  mkdir -p /tmp/.X11-unix && chmod 1777 /tmp/.X11-unix
  spawn_as_neo Xvnc :1 -geometry "$NEO_SCREEN" -depth 24 -SecurityTypes None \
    -localhost -rfbport 5900 -AlwaysShared -nolisten tcp
  xvnc_pid=$!
  for _ in $(seq 1 100); do
    [ -S /tmp/.X11-unix/X1 ] && break
    kill -0 "$xvnc_pid" 2>/dev/null || die "Xvnc failed to start"
    sleep 0.1
  done
  [ -S /tmp/.X11-unix/X1 ] || die "Xvnc did not create display :1"

  # Bridges the VNC clipboard and the X selection; without it copy and paste
  # through noVNC does nothing.
  spawn_as_neo vncconfig -nowin
  vncconfig_pid=$!
  spawn_as_neo openbox --config-file /etc/neo/openbox-rc.xml
  openbox_pid=$!
  spawn_as_neo websockify --web /usr/share/novnc 6080 localhost:5900 >/dev/null 2>&1
  websockify_pid=$!
}

# Chromium's sandbox needs user namespaces, which Docker's default seccomp
# profile (and AppArmor on Ubuntu hosts) denies, so it is off unless the
# container was started with security options that allow it (README).
sandbox_flags=(--no-sandbox)
is_true "$NEO_SANDBOX" && sandbox_flags=()

# --no-sandbox is deliberate here, so drop Chromium's "unsupported command-line
# flag" bar for it. Written only in that mode: the same policy would also hide
# warnings for risky flags passed through NEO_ARGS. This build reads policies
# from /etc/chromium/policies.
apply_policies() {
  local managed=/etc/chromium/policies/managed
  rm -f "$managed/neo-docker.json"
  if [ "${#sandbox_flags[@]}" -gt 0 ]; then
    mkdir -p "$managed"
    echo '{"CommandLineFlagSecurityWarningsEnabled": false}' > "$managed/neo-docker.json"
  fi
}

start_neo() {
  # shellcheck disable=SC2086  # NEO_ARGS is a flag list by design
  spawn_as_neo browserclaw \
    "${sandbox_flags[@]}" \
    --user-data-dir="$PROFILE" \
    --no-first-run \
    --no-default-browser-check \
    --password-store=basic \
    --browseros-proxy-port=9010 \
    --browseros-cdp-port="$CDP_PORT" \
    --browseros-server-port="$SERVER_PORT" \
    --hide-crash-restore-bubble \
    --start-maximized \
    $NEO_ARGS
  neo_pid=$!
  neo_signaled=0
}

# Applies container-level agent settings once claw-server answers, then marks
# the container configured; the healthcheck waits for that marker so agents
# never connect before the settings hold. Hosted mode already keeps human help
# off, but a claw-server older than BROWSERCLAW_MODE ignores the mode and
# defaults human help on, so it is switched off here too on every start.
configure_neo() {
  for _ in $(seq 1 240); do
    curl -fsS "$MCP_URL/system/ready" >/dev/null 2>&1 && break
    sleep 0.5
  done
  curl -fsS "$MCP_URL/system/ready" >/dev/null 2>&1 || { log "Neo did not become ready"; return 1; }
  curl -fsS -X PUT "$MCP_URL/api/v1/settings/agent" -H 'Content-Type: application/json' \
    -d '{"humanHelpEnabled":false}' >/dev/null || { log "Could not apply agent settings"; return 1; }
  touch "$CONFIGURED"
  log "Ready. These are the container's ports; use the host ports you mapped with -p."
  log "  Viewer: http://localhost:6080"
  log "  MCP:    http://localhost:9010/mcp"
  log "  Add to Claude Code: claude mcp add --transport http neo http://localhost:9010/mcp"
}

stopping=0
neo_pid=""
neo_signaled=0

# Quit Neo the way a user would (CDP Browser.close, see neo-quit.py) so it
# records a clean exit and flushes cookies; Chromium's own SIGTERM handling
# skips both. SIGTERM, sent once, is only the fallback.
term_neo() {
  if [ "$neo_signaled" = 0 ] && [ -n "$neo_pid" ]; then
    neo_signaled=1
    neo-quit "$CDP_PORT" || kill -TERM "$neo_pid" 2>/dev/null || true
  fi
}

# shellcheck disable=SC2329  # invoked by the TERM/INT trap
on_term() {
  stopping=1
  term_neo
}

# Neo first so it can save cookies and the session, then the display stack.
shutdown() {
  local code=$1
  if [ -n "$neo_pid" ] && kill -0 "$neo_pid" 2>/dev/null; then
    log "Stopping Neo"
    term_neo
    for _ in $(seq 1 150); do kill -0 "$neo_pid" 2>/dev/null || break; sleep 0.1; done
    kill -KILL "$neo_pid" 2>/dev/null || true
  fi
  [ -n "$configure_pid" ] && kill -TERM "$configure_pid" 2>/dev/null || true
  kill -TERM "$websockify_pid" "$openbox_pid" "$vncconfig_pid" "$xvnc_pid" 2>/dev/null || true
  wait 2>/dev/null || true
  exit "$code"
}

trap on_term TERM INT

rm -f "$CONFIGURED"
apply_ids
prepare_data
apply_policies
start_display
start_neo
configure_neo &
configure_pid=$!

while [ "$stopping" = 0 ]; do
  # A failed configure (Neo never ready, settings rejected) would otherwise
  # leave the container unhealthy forever; exiting lets Docker restart it.
  if [ -n "$configure_pid" ] && ! kill -0 "$configure_pid" 2>/dev/null; then
    if wait "$configure_pid"; then
      configure_pid=""
    else
      log "Could not configure Neo; stopping the container"
      shutdown 1
    fi
  fi
  for name in xvnc openbox websockify; do
    pid_var="${name}_pid"
    if ! kill -0 "${!pid_var}" 2>/dev/null; then
      log "$name exited; stopping the container"
      shutdown 1
    fi
  done
  if ! kill -0 "$neo_pid" 2>/dev/null; then
    wait "$neo_pid" 2>/dev/null || true
    log "Neo exited; restarting"
    sleep 2
    [ "$stopping" = 0 ] && start_neo
  fi
  # Backgrounded sleep keeps the loop interruptible, so `docker stop` is
  # handled within a second instead of after the next poll.
  sleep 1 &
  wait $! 2>/dev/null || true
done

shutdown 0
