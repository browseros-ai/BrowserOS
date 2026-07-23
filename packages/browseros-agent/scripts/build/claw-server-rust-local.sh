#!/usr/bin/env bash
set -euo pipefail

target="darwin-arm64"
agent_root=""
browseros_root=""

usage() {
  cat <<'USAGE'
Usage: claw-server-rust-local.sh [--target darwin-arm64] [--agent-root PATH] [--browseros-root PATH]

Build the local Rust BrowserClaw server and stage it in the BrowserOS resource
layout consumed by bos_build's resources step.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --target)
      target="${2:?--target requires a value}"
      shift 2
      ;;
    --target=*)
      target="${1#*=}"
      shift
      ;;
    --agent-root)
      agent_root="${2:?--agent-root requires a value}"
      shift 2
      ;;
    --agent-root=*)
      agent_root="${1#*=}"
      shift
      ;;
    --browseros-root)
      browseros_root="${2:?--browseros-root requires a value}"
      shift 2
      ;;
    --browseros-root=*)
      browseros_root="${1#*=}"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$target" in
  darwin-arm64) ;;
  *)
    echo "Unsupported local Rust staging target: $target" >&2
    exit 2
    ;;
esac

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -z "$agent_root" ]; then
  agent_root="$(cd "$script_dir/../.." && pwd)"
else
  agent_root="$(cd "$agent_root" && pwd)"
fi
if [ -z "$browseros_root" ]; then
  browseros_root="$(cd "$agent_root/../browseros" && pwd)"
else
  browseros_root="$(cd "$browseros_root" && pwd)"
fi

binary_name="browseros-claw-server-rs"
runtime_binary_name="browseros-claw-server"
cargo_target_dir="${CARGO_TARGET_DIR:-$agent_root/target}"
binary_path="$cargo_target_dir/release/$binary_name"
destination="$browseros_root/resources/binaries/browseros_claw_server_rust/$target"

echo "Building Rust BrowserClaw server for $target..."
cargo build --release -p claw-server-rust --bin "$binary_name" \
  --manifest-path "$agent_root/Cargo.toml"

if [ ! -f "$binary_path" ]; then
  echo "Missing compiled binary: $binary_path" >&2
  exit 1
fi

export BROWSEROS_AGENT_ROOT="$agent_root"
export BROWSEROS_ROOT="$browseros_root"
export BROWSEROS_TARGET="$target"
export BROWSEROS_RUST_BINARY="$binary_path"
export BROWSEROS_RUST_RUNTIME_BINARY="$runtime_binary_name"

uv run --project "$browseros_root" python <<'PY'
import hashlib
import json
import os
import shutil
import stat
import tempfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path

from bos_build.steps.storage.download import extract_artifact_zip

browseros_root = Path(os.environ["BROWSEROS_ROOT"])
agent_root = Path(os.environ["BROWSEROS_AGENT_ROOT"])
target = os.environ["BROWSEROS_TARGET"]
binary_path = Path(os.environ["BROWSEROS_RUST_BINARY"])
runtime_binary = os.environ["BROWSEROS_RUST_RUNTIME_BINARY"]
destination = (
    browseros_root
    / "resources/binaries/browseros_claw_server_rust"
    / target
)
stage_binary = destination / "resources/bin" / runtime_binary
source_resources = agent_root / "apps/claw-server-rust/resources"

if destination.exists():
    shutil.rmtree(destination)
if not source_resources.is_dir():
    raise SystemExit(f"Missing server resources: {source_resources}")
stage_binary.parent.mkdir(parents=True, exist_ok=True)
shutil.copy2(binary_path, stage_binary)
stage_binary.chmod(0o755)
shutil.copytree(source_resources, destination / "resources", dirs_exist_ok=True)

files = sorted(
    path
    for path in destination.rglob("*")
    if path.is_file() and path.name != "artifact-metadata.json"
)
metadata = {
    "version": "local",
    "target": target,
    "generatedAt": datetime.now(timezone.utc)
    .isoformat(timespec="milliseconds")
    .replace("+00:00", "Z"),
    "files": [
        {
            "path": path.relative_to(destination).as_posix(),
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "size": path.stat().st_size,
        }
        for path in files
    ],
}
metadata_path = destination / "artifact-metadata.json"
metadata_path.write_text(json.dumps(metadata, indent=2) + "\n")

with tempfile.TemporaryDirectory() as tmp:
    archive_path = Path(tmp) / "browseros-claw-server-rust-resources.zip"
    with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
        for path in [metadata_path, *files]:
            relative = path.relative_to(destination).as_posix()
            info = zipfile.ZipInfo(relative)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (stat.S_IMODE(path.stat().st_mode) & 0o777) << 16
            archive.writestr(info, path.read_bytes())

    validate_destination = Path(tmp) / "validated"
    extracted = extract_artifact_zip(archive_path, validate_destination)
    extracted_rel = sorted(
        path.relative_to(validate_destination).as_posix() for path in extracted
    )
    expected = sorted([
        f"resources/bin/{runtime_binary}",
        "resources/skills/browserclaw/SKILL.md",
    ])
    if extracted_rel != expected:
        raise SystemExit(
            f"Expected extracted files {expected}, got {extracted_rel}"
        )

print(f"Staged Rust BrowserClaw server resources: {destination}")
PY
