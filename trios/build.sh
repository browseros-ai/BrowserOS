#!/bin/bash
set -e

# Derive project dir from the script location so the build is portable.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="${TRIOS_ROOT:-$SCRIPT_DIR}"
OUTPUT="$PROJECT_DIR/trios_app"
LOG_DIR="$PROJECT_DIR/.trinity/logs"
LOG_FILE="$LOG_DIR/build_$(date +%s).log"

mkdir -p "$LOG_DIR"

# Find all Swift files
SWIFT_FILES=(
    "$PROJECT_DIR/main.swift"
    $(find "$PROJECT_DIR/rings" -name "*.swift" | sort)
    $(find "$PROJECT_DIR/BR-OUTPUT" -name "*.swift" | sort)
)

echo "Compiling ${#SWIFT_FILES[@]} Swift files..."

# Build with swiftc
swiftc -O -o "$OUTPUT" \
    -framework SwiftUI \
    -framework AppKit \
    -framework WebKit \
    -framework Combine \
    "${SWIFT_FILES[@]}" 2>&1 | tee "$LOG_FILE"

if [ ${PIPESTATUS[0]} -eq 0 ]; then
    echo "[OK] Build successful: $OUTPUT"
    chmod +x "$OUTPUT"

    # Ensure .app bundle structure and a correct Info.plist. A missing or
    # stale plist disables macOS single-instance activation by bundle ID and is a
    # known cause of recursive self-launch cascades when `open trios.app` is
    # invoked repeatedly.
    APP_BUNDLE="$PROJECT_DIR/trios.app"
    MACOS_DIR="$APP_BUNDLE/Contents/MacOS"
    RESOURCES_DIR="$APP_BUNDLE/Contents/Resources"
    PLIST="$APP_BUNDLE/Contents/Info.plist"
    mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"
    cat > "$PLIST" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>trios</string>
    <key>CFBundleIdentifier</key><string>com.browseros.trios</string>
    <key>CFBundleName</key><string>Trios</string>
    <key>CFBundleVersion</key><string>1.0.0</string>
    <key>CFBundleShortVersionString</key><string>1.0.0</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>TRIOS_MESH_PORT</key><string>9505</string>
    <key>TRIOS_MCP_PORT</key><string>9105</string>
    <key>TRIOS_A2A_PORT</key><string>9200</string>
    <key>TRIOS_CANARY_MCP_PORT</key><string>9205</string>
    <key>TRIOS_VARIANT</key><string>prod</string>
</dict>
</plist>
EOF

    # Copy to .app bundle
    cp "$OUTPUT" "$MACOS_DIR/trios"
    echo "[OK] Copied to .app bundle with Info.plist (bundle ID: com.browseros.trios)"
else
    echo "[FAIL] Build failed (log: $LOG_FILE)"
    exit 1
fi
