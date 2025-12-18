#!/usr/bin/env python3
"""
Unified icon generation script for BrowserOS.

Generates all platform-specific icons (Windows, macOS, Linux, ChromeOS) from
a single high-resolution source PNG.

Requirements:
- Python 3.12+
- Pillow (pip install Pillow)
- ImageMagick (brew install imagemagick) - for XPM generation
- macOS tools (iconutil, actool) - for .icns and Assets.car generation

Usage:
    python generate_icons.py [--config generate_icons.txt]
"""

import shutil
import subprocess
import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    print("Error: Pillow is required. Install with: pip install Pillow")
    sys.exit(1)

SCRIPT_DIR = Path(__file__).parent
DEFAULT_CONFIG = SCRIPT_DIR / "generate_icons.txt"
SOURCE_DIR = SCRIPT_DIR / "source"
STATIC_DIR = SCRIPT_DIR / "static"
OUTPUT_DIR = SCRIPT_DIR.parent.parent.parent / "resources" / "icons"

MIN_SOURCE_SIZE = 1024


def validate_source(source_path: Path) -> Image.Image:
    """Load and validate source image meets minimum requirements."""
    if not source_path.exists():
        print(f"✗ Error: Source file not found: {source_path}")
        sys.exit(1)

    img = Image.open(source_path)

    if img.width < MIN_SOURCE_SIZE or img.height < MIN_SOURCE_SIZE:
        print(f"✗ Error: Source image is {img.width}x{img.height}")
        print(f"  Minimum required: {MIN_SOURCE_SIZE}x{MIN_SOURCE_SIZE}")
        sys.exit(1)

    if img.mode != "RGBA":
        img = img.convert("RGBA")

    print(f"✓ Source validated: {source_path.name} ({img.width}x{img.height})")
    return img


def generate_png(img: Image.Image, size: int, output_path: Path) -> bool:
    """Generate a PNG at specified size."""
    try:
        output_path.parent.mkdir(parents=True, exist_ok=True)
        resized = img.resize((size, size), Image.Resampling.LANCZOS)
        resized.save(output_path, "PNG", optimize=True)
        return True
    except Exception as e:
        print(f"  ✗ Failed to generate {output_path}: {e}")
        return False


def generate_mono_png(img: Image.Image, size: int, output_path: Path) -> bool:
    """Generate a monochrome (white silhouette) PNG from alpha channel."""
    try:
        output_path.parent.mkdir(parents=True, exist_ok=True)
        resized = img.resize((size, size), Image.Resampling.LANCZOS)

        # Extract alpha channel and create white silhouette
        if resized.mode != "RGBA":
            resized = resized.convert("RGBA")

        # Create new image: white where alpha > 0, transparent elsewhere
        r, g, b, a = resized.split()
        white = Image.new("L", resized.size, 255)
        mono = Image.merge("RGBA", (white, white, white, a))

        mono.save(output_path, "PNG", optimize=True)
        return True
    except Exception as e:
        print(f"  ✗ Failed to generate mono {output_path}: {e}")
        return False


def generate_ico(img: Image.Image, sizes: list[int], output_path: Path) -> bool:
    """Generate Windows ICO with multiple sizes."""
    try:
        output_path.parent.mkdir(parents=True, exist_ok=True)

        sorted_sizes = sorted(sizes, reverse=True)
        icons = []
        for size in sorted_sizes:
            resized = img.resize((size, size), Image.Resampling.LANCZOS)
            icons.append(resized)

        icons[0].save(
            output_path,
            format="ICO",
            sizes=[(s, s) for s in sorted_sizes],
            append_images=icons[1:] if len(icons) > 1 else [],
        )
        return True
    except Exception as e:
        print(f"  ✗ Failed to generate {output_path}: {e}")
        return False


