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

: "${NEO_DATA_DIR:=/data}"
readonly NEO_DATA_DIR
readonly PROFILE="$NEO_DATA_DIR/profile"
readonly DOWNLOADS="$NEO_DATA_DIR/downloads"
# claw-server's database, recordings, skills and agent settings. It defaults to
# ~/.browserclaw, which is not on the volume and would vanish with the container.
readonly CLAW_DATA="$NEO_DATA_DIR/browserclaw"
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
# Agents drive Neo over MCP, which needs none of the bundled BrowserOS
# extensions, so they are off unless a deployment wants their UI or rrweb page
# recordings back (README).
: "${NEO_DISABLE_BROWSEROS_EXTENSIONS:=true}"
: "${PUID:=1000}" "${PGID:=1000}"
# Shared browser/server deployment flag (README). Hosted mode hides local-agent
# MCP tools and enables hosted browser reporting; both processes require exact 1.
: "${BROWSERCLAW_HOSTED_MODE:=1}"

is_true() {
  case "${1,,}" in
    true | 1 | yes | on) return 0 ;;
    false | 0 | no | off) return 1 ;;
    *) die "expected true or false, got '$1'" ;;
  esac
}

log() { printf '[neo] %s\n' "$*"; }
die() { log "$*" >&2; exit 1; }

