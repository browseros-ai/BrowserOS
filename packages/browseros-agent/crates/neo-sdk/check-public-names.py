#!/usr/bin/env python3
"""Reject private host identifiers in tracked paths and working source bytes.

The remote SDK gate receives a source snapshot without Git metadata. Only that builder's
explicit source root enables snapshot scanning; ordinary checkouts always use Git's index.
"""

import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys


# Keep identifying spellings out of public source while still matching compound identifiers.
# These fragments are a readable policy, not encryption or a claim of secrecy.
PRIVATE_NAMES = (b"zu" + b"zu", b"mirror" + b"d", b"mem" + b"0")
PUBLIC_PRODUCT = PRIVATE_NAMES[2]

# Exact private script basenames use hashes so public source contains no identifying list.
# Tokenize the whole basename: a public file with a different prefix is a different name.
FILE_NAME = re.compile(rb"[a-z0-9_-]+\.(?:js|go)\b")
PRIVATE_FILES = {
    "0088c87ddc79981d1a430049da14fc206e467c31ba6f0ca3fd2669db331876e8",
    "06bb4df0f19afc6e5b488c6cace5f217c3a6cd96179525f1f8cc6e9c6296f91a",
    "0fdc2247f84df5fdfd5ff6c172f86787ac1ab7b9f3875687df810a8c89729975",
    "15bcc7dd5735e7112514f6148b6d50eabe3413d8682cc6343b9b2b5c6cb5e9c1",
    "1fe10d1094163bba2a403883a2ce120c5430d8fbf90e1200ddd449d691d7798c",
    "3ec1bbf53d05c3da86e3dfe18294b88b9e97e80df308897ef4b6864238bba078",
    "44ee5bc3e530494217722c5f657c6f28cdea4c8481dce113f78e208e44ce08b2",
    "53dca64570a677d546165f98a169d0248d7b33790e0161de15d38ea07e1f85d3",
    "5407ae5da6842784fbb785c2020b1f287f1d264c51c629c1c5c6191a2a8bdedf",
    "5484d5ab9737d3c4a5f8941cebf656b5c1fb7fe6aed92a70be437ab99281d6f5",
    "559168c49021efa29f1b97cec41cbbc6260d3fd0f790c4f617e2fc7cd1dd938b",
    "5c437b812b14a51bd724a3a7fb7c7de54fbd28fcbe59fdf060fb4355fc60f600",
    "5ec146720016c0c0cfe72e590bfad4c15c6ebdf0039da158d2627c6034a268ea",
    "60d88a4dd067c9f62b9afe8a8133aa61cfcf9c4244a0b0a3c530abc5d3772f5e",
    "6bcd511eeb9af5e15a58840e35b2d6c5f23e858a761dd3c10716ee56db2cce88",
    "82d479e2e9cae9e5b85d9d16eb41e4609db0bc5e21648c1c70b5ecfb145479e4",
    "890dcfab0b0a973c88caa67915df7e53217dee72d6134e64a215fe0d5a55e760",
    "92dc251c61b821c9da58ad729066afc42bf9313ccee0acfa82d2604ef224d917",
    "977b1df65d642b42c727c3e123deec315e8cfa456d09e469e8d0f5f58cf7b3fb",
    "a5daa99a5e061978b2ffbe72b9c3a92991c982cf5309dc8568ae0e9034bc415d",
    "b4fbe701ee577f649804bf09270850a90630175e035e08d62e96fbc78063998d",
    "bc873c8a78a800ed8c63981a2cec9a2441973c724903b6d1e16e28e40a133c08",
    "cb04d39484e3178d64b3596b995253f37c6c5ab857f91036c246dda17d2a71a8",
    "dce01c8e0351db42944146cd3eb18d18a272a6d93b0c13dd5028caa816367e53",
    "dfa29941cde48e7571e7ba2042bdd34ac3b2b8a676856b4e1875d710d3802d13",
}

