#!/usr/bin/env python3
"""Tests for published and prepared bundled extension staging."""

import hashlib
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bos_build.core.context import Context
from bos_build.core.products import (
    BROWSEROS_AGENT_EXTENSION_ID,
    BROWSEROS_BUG_REPORTER_EXTENSION_ID,
    BROWSERCLAW_EXTENSION_ID,
    get_product_descriptor,
)
from bos_build.release.prepared_resources import (
    PreparedFile,
    PreparedResourcesManifest,
)
from bos_build.steps.extensions.bundled_extensions import (
    BundledExtensionsModule,
    ExtensionInfo,
)


MODULE = "bos_build.steps.extensions.bundled_extensions"
SOURCE_SHA = "2" * 40


class BundledExtensionsTest(unittest.TestCase):
    def test_manifest_url_defaults_to_live_bundled_manifest(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(
                self._ctx("browserclaw").get_extensions_manifest_url(),
                "https://cdn.browseros.com/extensions/bundled-manifest.xml",
            )

    def test_bundled_manifest_staging_file_is_coherent(self) -> None:
        repo_root = Path(__file__).resolve().parents[5]
        path = repo_root / "updates/extensions/bundled-manifest.xml"
        extensions = BundledExtensionsModule()._parse_manifest_xml(path.read_text())
        by_id = {extension.id: extension for extension in extensions}
        required = {
            extension_id: name
            for product in ("browseros", "browserclaw")
            for extension_id, name in get_product_descriptor(
                product
            ).required_extension_ids
        }
        for extension_id, name in required.items():
            self.assertIn(extension_id, by_id, name)
        for extension in extensions:
            self.assertRegex(
                extension.codebase,
                r"^https://cdn\.browseros\.com/extensions/[a-z]+-[0-9.]+\.crx$",
            )
            self.assertIn(f"-{extension.version}.crx", extension.codebase)

    def test_published_selection_is_product_exact(self) -> None:
        expected = {
            "browseros": {
                BROWSEROS_AGENT_EXTENSION_ID,
                BROWSEROS_BUG_REPORTER_EXTENSION_ID,
            },
            "browserclaw": {
                BROWSERCLAW_EXTENSION_ID,
                BROWSEROS_BUG_REPORTER_EXTENSION_ID,
            },
        }
        for product, extension_ids in expected.items():
            selected = BundledExtensionsModule()._select_product_extensions(
                self._all_extensions(), self._ctx(product)
            )
            self.assertEqual({extension.id for extension in selected}, extension_ids)

    def test_published_selection_pins_product_extension_version(self) -> None:
        cases = (
            (
                "browseros",
                BROWSEROS_AGENT_EXTENSION_ID,
                "0.0.125.0",
                "https://cdn.browseros.com/extensions/agent-0.0.125.0.crx",
            ),
            (
                "browserclaw",
                BROWSERCLAW_EXTENSION_ID,
                "0.2.2.0",
                "https://cdn.browseros.com/extensions/browserclaw-0.2.2.0.crx",
            ),
        )
        for product, product_id, version, url in cases:
            extensions = [
                extension
                for extension in self._all_extensions()
                if extension.id != product_id
            ]
            with (
                self.subTest(product=product),
                patch.dict(
                    os.environ,
                    {"BUNDLED_PRODUCT_EXTENSION_VERSION": version},
                    clear=True,
                ),
            ):
                selected = BundledExtensionsModule()._select_product_extensions(
                    extensions, self._ctx(product)
                )
                by_id = {extension.id: extension for extension in selected}
                self.assertEqual(by_id[product_id].version, version)
                self.assertEqual(by_id[product_id].codebase, url)
                self.assertEqual(
                    by_id[BROWSEROS_BUG_REPORTER_EXTENSION_ID].version,
                    "52.0.0.0",
                )

    def test_published_manifest_can_be_read_from_exact_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "bundled-manifest.xml"
            path.write_text(
                """<?xml version="1.0"?>
<gupdate xmlns="http://www.google.com/update2/response" protocol="2.0">
  <app appid="bflpfmnmnokmjhmgnolecpppdbdophmk">
    <updatecheck codebase="https://cdn.browseros.com/extensions/agent-0.0.125.0.crx" version="0.0.125.0" />
  </app>
</gupdate>
""",
                encoding="utf-8",
            )

            extensions = BundledExtensionsModule()._fetch_and_parse_manifest(str(path))

        self.assertEqual(
            extensions,
            [
                ExtensionInfo(
                    BROWSEROS_AGENT_EXTENSION_ID,
                    "0.0.125.0",
                    "https://cdn.browseros.com/extensions/agent-0.0.125.0.crx",
                )
            ],
        )

    def test_published_mode_downloads_selected_manifest_entries(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            chromium = Path(temp)
            module = BundledExtensionsModule()

            def write(extension: ExtensionInfo, output: Path) -> None:
                (output / f"{extension.id}.crx").write_bytes(extension.id.encode())

            with (
                patch.object(
                    module,
                    "_fetch_and_parse_manifest",
                    return_value=self._all_extensions(),
                ) as fetch,
                patch.object(module, "_download_extension", side_effect=write),
            ):
                module.execute(self._ctx("browseros", chromium_src=chromium))

            fetch.assert_called_once()
            output = module._get_output_dir(
                self._ctx("browseros", chromium_src=chromium)
            )
            self.assertEqual(
                {path.stem for path in output.glob("*.crx")},
                {BROWSEROS_AGENT_EXTENSION_ID, BROWSEROS_BUG_REPORTER_EXTENSION_ID},
            )

    def test_source_mode_copies_only_validated_common_crxs(self) -> None:
        for product, product_id, product_version in (
            ("browseros", BROWSEROS_AGENT_EXTENSION_ID, "0.0.116.0"),
            ("browserclaw", BROWSERCLAW_EXTENSION_ID, "0.0.2.0"),
        ):
            with self.subTest(product=product), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                prepared = root / "prepared"
                chromium = root / "chromium"
                chromium.mkdir()
                manifest = self._prepared_manifest(
                    prepared, product, product_id, product_version
                )
                ctx = self._ctx(
                    product,
                    chromium_src=chromium,
                    resource_mode="source",
                    prepared_resources=prepared,
                )
                module = BundledExtensionsModule()
                with (
                    patch(
                        f"{MODULE}.validated_common_resources", return_value=manifest
                    ),
                    patch.object(module, "_fetch_and_parse_manifest") as fetch,
                    patch.object(module, "_download_extension") as download,
                ):
                    module.execute(ctx)

                fetch.assert_not_called()
                download.assert_not_called()
                output = module._get_output_dir(ctx)
                self.assertEqual(
                    (output / f"{product_id}.crx").read_bytes(), b"product-crx"
                )
                self.assertEqual(
                    (
                        output / f"{BROWSEROS_BUG_REPORTER_EXTENSION_ID}.crx"
                    ).read_bytes(),
                    b"bug-crx",
                )
                generated = json.loads((output / "bundled_extensions.json").read_text())
                self.assertEqual(
                    generated[product_id]["external_version"], product_version
                )
                self.assertEqual(
                    ctx.artifact_registry.get("common_manifest_digest"),
                    manifest.digest(),
                )

    def test_source_mode_propagates_validation_failures_before_staging(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            chromium = root / "chromium"
            chromium.mkdir()
            ctx = self._ctx(
                "browseros",
                chromium_src=chromium,
                resource_mode="source",
                prepared_resources=root / "prepared",
            )
            with patch(
                f"{MODULE}.validated_common_resources",
                side_effect=ValueError("Prepared-resource checksum mismatch"),
            ):
                with self.assertRaisesRegex(ValueError, "checksum"):
                    BundledExtensionsModule().execute(ctx)

            output = BundledExtensionsModule()._get_output_dir(ctx)
            self.assertFalse(output.exists())

    def test_switching_product_clears_stale_crx_and_json_entries(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            chromium = root / "chromium"
            chromium.mkdir()
            prepared = root / "prepared"
            module = BundledExtensionsModule()
            manifests = {
                "browseros": self._prepared_manifest(
                    prepared, "browseros", BROWSEROS_AGENT_EXTENSION_ID, "0.0.116.0"
                ),
                "browserclaw": self._prepared_manifest(
                    prepared,
                    "browserclaw",
                    BROWSERCLAW_EXTENSION_ID,
                    "0.0.2.0",
                ),
            }
            contexts = [
                self._ctx(
                    product,
                    chromium_src=chromium,
                    resource_mode="source",
                    prepared_resources=prepared,
                )
                for product in ("browseros", "browserclaw")
            ]
            with patch(
                f"{MODULE}.validated_common_resources",
                side_effect=[manifests["browseros"], manifests["browserclaw"]],
            ):
                for ctx in contexts:
                    module.execute(ctx)

            output = module._get_output_dir(contexts[-1])
            self.assertEqual(
                {path.stem for path in output.glob("*.crx")},
                {BROWSERCLAW_EXTENSION_ID, BROWSEROS_BUG_REPORTER_EXTENSION_ID},
            )
            self.assertNotIn(
                BROWSEROS_AGENT_EXTENSION_ID,
                json.loads((output / "bundled_extensions.json").read_text()),
            )

    def _prepared_manifest(
        self,
        root: Path,
        product: str,
        product_id: str,
        product_version: str,
    ) -> PreparedResourcesManifest:
        extensions = root / product
        extensions.mkdir(parents=True, exist_ok=True)
        product_path = extensions / "product.crx"
        bug_path = extensions / "bug.crx"
        product_path.write_bytes(b"product-crx")
        bug_path.write_bytes(b"bug-crx")

        def prepared(path: Path, version: str, extension_id: str) -> PreparedFile:
            return PreparedFile(
                path=path.relative_to(root).as_posix(),
                size=path.stat().st_size,
                sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                version=version,
                extension_id=extension_id,
            )

        component = "agent" if product == "browseros" else "browserclaw"
        server = "server" if product == "browseros" else "claw-server-rust"
        onboarding = "app-onboard" if product == "browseros" else "claw-onboard"
        return PreparedResourcesManifest(
            product=product,
            parent_sha="1" * 40,
            source_sha=SOURCE_SHA,
            browser_version="0.0.1",
            component_versions={
                server: "0.0.1",
                component: product_version,
                onboarding: "0.0.1",
            },
            files={
                "product_crx": prepared(product_path, product_version, product_id),
                "bug_reporter_crx": prepared(
                    bug_path, "52.0.0.0", BROWSEROS_BUG_REPORTER_EXTENSION_ID
                ),
            },
        )

    def _ctx(
        self,
        product: str,
        *,
        chromium_src: Path = Path("/chromium"),
        resource_mode: str = "published",
        prepared_resources: Path | None = None,
    ) -> Context:
        return Context(
            root_dir=Path("/repo/packages/browseros"),
            chromium_src=chromium_src,
            architecture="arm64",
            build_type="release",
            chromium_version="1.0.0.0",
            browseros_build_offset="1",
            browseros_version_parts=(1, 0, 0, 0),
            browseros_chromium_version="1.0.0.1",
            semantic_version="0.0.1",
            product=get_product_descriptor(product),
            resource_mode=resource_mode,
            prepared_resources=prepared_resources,
            source_sha=SOURCE_SHA if resource_mode == "source" else "",
        )

    def _all_extensions(self) -> list[ExtensionInfo]:
        return [
            ExtensionInfo(
                BROWSEROS_AGENT_EXTENSION_ID,
                "0.0.115.0",
                "https://cdn.browseros.com/extensions/agent.crx",
            ),
            ExtensionInfo(
                BROWSEROS_BUG_REPORTER_EXTENSION_ID,
                "52.0.0.0",
                "https://cdn.browseros.com/extensions/bugreporter.crx",
            ),
            ExtensionInfo(
                BROWSERCLAW_EXTENSION_ID,
                "0.0.1.0",
                "https://cdn.browseros.com/extensions/browserclaw.crx",
            ),
        ]


if __name__ == "__main__":
    unittest.main()
