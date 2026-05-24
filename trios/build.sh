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
    # Copy to .app bundle
    cp "$OUTPUT" "$PROJECT_DIR/trios.app/Contents/MacOS/trios"
    echo "✅ Copied to .app bundle"
else
    echo "❌ Build failed"
    exit 1
fi