def generate_xpm(img: Image.Image, size: int, output_path: Path) -> bool:
    """Generate XPM using ImageMagick."""
    try:
        output_path.parent.mkdir(parents=True, exist_ok=True)

        # First create a temporary PNG
        temp_png = output_path.with_suffix(".tmp.png")
        resized = img.resize((size, size), Image.Resampling.LANCZOS)
        resized.save(temp_png, "PNG")

        # Convert to XPM using ImageMagick
        result = subprocess.run(
            ["convert", str(temp_png), str(output_path)],
            capture_output=True,
            text=True,
        )

        temp_png.unlink()  # Clean up temp file

        if result.returncode != 0:
            print(f"  ✗ ImageMagick error: {result.stderr}")
            return False

        return True
    except FileNotFoundError:
        print("  ✗ ImageMagick not found. Install with: brew install imagemagick")
        return False
    except Exception as e:
        print(f"  ✗ Failed to generate {output_path}: {e}")
        return False


def generate_icns(img: Image.Image, output_path: Path) -> bool:
    """Generate macOS .icns file using iconutil."""
    try:
        output_path.parent.mkdir(parents=True, exist_ok=True)

        # Create temporary iconset directory
        iconset_dir = output_path.with_suffix(".iconset")
        iconset_dir.mkdir(parents=True, exist_ok=True)

        # Generate all required sizes for iconset
        iconset_sizes = [
            (16, "icon_16x16.png"),
            (32, "icon_16x16@2x.png"),
            (32, "icon_32x32.png"),
            (64, "icon_32x32@2x.png"),
            (128, "icon_128x128.png"),
            (256, "icon_128x128@2x.png"),
            (256, "icon_256x256.png"),
            (512, "icon_256x256@2x.png"),
            (512, "icon_512x512.png"),
            (1024, "icon_512x512@2x.png"),
        ]

        for size, filename in iconset_sizes:
            resized = img.resize((size, size), Image.Resampling.LANCZOS)
            resized.save(iconset_dir / filename, "PNG")

        # Run iconutil
        result = subprocess.run(
            ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(output_path)],
            capture_output=True,
            text=True,
        )

        # Clean up iconset directory
        shutil.rmtree(iconset_dir)

        if result.returncode != 0:
            print(f"  ✗ iconutil error: {result.stderr}")
            return False

        return True
    except FileNotFoundError:
        print("  ✗ iconutil not found. This script must run on macOS.")
        return False
    except Exception as e:
        print(f"  ✗ Failed to generate {output_path}: {e}")
        return False


def generate_xcassets(img: Image.Image, output_dir: Path) -> bool:
    """Generate Assets.xcassets structure for macOS."""
    try:
        # AppIcon.appiconset
        appiconset_dir = output_dir / "Assets.xcassets" / "AppIcon.appiconset"
        appiconset_dir.mkdir(parents=True, exist_ok=True)

        appiconset_sizes = [16, 32, 64, 128, 256, 512, 1024]
        for size in appiconset_sizes:
            resized = img.resize((size, size), Image.Resampling.LANCZOS)
            resized.save(appiconset_dir / f"appicon_{size}.png", "PNG", optimize=True)

        # Contents.json for AppIcon.appiconset
        contents_json = """{
  "images" : [
    { "filename" : "appicon_16.png", "idiom" : "mac", "scale" : "1x", "size" : "16x16" },
    { "filename" : "appicon_32.png", "idiom" : "mac", "scale" : "2x", "size" : "16x16" },
    { "filename" : "appicon_32.png", "idiom" : "mac", "scale" : "1x", "size" : "32x32" },
    { "filename" : "appicon_64.png", "idiom" : "mac", "scale" : "2x", "size" : "32x32" },
    { "filename" : "appicon_128.png", "idiom" : "mac", "scale" : "1x", "size" : "128x128" },
    { "filename" : "appicon_256.png", "idiom" : "mac", "scale" : "2x", "size" : "128x128" },
    { "filename" : "appicon_256.png", "idiom" : "mac", "scale" : "1x", "size" : "256x256" },
    { "filename" : "appicon_512.png", "idiom" : "mac", "scale" : "2x", "size" : "256x256" },
    { "filename" : "appicon_512.png", "idiom" : "mac", "scale" : "1x", "size" : "512x512" },
    { "filename" : "appicon_1024.png", "idiom" : "mac", "scale" : "2x", "size" : "512x512" }
  ],
  "info" : { "author" : "xcode", "version" : 1 }
}"""
        (appiconset_dir / "Contents.json").write_text(contents_json)

        # Icon.iconset for document icons
        iconset_dir = output_dir / "Assets.xcassets" / "Icon.iconset"
        iconset_dir.mkdir(parents=True, exist_ok=True)

        resized_256 = img.resize((256, 256), Image.Resampling.LANCZOS)
        resized_256.save(iconset_dir / "icon_256x256.png", "PNG", optimize=True)

        resized_512 = img.resize((512, 512), Image.Resampling.LANCZOS)
        resized_512.save(iconset_dir / "icon_256x256@2x.png", "PNG", optimize=True)

        # Root Contents.json
        xcassets_dir = output_dir / "Assets.xcassets"
        root_contents = '{ "info" : { "author" : "xcode", "version" : 1 } }'
        (xcassets_dir / "Contents.json").write_text(root_contents)

        return True
    except Exception as e:
        print(f"  ✗ Failed to generate xcassets: {e}")
        return False


