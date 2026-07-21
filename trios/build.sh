#!/bin/bash
set -e

PROJECT_DIR="/Users/playra/BrowserOS-full/trios"
OUTPUT="$PROJECT_DIR/trios_app"

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
    "${SWIFT_FILES[@]}" 2>&1 | tee /tmp/trios_build.log

if [ ${PIPESTATUS[0]} -eq 0 ]; then
    echo "✅ Build successful: $OUTPUT"
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
</dict>
</plist>
EOF

    # Copy to .app bundle
    cp "$OUTPUT" "$MACOS_DIR/trios"
    echo "✅ Copied to .app bundle with Info.plist (bundle ID: com.browseros.trios)"
else
    echo "❌ Build failed"
    exit 1
fi
