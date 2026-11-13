#!/bin/sh
# neo-server-info: which Neo server this image runs, and where it lives.
#
#   neo-server-info path            the one bundled server executable, which the browser launches
#   neo-server-info kind [binary]   claw-server or claw-sdk (default binary: the one `path` prints)
#
# The browser launches the file named browseros-claw-server whatever was installed over
# it, so the name says nothing about the server. Each server build compiles in a version
# marker, browseros-<kind>-version=<version>;, and reading it runs nothing. A binary
# without the claw-sdk marker is claw-server, because servers bundled before markers
# existed are all claw-server. The Dockerfile, the entrypoint and the release checks all
# ask here, so what the image does can never disagree with the server it runs.
set -eu

die() { echo "neo-server-info: $*" >&2; exit 1; }

# Exactly one, so a changed package layout fails instead of using the wrong file.
server_path() {
  paths=$(find /usr/lib /opt -type f -name browseros-claw-server) \
    && [ -n "$paths" ] && [ "$(printf '%s\n' "$paths" | wc -l)" -eq 1 ] \
    || die "expected exactly one browseros-claw-server under /usr/lib or /opt"
  printf '%s\n' "$paths"
}

case "${1:-}" in
  path) server_path ;;
  kind)
    binary=${2:-$(server_path)}
    # grep exits 1 when the marker is absent and 2 when it cannot read the file. Only
    # the first means claw-server: the kind decides whether the display stays closed.
    status=0
    LC_ALL=C grep -aqF 'browseros-claw-sdk-version=' "$binary" || status=$?
    case $status in
      0) echo claw-sdk ;;
      1) echo claw-server ;;
      *) die "could not read $binary" ;;
    esac
    ;;
  *) echo "usage: neo-server-info path | kind [binary]" >&2; exit 2 ;;
esac
