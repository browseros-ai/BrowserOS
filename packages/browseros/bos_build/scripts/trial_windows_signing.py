"""Sign source-bound server copies without allocating versions or publishing.

The trial reads existing immutable resource objects and produces only local/CI
artifacts. It deliberately has no feed, upload, or release-finalization path.
"""

import argparse
import json
import os
import re
import shutil
import tempfile
from pathlib import Path

from bos_build.lib.env import EnvConfig
from bos_build.lib.r2 import download_file_from_r2, get_r2_client
from bos_build.lib.windows_signing import file_digest, sign_windows_files
from bos_build.lib.windows_signing.config import validate_signing
from bos_build.products.server_binaries import (
    expected_windows_binary_paths,
    server_ota_bundles_for_product,
)
from bos_build.release.resource_pins import (
    COMPONENT_RESOURCE_FAMILY,
    verify_prepared_resource_pin,
)
from bos_build.steps.storage.download import extract_artifact_zip


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--product", choices=("browseros", "browserclaw"), required=True
    )
    parser.add_argument(
        "--version",
        default="",
        help="Exact resource version; otherwise snapshot latest once",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    env = EnvConfig()
    validate_signing(env.windows_signing_provider, env)
    client = get_r2_client(env)
    if client is None:
        raise RuntimeError("R2 read access is required for a source-bound trial")
    bundle = server_ota_bundles_for_product(args.product)[0]
    key = bundle.unsigned_artifact_key("windows-x64", version=args.version or None)
    metadata = client.head_object(Bucket=env.r2_bucket, Key=key).get("Metadata", {})
    version = args.version or metadata.get("version", "")
    source_sha = metadata.get("release-sha", "")
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise RuntimeError("Trial resources must have a full source SHA binding")
    pin = verify_prepared_resource_pin(
        client, env.r2_bucket, COMPONENT_RESOURCE_FAMILY[bundle.id], version, source_sha
    )
    resource = next(item for item in pin.objects if item.target == "windows-x64")
    if not resource.sha256:
        raise RuntimeError("Trial resources must have a SHA-256 binding")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    os.environ["RELEASE_SHA"] = source_sha
    os.environ["PRODUCT"] = args.product
    os.environ["VERSION"] = version
    os.environ["WINDOWS_SIGNING_REPORT_DIR"] = str(output / "reports")
    with tempfile.TemporaryDirectory(prefix="server-signing-trial-") as name:
        stage = Path(name)
        archive = stage / "resources.zip"
        if not download_file_from_r2(
            client, resource.key, archive, env.r2_bucket, expected_etag=resource.etag
        ):
            raise RuntimeError("Could not download immutable server resources")
        if (
            archive.stat().st_size != resource.size
            or file_digest(archive) != resource.sha256
        ):
            raise RuntimeError("Server resource checksum/size mismatch")
        extract_artifact_zip(archive, stage / "extracted")
        resources = stage / "extracted" / "resources"
        files = expected_windows_binary_paths(resources / "bin", bundle)
        if not sign_windows_files(files, env, env.windows_signing_provider):
            raise RuntimeError("Server signing trial failed")
        shutil.make_archive(
            str(output / f"{args.product}-{version}-windows-signing-trial"),
            "zip",
            resources.parent,
            resources.name,
        )
    (output / "source.json").write_text(
        json.dumps(
            {
                "product": args.product,
                "version": version,
                "resource_source_sha": source_sha,
                "implementation_sha": os.environ.get("GITHUB_SHA", ""),
                "resource_pin": pin.to_dict(),
                "publication": "private-ci-artifact-only",
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
