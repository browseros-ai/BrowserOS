#!/usr/bin/env bash
# setup-desktop-entries.sh — Idempotent installer for BrowserOS desktop entries + icons
set -euo pipefail

HOME_DIR="${HOME}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
STABLE_ICON_DIR="${HOME_DIR}/.local/share/browseros/icons"
ICON_SRC_256="${STABLE_ICON_DIR}/browseros-256.png"
ICON_SRC_128="${REPO_ROOT}/packages/browseros-agent/apps/agent/public/icon/128.png"
APPIMAGE="${BROWSEROS_APP_PATH:-${HOME_DIR}/Downloads/alta/BrowserOS.AppImage}"
DEV_PROFILE="${HOME_DIR}/.browseros-dev-chrome"
DEV_EXT_DIR="${REPO_ROOT}/packages/browseros-agent/apps/agent/dist/chrome-mv3-dev"

ICON_DIR="${HOME_DIR}/.local/share/icons/hicolor"
APP_DIR="${HOME_DIR}/.local/share/applications"

# --- Extract icons from AppImage if not cached ---
ensure_icons_extracted() {
  mkdir -p "${STABLE_ICON_DIR}"
  if [[ ! -f "${STABLE_ICON_DIR}/browseros-256.png" ]] && [[ -x "${APPIMAGE}" ]]; then
    echo "Extracting icon from AppImage..."
    cd /tmp && "${APPIMAGE}" --appimage-extract browseros.png 2>/dev/null || true
    cp /tmp/squashfs-root/browseros.png "${STABLE_ICON_DIR}/browseros-256.png" 2>/dev/null || true
  fi
}

# --- Ensure hicolor index.theme exists ---
ensure_icon_theme_index() {
  if [[ ! -f "${ICON_DIR}/index.theme" ]]; then
    mkdir -p "${ICON_DIR}"
    cat > "${ICON_DIR}/index.theme" << 'THEME'
[Icon Theme]
Name=hicolor
Directories=128x128/apps,256x256/apps

[128x128/apps]
Size=128
Context=Applications
Type=Threshold

[256x256/apps]
Size=256
Context=Applications
Type=Threshold
THEME
  fi
}

# --- Install icons ---
install_icons() {
  ensure_icons_extracted
  ensure_icon_theme_index
  mkdir -p "${ICON_DIR}/256x256/apps" "${ICON_DIR}/128x128/apps"

  # Prod icons
  if [[ -f "${ICON_SRC_256}" ]]; then
    cp -f "${ICON_SRC_256}" "${ICON_DIR}/256x256/apps/browseros.png"
    echo "✔ Installed 256x256 prod icon"
  else
    echo "⚠ Missing ${ICON_SRC_256} — skip 256 prod icon"
  fi

  if [[ -f "${ICON_SRC_128}" ]]; then
    cp -f "${ICON_SRC_128}" "${ICON_DIR}/128x128/apps/browseros.png"
    echo "✔ Installed 128x128 prod icon"
  else
    echo "⚠ Missing ${ICON_SRC_128} — skip 128 prod icon"
  fi

  # Dev icons (green β badge overlay via Python/PIL)
  _create_dev_badge() {
    local size="$1" badge="$2" fsize="$3"
    local src="${ICON_DIR}/${size}x${size}/apps/browseros.png"
    local dst="${ICON_DIR}/${size}x${size}/apps/browseros-dev.png"
    if [[ ! -f "${src}" ]]; then
      echo "⚠ Missing ${src} — skip ${size} dev icon"
      return
    fi
    python3 - <<PYEOF
from PIL import Image, ImageDraw, ImageFont
size=${size}; badge=${badge}; fsize=${fsize}
src="${src}"; dst="${dst}"
img = Image.open(src).convert("RGBA")
overlay = Image.new("RGBA", img.size, (0,0,0,0))
draw = ImageDraw.Draw(overlay)
cx = img.size[0] - badge//2 - 4
cy = img.size[1] - badge//2 - 4
r = badge//2
draw.ellipse([cx-r, cy-r, cx+r, cy+r], fill=(34, 197, 94, 240))
try:
    font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", fsize)
except Exception:
    font = ImageFont.load_default(size=fsize)
bbox = draw.textbbox((0,0), "β", font=font)
tw, th = bbox[2]-bbox[0], bbox[3]-bbox[1]
draw.text((cx-tw//2, cy-th//2-2), "β", fill=(255,255,255,255), font=font)
Image.alpha_composite(img, overlay).save(dst, "PNG")
print(f"✔ Created {dst}")
PYEOF
  }

  _create_dev_badge 256 64 28
  _create_dev_badge 128 32 14

  gtk-update-icon-cache -f "${ICON_DIR}/" 2>/dev/null || true
  echo "✔ Icon cache updated"
}

# --- Desktop entries ---
install_desktop_entries() {
  mkdir -p "${APP_DIR}"

  cat > "${APP_DIR}/browseros.desktop" <<EOF
[Desktop Entry]
Version=1.0
Name=BrowserOS
GenericName=Web Browser
Comment=AI-powered browser agent
Exec=${APPIMAGE} --no-sandbox %U
Icon=browseros
Terminal=false
Type=Application
Categories=Network;WebBrowser;
StartupNotify=true
StartupWMClass=chromium-browser
EOF
  chmod +x "${APP_DIR}/browseros.desktop"
  echo "✔ Installed browseros.desktop"

  cat > "${APP_DIR}/browseros-dev.desktop" <<EOF
[Desktop Entry]
Version=1.0
Name=BrowserOS (Dev)
GenericName=Web Browser
Comment=BrowserOS Development Instance
Exec=${APPIMAGE} --no-sandbox --no-first-run --no-default-browser-check --use-mock-keychain --show-component-extension-options --disable-browseros-server --disable-browseros-extensions --name=BrowserOS-Dev --remote-debugging-port=9010 --browseros-mcp-port=9110 --browseros-server-port=9110 --user-data-dir=${DEV_PROFILE} --load-extension=${DEV_EXT_DIR} chrome://newtab
Icon=browseros-dev
Terminal=false
Type=Application
Categories=Development;Network;WebBrowser;
StartupNotify=true
StartupWMClass=BrowserOS-Dev
EOF
  chmod +x "${APP_DIR}/browseros-dev.desktop"
  echo "✔ Installed browseros-dev.desktop"

  update-desktop-database "${APP_DIR}/" 2>/dev/null || true
  echo "✔ Desktop database updated"
}

# --- Main ---
echo "=== BrowserOS Desktop Entry Setup ==="
install_icons
install_desktop_entries
echo "=== Done ==="