def generate_assets_car(output_dir: Path) -> bool:
    """Generate Assets.car from Assets.xcassets using actool."""
    try:
        xcassets_dir = output_dir / "Assets.xcassets"
        if not xcassets_dir.exists():
            print("  ✗ Assets.xcassets not found")
            return False

        result = subprocess.run(
            [
                "xcrun",
                "actool",
                "--compile",
                str(output_dir),
                str(xcassets_dir),
                "--platform",
                "macosx",
                "--minimum-deployment-target",
                "10.15",
                "--app-icon",
                "AppIcon",
                "--output-partial-info-plist",
                "/dev/null",
            ],
            capture_output=True,
            text=True,
        )

        if result.returncode != 0:
            print(f"  ✗ actool error: {result.stderr}")
            return False

        return True
    except FileNotFoundError:
        print("  ✗ actool not found. This script must run on macOS with Xcode.")
        return False
    except Exception as e:
        print(f"  ✗ Failed to generate Assets.car: {e}")
        return False


def copy_static(source: Path, dest: Path) -> bool:
    """Copy a static file."""
    try:
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, dest)
        return True
    except Exception as e:
        print(f"  ✗ Failed to copy {source} to {dest}: {e}")
        return False


def parse_config(config_path: Path) -> list[dict]:
    """Parse the generation config file."""
    if not config_path.exists():
        print(f"✗ Error: Config file not found: {config_path}")
        sys.exit(1)

    operations = []

    with open(config_path, "r") as f:
        for line_num, line in enumerate(f, 1):
            line = line.strip()

            # Skip empty lines and comments
            if not line or line.startswith("#"):
                continue

            parts = line.split()

            if len(parts) < 2:
                print(f"  Warning: Invalid line {line_num}: {line}")
                continue

            op = {"line": line_num, "raw": line}

            # Determine operation type and parse
            if parts[0] == "COPY":
                # COPY source dest
                op["type"] = "copy"
                op["source"] = parts[1]
                op["dest"] = parts[2]

            elif parts[0] == "ICO":
                # ICO source sizes dest
                # e.g., ICO source/app_icon.png 16,20,24,32,40,48,64,128,256 win/chromium.ico
                op["type"] = "ico"
                op["source"] = parts[1]
                op["sizes"] = [int(s) for s in parts[2].split(",")]
                op["dest"] = parts[3]

            elif parts[0] == "ICNS":
                # ICNS source dest
                op["type"] = "icns"
                op["source"] = parts[1]
                op["dest"] = parts[2]

            elif parts[0] == "XCASSETS":
                # XCASSETS source dest_dir
                op["type"] = "xcassets"
                op["source"] = parts[1]
                op["dest"] = parts[2]

            elif parts[0] == "ASSETS_CAR":
                # ASSETS_CAR mac_dir
                op["type"] = "assets_car"
                op["dest"] = parts[1]

            elif parts[0] == "XPM":
                # XPM source size dest
                op["type"] = "xpm"
                op["source"] = parts[1]
                op["size"] = int(parts[2])
                op["dest"] = parts[3]

            elif parts[0] == "PNG":
                # PNG source size dest
                op["type"] = "png"
                op["source"] = parts[1]
                op["size"] = int(parts[2])
                op["dest"] = parts[3]

            elif parts[0] == "MONO":
                # MONO source size dest
                op["type"] = "mono"
                op["source"] = parts[1]
                op["size"] = int(parts[2])
                op["dest"] = parts[3]

            else:
                print(f"  Warning: Unknown operation on line {line_num}: {parts[0]}")
                continue

            operations.append(op)

    return operations


