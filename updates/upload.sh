#!/bin/bash

set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

echo "=== CDN feed upload ==="
echo
echo "Browser appcasts:"
echo "  1) BrowserOS macOS arm64        (appcast.xml)"
echo "  2) BrowserOS macOS x64          (appcast-x86_64.xml)"
echo "  3) BrowserOS Windows x64        (appcast-win.xml)"
echo "  4) BrowserOS Windows arm64      (appcast-win-arm64.xml)"
echo "  5) BrowserClaw macOS arm64      (appcast-claw.xml)"
echo "  6) BrowserClaw macOS x64        (appcast-claw-x86_64.xml)"
echo "  7) BrowserClaw Windows x64      (appcast-claw-win.xml)"
echo "  8) BrowserClaw Windows arm64    (appcast-claw-win-arm64.xml)"
echo
echo "Server appcasts:"
echo "  9) BrowserOS Production         (appcast-server.xml)"
echo " 10) BrowserOS Alpha              (appcast-server.alpha.xml)"
echo " 11) BrowserClaw Production       (appcast-claw-server.xml)"
echo " 12) BrowserClaw Alpha            (appcast-claw-server.alpha.xml)"
echo
echo "Extensions:"
echo " 13) Production         (extensions.json + update-manifest.xml)"
echo " 14) Alpha              (extensions.alpha.json + update-manifest.alpha.xml)"
echo " 15) Bundled            (bundled-manifest.xml)"
echo
echo "  a) All files"
echo "  q) Quit"
echo

read -r -p "Choices [1-15; comma/space separated; a; q]: " choice

if [[ "$choice" == "q" ]]; then
    echo "Cancelled"
    exit 0
fi

invalid_choice() {
    echo "Invalid choice" >&2
    exit 1
}

if [[ "$choice" == "a" ]]; then
    selections=(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15)
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
        [[ "$token" =~ ^([1-9]|1[0-5])$ ]] || invalid_choice
        if [[ "$seen" != *" $token "* ]]; then
            selections+=("$token")
            seen+="$token "
        fi
    done
fi

keys=()
for selection in "${selections[@]}"; do
    case "$selection" in
        1) keys+=("appcast.xml") ;;
        2) keys+=("appcast-x86_64.xml") ;;
        3) keys+=("appcast-win.xml") ;;
        4) keys+=("appcast-win-arm64.xml") ;;
        5) keys+=("appcast-claw.xml") ;;
        6) keys+=("appcast-claw-x86_64.xml") ;;
        7) keys+=("appcast-claw-win.xml") ;;
        8) keys+=("appcast-claw-win-arm64.xml") ;;
        9) keys+=("appcast-server.xml") ;;
        10) keys+=("appcast-server.alpha.xml") ;;
        11) keys+=("appcast-claw-server.xml") ;;
        12) keys+=("appcast-claw-server.alpha.xml") ;;
        13)
            keys+=("extensions/extensions.json")
            keys+=("extensions/update-manifest.xml")
            ;;
        14)
            keys+=("extensions/extensions.alpha.json")
            keys+=("extensions/update-manifest.alpha.xml")
            ;;
        15) keys+=("extensions/bundled-manifest.xml") ;;
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
