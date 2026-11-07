#!/bin/bash

set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

# The original BrowserOS feeds (appcast.xml, appcast-x86_64.xml,
# appcast-win*.xml, appcast-server*.xml) are published by browseros-human.
echo "=== CDN feed upload ==="
echo
echo "Browser appcasts:"
echo "  1) BrowserClaw macOS arm64      (appcast-claw.xml)"
echo "  2) BrowserClaw macOS x64        (appcast-claw-x86_64.xml)"
echo "  3) BrowserClaw Windows x64      (appcast-claw-win.xml)"
echo "  4) BrowserClaw Windows arm64    (appcast-claw-win-arm64.xml)"
echo
echo "Server appcasts:"
echo "  5) BrowserClaw Production       (appcast-claw-server.xml)"
echo "  6) BrowserClaw Alpha            (appcast-claw-server.alpha.xml)"
echo
echo "Extensions:"
echo "  7) Production         (extensions.json + update-manifest.xml)"
echo "  8) Alpha              (extensions.alpha.json + update-manifest.alpha.xml)"
echo "  9) Bundled            (bundled-manifest.xml)"
echo
echo "  a) All files"
echo "  q) Quit"
echo

read -r -p "Choices [1-9; comma/space separated; a; q]: " choice

if [[ "$choice" == "q" ]]; then
    echo "Cancelled"
    exit 0
fi

invalid_choice() {
    echo "Invalid choice" >&2
    exit 1
}

if [[ "$choice" == "a" ]]; then
    selections=(1 2 3 4 5 6 7 8 9)
else
    if [[ -z "$choice" ||
          "$choice" =~ [^0-9,[:space:]] ||
          "$choice" =~ ^[[:space:]]*, ||
          "$choice" =~ ,[[:space:]]*$ ||
          "$choice" =~ ,[[:space:]]*, ]]; then
        invalid_choice
    fi

    read -r -a tokens <<< "${choice//,/ }"
    selections=()
    seen=" "
    for token in "${tokens[@]}"; do
        [[ "$token" =~ ^[1-9]$ ]] || invalid_choice
        if [[ "$seen" != *" $token "* ]]; then
            selections+=("$token")
            seen+="$token "
        fi
    done
fi

keys=()
for selection in "${selections[@]}"; do
    case "$selection" in
        1) keys+=("appcast-claw.xml") ;;
        2) keys+=("appcast-claw-x86_64.xml") ;;
        3) keys+=("appcast-claw-win.xml") ;;
        4) keys+=("appcast-claw-win-arm64.xml") ;;
        5) keys+=("appcast-claw-server.xml") ;;
        6) keys+=("appcast-claw-server.alpha.xml") ;;
        7)
            keys+=("extensions/extensions.json")
            keys+=("extensions/update-manifest.xml")
            ;;
        8)
            keys+=("extensions/extensions.alpha.json")
            keys+=("extensions/update-manifest.alpha.xml")
            ;;
        9) keys+=("extensions/bundled-manifest.xml") ;;
    esac
done

cd "$REPO_ROOT/packages/browseros"
command=(uv run browseros release feeds publish-local "${keys[@]}")
"${command[@]}"

echo
read -r -p "Publish these feeds? [y/N]: " confirm
if [[ "$confirm" =~ ^[Yy]$ ]]; then
    "${command[@]}" --publish
else
    echo "Dry run complete; nothing published"
fi
