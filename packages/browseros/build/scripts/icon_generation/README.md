# BrowserOS Icon Generation

This directory contains the unified icon generation system for BrowserOS. All browser icons
for Windows, macOS, Linux, and ChromeOS are generated from a single source image.

## Quick Start

```bash
# 1. Place your 1024x1024 (or larger) source icon
cp your_icon.png source/app_icon.png

# 2. Place any static files (SVG, AI) in static/
cp your_logo.svg static/product_logo.svg

# 3. Run generation
python generate_icons.py

# 4. Icons are output to packages/browseros/resources/icons/
```

## Directory Structure

```
icon_generation/
├── source/                    # Source images (high-res PNGs)
│   └── app_icon.png          # Main app icon (≥1024x1024, required)
├── static/                    # Static files (copied as-is)
│   ├── product_logo.svg      # Vector logo
│   ├── product_logo.ai       # Adobe Illustrator source
│   └── product_logo_animation.svg
├── generate_icons.py         # Main generation script
├── generate_icons.txt        # Generation configuration
└── README.md                 # This file
```

## Requirements

- **Python 3.12+**
- **Pillow** - `pip install Pillow` (or via pyproject.toml)
- **ImageMagick** - `brew install imagemagick` (for XPM generation)
- **macOS** - Required for .icns and Assets.car generation (uses iconutil, actool)

## Configuration Format

The `generate_icons.txt` file defines all icon generation operations:

```
# Operations:
PNG    source size dest     # Generate PNG at specified size
MONO   source size dest     # Generate monochrome (white silhouette) PNG
ICO    source sizes dest    # Generate Windows ICO (sizes comma-separated)
XPM    source size dest     # Generate Linux XPM via ImageMagick
ICNS   source dest          # Generate macOS .icns via iconutil
XCASSETS source dest_dir    # Generate Assets.xcassets structure
ASSETS_CAR dest_dir         # Generate Assets.car from xcassets
COPY   source dest          # Copy static file as-is
```

## Generated Icons Reference

### Source Files

| File | Type | Description |
|------|------|-------------|
| `source/app_icon.png` | PNG | Master source icon (≥1024x1024) |
| `static/product_logo.svg` | SVG | Vector logo (manually maintained) |
| `static/product_logo.ai` | AI | Adobe Illustrator source (manually maintained) |
| `static/product_logo_animation.svg` | SVG | Animated logo (manually maintained) |

### Common Icons (Generated)

| Output | Size | Description | Chromium Target |
|--------|------|-------------|-----------------|
| `product_logo_16.png` | 16x16 | Small icon | `chrome/app/theme/chromium/` |
| `product_logo_22.png` | 22x22 | Toolbar icon | `chrome/app/theme/chromium/` |
| `product_logo_24.png` | 24x24 | Menu icon | `chrome/app/theme/chromium/` |
| `product_logo_32.png` | 32x32 | Standard icon | `chrome/app/theme/chromium/` |
| `product_logo_48.png` | 48x48 | Medium icon | `chrome/app/theme/chromium/` |
| `product_logo_64.png` | 64x64 | Large icon | `chrome/app/theme/chromium/` |
| `product_logo_128.png` | 128x128 | Very large icon | `chrome/app/theme/chromium/` |
| `product_logo_192.png` | 192x192 | Android/PWA icon | `chrome/app/theme/chromium/` |
| `product_logo_256.png` | 256x256 | Extra large icon | `chrome/app/theme/chromium/` |
| `product_logo_1024.png` | 1024x1024 | Maximum size | `chrome/app/theme/chromium/` |
| `product_logo_22_mono.png` | 22x22 | Monochrome (white) | `chrome/app/theme/chromium/` |
| `product_logo_name_22.png` | 22x22 | With name (1x) | `chrome/app/theme/default_100_percent/chromium/` |
| `product_logo_name_22_2x.png` | 44x44 | With name (2x) | `chrome/app/theme/default_200_percent/chromium/` |
| `product_logo_name_22_white.png` | 22x22 | With name, white (1x) | `chrome/app/theme/default_100_percent/chromium/` |
| `product_logo_name_22_white_2x.png` | 44x44 | With name, white (2x) | `chrome/app/theme/default_200_percent/chromium/` |

### Windows Icons (Generated)

