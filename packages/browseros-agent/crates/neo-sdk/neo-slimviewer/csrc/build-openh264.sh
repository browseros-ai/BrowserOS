#!/bin/sh
# Source only. A moved tag fails the build rather than changing the binary silently.
set -eu
prefix=${1:?usage: build-openh264.sh absolute-install-prefix}
case "$prefix" in /*) ;; *) echo 'prefix must be absolute' >&2; exit 2;; esac
source_dir=$(mktemp -d)
trap 'rm -rf "$source_dir"' EXIT HUP INT TERM
git clone --quiet --depth 1 --branch v2.7.0 https://github.com/cisco/openh264.git "$source_dir"
commit=1bca62311e447fd9c39fc93de2c4b96cd5ce971c
test "$(git -C "$source_dir" rev-parse HEAD)" = "$commit"
make -C "$source_dir" -j"${OPENH264_JOBS:-2}" BUILDTYPE=Release PREFIX="$prefix" install-static
install -D -m 644 "$source_dir/LICENSE" "$prefix/share/doc/openh264/LICENSE"
printf '%s\n' "v2.7.0 $commit" > "$prefix/share/doc/openh264/SOURCE"