def main():
    """Main entry point."""
    import argparse

    parser = argparse.ArgumentParser(description="Generate BrowserOS icons")
    parser.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CONFIG,
        help="Path to generation config file",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=OUTPUT_DIR,
        help="Output directory for generated icons",
    )
    args = parser.parse_args()

    print("=" * 60)
    print("BrowserOS Icon Generation")
    print("=" * 60)
    print(f"Config: {args.config}")
    print(f"Output: {args.output}")
    print()

    # Parse config
    operations = parse_config(args.config)
    print(f"Loaded {len(operations)} operations from config\n")

    # Cache for loaded source images
    source_cache: dict[str, Image.Image] = {}

    def get_source(source_path: str) -> Image.Image:
        """Get source image, loading and validating if needed."""
        if source_path not in source_cache:
            full_path = SCRIPT_DIR / source_path
            source_cache[source_path] = validate_source(full_path)
        return source_cache[source_path]

    # Process operations
    success_count = 0
    fail_count = 0

    for op in operations:
        op_type = op["type"]
        dest_path = args.output / op.get("dest", "")

        if op_type == "copy":
            source_path = SCRIPT_DIR / op["source"]
            print(f"COPY {op['source']} -> {op['dest']}")
            if copy_static(source_path, dest_path):
                print(f"  ✓ Copied")
                success_count += 1
            else:
                fail_count += 1

        elif op_type == "png":
            print(f"PNG {op['source']} @ {op['size']} -> {op['dest']}")
            img = get_source(op["source"])
            if generate_png(img, op["size"], dest_path):
                print(f"  ✓ Generated {op['size']}x{op['size']}")
                success_count += 1
            else:
                fail_count += 1

        elif op_type == "mono":
            print(f"MONO {op['source']} @ {op['size']} -> {op['dest']}")
            img = get_source(op["source"])
            if generate_mono_png(img, op["size"], dest_path):
                print(f"  ✓ Generated mono {op['size']}x{op['size']}")
                success_count += 1
            else:
                fail_count += 1

        elif op_type == "ico":
            print(f"ICO {op['source']} @ {op['sizes']} -> {op['dest']}")
            img = get_source(op["source"])
            if generate_ico(img, op["sizes"], dest_path):
                print(f"  ✓ Generated ICO with {len(op['sizes'])} sizes")
                success_count += 1
            else:
                fail_count += 1

        elif op_type == "xpm":
            print(f"XPM {op['source']} @ {op['size']} -> {op['dest']}")
            img = get_source(op["source"])
            if generate_xpm(img, op["size"], dest_path):
                print(f"  ✓ Generated XPM {op['size']}x{op['size']}")
                success_count += 1
            else:
                fail_count += 1

        elif op_type == "icns":
            print(f"ICNS {op['source']} -> {op['dest']}")
            img = get_source(op["source"])
            if generate_icns(img, dest_path):
                print(f"  ✓ Generated ICNS")
                success_count += 1
            else:
                fail_count += 1

        elif op_type == "xcassets":
            print(f"XCASSETS {op['source']} -> {op['dest']}")
            img = get_source(op["source"])
            if generate_xcassets(img, dest_path):
                print(f"  ✓ Generated Assets.xcassets")
                success_count += 1
            else:
                fail_count += 1

        elif op_type == "assets_car":
            print(f"ASSETS_CAR -> {op['dest']}")
            if generate_assets_car(dest_path):
                print(f"  ✓ Generated Assets.car")
                success_count += 1
            else:
                fail_count += 1

    # Summary
    print()
    print("=" * 60)
    print(f"Complete: {success_count} succeeded, {fail_count} failed")
    print("=" * 60)

    if fail_count > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