| Output | Sizes | Description | Chromium Target |
|--------|-------|-------------|-----------------|
| `win/chromium.ico` | 16,20,24,32,40,48,64,128,256 | Main app icon | `chrome/app/theme/chromium/win/` |
| `win/incognito.ico` | 16,20,24,32,40,48,64,128,256 | Incognito mode | `chrome/app/theme/chromium/win/` |
| `win/app_list.ico` | 16,20,24,32,40,48,64,128,256 | App launcher | `chrome/app/theme/chromium/win/` |
| `win/chromium_doc.ico` | 16,32,48,256 | HTML documents | `chrome/app/theme/chromium/win/` |
| `win/chromium_pdf.ico` | 16,32,48,256 | PDF documents | `chrome/app/theme/chromium/win/` |
| `win/tiles/Logo.png` | 150x150 | Windows tile | `chrome/app/theme/chromium/win/tiles/` |
| `win/tiles/SmallLogo.png` | 70x70 | Small tile | `chrome/app/theme/chromium/win/tiles/` |

**Windows ICO sizes explained:**
- 16x16: Title bar, small list view
- 20x20: Notification area
- 24x24: Small taskbar (100% DPI)
- 32x32: Desktop, list view
- 40x40: Taskbar (125% DPI) - critical for most modern displays
- 48x48: Taskbar (100% DPI), medium icons
- 64x64: Large icons, jump list
- 128x128: Extra large (200% DPI)
- 256x256: Jumbo icons (PNG compressed)

### macOS Icons (Generated)

| Output | Description | Chromium Target |
|--------|-------------|-----------------|
| `mac/app.icns` | Main app icon bundle | `chrome/app/theme/chromium/mac/` |
| `mac/document.icns` | Document icon bundle | `chrome/app/theme/chromium/mac/` |
| `mac/Assets.car` | Compiled asset catalog | `chrome/app/theme/chromium/mac/` |
| `mac/Assets.xcassets/` | Asset catalog source | `chrome/app/theme/chromium/mac/` |

### Linux Icons (Generated)

| Output | Size | Description | Chromium Target |
|--------|------|-------------|-----------------|
| `linux/product_logo_24.png` | 24x24 | Menu icon | `chrome/app/theme/chromium/linux/` |
| `linux/product_logo_48.png` | 48x48 | Medium icon | `chrome/app/theme/chromium/linux/` |
| `linux/product_logo_64.png` | 64x64 | Large icon | `chrome/app/theme/chromium/linux/` |
| `linux/product_logo_128.png` | 128x128 | Very large | `chrome/app/theme/chromium/linux/` |
| `linux/product_logo_256.png` | 256x256 | Extra large | `chrome/app/theme/chromium/linux/` |
| `linux/product_logo_32.xpm` | 32x32 | X11 format | `chrome/app/theme/chromium/linux/` |

### ChromeOS Icons (Generated)

| Output | Size | Description | Chromium Target |
|--------|------|-------------|-----------------|
| `chromeos/chrome_app_icon_32.png` | 32x32 | Small app icon | `chrome/app/theme/chromium/chromeos/` |
| `chromeos/chrome_app_icon_192.png` | 192x192 | Large app icon | `chrome/app/theme/chromium/chromeos/` |

## Static Files (Copied)

| File | Description | Chromium Target |
|------|-------------|-----------------|
| `product_logo.svg` | Vector logo | `chrome/app/theme/chromium/` |
| `product_logo.ai` | Illustrator source | `chrome/app/theme/chromium/` |
| `product_logo_animation.svg` | Animated logo | `chrome/app/theme/chromium/` |

## Workflow

1. **Update source icon**: Replace `source/app_icon.png` with your new icon (≥1024x1024)
2. **Run generation**: `python generate_icons.py`
3. **Build BrowserOS**: The `copy_resources.yaml` copies generated icons to chromium source tree

## Troubleshooting

### "Source image is too small"
Your `source/app_icon.png` must be at least 1024x1024 pixels.

### "ImageMagick not found"
Install with: `brew install imagemagick`

### "iconutil not found" / "actool not found"
These are macOS-only tools. Run this script on macOS with Xcode Command Line Tools installed.

### Icons look blurry on Windows taskbar
Ensure the ICO includes 40x40 size (for 125% DPI scaling, very common on modern displays).