# This directory becomes private and owned by Neo; it must identify a data
# folder rather than the container root or a working-directory-relative path.
[[ "$NEO_DATA_DIR" = /* ]] || die "NEO_DATA_DIR must be an absolute path"
[ "$(realpath -m -- "$NEO_DATA_DIR")" != / ] || die "NEO_DATA_DIR must not be the filesystem root"

# Validate before starting services so the browser and its server agree.
case "$BROWSERCLAW_HOSTED_MODE" in
  0 | 1) ;;
  *) die "BROWSERCLAW_HOSTED_MODE must be 0 or 1, got '$BROWSERCLAW_HOSTED_MODE'" ;;
esac

# Starts "$@" as the neo user in the background. The subshell execs, so $! is
# the program's own pid (setpriv, env and Neo's launcher all exec in turn) and
# signals reach it directly; a plain backgrounded function would leave $! on
# a wrapper shell that dies on SIGTERM while the program keeps running.
spawn_as_neo() {
  (exec setpriv --reuid=neo --regid=neo --init-groups \
    env HOME="$NEO_HOME" USER=neo DISPLAY=:1 XDG_RUNTIME_DIR="$XDG_DIR" LANG=C.UTF-8 \
      BROWSERCLAW_DIR="$CLAW_DATA" BROWSERCLAW_HOSTED_MODE="$BROWSERCLAW_HOSTED_MODE" "$@") &
}

# Bind-mounted host folders are owned by the host user; running Neo as that
# uid/gid is what lets it write the profile without a recursive chown.
apply_ids() {
  [[ "$PUID" =~ ^[0-9]+$ && "$PGID" =~ ^[0-9]+$ ]] || die "PUID and PGID must be numeric"
  [ "$(id -g neo)" = "$PGID" ] || groupmod -o -g "$PGID" neo
  [ "$(id -u neo)" = "$PUID" ] || usermod -o -u "$PUID" neo
  chown -R neo:neo "$NEO_HOME"
}

# Applies a jq update to one of Chromium's JSON state files (Local State,
# Preferences), creating the file when missing. Those files also hold every
# setting the user made, so they are updated in place and never replaced. Run
# it only while Neo is stopped: Chromium rewrites these files as it exits.
# Returns non-zero when the file is not valid JSON or cannot be written, and
# callers decide whether that stops startup. Callers test it with ||, which
# turns off errexit inside, so every step is checked explicitly.
merge_json() {
  local file=$1 update=$2 tmp
  tmp=$(mktemp) || return 1
  if [ -s "$file" ]; then
    jq "$update" "$file" > "$tmp"
  else
    jq -n "$update" > "$tmp"
  fi && mv "$tmp" "$file" && chown neo:neo "$file" && return 0
  rm -f "$tmp"
  return 1
}

prepare_data() {
  mkdir -p "$PROFILE" "$DOWNLOADS" "$CLAW_DATA" "$XDG_DIR"
  # Derived images may share the parent mount with other users. Limit ownership
  # changes to Neo's root and data folders, and keep the cookies private there.
  chown neo:neo "$NEO_DATA_DIR" "$PROFILE" "$DOWNLOADS" "$CLAW_DATA" "$XDG_DIR"
  chmod 700 "$NEO_DATA_DIR" "$XDG_DIR"
  ln -sfn "$DOWNLOADS" "$NEO_HOME/Downloads"

  # Neo's MCP proxy drops connections that are not from loopback unless this
  # Local State pref is set, and Docker's port forwarding arrives from the
  # bridge address.
  merge_json "$PROFILE/Local State" '.browseros.server.allow_remote_in_mcp = true' \
    || die "Could not update $PROFILE/Local State; if it is not valid JSON, fix or remove it"

  # Chromium's profile lock outlives the container that held it. No Neo can be
  # running against this profile yet, so a lock here is stale. (One container
  # per data folder; two would corrupt the profile.)
  rm -f "$PROFILE"/SingletonLock "$PROFILE"/SingletonSocket "$PROFILE"/SingletonCookie

  # Disabling network updates still lets Chromium select a previously downloaded
  # server. Clear only its ready selectors (legacy and Claw layouts), so an old
  # profile cannot override this image's server. Keep downloaded files and all
  # application data; the browser/server processes have not started yet.
  rm -f "$PROFILE/.browseros/current_version" \
    "$PROFILE/.browseros/BrowserClawServer/current_version"
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

# Suppress only BrowserOS's managed extension provider. Keep these arguments
# additive to NEO_ARGS so callers retain their own flags; user extensions and
# the native MCP server stay enabled.
browseros_extension_flags=()
if is_true "$NEO_DISABLE_BROWSEROS_EXTENSIONS"; then
  browseros_extension_flags=(--disable-browseros-extensions)
fi

# The viewer is for watching agents and for a person taking over a login, code
# or payment, so Neo hides the browser controls neither needs (README lists the
# names). --disable-browseros-features forces their prefs for this run only:
# the values beat choices saved in the profile, are never written to it, and
# cannot be changed from the viewer. Chromium keeps the last value of a repeated
# switch, so a deployment replaces this whole list with its own
# --disable-browseros-features in NEO_ARGS, which start_neo passes last.
# Browsers before 0.51.2.5 ignore the switch and show every control.
readonly HIDDEN_BROWSEROS_FEATURES=VerticalTabs,HomeButton,SplitTabButton,TabSearchButton,BookmarkEditing,WebAppInstall,AvatarButton,ExtensionsToolbar,NewTabPage

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

# The two parts of the minimal frame that --disable-browseros-features has no
# names for (see HIDDEN_BROWSEROS_FEATURES): the bookmarks bar and the window
# buttons. Applied each time the container starts.
apply_minimal_ui() {
  local managed=/etc/chromium/policies/managed
  mkdir -p "$managed"
  echo '{"BookmarkBarEnabled": false}' > "$managed/neo-minimal-ui.json"

  # Chromium takes its frame-button layout from GTK. openbox keeps Neo
  # maximized, there is no taskbar to restore a minimized window from, and Neo
  # is restarted here whenever it closes, so the frame needs no buttons.
  local gtk
  for gtk in gtk-3.0 gtk-4.0; do
    mkdir -p "$NEO_HOME/.config/$gtk"
    printf '[Settings]\ngtk-decoration-layout=:\n' > "$NEO_HOME/.config/$gtk/settings.ini"
  done
  chown -R neo:neo "$NEO_HOME/.config"
}

start_neo() {
  # Releases verify the packaged/overlaid server, so pin it for the image's
  # lifetime. Keep this outside NEO_ARGS, which Compose/callers may replace.
  # NEO_ARGS goes last so a caller's value for a repeated switch wins.
  # shellcheck disable=SC2086  # NEO_ARGS is a flag list by design
  spawn_as_neo browserclaw \
    "${sandbox_flags[@]}" \
    "${browseros_extension_flags[@]}" \
    --user-data-dir="$PROFILE" \
    --no-first-run \
    --no-default-browser-check \
    --password-store=basic \
    --browseros-proxy-port=9010 \
    --browseros-cdp-port="$CDP_PORT" \
    --browseros-server-port="$SERVER_PORT" \
    --disable-browseros-server-updater \
    --hide-crash-restore-bubble \
    --start-maximized \
    --disable-browseros-features="$HIDDEN_BROWSEROS_FEATURES" \
    $NEO_ARGS
  neo_pid=$!
  neo_signaled=0
}

# Applies container-level agent settings once claw-server answers, then marks
# the container configured; the healthcheck waits for that marker so agents
# never connect before the settings hold. Hosted mode already keeps human help
# off, but a claw-server older than hosted-mode support ignores the flag and
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
apply_minimal_ui
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
