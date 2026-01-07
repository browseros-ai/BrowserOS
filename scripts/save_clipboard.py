#!/usr/bin/env python3
"""
Save clipboard image to a specified path.
Usage: python scripts/save_clipboard.py <output_path>
"""
import sys
import os

try:
    from PIL import ImageGrab
except ImportError:
    print("Installing Pillow...")
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "Pillow", "-q"])
    from PIL import ImageGrab

def main():
    if len(sys.argv) != 2:
        print("Usage: python scripts/save_clipboard.py <output_path>")
        print("Example: python scripts/save_clipboard.py docs/images/screenshot.png")
        sys.exit(1)

    output_path = sys.argv[1]

    # Ensure directory exists
    os.makedirs(os.path.dirname(output_path) or ".", exist_ok=True)

    # Grab from clipboard
    img = ImageGrab.grabclipboard()

    if img is None:
        print("❌ No image in clipboard. Copy an image first (Cmd+C).")
        sys.exit(1)

    img.save(output_path)
    print(f"✅ Saved to {output_path}")

if __name__ == "__main__":
    main()
