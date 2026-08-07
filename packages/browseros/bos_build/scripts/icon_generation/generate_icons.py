#!/usr/bin/env python3
"""Generate BrowserOS product icons from high-resolution source artwork."""

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
RESOURCE_DIR = SCRIPT_DIR.parent.parent.parent / "resources"
DEFAULT_PRODUCT = "browseros"
PRODUCTS = (DEFAULT_PRODUCT, "browserclaw")

MIN_SOURCE_SIZE = 1024


def product_output_dir(product: str) -> Path:
    """Return the committed icon output directory for a product."""
    if product not in PRODUCTS:
        raise ValueError(f"Unknown product: {product}")
    return RESOURCE_DIR / product / "icons"


def product_roots(product: str) -> dict[str, Path]:
    """Return source roots for config paths like source/app_icon.png."""
    if product == DEFAULT_PRODUCT:
        return {"source": SOURCE_DIR, "static": STATIC_DIR}
    if product in PRODUCTS:
        return {"source": SOURCE_DIR / product, "static": STATIC_DIR / product}
    raise ValueError(f"Unknown product: {product}")


def resolve_input_path(config_path: str, product: str) -> Path:
    """Resolve a config source path against the selected product roots."""
    path = Path(config_path)
    if len(path.parts) > 1 and path.parts[0] in {"source", "static"}:
        return product_roots(product)[path.parts[0]].joinpath(*path.parts[1:])
    return SCRIPT_DIR / path


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

        if resized.mode != "RGBA":
            resized = resized.convert("RGBA")

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

        temp_png = output_path.with_suffix(".tmp.png")
        resized = img.resize((size, size), Image.Resampling.LANCZOS)
        resized.save(temp_png, "PNG")

        result = subprocess.run(
            ["convert", str(temp_png), str(output_path)],
            capture_output=True,
            text=True,
        )

        temp_png.unlink()

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

        iconset_dir = output_path.with_suffix(".iconset")
        iconset_dir.mkdir(parents=True, exist_ok=True)

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

        result = subprocess.run(
            ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(output_path)],
            capture_output=True,
            text=True,
        )

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
        appiconset_dir = output_dir / "Assets.xcassets" / "AppIcon.appiconset"
        appiconset_dir.mkdir(parents=True, exist_ok=True)

        appiconset_sizes = [16, 32, 64, 128, 256, 512, 1024]
        for size in appiconset_sizes:
            resized = img.resize((size, size), Image.Resampling.LANCZOS)
            resized.save(appiconset_dir / f"appicon_{size}.png", "PNG", optimize=True)

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

        iconset_dir = output_dir / "Assets.xcassets" / "Icon.iconset"
        iconset_dir.mkdir(parents=True, exist_ok=True)

        resized_256 = img.resize((256, 256), Image.Resampling.LANCZOS)
        resized_256.save(iconset_dir / "icon_256x256.png", "PNG", optimize=True)

        resized_512 = img.resize((512, 512), Image.Resampling.LANCZOS)
        resized_512.save(iconset_dir / "icon_256x256@2x.png", "PNG", optimize=True)

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

            if not line or line.startswith("#"):
                continue

            parts = line.split()

            if len(parts) < 2:
                print(f"  Warning: Invalid line {line_num}: {line}")
                continue

            op = {"line": line_num, "raw": line}

            if parts[0] == "COPY":
                op["type"] = "copy"
                op["source"] = parts[1]
                op["dest"] = parts[2]

            elif parts[0] == "ICO":
                op["type"] = "ico"
                op["source"] = parts[1]
                op["sizes"] = [int(s) for s in parts[2].split(",")]
                op["dest"] = parts[3]

            elif parts[0] == "ICNS":
                op["type"] = "icns"
                op["source"] = parts[1]
                op["dest"] = parts[2]

            elif parts[0] == "XCASSETS":
                op["type"] = "xcassets"
                op["source"] = parts[1]
                op["dest"] = parts[2]

            elif parts[0] == "ASSETS_CAR":
                op["type"] = "assets_car"
                op["dest"] = parts[1]

            elif parts[0] == "XPM":
                op["type"] = "xpm"
                op["source"] = parts[1]
                op["size"] = int(parts[2])
                op["dest"] = parts[3]

            elif parts[0] == "PNG":
                op["type"] = "png"
                op["source"] = parts[1]
                op["size"] = int(parts[2])
                op["dest"] = parts[3]

            elif parts[0] == "MONO":
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

    parser = argparse.ArgumentParser(description="Generate BrowserOS product icons")
    parser.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CONFIG,
        help="Path to generation config file",
    )
    parser.add_argument(
        "--product",
        choices=PRODUCTS,
        default=DEFAULT_PRODUCT,
        help="Product icon tree to generate",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="Output directory for generated icons; defaults to the product resource tree",
    )
    args = parser.parse_args()
    output_dir = args.output or product_output_dir(args.product)
    roots = product_roots(args.product)

    print("=" * 60)
    print("BrowserOS Icon Generation")
    print("=" * 60)
    print(f"Product: {args.product}")
    print(f"Config: {args.config}")
    print(f"Source: {roots['source']}")
    print(f"Static: {roots['static']}")
    print(f"Output: {output_dir}")
    print()

    operations = parse_config(args.config)
    print(f"Loaded {len(operations)} operations from config\n")

    source_cache: dict[str, Image.Image] = {}

    def get_source(source_path: str) -> Image.Image:
        """Get source image, loading and validating if needed."""
        if source_path not in source_cache:
            full_path = resolve_input_path(source_path, args.product)
            source_cache[source_path] = validate_source(full_path)
        return source_cache[source_path]

    success_count = 0
    fail_count = 0

    for op in operations:
        op_type = op["type"]
        dest_path = output_dir / op.get("dest", "")

        if op_type == "copy":
            source_path = resolve_input_path(op["source"], args.product)
            print(f"COPY {op['source']} -> {op['dest']}")
            if copy_static(source_path, dest_path):
                print("  ✓ Copied")
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
                print("  ✓ Generated ICNS")
                success_count += 1
            else:
                fail_count += 1

        elif op_type == "xcassets":
            print(f"XCASSETS {op['source']} -> {op['dest']}")
            img = get_source(op["source"])
            if generate_xcassets(img, dest_path):
                print("  ✓ Generated Assets.xcassets")
                success_count += 1
            else:
                fail_count += 1

        elif op_type == "assets_car":
            print(f"ASSETS_CAR -> {op['dest']}")
            if generate_assets_car(dest_path):
                print("  ✓ Generated Assets.car")
                success_count += 1
            else:
                fail_count += 1

    print()
    print("=" * 60)
    print(f"Complete: {success_count} succeeded, {fail_count} failed")
    print("=" * 60)

    if fail_count > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
