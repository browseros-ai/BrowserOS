#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: prepare-server-bundle-release.sh --event-name <push|workflow_dispatch|workflow_call> --default-branch <branch> --ref-name <ref> --tag-prefix <prefix> (--package-json <path>|--cargo-toml <path>) --release-name <name> --component-name <name> [--legacy-prefix <prefix>] [--requested-version <X.Y.Z>] [--release-ref <ref>] [--release-records <path>] [--remote <name>]
EOF
}

event_name=""
default_branch=""
ref_name=""
requested_version=""
release_ref=""
release_records=""
remote="origin"
tag_prefix=""
legacy_csv=""
package_json=""
cargo_toml=""
release_name=""
component_name=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --event-name)
      event_name="${2:-}"
      shift 2
      ;;
    --default-branch)
      default_branch="${2:-}"
      shift 2
      ;;
    --ref-name)
      ref_name="${2:-}"
      shift 2
      ;;
    --requested-version)
      requested_version="${2:-}"
      shift 2
      ;;
    --release-ref)
      release_ref="${2:-}"
      shift 2
      ;;
    --release-records)
      release_records="${2:-}"
      shift 2
      ;;
    --remote)
      remote="${2:-}"
      shift 2
      ;;
    --tag-prefix)
      tag_prefix="${2:-}"
      shift 2
      ;;
    --legacy-prefix)
      if [ -n "$legacy_csv" ]; then
        legacy_csv+=",${2:-}"
      else
        legacy_csv="${2:-}"
      fi
      shift 2
      ;;
    --package-json)
      package_json="${2:-}"
      shift 2
      ;;
    --cargo-toml)
      cargo_toml="${2:-}"
      shift 2
      ;;
    --release-name)
      release_name="${2:-}"
      shift 2
      ;;
    --component-name)
      component_name="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [ -z "$event_name" ] ||
  [ -z "$default_branch" ] ||
  [ -z "$tag_prefix" ] ||
  [ -z "$release_name" ] ||
  [ -z "$component_name" ]; then
  usage
  exit 2
fi

if { [ -z "$package_json" ] && [ -z "$cargo_toml" ]; } ||
  { [ -n "$package_json" ] && [ -n "$cargo_toml" ]; }; then
  echo "Exactly one of --package-json or --cargo-toml is required." >&2
  usage
  exit 2
fi

if [ -n "$release_records" ] && [ ! -f "$release_records" ]; then
  echo "::error::Release records file does not exist: $release_records"
  exit 1
fi

git_root="$(git rev-parse --show-toplevel)"
git_root="$(cd "$git_root" && pwd -P)"
cd "$git_root"

