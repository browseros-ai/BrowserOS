#!/usr/bin/env python3
"""Reject Linux release binaries linked against a newer glibc than we support."""

import argparse
import re
import subprocess
from pathlib import Path


def validate_versions(output: str, maximum: str) -> str:
    versions = set(re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)\b", output))
    if not versions:
        raise ValueError(
            "No GLIBC symbol versions found; expected a dynamically linked GNU binary"
        )
    unsupported = set(re.findall(r"\bGLIBC_([A-Z][A-Z0-9_]+)\b", output))
    if unsupported:
        raise ValueError(
            f"Unsupported GLIBC ABI requirement: {', '.join(sorted(unsupported))}"
        )

    def key(version: str) -> tuple[int, ...]:
        return tuple(int(part) for part in version.split("."))

    newest = max(versions, key=key)
    if key(newest) > key(maximum):
        raise ValueError(
            f"Binary requires GLIBC_{newest}; supported maximum is GLIBC_{maximum}"
        )
    return newest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument(
        "--maximum", required=True, help="Maximum allowed GLIBC symbol version"
    )
    args = parser.parse_args()
    result = subprocess.run(
        ["readelf", "--version-info", "--wide", str(args.binary)],
        check=True,
        capture_output=True,
        text=True,
    )
    try:
        newest = validate_versions(result.stdout, args.maximum)
    except ValueError as error:
        parser.exit(1, f"{error}\n")
    print(f"{args.binary}: GLIBC_{newest} <= GLIBC_{args.maximum}")


if __name__ == "__main__":
    main()