# Only these reviewed product lines are exempt, at their exact paths. A new host reference
# in the same file must still fail, and edits to a product line require reviewing its digest.
PRODUCT_LINES = {
    "docs/browseros/features/connect-mcps.mdx": {
        "7f128c3fd2e5dbdf1f16ca6255f8831337aeef66bf6314db4944f03f3bc0a0a5",
    },
    "docs/browseros/features/use-with-claude-code.mdx": {
        "7f128c3fd2e5dbdf1f16ca6255f8831337aeef66bf6314db4944f03f3bc0a0a5",
    },
    "packages/browseros-agent/apps/app/components/mcp/McpServerIcon.tsx": {
        "c40f28a88015e98901e8449b1a49d28f16b22555a9fda284f3a1100e42c036ac",
        "2b779f7e30e2224e7c0783aadbe4923049678cc4c2be778b3e3d658ff7c063ed",
    },
    "packages/browseros-agent/apps/server/src/api/services/klavis/catalog.ts": {
        "cdff02cdd86eecc7ecd10292d57267e3e69099579f2c38b7827ac7f14bacc831",
    },
}


def source_paths(root):
    """Select the index in a checkout, or the remote builder's bounded source bundle."""
    if (root / ".git").exists():
        top = subprocess.check_output(["git", "-C", str(root), "rev-parse", "--show-toplevel"])
        if Path(os.fsdecode(top.rstrip(b"\n"))).resolve() != root:
            raise ValueError("expected the repository root")
        names = subprocess.check_output(["git", "-C", str(root), "ls-files", "--cached", "-z"])
        return sorted({os.fsdecode(name) for name in names.split(b"\0") if name}), "tracked"

    snapshot = os.environ.get("NEO_SDK_SOURCE_ROOT")
    if not snapshot or Path(snapshot).resolve() != root:
        raise ValueError("Git metadata missing outside an explicit remote source snapshot")
    agent = root / "packages/browseros-agent"
    workflow = root / ".github/workflows/test.yml"
    if not agent.is_dir() or not workflow.is_file():
        raise ValueError("incomplete remote source snapshot")
    paths = [path for path in agent.rglob("*") if path.is_symlink() or path.is_file()]
    paths.append(workflow)
    return sorted(path.relative_to(root).as_posix() for path in paths), "snapshot"


def private_filename(text):
    """Match a whole basename, keeping unrelated public files with other prefixes distinct."""
    return any(
        hashlib.sha256(match[0]).hexdigest() in PRIVATE_FILES
        for match in FILE_NAME.finditer(text)
    )


def main(root):
    paths, mode = source_paths(root)
    failed = False
    for relative in paths:
        path = root / relative
        lower_path = os.fsencode(relative).lower()
        if any(name in lower_path for name in PRIVATE_NAMES) or private_filename(lower_path):
            print(f"{relative!r}: private host reference in filename", file=sys.stderr)
            failed = True
        # Read a link's text, never a target outside the source view. Missing working files
        # are deletions; all other read failures must fail the check rather than hide a hit.
        try:
            path.lstat()
        except FileNotFoundError:
            continue
        if not path.parent.resolve().is_relative_to(root):
            raise ValueError(f"source path escapes repository: {relative!r}")
        if path.is_symlink():
            data = os.fsencode(os.readlink(path))
        elif path.is_dir():
            continue  # A submodule owns a separate index.
        else:
            data = path.read_bytes()
        for number, line in enumerate(data.lower().split(b"\n"), 1):
            if private_filename(line):
                print(f"{relative!r}:{number}: private host filename reference", file=sys.stderr)
                failed = True
            for name in PRIVATE_NAMES:
                if name not in line:
                    continue
                digest = hashlib.sha256(line.rstrip(b"\r")).hexdigest()
                if name == PUBLIC_PRODUCT and digest in PRODUCT_LINES.get(relative, set()):
                    continue
                print(f"{relative!r}:{number}: private host reference", file=sys.stderr)
                failed = True
    if failed:
        return 1
    print(f"Public naming check passed ({mode} source).")
    return 0


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2:
            raise ValueError("usage: check-public-names.py <repository-root>")
        sys.exit(main(Path(sys.argv[1]).resolve()))
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Public naming check failed: {error}", file=sys.stderr)
        sys.exit(2)