is_semver() {
  [[ "$1" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]
}

require_annotated_tag() {
  local tag="$1"
  local tag_type
  tag_type="$(git cat-file -t "refs/tags/$tag" 2>/dev/null || true)"
  if [ -z "$tag_type" ]; then
    echo "::error::Tag does not exist: $tag"
    exit 1
  fi
  if [ "$tag_type" != "tag" ]; then
    echo "::error::Tag $tag must be an annotated tag."
    exit 1
  fi
}

ensure_default_branch_release() {
  local sha="$1"
  if ! git merge-base --is-ancestor "$sha" "$remote/$default_branch"; then
    echo "::error::Tagged commit $sha is not reachable from $remote/$default_branch."
    exit 1
  fi
}

resolve_release_sha() {
  local ref="${1:-$remote/$default_branch}"
  if git rev-parse --verify --quiet "$ref^{commit}" >/dev/null; then
    git rev-parse "$ref^{commit}"
    return 0
  fi
  if git rev-parse --verify --quiet "$remote/$ref^{commit}" >/dev/null; then
    git rev-parse "$remote/$ref^{commit}"
    return 0
  fi
  echo "::error::Could not resolve release ref: $ref"
  exit 1
}

read_package_version_at_ref() {
  local ref="$1"
  if [ -n "$package_json" ]; then
    if ! git show "$ref:$package_json" | python3 -c '
import json
import sys

print(json.load(sys.stdin)["version"])
'; then
      echo "::error::Could not read $package_json version at $ref."
      exit 1
    fi
    return 0
  fi

  if ! git show "$ref:$cargo_toml" | python3 -c '
import re
import sys

in_package = False
for raw_line in sys.stdin:
    line = raw_line.strip()
    if line == "[package]":
        in_package = True
        continue
    if in_package and line.startswith("["):
        break
    if not in_package:
        continue
    match = re.fullmatch(r"version\s*=\s*\"([^\"]+)\"", line)
    if match:
        print(match.group(1))
        sys.exit(0)
raise SystemExit("missing [package] version")
'; then
    echo "::error::Could not read $cargo_toml version at $ref."
    exit 1
  fi
}

emit() {
  printf '%s=%s\n' "$1" "$2"
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    printf '%s=%s\n' "$1" "$2" >> "$GITHUB_OUTPUT"
  fi
}

git fetch "$remote" "$default_branch:refs/remotes/$remote/$default_branch" --no-tags
git fetch --force "$remote" --tags --prune

if [ "$event_name" = "push" ]; then
  tag="$ref_name"
  version="${tag#"$tag_prefix"}"
  if [ "$tag" = "$version" ] || ! is_semver "$version"; then
    echo "::error::Expected $component_name release tag like ${tag_prefix}X.Y.Z, got: $tag"
    exit 1
  fi
  require_annotated_tag "$tag"
  release_sha="$(git rev-list -n 1 "$tag")"
else
  release_sha="$(resolve_release_sha "${release_ref:-}")"
  version="${requested_version:-$(read_package_version_at_ref "$release_sha")}"
  tag=""
fi

ensure_default_branch_release "$release_sha"

resolution="$(python3 - "$event_name" "$release_sha" "$version" "$requested_version" "$tag_prefix" "$legacy_csv" "$release_records" "$component_name" <<'PY'
import json
import re
import subprocess
import sys
from pathlib import Path

event_name, release_sha, package_or_tag_version, requested_version = sys.argv[1:5]
tag_prefix, legacy_csv, release_records_path, component_name = sys.argv[5:9]
prefixes = [tag_prefix, *(value for value in legacy_csv.split(",") if value)]
semver_pattern = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def fail(message: str) -> None:
    print(f"::error::{message}", file=sys.stderr)
    raise SystemExit(1)


def parse_version(value: str) -> tuple[int, int, int]:
    if not semver_pattern.fullmatch(value):
        fail(f"Version must be MAJOR.MINOR.PATCH, got: {value}")
    return tuple(int(part) for part in value.split("."))


def component_version(tag_name: str) -> tuple[str, tuple[int, int, int]] | None:
    for prefix in prefixes:
        if tag_name.startswith(prefix):
            value = tag_name[len(prefix):]
            if semver_pattern.fullmatch(value):
                return prefix, parse_version(value)
    return None


allocations: dict[tuple[int, int, int], list[dict[str, object]]] = {}
git_tags = subprocess.check_output(["git", "tag", "-l"], text=True).splitlines()
for tag_name in git_tags:
    parsed = component_version(tag_name)
    if not parsed:
        continue
    prefix, version_tuple = parsed
    tag_type = subprocess.check_output(
        ["git", "cat-file", "-t", f"refs/tags/{tag_name}"], text=True
    ).strip()
    target = subprocess.check_output(
        ["git", "rev-list", "-n", "1", tag_name], text=True
    ).strip()
    allocations.setdefault(version_tuple, []).append(
        {
            "kind": "tag",
            "tag": tag_name,
            "prefix": prefix,
            "target": target,
            "annotated": tag_type == "tag",
        }
    )

records: list[dict[str, object]] = []
if release_records_path:
    loaded = json.loads(Path(release_records_path).read_text())
    if not isinstance(loaded, list):
        fail("Release records must be a JSON array")
    records = [record for record in loaded if isinstance(record, dict)]

for record in records:
    tag_name = str(record.get("tag_name", ""))
    parsed = component_version(tag_name)
    if not parsed:
        continue
    prefix, version_tuple = parsed
    allocations.setdefault(version_tuple, []).append(
        {
            "kind": "release",
            "tag": tag_name,
            "prefix": prefix,
            "target": str(record.get("target_commitish", "")),
            "draft": record.get("draft") is True,
        }
    )

candidate = parse_version(package_or_tag_version)

if event_name != "push" and not requested_version:
    reusable = []
    newest = max(allocations) if allocations else None
    for allocated_version, entries in allocations.items():
        has_tag = any(entry["kind"] == "tag" for entry in entries)
        same_sha_draft = any(
            entry["kind"] == "release"
            and entry.get("draft") is True
            and entry["prefix"] == tag_prefix
            and entry["target"] == release_sha
            for entry in entries
        )
        if allocated_version == newest and same_sha_draft and not has_tag:
            reusable.append(allocated_version)
    if reusable:
        candidate = max(reusable)
    elif newest is not None:
        package_version = parse_version(package_or_tag_version)
        candidate = (
            package_version
            if package_version > newest
            else (newest[0], newest[1], newest[2] + 1)
        )

version = ".".join(str(part) for part in candidate)
tag = f"{tag_prefix}{version}"
entries = allocations.get(candidate, [])
current_tags = [
    entry
    for entry in entries
    if entry["kind"] == "tag" and entry["tag"] == tag
]
other_allocations = [entry for entry in entries if entry.get("tag") != tag]

public_allocations = []
for allocated_version, allocated_entries in allocations.items():
    for entry in allocated_entries:
        is_public_tag = entry["kind"] == "tag" and entry.get("annotated") is True
        is_public_release = (
            entry["kind"] == "release" and entry.get("draft") is False
        )
        if is_public_tag or is_public_release:
            public_allocations.append((allocated_version, str(entry["tag"])))
if public_allocations:
    newest_public_version = max(version for version, _ in public_allocations)
    if candidate < newest_public_version:
        newest_public_tag = next(
            tag_name
            for version_value, tag_name in public_allocations
            if version_value == newest_public_version
        )
        newest_public = ".".join(str(part) for part in newest_public_version)
        fail(
            f"Release version {version} is older than newest public "
            f"{component_name} version {newest_public} ({newest_public_tag})."
        )

if other_allocations:
    duplicate = str(other_allocations[0]["tag"])
    fail(f"Release version {version} already exists as tag {duplicate}.")

reservation = "create"
if current_tags:
    current = current_tags[0]
    if current["target"] != release_sha:
        fail(f"{tag} is already allocated to a different source")
    if current.get("annotated") is not True:
        fail(f"Tag {tag} must be an annotated tag.")
    reservation = "tag"
else:
    current_releases = [
        entry
        for entry in entries
        if entry["kind"] == "release" and entry["tag"] == tag
    ]
    if current_releases:
        reusable = [
            entry
            for entry in current_releases
            if entry.get("draft") is True and entry["target"] == release_sha
        ]
        if reusable:
            reservation = "reuse"
        else:
            fail(f"{tag} is already allocated to a different source")

if event_name == "push" and reservation != "tag":
    fail(f"Tag does not exist: {tag}")

previous = []
for allocated_version, allocated_entries in allocations.items():
    if allocated_version >= candidate:
        continue
    for entry in allocated_entries:
        if entry["kind"] == "tag":
            previous.append((allocated_version, str(entry["tag"])))
if previous:
    previous_tag = max(previous, key=lambda item: item[0])[1]
else:
    previous_tag = ""

print(f"version={version}")
print(f"tag={tag}")
print(f"previous_tag={previous_tag}")
print(f"reservation={reservation}")
PY
)"

version=""
tag=""
previous_tag=""
reservation=""
while IFS='=' read -r key value; do
  case "$key" in
    version) version="$value" ;;
    tag) tag="$value" ;;
    previous_tag) previous_tag="$value" ;;
    reservation) reservation="$value" ;;
  esac
done <<< "$resolution"

if [ -z "$version" ] || [ -z "$tag" ] || [ -z "$reservation" ]; then
  echo "::error::Release resolver returned incomplete output"
  exit 1
fi

emit version "$version"
emit tag "$tag"
emit release_sha "$release_sha"
emit previous_tag "$previous_tag"
emit reservation "$reservation"

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  cat >> "$GITHUB_STEP_SUMMARY" <<EOF
$release_name release:
- Version: $version
- Tag: $tag
- Release commit: $release_sha
- Reservation: $reservation
EOF
fi
