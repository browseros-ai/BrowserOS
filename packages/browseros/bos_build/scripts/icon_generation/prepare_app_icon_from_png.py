#!/usr/bin/env python3
"""
Build source/app_icon.png (1024×1024, RGBA) from the repo brand PNG for icon_generation.

Default input: repository root red_logo.png (same asset as extension brand icons).
"""
from __future__ import annotations

import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError as e:
    print("Error: Pillow is required. Install with: pip install Pillow", file=sys.stderr)
    raise SystemExit(1) from e

SCRIPT_DIR = Path(__file__).resolve().parent
SOURCE_DIR = SCRIPT_DIR / "source"
OUT_SIZE = 1024
INNER = 920  # max logo size inside canvas


def repo_root() -> Path:
    # .../packages/browseros/build/scripts/icon_generation -> parents[4] = repo root
    return SCRIPT_DIR.parents[4]


def main() -> None:
    default_in = repo_root() / "red_logo.png"
    in_path = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else default_in
    if not in_path.exists():
        print(f"✗ Input not found: {in_path}", file=sys.stderr)
        raise SystemExit(1)

    SOURCE_DIR.mkdir(parents=True, exist_ok=True)
    out_path = SOURCE_DIR / "app_icon.png"

    src = Image.open(in_path).convert("RGBA")
    w, h = src.size
    scale = min(INNER / w, INNER / h, 1.0)
    if scale < 1.0:
        nw, nh = int(w * scale), int(h * scale)
        src = src.resize((nw, nh), Image.Resampling.LANCZOS)

    canvas = Image.new("RGBA", (OUT_SIZE, OUT_SIZE), (0, 0, 0, 0))
    x = (OUT_SIZE - src.width) // 2
    y = (OUT_SIZE - src.height) // 2
    canvas.paste(src, (x, y), src)

    canvas.save(out_path, "PNG", optimize=True)
    print(f"OK Wrote {out_path} ({OUT_SIZE}x{OUT_SIZE}) from {in_path}")


if __name__ == "__main__":
    main()
