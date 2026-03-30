# BrowserOS Browser (Chromium Fork)

Custom Chromium build with AI agent integration, enhanced privacy patches, and native MCP support.

> Based on **Chromium 146.0.7680.31** · Built with Python 3.12+ · Licensed under [AGPL-3.0](../../LICENSE)

## What This Is

This package contains the BrowserOS browser build system — everything needed to fetch Chromium source, apply BrowserOS patches, and produce signed binaries for macOS, Windows, and Linux. The build system is a Python CLI that orchestrates the entire pipeline from source to distributable.

BrowserOS patches add:
- Native AI agent sidebar and new tab integration
- MCP server endpoints baked into the browser
- Enhanced privacy via [ungoogled-chromium](https://github.com/ungoogled-software/ungoogled-chromium) patches
- Custom branding, icons, and entitlements
- Keychain access group management (macOS)
- Sparkle auto-update framework (macOS)

## Prerequisites

| Requirement | Details |
|-------------|---------|
| **Disk space** | ~100 GB for Chromium source + build artifacts |
| **Python** | 3.12+ |
| **macOS** | Xcode + Command Line Tools |
| **Linux** | `build-essential`, `clang`, `lld`, and Chromium's [Linux deps](https://chromium.googlesource.com/chromium/src/+/main/docs/linux/build_instructions.md) |
| **Windows** | Visual Studio 2022, Windows SDK |

## Directory Structure

```
packages/browseros/
├── build/                   # Build system (Python CLI)
│   ├── __main__.py          # CLI entry point
│   ├── browseros.py         # Main app definition
│   ├── modules/
│   │   ├── setup/           # Chromium source fetch and setup
│   │   ├── patches/         # Patch application logic
│   │   ├── apply/           # Apply patches to source tree
│   │   ├── extract/         # Extract patches from modified source
│   │   ├── feature/         # Feature flag management
│   │   ├── package/         # Binary packaging
│   │   ├── sign/            # Code signing (macOS, Windows)
│   │   ├── ota/             # Over-the-air update support
│   │   └── resources/       # Resource management
│   ├── config/              # Build configuration
│   └── features.yaml        # Feature flag definitions
│
├── chromium_patches/        # BrowserOS patches applied to Chromium source
│   ├── chrome/browser/      # Browser UI and feature patches
│   ├── components/          # Component patches (e.g., os_crypt)
│   └── ...                  # Organized to mirror Chromium source tree
│
├── chromium_files/          # New files added to Chromium (not patches)
├── series_patches/          # Ordered patch series
├── resources/               # Icons, entitlements, signing resources
│   └── entitlements/        # macOS entitlements (app, helper, GPU, etc.)
│
├── tools/
│   └── bdev                 # Developer tool
│
├── CHROMIUM_VERSION          # Pinned Chromium version (MAJOR.MINOR.BUILD.PATCH)
├── BASE_COMMIT              # Base Chromium commit hash
├── pyproject.toml           # Python project config
└── requirements.txt         # Python dependencies
```

## Build System

The `browseros` CLI manages the full build lifecycle:

```bash
# Install the build system
pip install -e .

# Or use uv
uv pip install -e .
```

Key commands:

```bash
browseros setup          # Fetch and prepare Chromium source
browseros apply          # Apply all patches to Chromium source
browseros build          # Build BrowserOS binary
browseros package        # Package into distributable (DMG, installer, AppImage)
browseros sign           # Code sign the binary (macOS/Windows)
```

## Patch System

BrowserOS applies patches on top of vanilla Chromium. Patches are organized in two directories:

- **`chromium_patches/`** — Individual file patches, organized to mirror the Chromium source tree. Each file here replaces or modifies the corresponding file in Chromium.
- **`series_patches/`** — Ordered patch series applied sequentially.

### Adding a New Patch

1. Make your changes in the Chromium source tree
2. Use `browseros extract` to pull changes back into patch format
3. Place the patch in the appropriate directory mirroring Chromium's structure
4. Test with a full `browseros apply && browseros build` cycle

### Chromium Version Pinning

The exact Chromium version is pinned in `CHROMIUM_VERSION`:

```
MAJOR=146
MINOR=0
BUILD=7680
PATCH=31
```

To update the base Chromium version, update this file and `BASE_COMMIT`, then resolve any patch conflicts.

## Signing (macOS)

macOS builds require code signing for Keychain access, Gatekeeper, and notarization:

- Entitlements are in `resources/entitlements/` (app, helper, GPU, renderer, etc.)
- Designated requirements pin to Team ID for Keychain persistence across updates
- The signing module is at `build/modules/sign/macos.py`

## Feature Flags

Feature flags are defined in `features.yaml` and control which BrowserOS-specific features are compiled into the build. The feature module (`build/modules/feature/`) manages flag resolution at build time.

## Related Resources

- [Chromium Build Instructions](https://chromium.googlesource.com/chromium/src/+/main/docs/linux/build_instructions.md)
- [ungoogled-chromium](https://github.com/ungoogled-software/ungoogled-chromium) — upstream privacy patches
- [BrowserOS Agent Platform](../browseros-agent/) — the TypeScript/Go agent system that runs inside the browser
