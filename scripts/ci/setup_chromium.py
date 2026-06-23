#!/usr/bin/env python3
"""Provision the pinned Chromium checkout for nightly CI builds.

Replaces the build CLI's git_setup module on ephemeral runners: git_setup
assumes a full clone where `git fetch --tags` is incremental, but on the
shallow CI checkout fetching all of Chromium's ~70k tags would effectively
unshallow the repo. Instead this script fetches exactly the pinned tag
(depth 2, no tag-following) and lets `gclient sync` do the rest.

Steps (run as separate invocations so the build CLI's `clean` module can
run between them — clean deletes hook-managed toolchains like
third_party/llvm-build, which sync then restores):
  checkout  ensure depot_tools + src at the pinned tag (no-op on warm cache)
  sync      gclient sync -D --no-history --shallow
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

CHROMIUM_SRC_URL = "https://chromium.googlesource.com/chromium/src.git"
DEPOT_TOOLS_URL = (
    "https://chromium.googlesource.com/chromium/tools/depot_tools.git"
)

GCLIENT_SPEC = """solutions = [
  {
    "name": "src",
    "url": "%s",
    "deps_file": "DEPS",
    "managed": False,
    "custom_deps": {},
    "custom_vars": {},
  },
]
""" % CHROMIUM_SRC_URL


def log(msg: str) -> None:
    print(f"[setup_chromium] {msg}", flush=True)


def run(cmd: list[str], cwd: Path, env: dict | None = None) -> None:
    log(f"$ {' '.join(cmd)}  (cwd={cwd})")
    subprocess.run(cmd, cwd=cwd, env=env, check=True)


def read_pinned_version(version_file: Path) -> str:
    parts = {}
    for line in version_file.read_text().strip().splitlines():
        key, value = line.split("=")
        parts[key.strip()] = value.strip()
    return f"{parts['MAJOR']}.{parts['MINOR']}.{parts['BUILD']}.{parts['PATCH']}"


def append_github_file(env_var: str, line: str) -> None:
    path = os.environ.get(env_var)
    if not path:
        return
    with open(path, "a") as f:
        f.write(line + "\n")


def ensure_depot_tools(root: Path) -> Path:
    depot_tools = root / "depot_tools"
    if not (depot_tools / ".git").exists():
        log("Cloning depot_tools...")
        run(
            ["git", "clone", "--depth", "1", DEPOT_TOOLS_URL, str(depot_tools)],
            cwd=root,
        )
    else:
        log("depot_tools already present")

    append_github_file("GITHUB_PATH", str(depot_tools))
    if sys.platform == "win32":
        append_github_file("GITHUB_ENV", "DEPOT_TOOLS_WIN_TOOLCHAIN=0")
    return depot_tools


def ensure_gclient_config(root: Path) -> None:
    gclient_file = root / ".gclient"
    if gclient_file.exists() and gclient_file.read_text() == GCLIENT_SPEC:
        return
    log(f"Writing {gclient_file}")
    gclient_file.write_text(GCLIENT_SPEC)


def git_output(args: list[str], cwd: Path) -> str:
    result = subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True
    )
    return result.stdout.strip() if result.returncode == 0 else ""


def ensure_src_at_tag(root: Path, version: str) -> Path:
    src = root / "src"
    tag_ref = f"refs/tags/{version}"

    if not (src / ".git").exists():
        log(f"Initializing fresh src checkout at {src}")
        src.mkdir(parents=True, exist_ok=True)
        run(["git", "init"], cwd=src)
        run(["git", "remote", "add", "origin", CHROMIUM_SRC_URL], cwd=src)
        if sys.platform == "win32":
            run(["git", "config", "core.longpaths", "true"], cwd=src)

    if not git_output(["rev-parse", "--verify", "--quiet", f"{tag_ref}^{{commit}}"], cwd=src):
        # depth 2 so positioning tools (git describe / lastchange) have a parent
        log(f"Fetching pinned tag {version} (shallow)...")
        run(
            [
                "git", "fetch", "--depth", "2", "--no-tags", "origin",
                f"+{tag_ref}:{tag_ref}",
            ],
            cwd=src,
        )
    else:
        log(f"Tag {version} already present")

    head = git_output(["rev-parse", "HEAD"], cwd=src)
    tag_commit = git_output(["rev-parse", f"{tag_ref}^{{commit}}"], cwd=src)
    if head != tag_commit:
        log(f"Checking out {version}...")
        run(["git", "checkout", "--force", "--detach", tag_ref], cwd=src)
    else:
        log(f"HEAD already at {version}")

    return src


def gclient_sync(root: Path, depot_tools: Path) -> None:
    env = os.environ.copy()
    env["PATH"] = str(depot_tools) + os.pathsep + env.get("PATH", "")
    env["DEPOT_TOOLS_WIN_TOOLCHAIN"] = "0"

    gclient = depot_tools / ("gclient.bat" if sys.platform == "win32" else "gclient")
    run(
        [str(gclient), "sync", "-D", "--no-history", "--shallow"],
        cwd=root / "src",
        env=env,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--chromium-root",
        required=True,
        type=Path,
        help="gclient root: holds depot_tools/, .gclient and src/",
    )
    parser.add_argument("--step", required=True, choices=["checkout", "sync"])
    parser.add_argument(
        "--version-file",
        type=Path,
        default=Path(__file__).resolve().parents[2]
        / "packages/browseros/CHROMIUM_VERSION",
        help="CHROMIUM_VERSION pin file (MAJOR=/MINOR=/BUILD=/PATCH= lines)",
    )
    args = parser.parse_args()

    root = args.chromium_root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    version = read_pinned_version(args.version_file)
    log(f"Pinned Chromium version: {version}")
    log(f"Chromium root: {root}")

    depot_tools = ensure_depot_tools(root)
    ensure_gclient_config(root)

    if args.step == "checkout":
        src = ensure_src_at_tag(root, version)
        log(f"Checkout ready: {src}")
    else:
        gclient_sync(root, depot_tools)
        log("gclient sync complete")


if __name__ == "__main__":
    main()
